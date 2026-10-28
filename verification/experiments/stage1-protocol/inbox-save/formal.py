"""Re-extract actual save candidates and wrong Rust under one complete contract."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path('verification/experiments/stage1-protocol/inbox-save')
PARENT = HERE.parent / 'inbox-record'
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
MODEL = 'controller-service/src/controller/lifecycle_inbox/codec.rs'
GENERATED = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')



def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('record_formal_sources', ROOT / HERE.parent / 'schema-leaves/formal.py')
checker = module('record_actual_checker', ROOT / HERE.parent / 'formal.py')
native = module('record_native_inputs', ROOT / HERE / 'native.py')
resolver = module('record_existing_closure', ROOT / 'tools/proof_batch.py')
library_binding = module('saved_existing_library_binding', ROOT/HERE/'library_binding.py')
sha = base.sha


admission = module('inbox_composition_admission', ROOT/HERE/'admission.py')
admit_ir, admit_generated = admission.admit_ir, admission.admit_generated
ROOTS, INCLUDE = admission.ROOTS, admission.INCLUDE
GRAPHIC = library_binding.GRAPHIC


def check_inventory(root):
    inventory = json.loads((root/HERE/'inventory.json').read_text())
    observed = {p.name.removesuffix('.lean.in'):re.findall(r'^#audit_controller_proof (\S+)$',p.read_text(),re.M)
        for p in sorted((root/HERE).glob('*.lean.in')) if '#audit_controller_proof ' in p.read_text()}
    if observed != {k:v for k,v in inventory['audits'].items() if v}:
        raise ValueError('missing or changed Inbox proof inventory')
    for stem,provider in inventory['providers'].items():
        if sha(root/provider['path']) != provider['installed_sha256']:
            raise ValueError('proof/provider content differs: '+stem)
    return inventory


def run(output, variant, require_native=False, development_native=None, prepare_only=False):
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='inbox-save-formal-development', variant=variant, complete=False,
                  ning_acceptance=False, shared_acceptance=False, full_v3=False,
                  production_adopted=True, private_refactor=False, original_equivalence_proved=False,
                  standard_provider_connection_proved=False, new_external_symbols=[], independently_extracted_primitives=['graphic_bytes','Result.is_err'], commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='4',
               LEAN_STACK_SIZE_KB='65536', CARGO_NET_OFFLINE='true')

    def command(project, folder, label, argv):
        begin = time.monotonic()
        log = folder/(label+'.log')
        with log.open('xb') as stream:
            done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(label=label, argv=argv, cwd=str(project),
            exit_code=done.returncode, seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output/'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if done.returncode:
            raise ValueError('command failed: '+str(log))

    try:
        native_result = None
        if development_native is not None:
            if require_native:
                raise ValueError('development evidence cannot substitute for a ning receipt')
            native_result = json.loads(development_native.read_text())
            record['development_native'] = dict(path=str(development_native), sha256=sha(development_native))
        if require_native:
            if prepare_only:
                raise ValueError('preparation is not formal acceptance')
            sys.path.insert(0, str(ROOT))
            from tools.acceptance_receipts import validate_native, native_case, digest
            validate_native(Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']))
            native_case('functional', ROOT, None)
            receipt = native_case('stage1-inbox-save', ROOT, None)
            record['native_receipt'] = receipt
            evidence = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])/'commands'/digest(receipt['key'])/'evidence'
            native_result = json.loads((evidence/'result.json').read_text())
        if native_result is not None and (native_result.get('complete') is not True
                or native_result.get('kind') != 'inbox-save-native-development'
                or set(native_result.get('variants', {})) != set(native.preparation.VARIANTS)):
            raise ValueError('incomplete native inbox-save evidence')
        resolver.ROOT, resolver.MODULES = ROOT, ('stage1-protocol',)
        os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(ROOT/'artifacts/inbox-save/runtime-cache'))
        runtime = resolver.runtime()
        record['runtime'] = runtime
        inventory = check_inventory(ROOT)
        for name, digest in inventory['production_inputs'].items():
            if sha(ROOT/name) != digest:
                raise ValueError('fixed production input differs: '+name)
        record['library_binding'] = library_binding.check(ROOT)
        inputs, libraries = base.snapshot(ROOT), native.registry_module.registry()
        if native_result is not None and (native_result['inputs'] != inputs or native_result['registry'] != libraries):
            raise ValueError('native and formal source/registry inputs differ')
        record.update(inputs=inputs, registry=libraries, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        fixed = json.loads((ROOT/'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT/'.tools/aeneas'/suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool differs: '+name)
        record['tools'] = fixed
        project = output/'source'
        project.mkdir(mode=0o700)
        expected = dict(inputs)
        for name, digest in inputs.items():
            path = project/name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT/name, path)
            if sha(path) != digest:
                raise ValueError('source changed while copying: '+name)
        (project/'.tools').symlink_to(ROOT/'.tools', target_is_directory=True)
        (project/'.lake').mkdir()
        (project/'.lake/packages').symlink_to(ROOT/'.lake/packages', target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        native_variant = 'candidate' if variant == 'correct' else variant
        changes, preparation = native.preparation.prepare(project, native_variant)
        mutation = preparation['mutation']
        record['preparation'] = preparation
        record.update(business_inputs=changes, mutation=mutation)
        expected.update(changes)
        if native_result is not None:
            for checked_variant in (native_variant,):
                if any(native_result['variants'][checked_variant]['inputs'].get(n) != d for n, d in changes.items()):
                    raise ValueError('native and formal business input differs')
            if mutation is not None and native_result['mutations'].get(native_variant) != mutation:
                raise ValueError('native and formal mutation differ')
        wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
            '--bind', str(project), str(VIRTUAL), '--chdir', str(VIRTUAL),
            '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL/'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '4',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
        generated_views = {}
        for label, roots, includes, namespace, destination in [
                ('request', ROOTS, INCLUDE, 'SandproofStage1Request', GENERATED),
                ('primitive', admission.PRIMITIVE_ROOTS, admission.PRIMITIVE_INCLUDE,
                 'SandproofStage1Graphic', GRAPHIC)]:
            raw_path = Path('artifacts')/(label+'.llbc')
            args = [str(VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
                    '--error-on-warnings', '--sysroot', 'default']
            for name in includes: args += ['--include', name]
            for name in admission.OPAQUE: args += ['--opaque', name]
            for name in roots: args += ['--start-from', name]
            args += ['--dest-file', str(VIRTUAL/raw_path), '--', '--manifest-path', str(PARENT/'Cargo.toml'),
                     '--lib', '--locked', '--offline', '--target', fixed['target']]
            command(project, output, label+'-charon', wrapper+args)
            raw = json.loads((project/raw_path).read_text())
            record.setdefault('llbc', {})[label] = dict(path=str(raw_path), sha256=sha(project/raw_path),
                binding=admit_ir(raw, primitive=label=='primitive'))
            record.setdefault('result_tags', {})[label] = library_binding.result_tags(raw)
            generated = output/('generated-'+label)
            generated.mkdir()
            command(project, output, label+'-aeneas', [str(project/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
                '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
                '-namespace', namespace+'Rust', '-subdir', namespace, '-dest', str(generated), str(project/raw_path)])
            generated_variant = 'primitive' if label=='primitive' else variant
            record.setdefault('translation', {})[label] = admit_generated(generated, generated_variant)
            command(project, output, label+'-admission-regressions', [sys.executable, str(HERE/'admission_tests.py'),
                '--llbc', str(raw_path), '--generated', str(generated), '--variant', generated_variant])
            record.setdefault('raw_generated', {})[label] = {str(p.relative_to(generated)):sha(p)
                for p in generated.rglob('*') if p.is_file()}
            (project/destination).mkdir(parents=True,exist_ok=True)
            for name in ('Types.lean', 'Funs.lean'):
                shutil.copy2(generated/namespace/name,project/destination/name)
                expected[str(destination/name)] = sha(project/destination/name)
            generated_views[label] = (generated,namespace,destination)
        expected.update(library_binding.install(ROOT,project))
        # Private project registration only; the public Lake patch belongs to the coordinator.
        lake = project/'lakefile.toml'
        if 'name = "SandproofStage1Graphic"' not in lake.read_text():
            with lake.open('a') as stream:
                stream.write('\n[[lean_lib]]\nname = "SandproofStage1Graphic"\nsrcDir = "verification/aeneas/stage1-protocol/graphic"\n')
            expected['lakefile.toml'] = sha(lake)
        stems = inventory['views'][variant]['stems']
        for name in inventory['views'][variant]['files']:
            if not name.startswith('SandboxModel/Lifecycle/Protocol/'): continue
            provider = inventory['providers'][Path(name).stem]
            shutil.copy2(ROOT/provider['path'],project/name)
            expected[name] = sha(project/name)
        if prepare_only:
            record['prepared'] = True
        else:
            command(project, output, 'build', ['lake', 'build',
                *['SandboxModel.Lifecycle.Protocol.'+stem for stem in stems]])
            files = sorted(resolver.closure(project, [
                *['SandboxModel/Lifecycle/Protocol/'+stem+'.lean' for stem in stems]]))
            if files != inventory['views'][variant]['files']:
                raise ValueError('actual import closure differs from reviewed inventory')
            forbidden = ['InboxSaveProofs','InboxSaveBoundaries','InboxSaveWitnesses']
            if variant != 'correct' and any('SandboxModel/Lifecycle/Protocol/'+n+'.lean' in files for n in forbidden):
                raise ValueError('wrong view imports a correct proof for its mutated save body')
            for generated,namespace,destination in generated_views.values():
                for name in ('Types.lean','Funs.lean'):
                    if (project/destination/name).read_bytes() != (generated/namespace/name).read_bytes():
                        raise ValueError('actual generated body changed')
            actual = checker.actual_files(project, files, output, env, command)
            declared = [n for f in files for n in re.findall(r'^#audit_controller_proof (\S+)$', (project/f).read_text(), re.M)]
            if sorted(actual['audits']) != sorted(declared):
                raise ValueError('incomplete audits')
            theorem = 'SandboxModel.Stage1Protocol.InboxSave.' + {
                'correct':'full_save_contract',
                'wrong-validation':'Values.wrong_validation_contract_rejected',
                'wrong-budget':'Values.wrong_budget_contract_rejected',
                'wrong-zero-row':'Values.wrong_zero_rows_contract_rejected',
                'wrong-hash':'Values.wrong_hash_contract_rejected',
            }[variant]
            if theorem not in actual['audits']:
                raise ValueError('complete shared contract theorem was not audited')
            record['actual'] = actual
            record['contract_theorem'] = theorem
        if any(sha(project/n) != d for n, d in expected.items()):
            raise ValueError('checked input changed')
        if inputs != base.snapshot(ROOT) or libraries != native.registry_module.registry() or resolver.runtime() != runtime:
            raise ValueError('source/tool/service/registry changed during checking')
        if record['library_binding'] != library_binding.check(ROOT):
            raise ValueError('library binding inputs changed during checking')
        check_inventory(ROOT)
        record['complete'] = not prepare_only
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output/'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=['correct', *native.preparation.FAILURES], default='correct')
    parser.add_argument('--require-native', action='store_true')
    parser.add_argument('--development-native', type=Path)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.variant, args.require_native, args.development_native, args.prepare_only)
    print(json.dumps({k: result.get(k) for k in ('variant', 'complete', 'prepared', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] or result.get('prepared') and 'error' not in result else 1)
