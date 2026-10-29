"""Original/wrong/restored standard caller checks with the same two assertions.

Development evidence only; ning owns shared native acceptance.
"""
import argparse
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
from urllib.parse import unquote, urlparse

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/native-readiness')
MODEL = 'controller-service/src/controller/lifecycle_inbox/readiness.rs'
TEST_ROOT = 'controller-service/src/controller/lifecycle_inbox/readiness/tests.rs'
TESTS = {'proof_native_completion_checks_original_fields',
         'proof_native_completion_preserves_its_declared_scope'}


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('native-readiness_native_sources', ROOT / HERE.parent / 'schema-leaves/formal.py')
registry_module = module('native-readiness_native_registry', ROOT / HERE.parent / 'envelope-writer/native.py')
# Native service unit tests use the unchanged workspace Cargo.lock.
# Proof extraction retains its separate original inbox-record lock.
registry_module.HERE = Path('.')
preparation = module('native-readiness_prepare', ROOT / HERE / 'prepare.py')
sha = base.sha


def run(output):
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='native-readiness-native-development', complete=False, commands=[],
                  ning_acceptance=False, shared_acceptance=False, full_v3=False, host=platform.node())
    try:
        inputs = base.snapshot(ROOT)
        inventory = json.loads((ROOT/HERE/'inventory.json').read_text())
        for name, digest in inventory['production_inputs'].items():
            if inputs.get(name) != digest:
                raise ValueError('fixed production source changed: '+name)
        registry = registry_module.registry()
        packages = tomllib.loads((ROOT/'Cargo.lock').read_text())['package']
        locked_packages = {p['name']+'@'+p['version'] for p in packages if 'checksum' in p}
        record.update(inputs=inputs, registry=registry, variants={}, tools={},
            source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        for name, args in [('cargo', ['-V']), ('rustc', ['-Vv']), ('clippy-driver', ['--version']), ('rustfmt', ['--version'])]:
            path = Path(subprocess.check_output(['rustup', 'which', name], text=True).strip())
            record['tools'][name] = dict(path=str(path), sha256=sha(path),
                version=subprocess.check_output([str(path), *args], text=True))
        for variant in preparation.VARIANTS:
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
            changes, preparation_record = preparation.prepare(project, variant)
            record.setdefault('preparations', {})[variant] = preparation_record
            mutation = preparation_record['mutation']
            expected.update(changes)
            if mutation is not None:
                record.setdefault('mutations', {})[variant] = mutation
            # Append unchanged assertions in their original parent test scope.
            # No pre-existing component assertion is removed or filtered here.
            with (project / TEST_ROOT).open('a') as source:
                source.write('\n'+(project/HERE/'native_common.rs.in').read_text())
            expected[TEST_ROOT] = sha(project / TEST_ROOT)
            env = dict(os.environ, CARGO_TARGET_DIR=str(folder / 'target'),
                       CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4')
            checks = [('format', []), ('test', ['--lib', '--message-format=json', 'controller::lifecycle_inbox::readiness::tests::proof_native_completion_', '--', '--nocapture', '--test-threads=1']),
                                ('clippy', ['--lib', '--tests', '--', '-D', 'warnings'])]
            for label, args in checks:
                argv = ['cargo', label, '-p', 'sandproof-controller-service', '--locked', '--offline', *args]
                if label == 'format':
                    argv = ['rustfmt', '--edition', '2024', '--check',
                        *sorted({preparation.SOURCE, TEST_ROOT, *[n for n in changes if n.endswith('.rs')]})]
                log = folder / (label + '.log')
                begin = time.monotonic()
                with log.open('xb') as stream:
                    done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                        stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
                row = dict(variant=variant, label=label, argv=argv, exit_code=done.returncode,
                    seconds=time.monotonic()-begin, log_sha256=sha(log))
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
                        package_source = artifact['package_id'].rsplit('#', 1)[0]
                        if (not package_source.startswith('path+file:')
                                or Path(unquote(urlparse(package_source[5:]).path)) != project/folder_name):
                            raise ValueError('native artifact has the wrong package identity')
                        if not artifact['filenames'] or any(not Path(n).is_file() for n in artifact['filenames']):
                            raise ValueError('missing freshly compiled native artifact')
                        if folder_name == 'controller-service':
                            executable = artifact.get('executable')
                            if not executable or not Path(executable).is_file():
                                raise ValueError('actual business test executable is missing')
                            Path(executable).relative_to(folder/'target')
                    row['artifacts'] = [dict(message=a, files={n: sha(Path(n)) for n in a['filenames']
                        if Path(n).is_file()}) for a in artifacts]
                    observed_tests = {line.split(' ... ', 1)[0].rsplit('::', 1)[-1]
                        for line in text.splitlines() if line.startswith('test controller::lifecycle_inbox::readiness::tests::proof_native_completion_')}
                    if observed_tests != TESTS:
                        raise ValueError('missing or extra exact native assertions')
                    row['observed_tests'] = sorted(observed_tests)
                    failures = set(preparation.FAILURES.get(variant, []))
                    count = 2
                    block = text.rsplit('\nfailures:\n',1)[-1].split('\ntest result:',1)[0]
                    actual = {line.strip().rsplit('::',1)[-1] for line in block.splitlines()
                              if line.startswith('    controller::')}
                    row['observed_failures'] = sorted(actual)
                    row['expected_failures'] = sorted(failures)
                    if failures:
                        if (done.returncode != 101 or actual != failures or
                                f'{count-len(failures)} passed; {len(failures)} failed; 0 ignored;' not in text):
                            raise ValueError('wrong Rust did not reach exact native-readiness assertions')
                        row['property_rejected'] = True
                    elif done.returncode or f'{count} passed; 0 failed; 0 ignored;' not in text:
                        raise ValueError('native-readiness original/candidate/restored assertions failed')
                elif done.returncode:
                    raise ValueError(label+' failed: '+variant)
            if any(sha(project / name) != digest for name, digest in expected.items()):
                raise ValueError('checked source changed: ' + variant)
            record['variants'][variant] = dict(inputs=expected, complete=True)
        if base.snapshot(ROOT) != inputs or registry_module.registry() != registry:
            raise ValueError('original sources or dependencies changed during native checking')
        record['complete'] = True
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute())
    print(json.dumps({k: result.get(k) for k in ('kind', 'complete', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] else 1)
