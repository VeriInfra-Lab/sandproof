"""Original/wrong/restored terminal checks through the actual transparent root.

Local development evidence; ning owns ordinary and native acceptance.
"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import time
import tomllib
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/terminal-binding')
PARENT = HERE.parent / 'inbox-record'


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('terminal_native_sources', ROOT/HERE.parent/'schema-leaves/formal.py')
registry_module = module('terminal_native_registry', ROOT/HERE.parent/'envelope-writer/native.py')
registry_module.HERE = PARENT
preparation = module('terminal_prepare', ROOT/HERE/'prepare.py')
sha = base.sha
FAILURES = {
    'key-only-observation': {'header_fields_are_not_omitted', 'accepted_center_is_not_omitted',
        'accepted_version_is_not_omitted', 'retention_fields_are_not_omitted',
        'observation_operation_is_not_omitted'},
    'allow-unload': {'non_destroy_operations_rejected'},
    'omit-retention-shape': {'zero_slot_rejected', 'upper_slot_rejected'},
    'omit-version-relation': {'wrong_version_rejected', 'expected_version_overflow_rejected'},
}


def run(output):
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='terminal-binding-native-development', complete=False, commands=[],
        ning_acceptance=False, shared_acceptance=False, full_v3=False, host=platform.node())
    try:
        inputs = base.snapshot(ROOT)
        registry = registry_module.registry()
        inventory = json.loads((ROOT/HERE/'inventory.json').read_text())
        for name, digest in inventory['production_inputs'].items():
            if inputs.get(name) != digest:
                raise ValueError('fixed production source changed: '+name)
        packages = tomllib.loads((ROOT/PARENT/'Cargo.lock').read_text())['package']
        locked = {p['name']+'@'+p['version'] for p in packages if 'checksum' in p}
        record.update(inputs=inputs, registry=registry, variants={}, tools={},
            source_commit=subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip())
        for name, args in [('cargo',['-V']),('rustc',['-Vv']),('clippy-driver',['--version']),('rustfmt',['--version'])]:
            tool = Path(subprocess.check_output(['rustup','which',name],text=True).strip())
            record['tools'][name] = dict(path=str(tool), sha256=sha(tool),
                version=subprocess.check_output([str(tool),*args],text=True))
        for variant in preparation.VARIANTS:
            folder = output/variant
            project = folder/'source'
            project.mkdir(parents=True, mode=0o700)
            expected = dict(inputs)
            for name, digest in inputs.items():
                p = project/name
                p.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT/name,p)
                if sha(p) != digest:
                    raise ValueError('source changed while copying: '+name)
            changes, prepared = preparation.prepare(project,variant)
            expected.update(changes)
            record.setdefault('preparations',{})[variant] = prepared
            if prepared['mutation'] is not None:
                record.setdefault('mutations',{})[variant] = prepared['mutation']
            manifest = PARENT/'Cargo.toml'
            with (project/manifest).open('a') as stream:
                stream.write('\n[[test]]\nname = "proof_terminal_binding"\npath = "../terminal-binding/native_tests.rs"\n')
            expected[str(manifest)] = sha(project/manifest)
            env = dict(os.environ,CARGO_TARGET_DIR=str(folder/'target'),CARGO_NET_OFFLINE='true',CARGO_BUILD_JOBS='4')
            commands = [
                ('format',['rustfmt','--edition','2024','--check',preparation.TERMINAL,preparation.SHIM,str(HERE/'native_tests.rs')]),
                ('test',['cargo','test','--manifest-path',str(manifest),'--locked','--offline','--test','proof_terminal_binding','--message-format=json','--','--nocapture','--test-threads=1']),
                ('clippy',['cargo','clippy','--manifest-path',str(manifest),'--locked','--offline','--lib','--tests','--','-D','warnings']),
            ]
            for label, argv in commands:
                log = folder/(label+'.log')
                begin = time.monotonic()
                with log.open('xb') as stream:
                    done = subprocess.run(argv,cwd=project,env=env,stdin=subprocess.DEVNULL,
                        stdout=stream,stderr=subprocess.STDOUT,timeout=1800)
                row = dict(variant=variant,label=label,argv=argv,exit_code=done.returncode,
                    seconds=time.monotonic()-begin,log_sha256=sha(log))
                record['commands'].append(row)
                (output/'progress.json').write_text(json.dumps(record,indent=2)+'\n')
                text = log.read_text()
                if label != 'test':
                    if done.returncode:
                        raise ValueError(label+' failed: '+variant)
                    continue
                artifacts = []
                names = {'sandproof_protocol','sandproof_stage1_inbox_record_extract','proof_terminal_binding'}
                for line in text.splitlines():
                    try: value = json.loads(line)
                    except ValueError: continue
                    if value.get('reason') != 'compiler-artifact': continue
                    if value['package_id'].startswith('registry+') and value['package_id'].rsplit('#',1)[1] not in locked:
                        raise ValueError('compiled registry package is outside verified lock')
                    if value['target']['name'] in names: artifacts.append(value)
                if len(artifacts) != 3 or {a['target']['name'] for a in artifacts} != names or any(a['fresh'] for a in artifacts):
                    raise ValueError('actual protocol, transparent-root library and native test must be freshly compiled')
                expected_artifacts = {
                    'sandproof_protocol':('sandbox-protocol/src/lib.rs','sandbox-protocol/Cargo.toml',False),
                    'sandproof_stage1_inbox_record_extract':(str(PARENT/'lib.rs'),str(manifest),False),
                    'proof_terminal_binding':(str(HERE/'native_tests.rs'),str(manifest),True),
                }
                for a in artifacts:
                    source, manifest_path, test = expected_artifacts[a['target']['name']]
                    if (Path(a['target']['src_path']).resolve() != (project/source).resolve()
                            or Path(a['manifest_path']).resolve() != (project/manifest_path).resolve()
                            or a['profile']['test'] != test):
                        raise ValueError('native artifact links wrong source/manifest/test target')
                row['artifacts'] = [dict(message=a,files={n:sha(Path(n)) for n in a['filenames'] if Path(n).is_file()}) for a in artifacts]
                if variant in FAILURES:
                    failures = FAILURES[variant]
                    block = text.rsplit('\nfailures:\n',1)[-1].split('\ntest result:',1)[0]
                    actual = {line.strip() for line in block.splitlines() if line.startswith('    ')}
                    if (done.returncode != 101 or actual != failures
                            or f'{18-len(failures)} passed; {len(failures)} failed; 0 ignored;' not in text):
                        raise ValueError('actual wrong Rust did not reach exact terminal assertions')
                    row['property_rejected'] = True
                elif done.returncode or '18 passed; 0 failed; 0 ignored;' not in text:
                    raise ValueError('original/restored terminal assertions failed')
            if any(sha(project/n) != d for n,d in expected.items()):
                raise ValueError('checked native source changed: '+variant)
            record['variants'][variant] = dict(inputs=expected,complete=True)
        if base.snapshot(ROOT) != inputs or registry_module.registry() != registry:
            raise ValueError('original source or library registry changed during checking')
        record['complete'] = True
    except BaseException as error:
        record.update(error=repr(error),traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output/'result.json').write_text(json.dumps(record,indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute())
    print(json.dumps({k:result.get(k) for k in ('complete','seconds','error')}))
    raise SystemExit(not result['complete'])
