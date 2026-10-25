"""Check an isolated production-route proposal; never label it current V3 acceptance."""
from pathlib import Path
import argparse
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/json-wire')
TEMPLATES = HERE / 'accepted'
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
LIBRARY = Path('verification/aeneas/stage1-protocol/request')
VIRTUAL_ROOT = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
CASES = ('correct', 'skip-envelope-guard')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def inventory(root):
    directory = root / TEMPLATES
    data = json.loads((directory / 'inventory.json').read_text())
    if data.get('schema') != 1 or data.get('production_adopted') is not False:
        raise ValueError('this driver checks an unadopted candidate only')
    actual = {p.name.removesuffix('.lean.in') for p in directory.glob('*.lean.in')}
    if actual != set(data['proof_templates']) or actual != {'AcceptedWireModel', 'AcceptedWire', 'NegativeGuard'}:
        raise ValueError('missing or unregistered proof template')
    if sha(directory / 'production-candidate.patch') != data['candidate_patch_sha256']:
        raise ValueError('candidate production diff changed')
    for name, digest in data['base_sources'].items():
        if sha(root / name) != digest:
            raise ValueError('fixed production source changed: ' + name)
    audits = []
    for name in actual:
        found = re.findall(r'^#audit_controller_proof (\S+)$', (directory / (name + '.lean.in')).read_text(), re.M)
        if found != data['audits'][name]:
            raise ValueError('proof audit declaration differs: ' + name)
        audits += found
    if len(audits) != len(set(audits)):
        raise ValueError('duplicate proof audit declaration')
    return data


def snapshot():
    names = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=ROOT).decode().split('\0')
    result = {}
    for name in sorted(set(filter(None, names))):
        p = ROOT / name
        if p.is_symlink() or not p.is_file():
            raise ValueError('only regular tracked/candidate source inputs: ' + name)
        result[name] = sha(p)
    return result


def run(output, selected):
    output.mkdir(mode=0o700, parents=True)
    start = time.monotonic()
    record = dict(kind='candidate-accepted-wire-local-formal', cases={}, commands=[],
        production_adopted=False, full_json_proof=False, full_v3_proof=False,
        native_checks='not-run-by-this-driver', full_acceptance=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')

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
        inputs = snapshot()
        record.update(source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
            inputs=inputs, candidate_patch_sha256=data['candidate_patch_sha256'])
        parent = ROOT / HERE.parent
        formal = module('wire_original_formal', parent / 'formal.py')
        replay = module('wire_request_formal', parent / 'replay_formal.py')
        config = replay.PROPERTIES['envelope']
        for case in selected:
            folder = output / case
            project = folder / 'source'
            project.mkdir(mode=0o700, parents=True)
            for name, digest in inputs.items():
                destination = project / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, destination)
                if sha(destination) != digest:
                    raise ValueError('source changed while copying: ' + name)
            (project / '.tools').symlink_to(ROOT / '.tools', target_is_directory=True)
            (project / '.lake').mkdir()
            # This task's private dependency cache is shared sequentially under
            # the outer heavy lock. Every source keeps its own .lake/build.
            (project / '.lake/packages').symlink_to(ROOT / '.lake/packages', target_is_directory=True)
            command(project, folder, 'candidate-patch', ['patch', '--strip=1', '--fuzz=0', '--batch',
                '--input=' + str(ROOT / TEMPLATES / 'production-candidate.patch')])
            expected = dict(inputs)
            for name, digest in data['candidate_sources'].items():
                if sha(project / name) != digest:
                    raise ValueError('actual production candidate differs: ' + name)
                expected[name] = digest
            root_source = HERE / 'src/lib.rs'
            shutil.copyfile(project / TEMPLATES / 'lib.rs.in', project / root_source)
            expected[str(root_source)] = sha(project / root_source)
            mutation = data['mutation']
            if case != 'correct':
                path = project / mutation['source']
                original = path.read_text()
                if original.count(mutation['old']) != 1:
                    raise ValueError('guard mutation must affect exactly one actual Rust expression')
                path.write_text(original.replace(mutation['old'], mutation['new'], 1))
                expected[mutation['source']] = sha(path)
            (project / 'artifacts/extract-target').mkdir(parents=True)
            command(project, folder, 'extract', ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev',
                '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/home',
                '--ro-bind', str(Path.home()), str(Path.home()), '--bind', str(project), str(VIRTUAL_ROOT),
                '--chdir', str(VIRTUAL_ROOT), '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL_ROOT / 'artifacts/extract-target'),
                '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--', sys.executable,
                str(HERE / 'extract.py'), '--profile', 'accepted', '--output', 'artifacts/extraction'])
            extraction = project / 'artifacts/extraction'
            result = json.loads((extraction / 'result.json').read_text())
            if result.get('success') is not True:
                raise ValueError('incomplete fixed-tool extraction')
            installed = {}
            for path in sorted((extraction / 'generated').rglob('*')):
                if path.is_file() and '_Template' not in path.name:
                    relative = LIBRARY / path.relative_to(extraction / 'generated')
                    shutil.copyfile(path, project / relative)
                    installed[str(relative)] = sha(path)
            shutil.copyfile(extraction / 'original.llbc', project / LIBRARY / 'original.llbc')
            installed[str(LIBRARY / 'original.llbc')] = sha(extraction / 'original.llbc')
            expected.update(installed)
            names = [*config['positive'], 'AcceptedWireModel']
            proof_names = ['AcceptedWireModel', 'AcceptedWire' if case == 'correct' else 'NegativeGuard']
            extras = []
            for name in proof_names:
                relative = (PROOFS / (name + '.lean')) if name != 'NegativeGuard' else Path('artifacts/NegativeGuard.lean')
                shutil.copyfile(project / TEMPLATES / (name + '.lean.in'), project / relative)
                expected[str(relative)] = sha(project / relative)
                if name == 'AcceptedWire':
                    names.append(name)
                elif name == 'NegativeGuard':
                    extras.append(str(relative))
            command(project, folder, 'build', ['lake', 'build', 'SandproofStage1Request',
                *['SandboxModel.Lifecycle.Protocol.' + name for name in names]])
            audits = config['audits'] + [a for n in proof_names for a in data['audits'][n]]
            actual = formal.check_actual(project, names, 'request', folder, env, command, audits, extras)
            for name, digest in expected.items():
                if sha(project / name) != digest:
                    raise ValueError('source/proof/IR changed during check: ' + name)
            record['cases'][case] = dict(complete=True, **actual,
                rust_sha256=sha(project / mutation['source']), installed_generated=installed,
                contract_sha256=sha(project / PROOFS / 'AcceptedWireModel.lean'))
            (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if inputs != snapshot():
            raise ValueError('source inputs changed during formal run')
        hashes = {v['contract_sha256'] for v in record['cases'].values()}
        if len(hashes) != 1:
            raise ValueError('correct and wrong Rust must use the identical full contract')
        record.update(complete=True, all_cases_complete=set(selected) == set(CASES))
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--case', choices=['all', *CASES], default='all')
    args = parser.parse_args()
    os.umask(0o077)
    selected = list(CASES) if args.case == 'all' else [args.case]
    result = run(args.output.absolute(), selected)
    print(json.dumps({k: result.get(k) for k in ['kind', 'complete', 'all_cases_complete', 'seconds', 'error']}))
    raise SystemExit(0 if result['complete'] else 1)
