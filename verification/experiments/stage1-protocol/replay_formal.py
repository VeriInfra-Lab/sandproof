"""Local full-request replay proofs on fresh Rust extractions; not batch acceptance.

Retains the historical component freeze. Each case has an isolated source and
Lean build directory; both use the same independently stated equality contract.
Ordinary tests and native mutant assertions still belong on the test machine.
"""
from pathlib import Path
import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]
HERE = Path('verification/experiments/stage1-protocol')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
LIBRARY = Path('verification/aeneas/stage1-protocol/request')
# Preserve the original extractor's source paths, without editing its output.
VIRTUAL_ROOT = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
POSITIVE = ['ReplayContract', 'ReplayEquality']
NEGATIVE = ['ReplayContract', 'Model', 'RequestValues']
AUDITS = ['bytes_eq', 'array_eq', 'vec_eq', 'ok_and', 'resources_eq',
          'create_eq', 'node_eq', 'key_eq', 'expected_eq', 'binding_eq',
          'operation_eq', 'request_eq', 'replay_complete',
          'replay_accepts_iff', 'replay_rejects_iff']


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def snapshot():
    names = subprocess.check_output(
        ['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'],
        cwd=ROOT).decode().split('\0')
    result = {}
    for name in sorted(set(filter(None, names))):
        path = ROOT / name
        if path.is_symlink() or not path.is_file():
            raise ValueError('snapshot requires regular source files: ' + name)
        result[name] = sha(path)
    return result


def run(output):
    output.mkdir(mode=0o700, parents=True)
    start = time.monotonic()
    record = dict(kind='stage1-full-replay-local-formal', cases={}, commands=[],
                  native_checks='not-run', full_acceptance=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
               LEAN_STACK_SIZE_KB='65536')

    def command(project, folder, name, argv):
        log = folder / (name + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            code = subprocess.run(argv, cwd=project, env=env,
                                  stdin=subprocess.DEVNULL, stdout=stream,
                                  stderr=subprocess.STDOUT, timeout=1800).returncode
        record['commands'].append(dict(argv=argv, cwd=str(project),
            exit_code=code, seconds=time.monotonic()-begin, log_sha256=sha(log)))
        if code:
            raise ValueError('formal command failed: ' + str(log))

    try:
        inputs = snapshot()
        record['source_commit'] = subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
        record['inputs'] = inputs  # Includes uncommitted proof changes, not just HEAD.
        formal = load_module('replay_actual_files', ROOT / HERE / 'formal.py')
        mutations = load_module('replay_mutations', ROOT / HERE / 'negative_native.py')
        mutation = next(m for m in mutations.MUTATIONS if m['name'] == 'key-only-replay')
        for case in ['correct', 'key-only-replay']:
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
            command(ROOT, folder, 'dependencies', ['cp', '-a', '--reflink=auto',
                str(ROOT / '.lake/packages'), str(project / '.lake/packages')])
            source = project / mutation['source']
            if case != 'correct':
                original = source.read_text()
                if original.count(mutation['old']) != 1:
                    raise ValueError('mutation must change exactly one actual Rust expression')
                source.write_text(original.replace(mutation['old'], mutation['new'], 1))
            expected_inputs = dict(inputs)
            expected_inputs[mutation['source']] = sha(source)

            # Only the private source is writable in this extraction namespace.
            # Host tools remain read-only; the target is private to this case.
            command(project, folder, 'extract', ['bwrap', '--ro-bind', '/', '/',
                '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp',
                '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
                '--bind', str(project), str(VIRTUAL_ROOT), '--chdir', str(VIRTUAL_ROOT),
                '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL_ROOT / 'artifacts/extract-target'),
                '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS',
                '--', sys.executable, str(HERE / 'extract.py'), '--profile', 'request',
                '--output', 'artifacts/extraction'])
            extraction = project / 'artifacts/extraction'
            generated = extraction / 'request/generated'
            extraction_record = json.loads((extraction / 'result.json').read_text())
            if extraction_record.get('success') is not True:
                raise ValueError('actual extraction incomplete')
            installed = {}
            for path in generated.rglob('*'):
                if path.is_file() and '_Template' not in path.name:
                    relative = LIBRARY / path.relative_to(generated)
                    shutil.copy2(path, project / relative)
                    installed[str(relative)] = sha(path)
            shutil.copy2(extraction / 'request/original.llbc', project / LIBRARY / 'original.llbc')
            installed[str(LIBRARY / 'original.llbc')] = sha(extraction / 'request/original.llbc')
            expected_inputs.update(installed)

            names = POSITIVE if case == 'correct' else NEGATIVE
            targets = ['SandboxModel.Lifecycle.Protocol.' + name for name in names]
            command(project, folder, 'build', ['lake', 'build', 'SandproofStage1Request', *targets])
            files = ['SandboxModel/ProofAuditCommand.lean']
            files += [str(PROOFS / (name + '.lean')) for name in names]
            files += [str(p.relative_to(project)) for p in sorted((project / LIBRARY).rglob('*.lean'))
                      if '_Template' not in p.name]
            files += ['verification/aeneas/foundations/SandproofFoundations/' + name + '.lean'
                      for name in ['TypesExternal', 'IdentifierLibrary', 'Types', 'FunsExternal']]
            if case == 'correct':
                expected_audits = ['SandboxModel.Stage1Protocol.Replay.' + n for n in AUDITS]
            else:
                relative = 'artifacts/NegativeFullReplay.lean'
                shutil.copy2(ROOT / HERE / 'NegativeFullReplay.lean.in', project / relative)
                files.append(relative)
                expected_inputs[relative] = sha(ROOT / HERE / 'NegativeFullReplay.lean.in')
                expected_audits = ['SandboxModel.Stage1Protocol.Request.' + n for n in
                                   ['mutant_accepts_changed_capture', 'key_only_not_full_replay']]
            actual = formal.actual_files(project, files, folder, env, command)
            if sorted(actual['audits']) != sorted(expected_audits):
                raise ValueError('full-replay audit inventory differs')
            for name, digest in expected_inputs.items():
                if sha(project / name) != digest:
                    raise ValueError('unexpected source/proof/generated change: ' + name)
            record['cases'][case] = dict(complete=True, **actual,
                rust_sha256=sha(source), installed_generated=installed,
                contract_sha256=sha(project / PROOFS / 'ReplayContract.lean'),
                extraction_record_sha256=sha(extraction / 'result.json'))
            (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if record['cases']['correct']['contract_sha256'] != record['cases']['key-only-replay']['contract_sha256']:
            raise ValueError('correct and wrong versions used different properties')
        if snapshot() != inputs:
            raise ValueError('source changed during the formal checks')
        record['complete'] = True
    except Exception as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.resolve())
    print(json.dumps({k: result.get(k) for k in
                     ['kind', 'complete', 'native_checks', 'full_acceptance', 'seconds', 'error']}))
    raise SystemExit(0 if result['complete'] else 1)
