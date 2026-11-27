"""Select and run the local Inbox Lookup obligation without a full Stage1 seal."""

import argparse
from collections import defaultdict
import fcntl
import importlib.util
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
FREEZE = 'verification/stage1-protocol-inputs.sha256'
METADATA_FILES = {'inventory.json', 'frozen.json', 'candidate.json', 'package-inputs.json'}

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


def registration(root):
    spec = importlib.util.spec_from_file_location('lookup_registered_stage1', root / STAGE1 / 'registered.py')
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def refreshed_inputs(value, hashes):
    if isinstance(value, list):
        return [refreshed_inputs(row, hashes) for row in value]
    if isinstance(value, dict):
        return {key: (hashes[key] if key in hashes and isinstance(row, str)
                      and re.fullmatch('[0-9a-f]{64}', row) else refreshed_inputs(row, hashes))
                for key, row in value.items()}
    return value


def reviewed_refresh(root, base, name, hashes):
    """Accept only an exact source-digest synchronization, never a new rule."""
    if not (root / name).is_file():
        return False
    old = git('show', f'{base}:{name}', root=root)
    if name == FREEZE:
        revised = re.sub(r'^([0-9a-f]{64})(  )([^\n]+)$',
                         lambda m: hashes.get(m[3], m[1]) + m[2] + m[3], old.decode(), flags=re.M)
        return revised != old.decode() and revised == (root / name).read_text()
    before = json.loads(old)
    revised = refreshed_inputs(before, hashes)
    return revised != before and revised == read(root / name)


def lean_consumers(root, seeds):
    """Follow proof pages before grouping them into original registered jobs."""
    stage = root / STAGE1
    modules = {Path(name).name.removesuffix('.lean.in') for name in seeds}
    templates = {}
    for path in sorted(stage.rglob('*.lean.in')):
        lines = re.findall(r'^\s*(?:(?:public|private) )?import[ \t]+([^\r\n]+)', path.read_text(), re.M)
        templates[path] = {name.rsplit('.', 1)[-1] for line in lines for name in line.split()}
    seen = set()
    while True:
        reached = {path for path, imports in templates.items() if imports & modules} - seen
        if not reached:
            break
        seen.update(reached)
        modules.update(path.name.removesuffix('.lean.in') for path in reached)
    # Original drivers copy leaves into separate physical projects. Include
    # their explicit template names and any names in frozen per-view recipes.
    tokens = '|'.join(re.escape(name) for name in sorted(modules))
    mentions = re.compile(r'(?<![\w])(?:' + tokens + r')(?=\.lean|[\'\"])')
    for path in sorted(stage.rglob('*')):
        if (path.parent != stage and (path.suffix == '.py' or path.name in METADATA_FILES)
                and path != root / HERE / 'module_acceptance.py' and mentions.search(path.read_text())):
            seen.add(path)
    consumers = {path.relative_to(stage).parts[0] for path in seen}
    for directory in ('SandboxModel', 'verification/aeneas'):
        for path in (root / directory).rglob('*.lean'):
            imports = re.findall(r'^\s*(?:(?:public|private) )?import[ \t]+([^\r\n]+)', path.read_text(), re.M)
            if any(name.rsplit('.', 1)[-1] in modules for line in imports for name in line.split()):
                consumers.add(str(path.relative_to(root)))
    return consumers


