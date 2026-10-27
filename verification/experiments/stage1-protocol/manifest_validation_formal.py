"""Complete manifest value validation, using the retained extractor and checker.

Reuse the stage1-manifest and retained protocol capture mutations and their
actual-acceptance proofs; append refutations of one unchanged complete
ValidationContract. Native and whole-batch acceptance remain separate.
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
BASE = Path('verification/experiments/stage1-manifest')
HERE = Path('verification/experiments/stage1-protocol/manifest-validation')
PROTOCOL = Path('verification/experiments/stage1-protocol')
BASE_PROOFS = Path('SandboxModel/Lifecycle/Manifest')
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
LIBRARY = Path('verification/aeneas/stage1-protocol/manifest')
VIRTUAL_ROOT = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
CAPTURE_CASE = 'skip-capture-binding'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read_inventory(root):
    inventory = json.loads((root / HERE / 'inventory.json').read_text())
    base_inventory = json.loads((root / BASE / 'inventory.json').read_text())
    mutations = {m['name']: m for m in json.loads((root / BASE / 'mutations.json').read_text())}
    protocol_native = load('manifest_capture_native_inventory', root / PROTOCOL / 'negative_native.py')
    capture = [m for m in protocol_native.MUTATIONS if m['name'] == CAPTURE_CASE]
    if len(capture) != 1:
        raise ValueError('one retained capture mutation is required')
    mutations[CAPTURE_CASE] = capture[0]
    base_inventory['cases'][CAPTURE_CASE] = dict(proofs=[], template='NegativeManifest')
    if set(inventory['cases']) != {'correct', *mutations}:
        raise ValueError('every registered mutation must have a formal case')
    actual = {p.stem for p in (root / PROOFS).glob('Manifest*.lean')} - {'Manifest', 'ManifestWitnesses'}
    if actual != set(inventory['modules']):
        raise ValueError('missing or unregistered manifest proof module')
    if set(inventory['cases']['correct']['proofs']) != actual | {'Manifest', 'ManifestWitnesses'}:
        raise ValueError('correct case must check every manifest validation module')
    if not (root / HERE / 'NegativeFullCapture.lean.in').is_file():
        raise ValueError('capture counterexample supplement is missing')
    for case, config in inventory['cases'].items():
        if len(config['audits']) != len(set(config['audits'])):
            raise ValueError('duplicate audit declaration: ' + case)
    return inventory, base_inventory, mutations


def adapt_capture_helpers(project):
    """Bind existing proof helper names to actual mutant output, never edit it."""
    witness = project / PROOFS / 'ManifestWitnesses.lean'
    original = witness.read_text()
    old = 'sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop'
    new = old + '0'
    raw = (project / LIBRARY / 'SandproofStage1Manifest/Funs.lean').read_text()
    if (original.count(old) != 6 or 'def ' + new + '.body\n' not in raw
            or 'def ' + new + '\n' not in raw):
        raise ValueError('actual capture mutant loop declarations differ')
    before = sha(witness)
    witness.write_text(original.replace(old, new))
    return dict(before=before, after=sha(witness), old=old, new=new, replacements=6)


def run(output, selected):
    output.mkdir(mode=0o700, parents=True)
    started = time.monotonic()
    record = dict(kind='complete-manifest-validation-local-formal', cases={}, commands=[],
                  native_checks='not-run', full_acceptance=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
               LEAN_STACK_SIZE_KB='65536')

    def command(project, folder, name, argv):
        log = folder / (name + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            code = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT, timeout=1800).returncode
        record['commands'].append(dict(argv=argv, cwd=str(project), exit_code=code,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        if code:
            raise ValueError('formal command failed: ' + str(log))

    try:
        snapshots = load('manifest_original_snapshot', ROOT / PROTOCOL / 'replay_formal.py')
        original = load('manifest_original_actual_files', ROOT / PROTOCOL / 'formal.py')
        inventory, base_inventory, mutations = read_inventory(ROOT)
        inputs = snapshots.snapshot()
        record.update(inputs=inputs, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        for case in selected:
            config = inventory['cases'][case]
            base_config = base_inventory['cases'][case]
            folder = output / case
            project = folder / 'source'
            project.mkdir(mode=0o700, parents=True)
            for name, digest in inputs.items():
                destination = project / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, destination)
                if sha(destination) != digest:
                    raise ValueError('source changed while copying: ' + name)
            runtime = load('protocol_stable_tools', ROOT / HERE / 'formal_runtime.py')
            (project / '.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
            (project / '.lake').mkdir()
            # Private worktree dependency cache; cases are sequential under the
            # operator's shared proof lock. Every case has its own .lake/build.
            # The original service checks dependency fingerprints in each case.
            (project / '.lake/packages').symlink_to(runtime.package_root(ROOT), target_is_directory=True)
            (project / 'artifacts/extract-target').mkdir(parents=True)
            expected = dict(inputs)
            if case != 'correct':
                mutation = mutations[case]
                source = project / mutation['source']
                text = source.read_text()
                if text.count(mutation['old']) != 1:
                    raise ValueError('mutation must change exactly one actual Rust expression')
                source.write_text(text.replace(mutation['old'], mutation['new'], 1))
                expected[mutation['source']] = sha(source)
            command(project, folder, 'extract', ['bwrap', '--ro-bind', '/', '/',
                '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/home',
                '--ro-bind', str(Path.home()), str(Path.home()),
                *runtime.tool_bind(ROOT), '--bind', str(project), str(VIRTUAL_ROOT), '--chdir', str(VIRTUAL_ROOT),
                '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL_ROOT / 'artifacts/extract-target'),
                '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS',
                '--', sys.executable, str(PROTOCOL / 'extract.py'), '--profile', 'manifest',
                '--output', 'artifacts/extraction'])
            extraction = project / 'artifacts/extraction'
            extracted = json.loads((extraction / 'result.json').read_text())
            if extracted.get('success') is not True:
                raise ValueError('actual extraction incomplete')
            generated = extraction / 'manifest/generated'
            funs = generated / 'SandproofStage1Manifest/Funs.lean'
            version = inventory['protocol_version']
            if f'def sandproof_protocol.lifecycle.PROTOCOL_VERSION : Std.U16 := {version}#u16' not in funs.read_text():
                raise ValueError('fresh generation does not match the declared protocol version')
            installed = {}
            differences = {}
            for path in generated.rglob('*'):
                if path.is_file() and '_Template' not in path.name:
                    relative = LIBRARY / path.relative_to(generated)
                    before = sha(project / relative)
                    shutil.copy2(path, project / relative)
                    installed[str(relative)] = sha(path)
                    if before != sha(path):
                        differences[str(relative)] = dict(retained=before, fresh=sha(path))
            raw = extraction / 'manifest/original.llbc'
            shutil.copy2(raw, project / LIBRARY / 'original.llbc')
            installed[str(LIBRARY / 'original.llbc')] = sha(raw)
            expected.update(installed)
            adaptation = None
            if case == CAPTURE_CASE:
                adaptation = adapt_capture_helpers(project)
                expected[str(PROOFS / 'ManifestWitnesses.lean')] = adaptation['after']
            # Retained generation is historical (protocol 3). Only these private
            # views receive the complete fresh output; no IR fields are dropped.
            names = config['proofs']
            targets = ['SandboxModel.Lifecycle.Protocol.' + name for name in names]
            targets += ['SandboxModel.Lifecycle.Manifest.' + name for name in base_config['proofs']]
            command(project, folder, 'build', ['lake', 'build', 'SandproofStage1Manifest', *targets])
            seeds = [str(PROOFS / (name + '.lean')) for name in names]
            seeds += [str(BASE_PROOFS / (name + '.lean')) for name in base_config['proofs']]
            seeds += [str(p.relative_to(project)) for p in sorted((project / LIBRARY).rglob('*.lean'))
                      if '_Template' not in p.name]
            if case != 'correct':
                base_directory = PROTOCOL if case == CAPTURE_CASE else BASE
                base_template = ROOT / base_directory / (base_config['template'] + '.lean.in')
                supplement = ROOT / HERE / (config['template'] + '.lean.in')
                relative = 'artifacts/' + config['template'] + '.lean'
                prefix = ('import SandboxModel.Lifecycle.Protocol.ManifestValidationModel\n'
                          if case == CAPTURE_CASE else
                          'import SandboxModel.Lifecycle.Protocol.ManifestValidationLaws\n'
                          'import SandboxModel.Lifecycle.Protocol.ManifestValidationValues\n')
                text = prefix + base_template.read_text() + '\n' + supplement.read_text()
                (project / relative).write_text(text)
                expected[relative] = sha(project / relative)
                seeds.append(relative)
            resolver = load('manifest_original_resolver', project / 'tools/proof_batch.py')
            files = sorted(resolver.closure(project, seeds))
            actual = original.actual_files(project, files, folder, env, command)
            if sorted(actual['audits']) != sorted(config['audits']):
                raise ValueError('exact manifest audit inventory differs: ' + case)
            for name, digest in expected.items():
                if sha(project / name) != digest:
                    raise ValueError('unexpected source/proof/generated change: ' + name)
            contracts = {name: sha(project / name) for name in inventory['contracts']}
            record['cases'][case] = dict(complete=True, **actual, contracts=contracts,
                rust_sha256=sha(project / 'sandbox-protocol/src/lifecycle/manifest.rs'),
                helper_adaptation=adaptation,
                installed_generated=installed, retained_generated_differences=differences,
                extraction_record_sha256=sha(extraction / 'result.json'))
            (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        baseline_contracts = {name: sha(ROOT / name) for name in inventory['contracts']}
        if any(c['contracts'] != baseline_contracts for c in record['cases'].values()):
            raise ValueError('correct and wrong versions used different contracts')
        if snapshots.snapshot() != inputs:
            raise ValueError('source changed during formal checks')
        record['all_cases_complete'] = set(selected) == set(inventory['cases'])
        record['complete'] = True
    except Exception as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    cases = list(json.loads((ROOT / HERE / 'inventory.json').read_text())['cases'])
    parser.add_argument('--case', choices=['all', *cases], default='all')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.resolve(), cases if args.case == 'all' else [args.case])
    print(json.dumps({k: result.get(k) for k in
                     ['kind', 'complete', 'all_cases_complete', 'native_checks', 'full_acceptance', 'seconds', 'error']}))
    raise SystemExit(0 if result['complete'] else 1)
