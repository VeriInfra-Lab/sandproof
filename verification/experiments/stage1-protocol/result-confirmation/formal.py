"""Fresh real terminal/close proofs and Rust counterexamples; not full acceptance."""
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
HERE = Path('verification/experiments/stage1-protocol/result-confirmation')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
LIBRARY = Path('verification/aeneas/stage1-protocol/request')
VIRTUAL_ROOT = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
NEW_PROOFS = {'ResultModel', 'ResultValues', 'ResultConfirmation', 'ResultExamples', 'ResultEquality'}
CASES = ('correct', 'terminal-key-only', 'confirmation-key-only', 'close-pending', 'close-newer-frontier')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def inventory(root):
    data = json.loads((root / HERE / 'inventory.json').read_text())
    parent = module('confirmation_inventory_parent', root / HERE.parent / 'formal.py')
    extra = set()
    sibling = root / HERE.parent / 'renewal-confirmation/formal.py'
    if sibling.is_file():
        renewal = module('registered_renewal_inventory', sibling)
        renewal.inventory(root)
        extra = renewal.NEW_PROOFS
    actual = {p.stem for p in (root / PROOFS).glob('*.lean')}
    if data.get('schema') != 1 or actual != set(parent.POSITIVE) | NEW_PROOFS | extra:
        raise ValueError('unregistered or missing retained protocol proof')
    templates = {p.name.removesuffix('.lean.in') for p in (root / HERE).glob('*.lean.in')}
    if templates != {m['template'] for m in data['mutations'].values()}:
        raise ValueError('unregistered or missing counterexample template')
    if set(data['mutations']) != set(CASES[1:]):
        raise ValueError('missing real Rust mutant')
    if set(data['audits']) != NEW_PROOFS | templates:
        raise ValueError('incomplete declaration inventory')
    audits = []
    for name, expected in data['audits'].items():
        path = root / (PROOFS / (name + '.lean') if name in NEW_PROOFS else HERE / (name + '.lean.in'))
        found = re.findall(r'^#audit_controller_proof (\S+)$', path.read_text(), re.M)
        if found != expected:
            raise ValueError('proof audit declaration differs: ' + name)
        audits += found
    if len(audits) != len(set(audits)):
        raise ValueError('duplicate proof audit declaration')
    for name, digest in data['base_sources'].items():
        if sha(root / name) != digest:
            raise ValueError('fixed actual production source changed: ' + name)
    for mutation in data['mutations'].values():
        if (root / mutation['source']).read_text().count(mutation['old']) != 1:
            raise ValueError('mutation must identify exactly one actual Rust expression')
    return data


def snapshot():
    names = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=ROOT).decode().split('\0')
    result = {}
    for name in sorted(set(filter(None, names))):
        path = ROOT / name
        if path.is_symlink() or not path.is_file():
            raise ValueError('only regular tracked/candidate inputs: ' + name)
        result[name] = sha(path)
    return result


