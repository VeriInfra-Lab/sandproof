"""Candidate/wrong/restored Create owner successor checks with identical native assertions.

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

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-owner-successor')
MODEL = 'controller-service/src/controller/lifecycle_inbox.rs'
TEST_ROOT = 'controller-service/src/controller/lifecycle_inbox/tests.rs'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('create-owner-successor_native_sources', ROOT / HERE.parent / 'schema-leaves/formal.py')
registry_module = module('create-owner-successor_native_registry', ROOT / HERE.parent / 'envelope-writer/native.py')
# Reuse the reviewed archive/source verifier, selecting this actual Cargo lock.
registry_module.HERE = HERE.parent / 'inbox-record'
support = module('create_owner_successor_prepare', ROOT / HERE / 'support.py')
sha = base.sha


def run(output):
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='create-owner-successor-native-development', complete=False, commands=[],
                  ning_acceptance=False, shared_acceptance=False, full_v3=False, host=platform.node(),
                  production_adopted=False,original_general_equivalence_proved=False)
    try:
        inventory = support.inventory()
        extra = {str(ROOT/HERE/name):sha(ROOT/HERE/name) for name in
            [*inventory['templates'],*inventory['drivers'],'inventory.json']}
        record['extra_inputs'] = extra
        inputs = base.snapshot(ROOT)
        registry = registry_module.registry()
        packages = tomllib.loads((ROOT/HERE.parent/'inbox-record/Cargo.lock').read_text())['package']
        locked_packages = {p['name']+'@'+p['version'] for p in packages if 'checksum' in p}
        record.update(inputs=inputs, registry=registry, variants={}, tools={},
            source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        for name, args in [('cargo', ['-V']), ('rustc', ['-Vv']), ('clippy-driver', ['--version']), ('rustfmt', ['--version'])]:
            path = Path(subprocess.check_output(['rustup', 'which', name], text=True).strip())
            record['tools'][name] = dict(path=str(path), sha256=sha(path),
                version=subprocess.check_output([str(path), *args], text=True))
        for variant in [*inventory['variants'],'restored-candidate']:
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
            def patch_command(project, folder, label, argv):
                log = folder/(label+'.log')
                with log.open('xb') as stream:
                    done = subprocess.run(argv,cwd=project,stdin=subprocess.DEVNULL,
                        stdout=stream,stderr=subprocess.STDOUT,timeout=60)
                record['commands'].append(dict(variant=variant,label=label,argv=argv,
                    exit_code=done.returncode,log_sha256=sha(log)))
                if done.returncode: raise ValueError('private candidate derivation failed')
            selected = 'candidate' if variant=='restored-candidate' else variant
            preparation_record = support.prepare(project,selected,inventory,patch_command,folder,append_root=False)
            record.setdefault('preparations', {})[variant] = preparation_record
            mutation = preparation_record['mutation']
            changes = {MODEL:sha(project/MODEL)}
            expected.update(changes)
            if mutation is not None:
                record.setdefault('mutations', {})[variant] = mutation
            test_source = (ROOT/HERE/'native-common.rs.in').read_bytes()
            combined = project/HERE/'native_tests.rs'
            if combined.exists():
                raise ValueError('unexpected preexisting combined load test source')
            combined.parent.mkdir(parents=True, exist_ok=True)
            combined.write_bytes(test_source)
            expected[str(HERE/'native_tests.rs')] = sha(combined)
            with (project / TEST_ROOT).open('a') as source:
                source.write('\n#[path = "../../../../verification/experiments/stage1-protocol/create-owner-successor/native_tests.rs"]\nmod proof_create_owner_successor;\n')
            expected[TEST_ROOT] = sha(project / TEST_ROOT)
            env = dict(os.environ, CARGO_TARGET_DIR=str(folder / 'target'),
                       CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4')
            checks = [('format', []), ('test', ['--lib', '--message-format=json', 'proof_create_owner_successor::', '--', '--nocapture', '--test-threads=1']),
                                ('clippy', ['--lib', '--tests', '--', '-D', 'warnings'])]
            for label, args in checks:
                argv = ['cargo', label, '-p', 'sandproof-controller-service', '--locked', '--offline', *args]
                if label == 'format':
                    argv = ['rustfmt', '--edition', '2024', '--check',
                        *sorted({MODEL, TEST_ROOT, *[n for n in changes if n.endswith('.rs')]}), str(HERE/'native_tests.rs')]
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
                    row['artifacts'] = [dict(message=a, files={n: sha(Path(n)) for n in a['filenames']
                        if Path(n).is_file()}) for a in artifacts]
                    failures = set(inventory['native_plan']['expected_failures'].get(variant, []))
                    count = len(__import__('re').findall(r'^#\[test\]$', test_source.decode(), __import__('re').M))
                    if (count != inventory['native_plan']['test_count'] or
                            __import__('re').findall(r'^fn (proof_successor_\w+)\(',test_source.decode(),__import__('re').M) != inventory['native_plan']['test_names']):
                        raise ValueError('native assertion inventory differs')
                    block = text.rsplit('\nfailures:\n',1)[-1].split('\ntest result:',1)[0]
                    actual = {line.strip().rsplit('::',1)[-1] for line in block.splitlines()
                              if line.startswith('    controller::')}
                    if failures:
                        if (done.returncode != 101 or actual != failures or
                                f'{count-len(failures)} passed; {len(failures)} failed; 0 ignored;' not in text):
                            raise ValueError('wrong Rust did not reach exact create-owner-successor assertions')
                        row['property_rejected'] = True
                    elif done.returncode or f'{count} passed; 0 failed; 0 ignored;' not in text:
                        raise ValueError('Create owner successor candidate/restored assertions failed')
                elif done.returncode:
                    raise ValueError(label+' failed: '+variant)
            if any(sha(project / name) != digest for name, digest in expected.items()):
                raise ValueError('checked source changed: ' + variant)
            record['variants'][variant] = dict(inputs=expected, complete=True)
        if any(sha(Path(n)) != h for n, h in extra.items()):
            raise ValueError('Create owner successor fixture/driver/candidate changed during checking')
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
