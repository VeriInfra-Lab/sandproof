"""Private candidate actual/wrong/restored BLOB reader checks for ning.

Development evidence only; ning owns shared native acceptance.
"""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import tomllib
import platform
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/standard-blob')
MODEL = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
TEST_ROOT = MODEL


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('blob-reader_native_sources', ROOT / 'verification/experiments/stage1-protocol/schema-leaves/formal.py')
registry_module = module('blob-reader_native_registry', ROOT / 'verification/experiments/stage1-protocol/envelope-writer/native.py')
# Reuse the reviewed archive/source verifier, selecting this actual Cargo lock.
registry_module.HERE = Path('.')
support = module('blob_native_current_package', ROOT/HERE/'support.py')
sha = base.sha


def run(output):
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='current-standard-blob-candidate-native-development', complete=False, commands=[],
                  ning_acceptance=False, shared_acceptance=False, full_v3=False, host=platform.node(),
                  original_general_equivalence_proved=False,production_modified=False,production_adopted=False,finite_provider_adopted=False)
    try:
        support.inventory()
        inventory = json.loads((ROOT/HERE/'plan.json').read_text())
        if inventory['native_run'] is not False or inventory['production_adopted'] is not False:
            raise ValueError('this entry is exclusively the unadopted source candidate')
        if sha(ROOT/MODEL) != inventory['actual_source_original_sha256']:
            raise ValueError('actual production source changed before candidate preparation')
        extra = {str(p):sha(p) for p in [ROOT/HERE/'plan.json', ROOT/HERE/'source-candidate.json', ROOT/HERE/'mutations.json', ROOT/HERE/'native-cases.rs.in', ROOT/HERE/'preparation.py', Path(__file__).resolve()]}
        if sha(ROOT/HERE/'native-cases.rs.in') != inventory['cases_sha256']:
            raise ValueError('native assertions changed')
        if any(Path(n).is_symlink() or not Path(n).is_file() for n in extra):
            raise ValueError('private native input must be a regular original file')
        extra_modes = {n:support.mode(Path(n)) for n in extra}
        record['extra_inputs'] = extra
        record['extra_input_modes'] = extra_modes
        inputs = base.snapshot(ROOT)
        input_modes = {n:('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') for n in inputs}
        registry = registry_module.registry()
        packages = tomllib.loads((ROOT/'Cargo.lock').read_text())['package']
        locked_packages = {p['name']+'@'+p['version'] for p in packages if 'checksum' in p}
        record.update(inputs=inputs, input_modes=input_modes, registry=registry, variants={}, tools={},
            source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        for name, args in [('cargo', ['-V']), ('rustc', ['-Vv']), ('clippy-driver', ['--version']), ('rustfmt', ['--version'])]:
            path = Path(subprocess.check_output(['rustup', 'which', name], text=True).strip())
            record['tools'][name] = dict(path=str(path), sha256=sha(path),
                version=subprocess.check_output([str(path), *args], text=True))
        for variant, plan in inventory['native_variants'].items():
            folder = output / variant
            project = folder / 'source'
            project.mkdir(mode=0o700, parents=True)
            expected = dict(inputs)
            for name, digest in inputs.items():
                target = project / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, target)
                if sha(target) != digest:
                    raise ValueError('source changed while copying: ' + name)
            derivation, changed = support.preparation.candidate(project, variant)
            expected.update(changed)
            record.setdefault('preparations', {})[variant] = derivation
            combined = project/HERE/'native-common.rs'
            combined.write_bytes((ROOT/HERE/'native-cases.rs.in').read_bytes())
            expected[str(HERE/combined.name)] = sha(combined)
            with (project/TEST_ROOT).open('a') as stream:
                stream.write('\n#[cfg(test)]\n#[path = "../../../../verification/experiments/stage1-protocol/standard-blob/native-common.rs"]\nmod proof_load_blob_common;\n')
            expected[TEST_ROOT] = sha(project/TEST_ROOT)
            test_source = combined.read_bytes()
            test_filter = 'proof_load_blob_common::'
            env = dict(os.environ, CARGO_TARGET_DIR=str(folder / 'target'),
                       CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4')
            checks = [('format', []), ('test', ['--lib', '--message-format=json', test_filter, '--', '--nocapture', '--test-threads=1']),
                                ('clippy', ['--lib', '--tests', '--message-format=json', '--', '-D', 'warnings'])]
            for label, args in checks:
                argv = ['cargo', label, '-p', 'sandproof-controller-service', '--locked', '--offline', *args]
                if label == 'format':
                    argv = ['rustfmt','--edition','2024','--check',MODEL,
                        str(HERE/'native-common.rs')]
                log = folder / (label + '.log')
                begin = time.monotonic()
                with log.open('xb') as stream:
                    done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                        stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
                row = dict(variant=variant, label=label, argv=argv, exit_code=done.returncode,
                    seconds=time.monotonic()-begin, cwd=str(project), log_sha256=sha(log))
                record['commands'].append(row)
                (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
                text = log.read_text()
                if label == 'test':
                    artifacts = []
                    for line in text.splitlines():
                        try:
                            value = json.loads(line)
                        except ValueError:
                            continue
                        if value.get('reason') == 'compiler-artifact' and value['package_id'].startswith('registry+'):
                            if value['package_id'].rsplit('#',1)[1] not in locked_packages:
                                raise ValueError('compiled registry package is outside the verified lock')
                        if value.get('reason') == 'compiler-artifact' and value['target']['name'] in (
                                'sandproof_protocol', 'sandproof_controller_service'):
                            artifacts.append(value)
                    if len(artifacts) != 2 or any(a['fresh'] for a in artifacts):
                        raise ValueError('actual protocol library and service test must be freshly compiled')
                    for artifact in artifacts:
                        source = Path(artifact['target']['src_path'])
                        folder_name = 'sandbox-protocol' if artifact['target']['name'] == 'sandproof_protocol' else 'controller-service'
                        if (source != project/folder_name/'src/lib.rs'
                                or artifact['manifest_path'] != str(project/folder_name/'Cargo.toml')
                                or artifact['profile']['test'] != (folder_name == 'controller-service')):
                            raise ValueError('native artifact links the wrong source/manifest/test target')
                    row['artifacts'] = [dict(message=a, files={n: sha(Path(n)) for n in a['filenames']
                        if Path(n).is_file()}) for a in artifacts]
                    failures = set(plan['expected_failures'])
                    count = len(__import__('re').findall(r'^#\[test\]$', test_source.decode(), __import__('re').M))
                    if count!=plan['test_count']:
                        raise ValueError('native assertion inventory differs')
                    block = text.rsplit('\nfailures:\n',1)[-1].split('\ntest result:',1)[0]
                    actual = {line.strip().rsplit('::',1)[-1] for line in block.splitlines()
                              if line.startswith('    controller::')}
                    if failures:
                        if (done.returncode != 101 or actual != failures or
                                f'{count-len(failures)} passed; {len(failures)} failed; 0 ignored;' not in text):
                            raise ValueError('wrong Rust did not reach exact blob-reader assertions')
                        row['property_rejected'] = True
                    elif done.returncode or f'{count} passed; 0 failed; 0 ignored;' not in text:
                        raise ValueError('BLOB reader actual/restored assertions failed')
                elif label == 'clippy':
                    errors = []
                    for line in text.splitlines():
                        try:
                            message = json.loads(line)
                        except ValueError:
                            continue
                        if message.get('reason') != 'compiler-message' or message['message']['level'] != 'error':
                            continue
                        diagnostic = message['message']
                        primary = [span for span in diagnostic['spans'] if span['is_primary']]
                        if len(primary) != 1:
                            raise ValueError('unexpected Clippy primary diagnostic')
                        span = primary[0]
                        source = Path(span['file_name'])
                        if not source.is_absolute():
                            source = project/source
                        errors.append(dict(code=(diagnostic.get('code') or {}).get('code'),
                            source=str(source.relative_to(project)), line=span['line_start']))
                    expected_errors = []
                    if plan['expected_clippy_codes']:
                        raise ValueError('no lint exception is registered for this component')
                    if errors != expected_errors or done.returncode != (101 if expected_errors else 0):
                        raise ValueError('strict Clippy differs from the exact mutant-lint inventory')
                    row['expected_mutant_lint_rejection'] = bool(expected_errors)
                    row['clippy_errors'] = errors
                    # A lint rejection is never a business-contract counterexample.
                elif done.returncode:
                    raise ValueError(label+' failed: '+variant)
            if any(sha(project / name) != digest for name, digest in expected.items()):
                raise ValueError('checked source changed: ' + variant)
            private_modes = {n:('100755' if (project/n).stat().st_mode & 0o111 else '100644') for n in expected}
            if any(private_modes[n] != m for n,m in input_modes.items()):
                raise ValueError('private source execution mode changed')
            record['variants'][variant] = dict(inputs=expected, input_modes=private_modes, complete=True)
        if any(Path(n).is_symlink() or sha(Path(n)) != h or support.mode(Path(n)) != extra_modes[n] for n, h in extra.items()):
            raise ValueError('BLOB reader fixture/driver/source changed during checking')
        if any(('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') != m for n,m in input_modes.items()):
            raise ValueError('original source execution mode changed')
        if base.snapshot(ROOT) != inputs or registry_module.registry() != registry:
            raise ValueError('original sources or dependencies changed during native checking')
        for row in record['tools'].values():
            if sha(Path(row['path'])) != row['sha256']:
                raise ValueError('ordinary tool changed during native checking')
        record['complete'] = True
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True)
    parser.add_argument('--lock-fd', type=int)
    args = parser.parse_args()
    lock_path = args.lock.resolve(strict=True)
    with (lock_path.open('r+') if args.lock_fd is None else os.fdopen(os.dup(args.lock_fd), 'r+')) as lock:
        expected, actual = lock_path.stat(), os.fstat(lock.fileno())
        if (expected.st_dev, expected.st_ino) != (actual.st_dev, actual.st_ino):
            raise ValueError('inherited ordinary coordination descriptor differs')
        fcntl.flock(lock, fcntl.LOCK_EX | (fcntl.LOCK_NB if args.lock_fd is not None else 0))
        result = run(args.output.resolve())
        print(json.dumps({name: result.get(name) for name in ('complete', 'seconds', 'error')}), flush=True)
        raise SystemExit(0 if result['complete'] else 1)
