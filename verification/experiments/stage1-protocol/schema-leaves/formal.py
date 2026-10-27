"""Check the seven real protocol role leaves in an isolated shared-V0 candidate."""
from pathlib import Path
import argparse
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/schema-leaves')
PROOF = 'SandboxModel/Lifecycle/Protocol/RoleLeaves.lean'
VIRTUAL_ROOT = Path('/home/zn/u333-5tuoxof8/std-mutex-source')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


view = module('protocol_independent_view', ROOT / HERE.parent / 'local_view.py')


def inventory(root):
    data = json.loads((root / HERE / 'inventory.json').read_text())
    if data.get('schema') != 1 or data.get('v0_adopted') is not False:
        raise ValueError('only the explicit unadopted shared-V0 candidate is supported')
    for name, digest in data['inputs'].items():
        if sha(root / name) != digest:
            raise ValueError('reviewed source/candidate changed: ' + name)
    proof = (root / HERE / 'RoleLeaves.lean.in').read_text()
    audits = re.findall(r'^#audit_controller_proof (\S+)$', proof, re.M)
    if audits != data['audits'] or len(audits) != len(set(audits)):
        raise ValueError('missing, duplicate or unregistered role-leaf audit')
    if len(data['roles']) != 7 or set(data['roles']) != {'session', 'center', 'incarnation', 'capture',
                             'request_window', 'request_sequence', 'retention_window'}:
        raise ValueError('incomplete actual role inventory')
    return data


def snapshot(root):
    names = subprocess.check_output(['git', 'ls-files', '--cached', '--others',
                                     '--exclude-standard', '-z'], cwd=root).decode().split('\0')
    result = {}
    for name in sorted(set(filter(None, names))):
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise ValueError('source must be a regular tracked/candidate file: ' + name)
        result[name] = sha(path)
    return result


def run(output, prepare_only=False):
    output.mkdir(parents=True, mode=0o700)
    started = time.monotonic()
    record = dict(kind='candidate-protocol-role-leaves', v0_adopted=False,
                  full_schema_proof=False, full_json_proof=False, full_v3_proof=False,
                  native_checks='delegated-to-ning', full_acceptance=False, commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
               LEAN_STACK_SIZE_KB='65536', CARGO_NET_OFFLINE='true')

    def command(project, folder, name, argv):
        begin = time.monotonic()
        log = folder / (name + '.log')
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(argv=argv, cwd=str(project), exit_code=result.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        if result.returncode:
            raise ValueError('candidate command failed: ' + str(log))

    try:
        data = inventory(ROOT)
        inputs = snapshot(ROOT)
        record.update(inputs=inputs, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        project = output / 'source'
        project.mkdir(mode=0o700)
        for name, digest in inputs.items():
            destination = project / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
            if sha(destination) != digest:
                raise ValueError('source changed while copying: ' + name)
        expected = dict(inputs)
        expected.update(view.install(project, 'schema-leaves'))
        if prepare_only:
            record.update(prepared=True, complete=False)
        else:
            runtime = module('protocol_stable_tools', ROOT / HERE.parent / 'formal_runtime.py')
            (project / '.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
            (project / '.lake').mkdir()
            (project / '.lake/packages').symlink_to(ROOT / '.lake/packages', target_is_directory=True)
            (project / 'artifacts/extract-target').mkdir(parents=True)
            fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
            for name, relative in [('charon', 'aeneas/charon/bin/charon'), ('aeneas', 'aeneas/bin/aeneas')]:
                if sha(ROOT / '.tools' / relative) != fixed[name]['binary_sha256']:
                    raise ValueError('fixed tool binary differs: ' + name)
            record['tools'] = {name: fixed[name] for name in ('charon', 'aeneas')}
            command(project, output, 'extract', ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev',
                '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()),
                str(Path.home()), *runtime.tool_bind(ROOT), '--bind', str(project), str(VIRTUAL_ROOT), '--chdir', str(VIRTUAL_ROOT),
                '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL_ROOT / 'artifacts/extract-target'),
                '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--', 'bash', '-c',
                'bash ' + str(HERE / 'extract-map.sh') + ' && ' +
                'python3 ' + str(HERE.parent / 'envelope-bytes/extract-json.py')])
            irs = ['artifacts/schema-map.llbc', 'artifacts/envelope-json-foundations.llbc']
            record['llbc_sha256'] = {}
            for name in irs:
                ir = json.loads((project / name).read_text())
                if ir.get('has_errors') is not False:
                    raise ValueError('incomplete original Charon input: ' + name)
                targets = {t['key'] for t in ir['translated']['target_information']}
                if targets != {fixed['target']}:
                    raise ValueError('wrong extraction target')
                record['llbc_sha256'][name] = sha(project / name)
            generated = {}
            for directory in ['verification/aeneas/stage1-protocol/json-foundations',
                              'verification/aeneas/stage1-protocol/schema-result']:
                for path in sorted((project / directory).rglob('*')):
                    if path.is_file():
                        if '_Template.lean' in path.name:
                            raise ValueError('unreviewed external template in actual map generation')
                        generated[str(path.relative_to(project))] = sha(path)
            expected.update(generated)
            expected.update(record['llbc_sha256'])
            shutil.copy2(ROOT / HERE / 'RoleLeaves.lean.in', project / PROOF)
            expected[PROOF] = sha(project / PROOF)
            command(project, output, 'build', ['lake', 'build', 'SandboxModel.Lifecycle.Protocol.RoleLeaves'])
            formal = module('role_original_formal', ROOT / HERE.parent / 'formal.py')
            resolver = module('role_original_closure', ROOT / 'tools/proof_batch.py')
            seeds = [PROOF, *[name for name in generated if name.endswith('.lean')]]
            files = sorted(resolver.closure(project, seeds))
            actual = formal.actual_files(project, files, output, env, command)
            declared = [name for file in files for name in re.findall(
                r'^#audit_controller_proof (\S+)$', (project / file).read_text(), re.M)]
            if (sorted(actual['audits']) != sorted(declared)
                    or not set(data['audits']).issubset(actual['audits'])):
                raise ValueError('incomplete actual leaf/import dependency audit')
            record.update(actual=actual, generated_sha256=generated, complete=True)
        for name, digest in expected.items():
            if sha(project / name) != digest:
                raise ValueError('candidate/source/generated file changed: ' + name)
        if inputs != snapshot(ROOT):
            raise ValueError('original inputs changed during candidate check')
    except BaseException as error:
        record.pop('prepared', None)
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.prepare_only)
    print(json.dumps({k: result.get(k) for k in ['kind', 'complete', 'prepared', 'seconds', 'error']}))
    raise SystemExit(0 if result.get('complete') or result.get('prepared') else 1)
