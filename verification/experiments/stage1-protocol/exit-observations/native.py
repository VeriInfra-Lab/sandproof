"""Twenty-two identical real-candidate matching assertions in fresh correct/wrong/restored views.

Local development tests are authorized; ning owns ordinary/native acceptance.
The original real SQLite fixtures and controlled host evidence stay distinct from physical process facts.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path(__file__).resolve().parent
BASE = '9c50da7cc9391518514f14405e6c779e5c5cbe2c'
SOURCES = ('controller-service/src/controller/lifecycle_inbox/source_work/restore/exit.rs',
           'controller-service/src/controller/lifecycle_inbox/source_work/restore/continued_exit.rs')
TEST_FRAGMENTS = {
    'controller-service/src/controller/lifecycle_inbox/source_work/tests/restore_exit.rs': 'native-direct-tests.rs.in',
    'controller-service/src/controller/lifecycle_inbox/source_work/tests/continued_exit.rs': 'native-continued-tests.rs.in',
}
MUTATIONS = {
    'direct-drop-start-ticks': (SOURCES[0], '            && observed.process.start_time_ticks > 0\n', ''),
    'before-pid-only': (SOURCES[1], '            && observed.process == self.process\n',
                        '            && observed.process.pid == self.process.pid\n'),
    'continued-boot-only': (SOURCES[1], '            && observed.process == self.work.source().source\n',
                           '            && observed.process.boot_id == self.work.source().source.boot_id\n'),
}
TESTS = {'proof_exit_observations_continued_pid', 'proof_exit_observations_continued_boot', 'proof_exit_observations_continued_effect_attempt', 'proof_exit_observations_before_normal', 'proof_exit_observations_before_pid', 'proof_exit_observations_direct_pid', 'proof_exit_observations_continued_normal', 'proof_exit_observations_continued_digest', 'proof_exit_observations_direct_normal', 'proof_exit_observations_direct_ticks', 'proof_exit_observations_direct_identity', 'proof_exit_observations_direct_positive_process_is_value_only', 'proof_exit_observations_before_boot', 'proof_exit_observations_direct_digest', 'proof_exit_observations_continued_identity', 'proof_exit_observations_before_identity', 'proof_exit_observations_before_digest', 'proof_exit_observations_direct_effect_attempt', 'proof_exit_observations_direct_boot', 'proof_exit_observations_before_effect_attempt', 'proof_exit_observations_before_ticks', 'proof_exit_observations_continued_ticks'}
FAILURES = {'correct': set(), 'direct-drop-start-ticks': {'proof_exit_observations_direct_ticks'}, 'before-pid-only': {'proof_exit_observations_before_boot', 'proof_exit_observations_before_ticks'}, 'continued-boot-only': {'proof_exit_observations_continued_pid', 'proof_exit_observations_continued_ticks'}, 'restored': set()}


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


base = module('exit_observations_source_snapshot', HERE.parent / 'schema-leaves/formal.py')
registry = module('exit_observations_native_registry', HERE.parent / 'envelope-writer/native.py')
registry.HERE = Path('.')
sha = base.sha


def require(condition, message):
    if not condition:
        raise ValueError(message)


def mode(path):
    return '100755' if path.stat().st_mode & 0o111 else '100644'


def source_guard():
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    require(subprocess.run(['git', 'merge-base', '--is-ancestor', BASE, head], cwd=ROOT,
                           check=False).returncode == 0, 'fixed integration is not an ancestor')
    names = subprocess.check_output(['git', 'diff', '--name-only', '-z', 'HEAD'], cwd=ROOT).decode().split('\0')
    names += subprocess.check_output(['git', 'ls-files', '--others', '--exclude-standard', '-z'],
                                     cwd=ROOT).decode().split('\0')
    require(all(name.startswith(('verification/experiments/stage1-protocol/exit-observations/',
                                 'verification/aeneas/stage1-protocol/exit-observations/')) or
                name == 'docs/formal/03-protocol-inbox.md' for name in filter(None, names)),
            'unowned working source changed')
    return head


def run(output):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='exit-observations-native-development', complete=False, ning_acceptance=False,
                  shared_acceptance=False, full_v3=False, commands=[], views={}, restorations=[])
    try:
        require(1 <= len(os.sched_getaffinity(0)) <= 4, 'native CPU allocation differs')
        head = source_guard()
        inputs = base.snapshot(ROOT)
        modes = {name: mode(ROOT / name) for name in inputs}
        libraries = registry.registry()
        record.update(inputs=inputs, input_modes=modes, registry=libraries, tools={},
                      cpus=sorted(os.sched_getaffinity(0)),
                      source_commit=head)
        for name in ('rustc', 'cargo', 'clippy-driver', 'rustfmt'):
            path = Path(subprocess.check_output(['rustup', 'which', name], text=True).strip())
            record['tools'][name] = dict(path=str(path), sha256=sha(path))
        project = output / 'source'
        project.mkdir(mode=0o700)
        for name, digest in inputs.items():
            target = project / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, target)
            require(sha(target) == digest, 'source changed while copying')
        expected = dict(inputs)
        original_work = {name: (project / name).read_bytes() for name in SOURCES}
        for name, fragment in TEST_FRAGMENTS.items():
            path = project / name
            original = path.read_bytes()
            require(b'fn proof_exit_observations_' not in original, 'test fixture root already present')
            path.write_bytes(original + (HERE / fragment).read_bytes())
            expected[name] = sha(path)
        tests = set()
        for name in TEST_FRAGMENTS:
            tests.update(re.findall(r'^#\[test\]\nfn (proof_exit_observations_\w+)\(\)',
                                    (project / name).read_text(), re.M))
        require(tests == TESTS, 'native source differs from exact twenty-two assertions')
        expected_modes = dict(modes)
        for view, failures in FAILURES.items():
            folder = output / view
            folder.mkdir(mode=0o700)
            target = folder / 'target'
            require(not target.exists(), 'variant target was already populated')
            view_inputs = dict(expected)
            mutation = MUTATIONS.get(view)
            if mutation:
                source_name, before, after = mutation
                path = project / source_name
                text = path.read_text()
                require(text.count(before) == 1, 'actual Rust mutation site changed')
                path.write_text(text.replace(before, after))
                view_inputs[source_name] = sha(path)
            env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                       CARGO_TARGET_DIR=str(target), CARGO_TERM_COLOR='never',
                       PYTHONDONTWRITEBYTECODE='1')
            entry = dict(source_inputs=view_inputs, target_initially_absent=True,
                         expected_failures=sorted(failures))
            for label, argv in (
                ('format', ['rustfmt', '--edition', '2024', '--check', *SOURCES, *TEST_FRAGMENTS]),
                ('test', ['cargo', 'test', '-p', 'sandproof-controller-service', '--locked', '--offline',
                          '--lib', '--message-format=json', 'proof_exit_observations_', '--', '--test-threads=1']),
                ('clippy', ['cargo', 'clippy', '-p', 'sandproof-controller-service', '--locked', '--offline',
                            '--lib', '--tests', '--', '-D', 'warnings']),
            ):
                log = folder / (label + '.log')
                begin = time.monotonic()
                with log.open('xb') as stream:
                    result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                            stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
                row = dict(view=view, label=label, argv=argv, exit_code=result.returncode,
                           seconds=time.monotonic()-begin, log=str(log), log_sha256=sha(log))
                record['commands'].append(row)
                (output / 'progress.json').write_text(json.dumps(record, indent=2) + '\n')
                if label != 'test':
                    require(result.returncode == 0, 'native ' + label + ' failed: ' + view)
                    continue
                text = log.read_text()
                outcomes = re.findall(r'^test [\w:]+::(proof_exit_observations_\w+) \.\.\. (ok|FAILED)$',
                                      text, re.M)
                require(len(outcomes) == len(TESTS) and {name for name, _ in outcomes} == TESTS and
                        {name for name, status in outcomes if status == 'FAILED'} == failures,
                        'native full assertion set or exact failures differ')
                require(result.returncode == (101 if failures else 0) and
                        f'{len(TESTS)-len(failures)} passed; {len(failures)} failed; 0 ignored;' in text,
                        'native process did not reach the original assertions')
                if failures:
                    require('assertion failed: !qualified.matches' in text,
                            'wrong Rust did not reach the same original matching assertion')
                artifacts = []
                for line in text.splitlines():
                    try:
                        message = json.loads(line)
                    except ValueError:
                        continue
                    if message.get('reason') == 'compiler-artifact' and message['target']['name'] in (
                            'sandproof_protocol', 'sandproof_controller_service'):
                        artifacts.append(message)
                require(len(artifacts) == 2 and not any(item['fresh'] for item in artifacts),
                        'actual protocol and service test artifacts were not freshly compiled')
                for artifact in artifacts:
                    package = 'sandbox-protocol' if artifact['target']['name'] == 'sandproof_protocol' else 'controller-service'
                    require(artifact['manifest_path'] == str(project / package / 'Cargo.toml') and
                            artifact['target']['src_path'] == str(project / package / 'src/lib.rs') and
                            artifact['profile']['test'] == (package == 'controller-service'),
                            'native artifact comes from another source/manifest/test target')
                protocol, = [item for item in artifacts if item['target']['name'] == 'sandproof_protocol']
                linked = list(target.glob('debug/deps/libsandproof_protocol-*.rlib'))
                require(len(linked) == 1 and str(linked[0]) in protocol['filenames'],
                        'target contains a different protocol library')
                entry.update(native_assertions_checked=True, failed_assertions=sorted(failures),
                             property_rejected=bool(failures), artifacts=[{
                                 'message': item,
                                 'files': {name: sha(Path(name)) for name in item['filenames'] if Path(name).is_file()}
                             } for item in artifacts])
            for name, digest in view_inputs.items():
                path = project / name
                require(not path.is_symlink() and sha(path) == digest and mode(path) == expected_modes[name],
                        'native source or mode changed')
            if mutation:
                path = project / source_name
                # Restoring from the exact retained bytes also handles a removed
                # guard, whose empty replacement is not an invertible string key.
                path.write_bytes(original_work[source_name])
                require(sha(path) == expected[source_name], 'actual original Rust restoration differs')
                record['restorations'].append(dict(view=view, source=source_name, restored_sha256=sha(path)))
            record['views'][view] = entry
        require(base.snapshot(ROOT) == inputs and registry.registry() == libraries,
                'native original source or fixed libraries changed')
        require(all(mode(ROOT / name) == value for name, value in modes.items()), 'source modes changed')
        require(all(sha(Path(value['path'])) == value['sha256'] for value in record['tools'].values()),
                'native tool changed')
        require(all(sha(project / name) == digest for name, digest in expected.items()),
                'final restored native source differs')
        record['complete'] = True
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2) + '\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    # The shared native suite coordinates its own lock. Local proof-host
    # development holds the existing heavy lock in the outer invocation.
    cpus = sorted(os.sched_getaffinity(0))
    if len(cpus) > 4:
        os.sched_setaffinity(0, set(cpus[:4]))
    result = run(args.output.absolute())
    print(json.dumps({name: result.get(name) for name in ('complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)
