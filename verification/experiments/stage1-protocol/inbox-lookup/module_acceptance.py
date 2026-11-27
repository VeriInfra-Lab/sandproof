"""Select and run the local Inbox Lookup obligation without a full Stage1 seal."""

import argparse
from collections import defaultdict
import fcntl
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[4]
STAGE1 = Path('verification/experiments/stage1-protocol')
HERE = STAGE1 / 'inbox-lookup'
MODULE = 'inbox-lookup'
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
ISOLATED_INPUTS = {
    str(HERE / 'module_acceptance.py'),
    str(HERE / 'InboxLookupWrong.lean.in'),
    'tools/tests/test_inbox_lookup_module_acceptance.py',
}
VARIANTS = {
    'correct': ('candidate', 'InboxLookup', 'InboxLookup.find_contract'),
    'key-only': ('key-only', 'InboxLookupWrong', 'InboxLookup.Wrong.rejects_same_full_contract'),
}
THEOREM_TYPE = 'SandboxModel.Stage1Protocol.InboxLookup.FullLookupContract '
LOOKUP = 'SandproofStage1RequestRust.sandproof_controller_service.controller.lifecycle_inbox.model.Inbox.find'

sys.path.insert(0, str(ROOT))
from tools.acceptance_receipts import native_test_configuration, sha, write_json


def read(path):
    return json.loads(path.read_text())


def git(*args, root=ROOT):
    return subprocess.check_output(['git', *args], cwd=root)


def clean_candidate(root=ROOT):
    if git('status', '--porcelain', '-z', '--untracked-files=normal', root=root):
        raise ValueError('module acceptance requires a clean committed checkout')
    return {
        'commit': git('rev-parse', 'HEAD', root=root).decode().strip(),
        'tree': git('rev-parse', 'HEAD^{tree}', root=root).decode().strip(),
    }


def changed_files(base, candidate, root=ROOT):
    base = git('rev-parse', '--verify', '--end-of-options', base + '^{commit}', root=root).decode().strip()
    if subprocess.run(['git', 'merge-base', '--is-ancestor', base, candidate], cwd=root).returncode != 0:
        raise ValueError('the comparison base must be an ancestor of the candidate')
    paths = git('diff', '--name-only', '--no-renames', '-z', base, candidate, '--', root=root)
    return base, [os.fsdecode(path) for path in paths.split(b'\0') if path]


def downstream(root, seed_templates):
    """Find concrete reverse imports/copy references for the review report.

    This scan is not an independence proof. Shared or unknown paths always
    require wider review, even if the scan finds no additional consumers.
    """
    stage = root / STAGE1
    owners = defaultdict(set)
    templates = sorted(stage.rglob('*.lean.in'))
    for path in templates:
        owner = path.relative_to(stage).parts[0]
        owners[path.name.removesuffix('.lean.in')].add(owner)
    reverse = defaultdict(set)
    for path in templates:
        owner = path.relative_to(stage).parts[0]
        imports = re.findall(r'^\s*(?:public )?import\s+(\S+)', path.read_text(), re.M)
        for name in imports:
            stem = name.rsplit('.', 1)[-1]
            providers = owners[stem]
            # Types/Funs are repeated for separate physical views; their names
            # alone cannot identify which physical provider a driver uses.
            if len(providers) != 1:
                continue
            provider, = providers
            if provider != owner and (provider != MODULE or stem in seed_templates):
                reverse[provider].add(owner)

    references = re.compile(r"['\"/]([a-z][a-z0-9]*(?:-[a-z0-9]+)+)(?=['\"/])")
    directories = {path.name for path in stage.iterdir() if path.is_dir()}
    for path in sorted(stage.rglob('*.py')):
        parts = path.relative_to(stage).parts
        if len(parts) < 2:
            continue
        owner = parts[0]
        text = path.read_text()
        for provider in (set(references.findall(text)) & directories) - {owner}:
            if provider == MODULE and seed_templates == {'InboxLookupWrong'} and 'InboxLookupWrong' not in text:
                continue
            reverse[provider].add(owner)
    direct = reverse[MODULE]
    affected = set(direct)
    pending = list(direct)
    while pending:
        for name in reverse[pending.pop()] - affected - {MODULE}:
            affected.add(name)
            pending.append(name)
    return sorted(direct), sorted(affected)


