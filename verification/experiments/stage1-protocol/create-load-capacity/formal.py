"""Develop actual Store24 load-to-install capacity proofs and a real Rust counterexample."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback
import importlib.util

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/create-load-capacity')


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


support = module('capacity_current_inputs', ROOT/HERE/'support.py')
checker, resolver, sha = support.load.checker, support.load.resolver, support.sha


def run(output, variant, data, inputs, modes):
    output.mkdir(mode=0o700)
    started = time.monotonic()
    rec = dict(kind='actual-create-load-capacity-development', variant=variant, complete=False,
               capture_complete=False, proof_complete=False, property_rejected=False,
               production_modified=False, private_business_replacement=False,
               generated_bodies_modified=False, new_axioms=False, store_format=24, protocol_version=4,
               native_checks_run=False, formal_acceptance=False, ning_acceptance=False,
               shared_acceptance=False, original_full_attachment_proved=False, full_v3=False, commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true',
               LEAN_NUM_THREADS='4', LEAN_STACK_SIZE_KB='65536')

    def command(project, folder, label, argv):
        begin = time.monotonic()
        log = folder/(label+'.log')
        with log.open('xb') as stream:
            done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec['commands'].append(dict(label=label, argv=argv, cwd=str(project), exit_code=done.returncode,
                                   seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(rec, indent=2)+'\n')
        if done.returncode:
            raise ValueError('command failed; not a property rejection: '+str(log))

    try:
        if support.inventory() != data:
            raise ValueError('reviewed inputs changed')
        capacity = os.statvfs(ROOT)
        available = capacity.f_bavail*capacity.f_frsize
        required = 64*2**30+2*858993459
        rec['resource_admission'] = dict(available_bytes=available, original_floor_bytes=64*2**30,
                                        view_budget_bytes=2*858993459, required_bytes=required,
                                        admitted=available >= required)
        if available < required:
            raise ValueError('original resource floor and declared budget refused before compiler')
        rec.update(inputs=inputs, input_modes=modes,
                   source_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip(),
                   source_tree=subprocess.check_output(['git', 'rev-parse', 'HEAD^{tree}'], cwd=ROOT, text=True).strip())
        project = output/'source'
        project.mkdir(mode=0o700)
        expected = dict(inputs)
        for name, digest in inputs.items():
            target = project/name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT/name, target)
            if sha(target) != digest:
                raise ValueError('source changed while copying: '+name)
        (project/'.tools').symlink_to(support.runtime.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir()
        (project/'.lake/packages').symlink_to(support.runtime.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        rec['private_source_derivation'], changed = support.prepare(project, variant, data)
        expected.update(changed)
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool changed: '+name)
        rec['tools'] = fixed
        recipe = data['variants'][variant]
        admission = support.load.admission
        admission.ROOTS = support.ROOTS
        for label, roots, includes, namespace, destination in [
            ('original', support.ROOTS, support.load.INCLUDE, 'SandproofStage1Request', None),
            ('request', support.ROOTS, support.load.INCLUDE, 'SandproofStage1Request', support.GEN),
            ('primitive', admission.PRIMITIVE_ROOTS, admission.PRIMITIVE_INCLUDE, 'SandproofStage1Graphic', support.GRAPHIC)]:
            ir = Path('artifacts')/(label+'.llbc')
            argv = [str(support.runtime.VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
                    '--error-on-warnings', '--sysroot', 'default']
            for name in includes:
                argv += ['--include', name]
            opaque = ['rusqlite::error::Error']+(['sandproof_protocol::ContractError'] if label == 'request' else [])
            admission.OPAQUE = opaque
            for name in opaque:
                argv += ['--opaque', name]
            for name in roots:
                argv += ['--start-from', name]
            argv += ['--dest-file', str(support.runtime.VIRTUAL/ir), '--', '--manifest-path',
                     str(HERE.parent/'inbox-record/Cargo.toml'), '--lib', '--locked', '--offline', '--target', fixed['target']]
            command(project, output, label+'-charon', support.wrapper(project)+argv)
            raw = json.loads((project/ir).read_text())
            admitted = dict(original_admission=support.load.admit_ir(raw, opaque=opaque, primitive=label == 'primitive'))
            if label != 'primitive':
                admitted['complete_object_roots'] = support.binding.admit(raw)
                admitted['rejected_changed_roots'] = support.binding.rejections(raw)
            if label == 'original':
                enum = support.error_binding.contract_error(raw)
                foundations = json.loads((project/'verification/aeneas/llbc/sandproof_foundations.llbc').read_text())
                if enum != support.error_binding.contract_error(foundations):
                    raise ValueError('complete original ContractError carrier changed')
                rec['complete_original_contract_error'] = enum
                del foundations
            rec.setdefault('captures', {})[label] = dict(path=str(ir), sha256=sha(project/ir), binding=admitted)
            rec.setdefault('result_tags', {})[label] = support.load.library_binding.result_tags(raw)
            del raw
            generated = output/('generated-'+label)
            generated.mkdir()
            command(project, output, label+'-aeneas', [str(ROOT/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
                '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json', '-namespace', namespace+'Rust',
                '-subdir', namespace, '-dest', str(generated), str(project/ir)])
            translated = json.loads((generated/'translation.json').read_text())
            functions = [{k: f[k] for k in ('lean_name', 'rust_name', 'is_opaque', 'lean_file')}
                         for f in translated['functions']]
            if functions != recipe['generated_functions'][label]:
                raise ValueError('transparent/external generated closure changed: '+label)
            for name, digest in recipe['generated_raw_files'][label].items():
                if sha(generated/namespace/name) != digest:
                    raise ValueError('unmodified actual generated bytes differ: '+label+'/'+name)
            rec.setdefault('translation', {})[label] = translated
            rec.setdefault('raw_generated', {})[label] = {str(p.relative_to(generated)): sha(p)
                for p in generated.rglob('*') if p.is_file()}
            if destination is not None:
                (project/destination).mkdir(parents=True, exist_ok=True)
                for name in ('Types.lean', 'Funs.lean'):
                    shutil.copy2(generated/namespace/name, project/destination/name)
                    expected[str(destination/name)] = sha(project/destination/name)
        rec['capture_complete'] = True
        installed, rec['deduplicated_providers'] = support.install(project, variant, data)
        expected.update(installed)
        seeds = [n for n in installed if n.endswith('.lean')]
        files = sorted(resolver.closure(project, seeds, active_view='lakefile.toml'))
        audits = [a for name in files for a in re.findall(r'^#audit_controller_proof (\S+)$', (project/name).read_text(), re.M)]
        if files != recipe['expected_files'] or audits != recipe['expected_audits'] or len(audits) != len(set(audits)):
            raise ValueError('missing, duplicate or changed physical file/audit inventory')
        if variant != 'actual' and any(Path(n).stem in support.FORBIDDEN_WRONG for n in files):
            raise ValueError('wrong view imports a correct full-validator/load correspondence')
        rec.update(files={n: sha(project/n) for n in files}, audits=audits)
        command(project, output, 'build', ['lake', 'build', *['SandboxModel.Lifecycle.Protocol.'+n for n in recipe['seeds']]])
        physical = output/'actual-files'
        physical.mkdir()
        rec['actual'] = checker.actual_files(project, files, physical, env, command)
        if rec['actual']['files'] != rec['files'] or rec['actual']['audits'] != audits:
            raise ValueError('strict checker inventory differs')
        if any(sha(project/n) != h for n, h in expected.items()):
            raise ValueError('private checked source, generation or proof changed')
        if any(('100755' if (project/n).stat().st_mode & 0o111 else '100644') != m for n, m in modes.items()):
            raise ValueError('private original input modes changed')
        rec['private_derived_inputs'] = {n: h for n, h in expected.items() if inputs.get(n) != h}
        rec.update(complete=True, proof_complete=True, property_rejected=variant != 'actual')
    except BaseException as error:
        rec.update(error=repr(error), traceback=traceback.format_exc())
    finally:
        rec['original_inputs_unchanged'] = support.load.base.snapshot(ROOT) == inputs
        rec['original_modes_unchanged'] = all(('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') == m for n, m in modes.items())
        if not rec['original_inputs_unchanged'] or not rec['original_modes_unchanged']:
            rec.update(complete=False, proof_complete=False, property_rejected=False)
        rec['execution_seconds'] = time.monotonic()-started
        (output/'result.json').write_text(json.dumps(rec, indent=2)+'\n')
    return rec


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--variant', choices=('all', 'actual', 'wrong-owner-limit'), default='all')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--lock', type=Path, required=True, help='existing shared proof lock')
    args = parser.parse_args()
    os.umask(0o077)
    output = args.output.absolute()
    output.mkdir(parents=True, mode=0o700)
    batch = dict(kind='actual-create-load-capacity-development-batch', complete=False, variants={},
                 native_checks_run=False, formal_acceptance=False, ning_acceptance=False,
                 shared_acceptance=False, original_full_attachment_proved=False, full_v3=False)
    waiting = time.monotonic()
    try:
        with args.lock.open('r+') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            batch['lock_wait_seconds'] = time.monotonic()-waiting
            started = time.monotonic()
            data = support.inventory()
            resolver.ROOT, resolver.MODULES = ROOT, ('stage1-protocol',)
            os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(output/'runtime-cache'))
            profile = resolver.runtime()
            registry = support.load.native.registry_module.registry()
            inputs = support.load.base.snapshot(ROOT)
            modes = {n: ('100755' if (ROOT/n).stat().st_mode & 0o111 else '100644') for n in inputs}
            for variant in data['variants'] if args.variant == 'all' else [args.variant]:
                result = run(output/variant, variant, data, inputs, modes)
                batch['variants'][variant] = dict(proof_complete=result['proof_complete'], property_rejected=result['property_rejected'],
                                                 result_sha256=sha(output/variant/'result.json'))
                if not result['proof_complete']:
                    raise ValueError('development view failed: '+variant)
            if resolver.runtime() != profile or support.load.native.registry_module.registry() != registry:
                raise ValueError('fixed runtime or Cargo source changed')
            batch.update(complete=True, actual_runtime=profile, registry=registry, execution_seconds=time.monotonic()-started)
    except BaseException as error:
        batch.update(error=repr(error), traceback=traceback.format_exc())
    (output/'result.json').write_text(json.dumps(batch, indent=2)+'\n')
    print(json.dumps({k: batch.get(k) for k in ('complete', 'lock_wait_seconds', 'execution_seconds', 'error')}))
    raise SystemExit(not batch['complete'])
