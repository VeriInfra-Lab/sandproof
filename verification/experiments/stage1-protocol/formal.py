"""All retained Stage1 protocol views and actual Rust mutants; not whole V0 acceptance."""
from pathlib import Path
import argparse
import contextlib
import hashlib
import importlib.util
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
HERE = ROOT / 'verification/experiments/stage1-protocol'
LIBRARY = Path('verification/aeneas/stage1-protocol')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
POSITIVE_BY_PROFILE = {
    'carriers': ['Model', 'Carriers'],
    'request': ['Model', 'Library', 'ReplayContract', 'ReplayEquality',
                'ValidationModel', 'Validation', 'ValidationExamples',
                'RequestValues', 'Request', 'EnvelopeValues', 'Accepted', 'Recorded'],
    'manifest': ['Model', 'LoopWitness', 'ManifestWitnesses', 'Manifest'],
}
POSITIVE = sorted({n for names in POSITIVE_BY_PROFILE.values() for n in names})
NEGATIVE = {
    'same-incarnation': dict(imports=['Model', 'RequestValues', 'ValidationModel'],
        templates=['NegativeResume', 'NegativeValidation'], contracts=['Model', 'ValidationModel']),
    'key-only-replay': dict(imports=['Model', 'RequestValues', 'ReplayContract'],
        templates=['NegativeReplay', 'NegativeFullReplay'], contracts=['Model', 'ReplayContract']),
    'skip-capture-binding': dict(imports=['Model', 'LoopWitness', 'ManifestWitnesses'],
        templates=['NegativeManifest'], contracts=['Model']),
    'session-only-envelope': dict(imports=['Model', 'ValidationModel', 'EnvelopeValues'],
        templates=['NegativeSessionOnly'], contracts=['ValidationModel', 'EnvelopeValues']),
}


def component_files(project, names, profile, extra=()):
    # The existing resolver includes the entire local Lean import closure,
    # including retained helpers not imported by any public root.
    resolver = load_module('stage1_original_closure', project / 'tools/proof_batch.py')
    seeds = [str(PROOFS / (name + '.lean')) for name in names]
    seeds += [str(p.relative_to(project)) for p in sorted((project / LIBRARY / profile).rglob('*.lean'))
              if '_Template' not in p.name]
    seeds += list(extra)
    return sorted(resolver.closure(project, seeds, active_view='lakefile.toml'))


def check_inventory(project, frozen):
    inventory = load_module('unified_protocol_source_inventory', project / HERE / 'source_inventory.py')
    inventory.retained(project, POSITIVE)
    if frozen['positive_files'] != POSITIVE_BY_PROFILE:
        raise ValueError('unregistered or missing retained protocol proof file')


def check_actual(project, names, profile, folder, env, run, expected, extra=()):
    files = component_files(project, names, profile, extra)
    actual = actual_files(project, files, folder, env, run)
    if sorted(actual['audits']) != sorted(expected):
        raise ValueError('actual complete proof dependency inventory differs')
    return actual


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def generated_files(profile):
    return [str(p.relative_to(ROOT)) for p in sorted((ROOT / LIBRARY / profile).rglob('*.lean'))]


def actual_files(project, files, output, env, run):
    """Use the existing strict batch client, with an explicit component inventory."""
    before = {n: sha(project / n) for n in files}
    for index, name in enumerate(files):
        run(project, output, 'kernel-' + str(index), ['lake', 'env', 'lean', name])
    service = load_module('stage1_actual_service', project / 'tools/proof_services.py')
    # Only select this component's physical files; do not change validator logic.
    service.ROOT, service.FILES = project, tuple(files)
    previous = set((project / 'artifacts').glob('physical-service-*'))
    stdout = io.StringIO()
    with contextlib.redirect_stdout(stdout):
        service.main()
    created = set((project / 'artifacts').glob('physical-service-*')) - previous
    if len(created) != 1:
        raise ValueError('expected one original batch-service evidence directory')
    physical = output / 'physical-files'
    shutil.move(str(created.pop()), physical)
    expected = [n for name in files for n in re.findall(
        r'^#audit_controller_proof (\S+)$', (project / name).read_text(), re.M)]
    observed = re.findall(r'^Checked (\S+): \d+ standard logical dependencies$', stdout.getvalue(), re.M)
    if len(set(expected)) != len(expected) or sorted(expected) != sorted(observed):
        raise ValueError('missing, duplicate or extra component audit messages')
    if before != {n: sha(project / n) for n in files}:
        raise ValueError('physical files changed during checking')
    (output / 'actual-service.stdout').write_text(stdout.getvalue())
    # Forward only the original service's admitted audit messages.
    for line in stdout.getvalue().splitlines():
        if line.startswith('Checked '):
            print(line, flush=True)
    return dict(files=before, audits=expected, service_log_sha256=sha(physical / 'service.log'))