def select(paths, root=ROOT):
    inventory = read(root / HERE / 'inventory.json')
    paths = sorted(set(paths))
    scope = dict(changed_paths=paths, affected_modules=[], checks=[], direct_consumers=[])
    if not paths:
        return dict(scope, status='empty', reason='no changed inputs; no module was accepted')
    if set(paths) <= ISOLATED_INPUTS:
        # If even the isolated mutant template gets a new downstream reader,
        # stop treating it as isolated. Other inputs here are pilot tooling.
        changes_mutant = str(HERE / 'InboxLookupWrong.lean.in') in paths
        _, affected = downstream(root, {'InboxLookupWrong'}) if changes_mutant else ([], [])
        if not affected:
            return dict(scope, status='ready', affected_modules=[MODULE],
                        checks=['original-native', 'formal-correct', 'formal-key-only'],
                        production_entry=f'{MODEL}:Inbox::find / Session::find_entry',
                        rust_test_filter='sandproof-controller-service: proof_lookup::',
                        extraction_roots=str(HERE / 'formal.py') + ':ROOTS',
                        input_inventory=str(HERE / 'inventory.json'),
                        contract=THEOREM_TYPE.strip())
    shared_modules = set(inventory['production_inputs']) | {
        'Cargo.lock', 'Cargo.toml', 'lean-toolchain', 'rust-toolchain.toml', 'lakefile.toml',
        'tools/proof_batch.py', 'tools/proof_services.py', 'tools/acceptance_receipts.py',
        str(STAGE1 / 'formal_runtime.py'), str(STAGE1 / 'registered.py'),
        'verification/foundations-environment.json', 'verification/stage1-protocol-inputs.sha256',
    }
    shared = [path for path in paths if path in shared_modules or Path(path).is_relative_to(HERE)]
    unknown = sorted(set(paths) - set(shared) - ISOLATED_INPUTS)
    direct, affected = downstream(root, set(inventory['audits']))
    return dict(scope, status='review-required', affected_modules=[MODULE, *affected],
        direct_consumers=direct, shared_inputs=shared, unmapped_inputs=unknown,
        reason='use the registered drivers for the reviewed dependency set or run full Stage1 at promotion; no module pass was issued')


def run_driver(output, tool, *args):
    target = output / tool
    name = 'native.py' if tool == 'original-native' else 'formal.py'
    log = output / (tool + '.log')
    argv = [sys.executable, str(ROOT / HERE / name), '--output', str(target), *args]
    with log.open('xb') as stream:
        result = subprocess.run(argv, cwd=ROOT, stdin=subprocess.DEVNULL,
                                stdout=stream, stderr=subprocess.STDOUT)
    if result.returncode:
        raise ValueError(f'{tool} failed with exit {result.returncode}; see {log}')
    return read(target / 'result.json')


def admit_native(record, folder, commit):
    expected = {'original', 'candidate', 'key-only', 'restored'}
    if (record['kind'] != 'inbox-lookup-native-development' or record['complete'] is not True
            or record['source_commit'] != commit or set(record['variants']) != expected
            or any(row['complete'] is not True for row in record['variants'].values())):
        raise ValueError('missing current complete original Rust views')
    seen = set()
    for row in record['commands']:
        variant, label = key = (row['variant'], row['label'])
        mutant = key == ('key-only', 'test')
        if (variant not in expected or label not in ('test', 'clippy') or key in seen
                or row['exit_code'] != (101 if mutant else 0)
                or row.get('property_rejected', False) is not mutant
                or sha(folder / variant / (label + '.log')) != row['log_sha256']):
            raise ValueError('invalid original Rust command evidence: ' + repr(key))
        seen.add(key)
    if len(seen) != 2 * len(expected):
        raise ValueError('missing original Rust test or Clippy evidence')


def admit_formal(native, record, folder, variant, commit):
    # Apply the same original component source and registry admission as the
    # full driver, with the narrow original-native record created in this run.
    from native_admission import admit

    rust_variant, module, theorem = VARIANTS[variant]
    if (record['kind'] != 'inbox-lookup-formal-development' or record['complete'] is not True
            or record['source_commit'] != commit or record['variant'] != variant):
        raise ValueError('missing current complete original Lean view: ' + variant)
    binding = {}
    admit(binding, native, record['inputs'], record['registry'], ROOT, HERE / 'Cargo.lock')
    source_hash = record['candidate_model_sha256']
    source = folder / 'source'
    if (native['variants'][rust_variant]['inputs'][MODEL] != source_hash
            or sha(source / MODEL) != source_hash
            or native['variants']['original']['inputs'][MODEL] != native['variants']['restored']['inputs'][MODEL]):
        raise ValueError('Rust and Lean views do not bind the same lookup implementation')
    for name in ('cargo', 'rustc'):
        if native['tools'][name]['sha256'] != record['runtime']['tools'][name]['sha256']:
            raise ValueError('Rust and Lean driver tool differs: ' + name)
    full_theorem = 'SandboxModel.Stage1Protocol.' + theorem
    proof_page = f'SandboxModel/Lifecycle/Protocol/{module}.lean'
    actual = record['actual']
    if (full_theorem not in actual['audits'] or sha(source / proof_page) != actual['files'][proof_page]
            or sha(folder / 'physical-files/service.log') != actual['service_log_sha256']):
        raise ValueError('missing original full lookup theorem, physical proof, or service evidence')
    if set(record['llbc']) != {'original', 'candidate'}:
        raise ValueError('missing original or candidate LLBC')
    for ir in record['llbc'].values():
        if sha(source / ir['path']) != ir['sha256']:
            raise ValueError('LLBC evidence changed')
    # Audited declaration names alone do not give the theorem's Lean type.
    # Check the precise proposition even when the mutant proof template is edited.
    negation = '¬ ' if variant == 'key-only' else ''
    proposition = negation + THEOREM_TYPE + LOOKUP
    check = source / 'artifacts/ModuleAcceptance.lean'
    check.write_text(f'import SandboxModel.Lifecycle.Protocol.{module}\n'
                     f'#check ({full_theorem} : {proposition})\n')
    log = folder / 'contract-type.log'
    with log.open('xb') as stream:
        checked = subprocess.run(['lake', 'env', 'lean', str(check)], cwd=source,
                                 stdin=subprocess.DEVNULL, stdout=stream,
                                 stderr=subprocess.STDOUT, timeout=120)
    if checked.returncode != 0:
        raise ValueError('wrong full lookup theorem type; see ' + str(log))
    return dict(theorem=full_theorem, property_rejected=variant == 'key-only',
                rust_formal_inputs_match=binding['native_inputs_match'],
                registry_content_match=binding['registry_content_match'],
                candidate_model_sha256=source_hash, seconds=record['seconds'],
                contract_type_log_sha256=sha(log), contract_type_source_sha256=sha(check),
                result_sha256=sha(folder / 'result.json'))