def registered_impact(paths, root=ROOT, base=None):
    """Resolve one reviewed business Rust file or original lookup Lean pages
    to concrete registered jobs, keeping every original native prerequisite.
    """
    inventory = read(root / HERE / 'inventory.json')
    pages = {str(HERE / (name + '.lean.in')) for name in inventory['audits']}
    seeds = set(paths) & (pages | {MODEL})
    if not seeds:
        return None
    hashes = {name: sha(root / name) for name in seeds if (root / name).is_file()}
    if set(hashes) != seeds:
        return None
    pending = set(paths) - seeds - ISOLATED_INPUTS - {FREEZE}
    if any(base is None or not Path(name).is_relative_to(STAGE1) or Path(name).name not in METADATA_FILES
           for name in pending):
        return None
    while pending:
        refreshed = {name for name in pending if reviewed_refresh(root, base, name, hashes)}
        if not refreshed:
            return None
        hashes.update({name: sha(root / name) for name in refreshed})
        pending -= refreshed
    if FREEZE in paths and (base is None or not reviewed_refresh(root, base, FREEZE, hashes)):
        return None

    stage = root / STAGE1
    actual_sources = []
    affected = {MODULE, *lean_consumers(root, seeds & pages)} if MODEL not in seeds else {
        MODULE, *downstream(root, set(inventory['audits']))[1]}
    if MODEL in seeds:
        # A file is the current registered Rust freeze unit. This file also
        # implements recording, validation, and retention, not lookup alone.
        for path in sorted(stage.rglob('*.json')):
            if path.name not in METADATA_FILES:
                continue
            value = read(path)
            if value != refreshed_inputs(value, {MODEL: ''}):
                relative = path.relative_to(stage)
                actual_sources.append(str(path.relative_to(root)))
                affected.add(relative.parts[0] if len(relative.parts) > 1 else 'stage1-protocol')

    registry = registration(root)
    aliases = {'inbox-append': 'inbox-record-total', 'source-live': 'source-live',
               'unload-parent-link': registry.UNLOAD.PARENT_LABEL,
               **dict.fromkeys(registry.UNLOAD.PACKAGES, registry.UNLOAD.LABEL)}
    jobs_by_module = defaultdict(list)
    for job in registry.formal_plan(Path('/unused'), root=root):
        directory = Path(job['driver']).parent.name
        # Root-level and external drivers serve independent roles. Map such
        # consumers by their registered names, not the common driver folder.
        if directory in ('stage1-protocol', 'tools'):
            directory = job['name']
        jobs_by_module[directory].append(job)
    modules = {name for name in affected if name != 'stage1-protocol'}
    unmapped = sorted(name for name in modules if aliases.get(name, name) not in jobs_by_module)
    if 'stage1-protocol' in affected:
        unmapped.append('stage1-protocol-retained')
    selected = {job['name']: job for name in modules
                for job in jobs_by_module[aliases.get(name, name)]}
    jobs = [dict(name=job['name'], driver=job['driver'], args=job['args'],
                 native=job['native'], cases=job['cases'])
            for job in sorted(selected.values(), key=lambda job: job['name'])]
    rust = []
    if MODEL in seeds:
        rust = [
            ['cargo', '--config', 'scripts/check-profiles.toml', 'test', '--locked', '--offline',
             '-p', 'sandproof-controller-service', '--lib', 'controller::lifecycle_inbox::tests::'],
            ['cargo', '--config', 'scripts/check-profiles.toml', 'clippy', '--locked', '--offline',
             '-p', 'sandproof-controller-service', '--all-targets', '--', '-D', 'warnings'],
        ]
    native_labels = {'functional', *(label for job in jobs for label in job['native'])}
    return dict(seed_inputs=sorted(seeds), affected_modules=sorted(affected), required_jobs=jobs, unmapped=unmapped,
                related_rust_checks=rust, registered_rust_inputs=actual_sources,
                original_native_prerequisites=sorted(native_labels),
                native_scope='stage1-protocol',
                native_command=['bash', 'scripts/python-entry.sh', 'tools/proof_native_suite.py', 'run',
                                '--scope', 'stage1-protocol', '--output', '<current-native-evidence>'],
                native_slice_limitation=('original native rules include tracked proof templates; functional also '
                                         'binds every tracked repository input. Stale seals are rejected.'),
                executor='--output <new-private-dir> --formal-lock <operator-lock> --registered-native <current-native-evidence>')


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
    return dict(theorem=full_theorem, property_rejected=variant == 'key-only',
                rust_formal_inputs_match=binding['native_inputs_match'],
                registry_content_match=binding['registry_content_match'],
                candidate_model_sha256=source_hash, seconds=record['seconds'],
                result_sha256=sha(folder / 'result.json'), **check_contract_type(folder, variant))


def check_contract_type(folder, variant, evidence=None):
    # Audited declaration names alone do not give the theorem's Lean type.
    # Check the precise proposition even when the mutant proof template is edited.
    _, module, theorem = VARIANTS[variant]
    full_theorem = 'SandboxModel.Stage1Protocol.' + theorem
    source = folder / 'source'
    negation = '¬ ' if variant == 'key-only' else ''
    proposition = negation + THEOREM_TYPE + LOOKUP
    if evidence is not None:
        evidence.mkdir(mode=0o700, parents=True)
    check = (evidence if evidence is not None else source / 'artifacts') / 'ModuleAcceptance.lean'
    check.write_text(f'import SandboxModel.Lifecycle.Protocol.{module}\n'
                     f'#check ({full_theorem} : {proposition})\n')
    log = (evidence if evidence is not None else folder) / 'contract-type.log'
    args = ['lake', 'env', 'lean']
    if evidence is not None:
        args += ['--root=' + str(check.parent)]
    with log.open('xb') as stream:
        checked = subprocess.run([*args, str(check)], cwd=source,
                                 stdin=subprocess.DEVNULL, stdout=stream,
                                 stderr=subprocess.STDOUT, timeout=120)
    if checked.returncode != 0:
        raise ValueError('wrong full lookup theorem type; see ' + str(log))
    return dict(contract_type_log_sha256=sha(log), contract_type_source_sha256=sha(check))


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


