"""Actual Create guard and four Rust counterexamples in the unadopted V0 candidate."""
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
CANDIDATE = HERE / 'guard-candidate'
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
LIBRARY = Path('verification/aeneas/stage1-protocol/request')
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
POSITIVE = ['CreateObservationBindings', 'CreateObservationExamples']
CASES = ('correct', 'create-response-key-only', 'create-current-retention-omitted',
         'create-refused-after-success', 'create-renewal-negative-wrap')


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def inventory(root):
    data = json.loads((root / CANDIDATE / 'inventory.json').read_text())
    parent = module('create_guard_parent_admission', root / HERE / 'admission.py').inventory(root)
    if (data.get('schema') != 1 or data.get('candidate_only') is not True
            or data.get('v0_adopted') is not False or set(data['mutations']) != set(CASES[1:])):
        raise ValueError('incomplete/unadopted guard candidate registration')
    templates = {p.name.removesuffix('.lean.in') for p in (root / CANDIDATE).glob('*.lean.in')}
    expected = set(POSITIVE) | set(data['mutations'].values())
    if templates != expected or set(data['audits']) != expected:
        raise ValueError('unregistered/missing guard proof or audit inventory')
    all_audits = []
    for name in sorted(expected):
        found = re.findall(r'^#audit_controller_proof (\S+)$',
            (root / CANDIDATE / (name + '.lean.in')).read_text(), re.M)
        if found != data['audits'][name] or not found:
            raise ValueError('guard audit declaration differs: ' + name)
        all_audits += found
    if len(all_audits) != len(set(all_audits)):
        raise ValueError('duplicate guard proof audit')
    for name, digest in data['unchanged_inputs'].items():
        if sha(root / name) != digest:
            raise ValueError('same contract, witnesses or conversion candidate changed: ' + name)
    extractor = module('create_guard_extractor_inventory', root / HERE.parent / 'observation-bindings/extract.py')
    if extractor.PROFILES['create'] != data['roots']:
        raise ValueError('actual Create extraction roots changed')
    return data, parent


def mutate(project, mutation):
    path = project / mutation['source']
    before = path.read_text()
    if before.count(mutation['old']) != 1:
        raise ValueError('mutation must identify one actual Rust expression')
    after = before.replace(mutation['old'], mutation['new'], 1)
    if after == before:
        raise ValueError('mutation did not change actual Rust')
    path.write_text(after)
    return dict(source=mutation['source'], before_sha256=hashlib.sha256(before.encode()).hexdigest(),
                after_sha256=sha(path))


