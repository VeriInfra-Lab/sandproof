"""Isolated TryFrom representation candidate for V0 review, not adoption."""
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
CANDIDATE = HERE / 'conversion-candidate'
PROOFS = Path('SandboxModel/Lifecycle/Protocol')
LIBRARY = Path('verification/aeneas/stage1-protocol/request')
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
NEW = ['CreateConversionRepresentation', 'CreateConversionBindings']


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


def type_shape(llbc):
    """Check only the explicit type shapes used by this reviewed relation.

    This is not an LLBC semantics interpreter or a translation proof. Reject a
    different/interned field shape instead of guessing its interpretation.
    """
    ir = json.loads(llbc.read_text())
    if ir.get('has_errors') is not False:
        raise ValueError('incomplete LLBC')

    def find(name):
        expected = [{'Ident': [part, 0]} for part in name.split('::')]
        found = [d for d in ir['translated']['type_decls'] if d and d['item_meta']['name'] == expected]
        if len(found) != 1:
            raise ValueError('missing or ambiguous actual type: ' + name)
        return found[0]

    error = find('core::num::error::TryFromIntError')
    kind = find('core::num::error::IntErrorKind')
    for item in [error, kind]:
        if item['item_meta']['opacity'] != 'Transparent' or any(item['generics'].values()):
            raise ValueError('unexpected generic/opaque error type')
    fields = error['kind'].get('Struct', [])
    if len(fields) != 1 or fields[0]['name'] != '_0' or fields[0]['is_positional'] is not True:
        raise ValueError('TryFromIntError is not the reviewed single-field struct')
    field_type = fields[0]['ty'].get('Value')
    if not isinstance(field_type, list) or len(field_type) != 2:
        raise ValueError('unexpected field interning; re-review the LLBC')
    adt = field_type[1].get('Adt', {})
    if adt.get('id') != kind['def_id'] or adt.get('builtin') is not None or any(adt['generics'].values()):
        raise ValueError('TryFromIntError field no longer contains exactly IntErrorKind')
    variants = kind['kind'].get('Enum', [])
    expected = ['Empty', 'InvalidDigit', 'PosOverflow', 'NegOverflow', 'Zero', 'NotAPowerOfTwo']
    if [v['name'] for v in variants] != expected or any(v['fields'] for v in variants):
        raise ValueError('IntErrorKind variant set changed')
    return dict(error_def_id=error['def_id'], kind_def_id=kind['def_id'], variants=expected,
        llbc_sha256=sha(llbc), claim='value-field relation only; no memory layout or ABI claim')