def reconstruct_native_source(native_tar, mutation, row, destination):
    """Rebuild equal bytes in this component's work, never in sealed evidence."""
    source = Path(mutation['source'])
    if source.is_absolute() or '..' in source.parts:
        raise ValueError('unsafe registered mutation source')
    member = native_tar.getmember(str(source))
    if not member.isfile():
        raise ValueError('native original mutation source is not a regular file')
    original = native_tar.extractfile(member).read()
    before, after = mutation['old'].encode(), mutation['new'].encode()
    if original.count(before) != 1:
        raise ValueError('ambiguous original native mutation source')
    rebuilt = original.replace(before, after, 1)
    if (hashlib.sha256(original).hexdigest() != row['original_source_sha256']
            or hashlib.sha256(rebuilt).hexdigest() != row['wrong_source_sha256']):
        raise ValueError('reconstructed native wrong source differs')
    destination.mkdir(mode=0o700, parents=True)
    file = destination / 'mutation.rs'
    with file.open('xb') as stream:
        stream.write(rebuilt)
    return dict(kind='equal-byte reconstruction; original retained on native host',
                file=str(file), sha256=sha(file), original_sha256=hashlib.sha256(original).hexdigest())


def check_native_phase(folder, row, mutation, build, assertion, wrong):
    build_key = 'compiler_artifacts' if build == 'build' else build + '_compiler_artifacts'
    hashes_key = 'compiled_sha256' if build == 'build' else build + '_compiled_sha256'
    log_name = 'assertions' if assertion == 'assertion' else assertion
    for key, file in ((build, build + '.log'), (assertion, log_name + '.log')):
        if sha(folder / file) != row[key]['log_sha256']:
            raise ValueError('original native phase log differs: ' + key)
    if row[build]['exit_code'] != 0 or row[assertion]['exit_code'] != (101 if wrong else 0):
        raise ValueError('native compile or exact assertion result differs')
    expected_sources = {'sandproof_protocol': 'sandbox-protocol/src/lib.rs',
                        mutation['target']: mutation['test_source']}
    artifacts = row[build_key]
    observed = []
    for line in (folder / (build + '.log')).read_text().splitlines():
        if line.startswith('{'):
            item = json.loads(line)
            if item.get('reason') == 'compiler-artifact' and item['target']['name'] in expected_sources:
                observed.append(item)
    if observed != artifacts or len(artifacts) != 2 or not row[hashes_key]:
        raise ValueError('native actual compiler records or preserved artifact inventory differ')
    prefixes, names = set(), set()
    for item in artifacts:
        name = item['target']['name']
        source = str(item['target']['src_path'])
        suffix = expected_sources[name]
        if item['fresh'] is not False or not source.endswith('/' + suffix):
            raise ValueError('native own artifact is stale or from another source')
        prefix = source[:-len(suffix)]
        if item['manifest_path'] != prefix + 'sandbox-protocol/Cargo.toml':
            raise ValueError('native actual manifest/source differs')
        is_test = name == mutation['target']
        if (item['profile']['test'] is not is_test or
                ('test' if is_test else 'lib') not in item['target']['kind']):
            raise ValueError('native artifact kind/profile differs')
        if is_test:
            executable = item['executable']
            if (not executable or Path(executable).name not in row[hashes_key]
                    or row[assertion]['argv'] != [executable, mutation['test'],
                        '--exact', '--nocapture', '--test-threads=1']):
                raise ValueError('native assertion is not bound to the actual test program')
        prefixes.add(prefix)
        names.add(name)
    if len(prefixes) != 1 or names != set(expected_sources):
        raise ValueError('native library and test do not share one original source')
    text = (folder / (log_name + '.log')).read_text()
    if wrong:
        if (mutation.get('failure_message', 'assertion') not in text
                or 'test result: FAILED. 0 passed; 1 failed; 0 ignored;' not in text
                or not re.search(r'^\s*' + re.escape(mutation['test']) + r'\s*$', text, re.M)):
            raise ValueError('compiled wrong implementation was not rejected by the original exact assertion')
    elif ('test result: ok. 1 passed; 0 failed; 0 ignored;' not in text
          or 'test ' + mutation['test'] + ' ... ok' not in text):
        raise ValueError('native original/restored exact assertion did not pass')