def run(output, selected, packages):
    output.mkdir(mode=0o700, parents=True)
    start = time.monotonic()
    record = dict(kind='stage1-terminal-confirmation-local-formal', cases={}, commands=[],
        full_v3_proof=False, native_checks='not-run-by-this-driver', full_acceptance=False)
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
        data = inventory(ROOT)
        inputs = snapshot()
        record.update(source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(), inputs=inputs)
        formal = module('confirmation_original_formal', ROOT / HERE.parent / 'formal.py')
        replay = module('confirmation_request_formal', ROOT / HERE.parent / 'replay_formal.py')
        config = replay.PROPERTIES['envelope']
        if not packages.is_dir():
            raise ValueError('this task needs its allocated fixed-tool dependency cache')
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
            # Sequential reuse of this task's package cache under the outer heavy
            # lock; each case always has its own .lake/build and Rust target.
            (project / '.lake/packages').symlink_to(packages, target_is_directory=True)
            expected = dict(inputs)
            if case != 'correct':
                mutation = data['mutations'][case]
                path = project / mutation['source']
                original = path.read_text()
                if original.count(mutation['old']) != 1:
                    raise ValueError('mutation no longer unique')
                path.write_text(original.replace(mutation['old'], mutation['new'], 1))
                expected[mutation['source']] = sha(path)
            (project / 'artifacts/extract-target').mkdir(parents=True)
            command(project, folder, 'extract', ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev',
                '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/home',
                '--ro-bind', str(Path.home()), str(Path.home()), '--bind', str(project), str(VIRTUAL_ROOT),
                '--chdir', str(VIRTUAL_ROOT), '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL_ROOT / 'artifacts/extract-target'),
                '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--', sys.executable,
                str(HERE / 'extract.py'), '--profile', 'terminal', '--output', 'artifacts/extraction'])
            extraction = project / 'artifacts/extraction'
            result = json.loads((extraction / 'result.json').read_text())
            if result.get('success') is not True or result['profiles']['terminal'].get('generation_success') is not True:
                raise ValueError('incomplete fixed-tool extraction')
            extraction = extraction / 'terminal'
            installed = {}
            for path in sorted((extraction / 'generated').rglob('*')):
                if path.is_file() and '_Template' not in path.name:
                    relative = LIBRARY / path.relative_to(extraction / 'generated')
                    shutil.copyfile(path, project / relative)
                    installed[str(relative)] = sha(path)
            shutil.copyfile(extraction / 'original.llbc', project / LIBRARY / 'original.llbc')
            installed[str(LIBRARY / 'original.llbc')] = sha(extraction / 'original.llbc')
            expected.update(installed)
            names = [*config['positive'], 'ResultModel', 'ResultValues', 'ResultEquality']
            proof_names = ['ResultModel', 'ResultValues', 'ResultEquality']
            extras = []
            if case == 'correct':
                names += ['ResultConfirmation', 'ResultExamples']
                proof_names += ['ResultConfirmation', 'ResultExamples']
            else:
                template = mutation['template']
                relative = Path('artifacts') / (template + '.lean')
                shutil.copyfile(project / HERE / (template + '.lean.in'), project / relative)
                expected[str(relative)] = sha(project / relative)
                proof_names.append(template)
                extras.append(str(relative))
            command(project, folder, 'build', ['lake', 'build', 'SandproofStage1Request',
                *['SandboxModel.Lifecycle.Protocol.' + name for name in names]])
            audits = config['audits'] + [a for n in proof_names for a in data['audits'][n]]
            actual = formal.check_actual(project, names, 'request', folder, env, command, audits, extras)
            for name, digest in expected.items():
                if sha(project / name) != digest:
                    raise ValueError('source/proof/IR changed during check: ' + name)
            record['cases'][case] = dict(complete=True, **actual,
                production_sha256={n: sha(project / n) for n in data['base_sources']},
                installed_generated=installed,
                contract_sha256=sha(project / PROOFS / 'ResultModel.lean'),
                witnesses_sha256=sha(project / PROOFS / 'ResultValues.lean'))
            (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if inputs != snapshot():
            raise ValueError('source inputs changed during formal run')
        for field in ('contract_sha256', 'witnesses_sha256'):
            if len({v[field] for v in record['cases'].values()}) != 1:
                raise ValueError('correct and wrong Rust must use identical full contracts and witnesses')
        record.update(complete=True, all_cases_complete=set(selected) == set(CASES))
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--packages', required=True, type=Path, help='allocated fixed-tool Lean package cache')
    parser.add_argument('--case', choices=['all', *CASES], default='all')
    args = parser.parse_args()
    os.umask(0o077)
    selected = list(CASES) if args.case == 'all' else [args.case]
    result = run(args.output.absolute(), selected, args.packages.absolute())
    print(json.dumps({k: result.get(k) for k in ['kind', 'complete', 'all_cases_complete', 'seconds', 'error']}))
    raise SystemExit(0 if result['complete'] else 1)