def execute(output, plan, candidate, formal_lock):
    output = output.resolve()
    if output.is_relative_to(ROOT):
        raise ValueError('use a fresh private output and caches outside the source checkout')
    output.mkdir(mode=0o700)
    started = time.monotonic()
    report = dict(schema=1, kind='stage1-module-acceptance', module=MODULE, complete=False,
                  candidate=candidate, plan=plan, fresh_runs=True, full_stage1_acceptance=False,
                  host=platform.node(), native_test_configuration=native_test_configuration(os.environ),
                  input_inventory_sha256=sha(ROOT / HERE / 'inventory.json'), formal={})
    if hasattr(os, 'sched_getaffinity'):
        report['available_cpus'] = sorted(os.sched_getaffinity(0))
    try:
        inventory = read(ROOT / HERE / 'inventory.json')
        for name, digest in inventory['production_inputs'].items():
            if sha(ROOT / name) != digest:
                raise ValueError('reviewed Inbox Lookup input differs before running checks: ' + name)
        from formal import execution
        execution.tool_bind(ROOT)
        native = run_driver(output, 'original-native')
        native_dir = output / 'original-native'
        admit_native(native, native_dir, candidate['commit'])
        report['native'] = dict(seconds=native['seconds'], result_sha256=sha(native_dir / 'result.json'))
        waiting = time.monotonic()
        print('Original Rust views admitted; waiting for the existing formal lock.', flush=True)
        with formal_lock.open('rb') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            report['formal_lock_wait_seconds'] = time.monotonic() - waiting
            for variant in VARIANTS:
                phase = 'formal-' + variant
                result = run_driver(output, phase, '--variant', variant)
                report['formal'][variant] = admit_formal(native, result, output / phase, variant, candidate['commit'])
        if clean_candidate() != candidate or native_test_configuration(os.environ) != report['native_test_configuration']:
            raise ValueError('candidate or native test configuration changed during checking')
        report['complete'] = True
    except Exception as exc:
        report['error'] = str(exc)
    report['seconds'] = time.monotonic() - started
    write_json(output / 'module-result.json', report)
    print(json.dumps(report, ensure_ascii=False))
    return 0 if report['complete'] else 1


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', required=True, help='reviewed integration base commit')
    purpose = parser.add_mutually_exclusive_group(required=True)
    purpose.add_argument('--plan', action='store_true')
    purpose.add_argument('--output', type=Path, help='new private evidence directory')
    parser.add_argument('--formal-lock', type=Path, help='existing operator formal.lock for execution')
    args = parser.parse_args(argv)
    os.umask(0o077)
    try:
        candidate = clean_candidate()
        base, paths = changed_files(args.base, candidate['commit'])
        plan = dict(select(paths), base_commit=base)
        if args.plan or plan['status'] != 'ready':
            print(json.dumps(dict(plan, candidate=candidate), ensure_ascii=False))
            return 0 if plan['status'] == 'ready' else 2
        if args.formal_lock is None or args.formal_lock.is_symlink() or not args.formal_lock.is_file():
            raise ValueError('execution requires the existing regular operator --formal-lock')
        return execute(args.output, plan, candidate, args.formal_lock)
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        parser.exit(2, str(exc) + '\n')


if __name__ == '__main__':
    raise SystemExit(main())