def run(output, packages, result_tag):
    output.mkdir(mode=0o700, parents=True)
    started = time.monotonic()
    record = dict(kind='stage1-create-conversion-v0-review-candidate', commands=[],
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
            raise ValueError('candidate command failed: ' + str(log))

    try:
        review = json.loads((ROOT / CANDIDATE / 'review-inputs.json').read_text())
        if review.get('candidate_only') is not True or review.get('v0_adopted') is not False:
            raise ValueError('this entry cannot adopt a new public model')
        base = module('create_value_driver', ROOT / HERE / 'formal.py')
        data = module('create_admission', ROOT / HERE / 'admission.py').inventory(ROOT)
        if sha(result_tag) != review['result_tag']['sha256']:
            raise ValueError('the exact existing #696 Result tag candidate is required')
        backend = ROOT / '.tools/aeneas/backends/lean/Aeneas/Std/Scalar/CoreConvertNum.lean'
        if sha(backend) != review['backend_scalar_model_sha256']:
            raise ValueError('fixed backend scalar model differs')
        external_inputs = {str(result_tag): sha(result_tag), str(backend): sha(backend)}
        source_evidence = output / 'stdlib'
        for toolchain, files in review['stdlib'].items():
            sysroot = Path(subprocess.check_output(['rustup', 'run', toolchain, 'rustc', '--print', 'sysroot'], text=True).strip())
            for name, expected in files.items():
                path = sysroot / 'lib/rustlib/src/rust/library/core/src' / name
                if sha(path) != expected:
                    raise ValueError('reviewed fixed std source differs: ' + str(path))
                external_inputs[str(path)] = expected
                destination = source_evidence / toolchain / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(path, destination)
        record['external_inputs'] = external_inputs
        inputs = base.snapshot()
        record.update(source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(), inputs=inputs)
        project = output / 'source'
        project.mkdir(mode=0o700)
        for name, digest in inputs.items():
            destination = project / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
            if sha(destination) != digest:
                raise ValueError('source changed while copying: ' + name)
        if not packages.is_dir():
            raise ValueError('allocated fixed-tool package cache missing')
        runtime = module('protocol_stable_tools', ROOT / HERE.parent / 'formal_runtime.py')
        (project / '.tools').symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
        (project / '.lake').mkdir()
        (project / '.lake/packages').symlink_to(runtime.package_root(ROOT, packages), target_is_directory=True)
        (project / 'artifacts/extract-target').mkdir(parents=True)
        command(project, output, 'extract', ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev',
            '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', '/home',
            '--ro-bind', str(Path.home()), str(Path.home()), *runtime.tool_bind(ROOT), '--bind', str(project), str(VIRTUAL),
            '--chdir', str(VIRTUAL), '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL / 'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--', sys.executable,
            str(base.EXTRACTOR), '--profile', 'create', '--output', 'artifacts/extraction'])
        extracted = json.loads((project / 'artifacts/extraction/result.json').read_text())
        extractor = module('conversion_original_extractor', project / base.EXTRACTOR)
        if (extracted.get('success') is not True or extracted['profiles']['create'].get('generation_success') is not True
                or extracted['profiles']['create']['roots'] != extractor.PROFILES['create']):
            raise ValueError('incomplete/different actual extraction')
        generated = project / 'artifacts/extraction/create/generated'
        llbc = project / 'artifacts/extraction/create/original.llbc'
        record['type_shape'] = type_shape(llbc)
        installed = {}
        for path in sorted(generated.rglob('*')):
            if path.is_file() and '_Template' not in path.name:
                relative = LIBRARY / path.relative_to(generated)
                shutil.copyfile(path, project / relative)
                installed[str(relative)] = sha(path)
        shutil.copyfile(llbc, project / LIBRARY / 'original.llbc')
        installed[str(LIBRARY / 'original.llbc')] = sha(llbc)
        candidate_inputs = {}
        for name in NEW:
            target = project / PROOFS / (name + '.lean')
            shutil.copyfile(project / CANDIDATE / (name + '.lean.in'), target)
            candidate_inputs[str(target.relative_to(project))] = sha(target)
        target = project / review['result_tag']['path']
        shutil.copyfile(result_tag, target)
        candidate_inputs[str(target.relative_to(project))] = sha(target)
        external = project / LIBRARY / 'SandproofStage1Request/FunsExternal.lean'
        # Change only the handwritten private external environment. The original
        # four function models and all generated bodies retain their exact bytes.
        before = external.read_bytes()
        prefix = ('import SandboxModel.Lifecycle.Protocol.RenewalResultTag\n'
            'import SandboxModel.Lifecycle.Protocol.CreateConversionRepresentation\n').encode()
        external.write_bytes(prefix + before)
        candidate_inputs[str(external.relative_to(project))] = sha(external)
        formal = module('conversion_actual_formal', project / HERE.parent / 'formal.py')
        replay = module('conversion_request_formal', project / HERE.parent / 'replay_formal.py')
        names = list(replay.PROPERTIES['envelope']['positive'])
        audits = list(replay.PROPERTIES['envelope']['audits'])
        for component, inherited in base.INHERITED.items():
            inv = json.loads((project / HERE.parent / component / 'inventory.json').read_text())
            names += inherited
            audits += [a for name in inherited for a in inv['audits'][name]]
        names += base.NEW_PROOFS + NEW + ['RenewalResultTag']
        audits += [a for name in base.NEW_PROOFS for a in data['audits'][name]]
        audits += review['audits'] + ['SandboxModel.Stage1Protocol.RenewalResultTag.' + n for n in ['ok_tag', 'err_tag']]
        command(project, output, 'build', ['lake', 'build', 'SandproofStage1Request',
            *['SandboxModel.Lifecycle.Protocol.' + name for name in names]])
        actual = formal.check_actual(project, names, 'request', output, env, command, audits)
        for name, digest in (inputs | installed | candidate_inputs).items():
            if sha(project / name) != digest:
                raise ValueError('source/proof/generated input changed: ' + name)
        if inputs != base.snapshot() or any(sha(Path(p)) != d for p, d in external_inputs.items()):
            raise ValueError('task/external inputs changed during checking')
        record.update(complete=True, all_candidate_files_complete=True, actual=actual,
            installed_generated=installed, candidate_inputs=candidate_inputs,
            external_prefix_sha256=hashlib.sha256(prefix).hexdigest(),
            retained_external_body_sha256=hashlib.sha256(before).hexdigest())
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
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.packages.absolute(), args.result_tag_source.absolute())
    print(json.dumps({k: result.get(k) for k in ['kind', 'complete', 'candidate_only', 'v0_adopted', 'seconds', 'error']}))
    raise SystemExit(0 if result['complete'] else 1)