def execute_registered(output, plan, candidate, formal_lock, native):
    """Run the original registered subset against the original same-tree seal."""
    registered = registration(ROOT)
    shared = registered._shared()
    impact = plan['registered_impact']
    if impact['unmapped'] or not impact['required_jobs']:
        raise ValueError('required consumer has no original registered job; use complete Stage1')
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('registered execution requires the original formal role')
    native = native.resolve(strict=True)
    output = output.resolve()
    if output.is_relative_to(ROOT) or output.is_relative_to(native) or native.is_relative_to(output):
        raise ValueError('use a private output separate from the candidate and native evidence')
    directory = output / 'registered'
    job_names = {row['name'] for row in impact['required_jobs']}
    jobs = [job for job in registered.formal_plan(directory, root=ROOT) if job['name'] in job_names]
    if {job['name'] for job in jobs} != job_names:
        raise ValueError('a selected original job is absent from the current registry')
    output.mkdir(mode=0o700)
    report = dict(kind='stage1-module-registered-acceptance', candidate=candidate, plan=plan,
                  complete=False, full_stage1_acceptance=False, results=[])
    started = time.monotonic()
    try:
        os.environ['SANDPROOF_NATIVE_EVIDENCE'] = str(native)
        registered.validate_registration(ROOT, profile='full')
        shared.validate_native(native, scope='stage1-protocol')
        receipts = {label: shared.native_case(label, ROOT, None, scope='stage1-protocol')
                    for label in impact['original_native_prerequisites']}
        for label in receipts.keys() - {'functional'}:
            folder = registered._original_file(receipts[label], 'result.json').parent
            for create_label, driver, _ in registered.CREATE_NATIVE_DRIVERS:
                if label == create_label:
                    registered._check_create_native_result(ROOT, Path(driver).parent.name,
                                                           folder, read(folder / 'result.json'))
        paths = {name: registered._evidence(receipt) for name, receipt in receipts.items()}
        directory.mkdir(mode=0o700)
        with formal_lock.open('r+b') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            os.environ.update(SANDPROOF_LOCK_FILE=str(formal_lock.resolve()),
                              SANDPROOF_LOCK_FD=str(lock.fileno()))
            registered.check_create_sources(ROOT)
            helper = registered._execution_helper(ROOT)
            context = registered._execution_context(ROOT, shared, helper, receipts)
            report['native_seal_sha256'] = context['native']['seal_sha256']

            def source_guard():
                if (clean_candidate() != candidate or sha(native / 'complete.json') != report['native_seal_sha256']
                        or shared.repository_inputs(ROOT) != context['repository']['files']
                        or registered._source_modes(ROOT) != context['repository']['modes']):
                    raise ValueError('candidate, repository inputs or original native seal changed')
                registered.check_create_sources(ROOT)

            options = registered._execution_options()
            rows, failures = registered.execute_formal_jobs(
                ROOT, directory, jobs, context, paths, jobs=options['workers'], source_guard=source_guard,
                runtime_seed=Path(os.environ['SANDPROOF_FORMAL_CACHE']) / 'runtime-file-hashes.json')
            report.update(results=rows, failures=failures)
            if failures or [row['name'] for row in rows] != [job['name'] for job in jobs]:
                raise ValueError('one or more selected original proof jobs did not produce an admitted checkpoint')
            report['lookup_contract_types'] = {
                variant: check_contract_type(directory / f'{MODULE}-{variant}', variant,
                                             output / 'lookup-contract-types' / variant)
                for variant in VARIANTS
            }
            source_guard()
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
    parser.add_argument('--registered-native', type=Path,
                        help='original current Stage1 native-evidence path for a mapped registered selection')
    args = parser.parse_args(argv)
    os.umask(0o077)
    try:
        candidate = clean_candidate()
        base, paths = changed_files(args.base, candidate['commit'])
        plan = dict(select(paths), base_commit=base)
        impact = registered_impact(paths, base=base) if plan['status'] == 'review-required' else None
        if impact is not None:
            plan['registered_impact'] = impact
            plan['observed_package_references'] = plan.pop('direct_consumers')
            plan['affected_modules'] = impact['affected_modules']
            plan['reason'] = ('selected registered jobs require the original current Stage1 native evidence; '
                              'the plan alone issues no module pass')
        if args.plan or (plan['status'] != 'ready' and (impact is None or args.registered_native is None)):
            print(json.dumps(dict(plan, candidate=candidate), ensure_ascii=False))
            return 0 if plan['status'] == 'ready' else 2
        if args.formal_lock is None or args.formal_lock.is_symlink() or not args.formal_lock.is_file():
            raise ValueError('execution requires the existing regular operator --formal-lock')
        if args.registered_native is not None:
            if impact is None:
                raise ValueError('registered evidence cannot be supplied for an unmapped or isolated selection')
            return execute_registered(args.output, plan, candidate, args.formal_lock, args.registered_native)
        return execute(args.output, plan, candidate, args.formal_lock)
    except (OSError, ValueError, subprocess.CalledProcessError) as exc:
        parser.exit(2, str(exc) + '\n')


if __name__ == '__main__':
    raise SystemExit(main())