def run(output, packages, result_tag, result_bindings, selected):
    output.mkdir(mode=0o700, parents=True)
    started = time.monotonic()
    record = dict(kind='stage1-create-guard-v0-review-candidate', commands=[], cases={},
        candidate_only=True, v0_adopted=False, full_guard_refinement=False,
        formal_mutant_checks=False, full_v3_proof=False, full_acceptance=False,
        native_acceptance='pending ning; not supplied by this formal candidate')
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')

    def command(project, folder, name, argv):
        log = folder / (name + '.log')
        begin = time.monotonic()
        with log.open('xb') as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(argv=argv, cwd=str(project), exit_code=result.returncode,
            seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if result.returncode:
            raise ValueError('guard candidate command failed: ' + str(log))

    try:
        if not selected or len(selected) != len(set(selected)) or not set(selected) <= set(CASES):
            raise ValueError('invalid selected cases')
        data, parent = inventory(ROOT)
        conversion = module('create_guard_conversion', ROOT / HERE / 'conversion_candidate.py')
        review = json.loads((ROOT / conversion.CANDIDATE / 'review-inputs.json').read_text())
        base = module('create_guard_values', ROOT / HERE / 'formal.py')
        if sha(result_tag) != review['result_tag']['sha256'] or sha(result_bindings) != data['result_bindings']['sha256']:
            raise ValueError('exact existing #696 tag and is_err proofs required')
        backend = ROOT / '.tools/aeneas/backends/lean/Aeneas/Std/Scalar/CoreConvertNum.lean'
        if sha(backend) != review['backend_scalar_model_sha256']:
            raise ValueError('fixed backend scalar model differs')
        external_inputs = {str(p): sha(p) for p in (result_tag, result_bindings, backend)}
        for toolchain, files in review['stdlib'].items():
            sysroot = Path(subprocess.check_output(['rustup', 'run', toolchain, 'rustc', '--print', 'sysroot'], text=True).strip())
            for name, digest in files.items():
                path = sysroot / 'lib/rustlib/src/rust/library/core/src' / name
                if sha(path) != digest:
                    raise ValueError('fixed Std source differs: ' + str(path))
                external_inputs[str(path)] = digest
                target = output / 'stdlib' / toolchain / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(path, target)
        record['external_inputs'] = external_inputs
        inputs = base.snapshot()
        record.update(source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(), inputs=inputs)
        if not packages.is_dir():
            raise ValueError('allocated fixed-tool package cache missing')
        for case in selected:
            folder = output / case
            project = folder / 'source'
            project.mkdir(mode=0o700, parents=True)
            for name, digest in inputs.items():
                target = project / name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / name, target)
                if sha(target) != digest:
                    raise ValueError('source changed while copying: ' + name)
            expected = dict(inputs)
            mutation = None
            if case != 'correct':
                mutation = mutate(project, parent['mutations'][case])
                expected[mutation['source']] = mutation['after_sha256']
            runtime = module('protocol_stable_tools', ROOT / HERE.parent / 'formal_runtime.py')
            (project / '.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
            (project / '.lake').mkdir()
            (project / '.lake/packages').symlink_to(packages, target_is_directory=True)
            (project / 'artifacts/extract-target').mkdir(parents=True)
            command(project, folder, 'extract', ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev',
                '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/home',
                '--ro-bind', str(Path.home()), str(Path.home()), *runtime.tool_bind(ROOT), '--bind', str(project), str(VIRTUAL),
                '--chdir', str(VIRTUAL), '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL / 'artifacts/extract-target'),
                '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
                '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--', sys.executable,
                str(base.EXTRACTOR), '--profile', 'create', '--output', 'artifacts/extraction'])
            extracted = json.loads((project / 'artifacts/extraction/result.json').read_text())
            if (extracted.get('success') is not True or extracted['profiles']['create'].get('generation_success') is not True
                    or extracted['profiles']['create']['roots'] != data['roots']):
                raise ValueError('incomplete/different actual extraction')
            generated = project / 'artifacts/extraction/create/generated'
            llbc = project / 'artifacts/extraction/create/original.llbc'
            shape = conversion.type_shape(llbc)
            installed = {}
            for path in sorted(generated.rglob('*')):
                if path.is_file() and '_Template' not in path.name:
                    relative = LIBRARY / path.relative_to(generated)
                    shutil.copyfile(path, project / relative)
                    installed[str(relative)] = sha(path)
            shutil.copyfile(llbc, project / LIBRARY / 'original.llbc')
            installed[str(LIBRARY / 'original.llbc')] = sha(llbc)
            candidate_inputs = {}
            for name in conversion.NEW:
                target = project / PROOFS / (name + '.lean')
                shutil.copyfile(project / conversion.CANDIDATE / (name + '.lean.in'), target)
                candidate_inputs[str(target.relative_to(project))] = sha(target)
            for source, info in [(result_tag, review['result_tag']), (result_bindings, data['result_bindings'])]:
                target = project / info['path']
                shutil.copyfile(source, target)
                candidate_inputs[str(target.relative_to(project))] = sha(target)
            external = project / LIBRARY / 'SandproofStage1Request/FunsExternal.lean'
            before = external.read_bytes()
            prefix = ('import SandboxModel.Lifecycle.Protocol.RenewalResultTag\n'
                'import SandboxModel.Lifecycle.Protocol.CreateConversionRepresentation\n').encode()
            external.write_bytes(prefix + before)
            candidate_inputs[str(external.relative_to(project))] = sha(external)
            formal = module('create_guard_original_formal', project / HERE.parent / 'formal.py')
            replay = module('create_guard_original_replay', project / HERE.parent / 'replay_formal.py')
            names = list(replay.PROPERTIES['envelope']['positive'])
            audits = list(replay.PROPERTIES['envelope']['audits'])
            for component, inherited in base.INHERITED.items():
                inv = json.loads((project / HERE.parent / component / 'inventory.json').read_text())
                for name in inherited:
                    found = re.findall(r'^#audit_controller_proof (\S+)$', (project / PROOFS / (name + '.lean')).read_text(), re.M)
                    if found != inv['audits'][name]:
                        raise ValueError('inherited audit inventory differs: ' + name)
                names += inherited
                audits += [a for name in inherited for a in inv['audits'][name]]
            names += base.NEW_PROOFS + conversion.NEW + ['RenewalResultTag', 'RenewalResultBindings']
            audits += [a for name in base.NEW_PROOFS for a in parent['audits'][name]]
            audits += review['audits'] + ['SandboxModel.Stage1Protocol.RenewalResultTag.' + n for n in ['ok_tag', 'err_tag', 'actual_is_err']]
            extras = []
            targets = POSITIVE if case == 'correct' else [data['mutations'][case]]
            for name in targets:
                relative = (PROOFS if case == 'correct' else Path('artifacts')) / (name + '.lean')
                shutil.copyfile(project / CANDIDATE / (name + '.lean.in'), project / relative)
                candidate_inputs[str(relative)] = sha(project / relative)
                audits += data['audits'][name]
                if case == 'correct':
                    names.append(name)
                else:
                    extras.append(str(relative))
            command(project, folder, 'build', ['lake', 'build', 'SandproofStage1Request',
                *['SandboxModel.Lifecycle.Protocol.' + name for name in names]])
            actual = formal.check_actual(project, names, 'request', folder, env, command, audits, extras)
            for name, digest in (expected | installed | candidate_inputs).items():
                if sha(project / name) != digest:
                    raise ValueError('source/proof/generated input changed: ' + name)
            record['cases'][case] = dict(complete=True, **actual, mutation=mutation, type_shape=shape,
                installed_generated=installed, candidate_inputs=candidate_inputs,
                retained_external_body_sha256=hashlib.sha256(before).hexdigest(),
                contract_sha256=sha(project / PROOFS / 'CreateObservationModel.lean'),
                witnesses_sha256=sha(project / PROOFS / 'CreateObservationValues.lean'))
            (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if inputs != base.snapshot() or any(sha(Path(p)) != d for p, d in external_inputs.items()):
            raise ValueError('task/external inputs changed during checking')
        for field in ('contract_sha256', 'witnesses_sha256'):
            if len({row[field] for row in record['cases'].values()}) != 1:
                raise ValueError('correct and wrong Rust must have identical contracts and witnesses')
        record.update(complete=True, all_cases_complete=set(selected) == set(CASES),
            full_guard_refinement='correct' in selected,
            formal_mutant_checks=set(CASES[1:]) <= set(selected))
    except BaseException as error:
        record.update(complete=False, error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-started
    (output / 'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--packages', required=True, type=Path)
    parser.add_argument('--result-tag-source', type=Path,
        default=ROOT / PROOFS / 'RenewalResultTag.lean')
    parser.add_argument('--result-bindings-source', type=Path,
        default=ROOT / PROOFS / 'RenewalResultBindings.lean')
    parser.add_argument('--case', choices=['all', *CASES], default='all')
    args = parser.parse_args()
    os.umask(0o077)
    selected = list(CASES) if args.case == 'all' else [args.case]
    result = run(args.output.absolute(), args.packages.absolute(), args.result_tag_source.absolute(),
                 args.result_bindings_source.absolute(), selected)
    print(json.dumps({k: result.get(k) for k in ['kind', 'complete', 'all_cases_complete', 'candidate_only', 'v0_adopted', 'seconds', 'error']}))
    raise SystemExit(0 if result['complete'] else 1)
