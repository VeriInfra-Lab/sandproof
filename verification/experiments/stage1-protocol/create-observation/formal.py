"""Fresh Create value equality and contract witnesses, NOT full guard refinement."""
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
HERE = Path('verification/experiments/stage1-protocol/create-observation')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
LIBRARY = Path('verification/aeneas/stage1-protocol/request')
EXTRACTOR = HERE.parent / 'observation-bindings/extract.py'
VIRTUAL_ROOT = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
PROFILE = 'create-values'
NEW_PROOFS = ['CreateObservationModel', 'CreateObservationEquality', 'CreateObservationValues']
INHERITED = {
    'observation-bindings': ['ReadyFactModel', 'ReadyFactValues', 'ReadyFactBindings', 'ReadyFactExamples'],
    'resume-observation': ['ResumeModel', 'ResumeValues', 'ResumeBindings', 'ResumeExamples'],
}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def snapshot():
    names = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=ROOT).decode().split('\0')
    result = {}
    for name in sorted(set(filter(None, names))):
        path = ROOT / name
        if path.is_symlink() or not path.is_file():
            raise ValueError('only regular tracked/candidate inputs: ' + name)
        result[name] = sha(path)
    return result


def run(output, packages):
    output.mkdir(mode=0o700, parents=True)
    start = time.monotonic()
    record = dict(kind='stage1-create-value-support-local-formal', commands=[],
        scope='derived equality and contract witnesses only', full_guard_refinement=False,
        formal_mutant_checks=False, full_v3_proof=False, full_acceptance=False)
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
            raise ValueError('formal command failed: ' + str(log))

    try:
        admission = module('create_value_admission', ROOT / HERE / 'admission.py')
        data = admission.inventory(ROOT)
        inputs = snapshot()
        record.update(source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(), inputs=inputs)
        formal = module('create_original_formal', ROOT / HERE.parent / 'formal.py')
        replay = module('create_original_request_formal', ROOT / HERE.parent / 'replay_formal.py')
        config = replay.PROPERTIES['envelope']
        if not packages.is_dir():
            raise ValueError('allocated fixed-tool package cache is missing')
        project = output / 'source'
        project.mkdir(mode=0o700)
        for name, digest in inputs.items():
            destination = project / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
            if sha(destination) != digest:
                raise ValueError('source changed while copying: ' + name)
        (project / '.tools').symlink_to(ROOT / '.tools', target_is_directory=True)
        (project / '.lake').mkdir()
        (project / '.lake/packages').symlink_to(packages, target_is_directory=True)
        (project / 'artifacts/extract-target').mkdir(parents=True)
        command(project, output, 'extract', ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev',
            '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/home',
            '--ro-bind', str(Path.home()), str(Path.home()), '--bind', str(project), str(VIRTUAL_ROOT),
            '--chdir', str(VIRTUAL_ROOT), '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL_ROOT / 'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--', sys.executable,
            str(EXTRACTOR), '--profile', PROFILE, '--output', 'artifacts/extraction'])
        extraction = project / 'artifacts/extraction'
        result = json.loads((extraction / 'result.json').read_text())
        if (result.get('success') is not True
                or result['profiles'][PROFILE].get('generation_success') is not True
                or result['profiles'][PROFILE]['roots'] != data['scope']['actual_roots']):
            raise ValueError('incomplete or different fixed-tool extraction')
        extraction /= PROFILE
        installed = {}
        for path in sorted((extraction / 'generated').rglob('*')):
            if path.is_file() and '_Template' not in path.name:
                relative = LIBRARY / path.relative_to(extraction / 'generated')
                shutil.copyfile(path, project / relative)
                installed[str(relative)] = sha(path)
        shutil.copyfile(extraction / 'original.llbc', project / LIBRARY / 'original.llbc')
        installed[str(LIBRARY / 'original.llbc')] = sha(extraction / 'original.llbc')
        names, audits = list(config['positive']), list(config['audits'])
        for component, inherited_names in INHERITED.items():
            inherited = json.loads((project / HERE.parent / component / 'inventory.json').read_text())
            for name in inherited_names:
                found = re.findall(r'^#audit_controller_proof (\S+)$', (project / PROOFS / (name + '.lean')).read_text(), re.M)
                if found != inherited['audits'][name]:
                    raise ValueError('retained audit inventory differs: ' + name)
                names.append(name)
                audits += found
        names += NEW_PROOFS
        audits += [a for name in NEW_PROOFS for a in data['audits'][name]]
        command(project, output, 'build', ['lake', 'build', 'SandproofStage1Request',
            *['SandboxModel.Lifecycle.Protocol.' + name for name in names]])
        actual = formal.check_actual(project, names, 'request', output, env, command, audits)
        for name, digest in (inputs | installed).items():
            if sha(project / name) != digest:
                raise ValueError('source/proof/generated input changed during check: ' + name)
        if inputs != snapshot():
            raise ValueError('task inputs changed during formal run')
        record.update(complete=True, all_value_files_complete=True, actual=actual,
            installed_generated=installed, production_sha256={n: sha(project / n) for n in data['base_sources']},
            contract_sha256=sha(project / PROOFS / 'CreateObservationModel.lean'),
            witnesses_sha256=sha(project / PROOFS / 'CreateObservationValues.lean'))
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--packages', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.packages.absolute())
    print(json.dumps({k: result.get(k) for k in ['kind', 'complete', 'all_value_files_complete', 'full_guard_refinement', 'seconds', 'error']}))
    raise SystemExit(0 if result['complete'] else 1)