def replay(output, archive, native, native_archive, selected):
    output.mkdir(mode=0o700, parents=True)
    started = time.monotonic()
    record = dict(kind='stage1-protocol-scoped-formal-component', cases={}, commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
               LEAN_STACK_SIZE_KB='65536')
    frozen = json.loads((HERE / 'frozen.json').read_text())

    def check_inputs(project):
        for name, digest in frozen['inputs'].items():
            if sha(project / name) != digest:
                raise ValueError('frozen module input differs: ' + name)

    def run(project, folder, name, argv):
        begin = time.monotonic()
        log = folder / (name + '.log')
        with log.open('xb') as stream:
            code = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT, timeout=1800).returncode
        row = dict(argv=argv, cwd=str(project), exit_code=code,
                   seconds=time.monotonic()-begin, log_sha256=sha(log))
        record['commands'].append(row)
        if code:
            raise ValueError('component command failed: ' + name)

    try:
        check_inputs(ROOT)
        check_inventory(ROOT, frozen)
        if 'correct' in selected:
            folder = output / 'correct'
            folder.mkdir(mode=0o700)
            run(ROOT, folder, 'extract', [sys.executable, str(HERE / 'extract.py'),
                                        '--output', str(folder / 'extraction')])
            sys.path.insert(0, str(ROOT))
            from tools.check_foundations_extraction import llbc_digest
            extracted = json.loads((folder / 'extraction/result.json').read_text())
            if (extracted.get('success') is not True or extracted.get('sources_unchanged') is not True
                    or extracted['source_sha256'] != frozen['extraction_inputs']
                    or set(extracted['profiles']) != set(POSITIVE_BY_PROFILE)
                    or set(frozen['generations']) != set(POSITIVE_BY_PROFILE)):
                raise ValueError('incomplete three-profile generation or source inventory')
            for profile, expected in frozen['generations'].items():
                fresh = folder / 'extraction' / profile
                if (llbc_digest(json.loads((fresh / 'original.llbc').read_text())) != expected['llbc']
                        or llbc_digest(json.loads((ROOT / LIBRARY / profile / 'original.llbc').read_text())) != expected['llbc']):
                    raise ValueError('complete original IR differs: ' + profile)
                generated = {str(p.relative_to(fresh / 'generated')): sha(p)
                             for p in sorted((fresh / 'generated').rglob('*')) if p.is_file()}
                if generated != dict(expected['generated'], **expected['templates']):
                    raise ValueError('missing, extra or changed original generation: ' + profile)
                for name, digest in expected['generated'].items():
                    if sha(ROOT / LIBRARY / profile / name) != digest:
                        raise ValueError('retained original output differs: ' + name)
            record['extraction_inputs'] = extracted['source_sha256']
            views = {}
            for profile, names in POSITIVE_BY_PROFILE.items():
                view = folder / profile
                view.mkdir(mode=0o700)
                run(ROOT, view, 'build', ['lake', 'build', *[
                    'SandboxModel.Lifecycle.Protocol.' + n for n in names]])
                views[profile] = check_actual(ROOT, names, profile, view, env, run,
                    frozen['audits']['positive'][profile])
            record['cases']['correct'] = dict(complete=True, views=views)
        wrong = [name for name in selected if name != 'correct']
        if wrong:
            if archive is None or native is None or native_archive is None:
                raise ValueError('isolated source archive and original native mutant evidence required')
            native_record = json.loads(native.read_text())
            if native_record.get('success') is not True:
                raise ValueError('native component is incomplete')
            if sha(native_archive) != native_record['archive_sha256']:
                raise ValueError('original native source archive differs')
            with tarfile.open(native_archive) as native_tar:
                for file, digest in frozen['native_rust_inputs'].items():
                    entry = native_tar.getmember(file)
                    if not entry.isfile() or hashlib.sha256(native_tar.extractfile(entry).read()).hexdigest() != digest:
                        raise ValueError('native baseline source differs: ' + file)
            mutations = load_module('stage1_native_mutations', HERE / 'negative_native.py').MUTATIONS
            expected_mutations = {m['name']: m for m in mutations}
            rows = native_record['variants']
            if (len(rows) != len(expected_mutations)
                    or {row['name'] for row in rows} != set(expected_mutations)):
                raise ValueError('missing or duplicated original native mutation')
            with tarfile.open(native_archive) as native_tar:
                expected_snapshot = {}
                for member in native_tar.getmembers():
                    path = Path(member.name)
                    if (path.is_absolute() or '..' in path.parts or not member.isfile()
                            or member.name in expected_snapshot):
                        raise ValueError('native canonical archive has invalid or duplicate input')
                    expected_snapshot[member.name] = dict(
                        sha256=hashlib.sha256(native_tar.extractfile(member).read()).hexdigest(),
                        mode=member.mode & 0o777)
                for row in rows:
                    mutation = expected_mutations[row['name']]
                    if any(row.get(k) != v for k, v in mutation.items()):
                        raise ValueError('native mutation differs from registered single expression')
                    reconstructed = reconstruct_native_source(native_tar, mutation, row,
                        output / 'native-source-reconstruction' / row['name'])
                    folder = native.parent / row['name']
                    saved = folder / 'mutation.rs'
                    if saved.exists() and sha(saved) != row['wrong_source_sha256']:
                        raise ValueError('preserved original native mutation source differs')
                    for build, assertion, wrong_phase in (
                            ('baseline_build', 'baseline_assertion', False),
                            ('build', 'assertion', True),
                            ('restored_build', 'restored_assertion', False)):
                        check_native_phase(folder, row, mutation, build, assertion, wrong_phase)
                    if (row.get('rejected_by_unchanged_assertion') is not True
                            or row.get('source_changes_after_restore') != {}
                            or row.get('restored_source_sha256') != row['original_source_sha256']
                            or row.get('target_final_state') != 'correct-restored-build-verified'
                            or row['test_source_sha256'] != expected_snapshot[mutation['test_source']]['sha256']
                            or json.loads((folder / 'source-before.json').read_text()) != expected_snapshot):
                        raise ValueError('native original/test/full-source restoration evidence differs')
                    row['formal_source_evidence'] = reconstructed
            record['native_evidence'] = dict(record_sha256=sha(native),
                original_archive_sha256=sha(native_archive),
                baseline_rust_inputs=frozen['native_rust_inputs'])
            native_rows = {v['name']: v for v in native_record['variants']}
            for mutation in mutations:
                name = mutation['name']
                if name not in wrong:
                    continue
                folder = output / name
                folder.mkdir(mode=0o700)
                project = folder / 'source'
                project.mkdir(mode=0o700)
                with tarfile.open(archive) as tar:
                    tar.extractall(project, filter='data')
                check_inputs(project)
                runtime = load_module('protocol_stable_tools', ROOT / HERE / 'formal_runtime.py')
                (project / '.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
                (project / '.lake').mkdir()
                (project / '.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
                source = project / mutation['source']
                text = source.read_text()
                if text.count(mutation['old']) != 1 or sha(source) != native_rows[name]['original_source_sha256']:
                    raise ValueError('original Rust differs from native case')
                source.write_text(text.replace(mutation['old'], mutation['new'], 1))
                if sha(source) != native_rows[name]['wrong_source_sha256']:
                    raise ValueError('wrong Rust differs from actually tested native source')
                if (project / PROOFS / 'Model.lean').read_bytes() != (ROOT / PROOFS / 'Model.lean').read_bytes():
                    raise ValueError('wrong case changed the independent property')
                run(project, folder, 'extract', [sys.executable, 'verification/experiments/stage1-protocol/extract.py',
                    '--profile', mutation['profile'], '--output', str(folder / 'extraction')])
                profile = mutation['profile']
                generated = folder / 'extraction' / profile / 'generated'
                for path in generated.rglob('*'):
                    if path.is_file() and '_Template' not in path.name:
                        target = project / LIBRARY / profile / path.relative_to(generated)
                        target.parent.mkdir(parents=True, exist_ok=True)
                        shutil.copy2(path, target)
                shutil.copy2(folder / 'extraction' / profile / 'original.llbc',
                             project / LIBRARY / profile / 'original.llbc')
                proof_adaptation = None
                if name == 'skip-capture-binding':
                    # The real mutant duplicates the suffix into loop0/loop1.
                    # Binding mismatch is the true branch and actually uses loop0.
                    # Adapt proof references only; never edit generated functions or Model.
                    witness = project / PROOFS / 'ManifestWitnesses.lean'
                    original_proof = witness.read_text()
                    before = sha(witness)
                    old_loop = 'sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop'
                    new_loop = old_loop + '0'
                    raw = (project / LIBRARY / 'manifest/SandproofStage1Manifest/Funs.lean').read_text()
                    if (original_proof.count(old_loop) != 6 or
                            ('def ' + new_loop + '.body\n') not in raw or
                            ('def ' + new_loop + '\n') not in raw):
                        raise ValueError('unexpected real mutant loop declarations/proof references')
                    (folder / 'original-witness.lean').write_bytes(witness.read_bytes())
                    witness.write_text(original_proof.replace(old_loop, new_loop))
                    proof_adaptation = dict(path=str(PROOFS / 'ManifestWitnesses.lean'),
                        before_sha256=before, after_sha256=sha(witness),
                        original_reference=old_loop, actual_reference=new_loop, replacements=6,
                        scope='proof helper references for actual binding-mismatch branch only')
                config = NEGATIVE[name]
                for contract in config['contracts']:
                    if ((project / PROOFS / (contract + '.lean')).read_bytes()
                            != (ROOT / PROOFS / (contract + '.lean')).read_bytes()):
                        raise ValueError('wrong case changed the independent contract/witness: ' + contract)
                targets = ['SandboxModel.Lifecycle.Protocol.' + n for n in config['imports']]
                targets += ['SandproofStage1' + profile.title()]
                run(project, folder, 'build', ['lake', 'build', *targets])
                proofs = {}
                for proof in config['templates']:
                    view = folder / proof
                    view.mkdir(mode=0o700)
                    relative = 'artifacts/stage1-protocol/' + proof + '.lean'
                    (project / relative).parent.mkdir(parents=True, exist_ok=True)
                    (project / relative).write_bytes((HERE / (proof + '.lean.in')).read_bytes())
                    proofs[proof] = check_actual(project, config['imports'], profile, view, env, run,
                        frozen['audits']['negative'][proof], [relative])
                record['cases'][name] = dict(complete=True, rust_sha256=sha(source),
                    model_sha256=sha(project / PROOFS / 'Model.lean'),
                    contract_sha256={n: sha(project / PROOFS / (n + '.lean')) for n in config['contracts']},
                    native_source_evidence=native_rows[name]['formal_source_evidence'],
                    proof_adaptation=proof_adaptation, proofs=proofs)
                (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        check_inputs(ROOT)
        record['complete'] = True
        record['all_cases_complete'] = set(selected) == {'correct', *NEGATIVE}
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--source-archive', type=Path)
    parser.add_argument('--native-mutants', type=Path)
    parser.add_argument('--native-source-archive', type=Path)
    parser.add_argument('--case', choices=['all', 'correct', *NEGATIVE], default='all')
    args = parser.parse_args()
    if sys.platform != 'linux' or not os.environ.get('CARGO_TARGET_DIR'):
        raise ValueError('fixed isolated formal Linux runtime required')
    os.umask(0o077)
    selected = ['correct', *NEGATIVE] if args.case == 'all' else [args.case]
    result = replay(args.output.absolute(), args.source_archive, args.native_mutants, args.native_source_archive, selected)
    print(json.dumps({k: result.get(k) for k in ['kind', 'complete', 'all_cases_complete', 'seconds', 'error']}))
    raise SystemExit(0 if result['complete'] else 1)
