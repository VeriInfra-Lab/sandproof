"""Re-extract original execution lineage and check the same full contract in every view."""
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
HERE = Path('verification/experiments/stage1-protocol/inbox-lineage')
PARENT = HERE.parent / 'inbox-record'
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
GENERATED = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
ROOTS = ['sandproof_controller_service::controller::lifecycle_inbox::model::proof_execution_lineage',
         *['crate::' + name for name in ('request_validate', 'replay_matches', 'accepted_validate', 'recorded_matches')]]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('record_formal_sources', ROOT / HERE.parent / 'schema-leaves/formal.py')
checker = module('record_actual_checker', ROOT / HERE.parent / 'formal.py')
native = module('record_native_inputs', ROOT / HERE / 'native.py')
resolver = module('record_existing_closure', ROOT / 'tools/proof_batch.py')
binding = module('lineage_enum_binding', ROOT / HERE.parent / 'envelope-writer/binding.py')
library_binding = module('lineage_library_binding', ROOT / HERE / 'library_binding.py')
sha = base.sha
journal_binding = module('current_journal_binding', ROOT / HERE.parent / 'inbox-lookup/journal_binding.py')
native_admission = module('inbox_native_admission', ROOT / HERE.parent / 'inbox-lookup/native_admission.py')
execution = module('inbox_stable_tools', ROOT / HERE.parent / 'formal_runtime.py')


def admit_ir(raw, opaque):
    if raw.get('has_errors') is not False:
        raise ValueError('incomplete Charon extraction')
    journal_binding.admit(raw)
    translated = raw['translated']
    options = translated['options']
    if (options['start_from'] != ROOTS or options['include'] != [
            'sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service']
            or options['opaque'] != opaque or options['exclude'] or options['skip_borrowck']
            or options['no_typecheck'] or options['monomorphize']):
        raise ValueError('unreviewed extraction options')
    if {t['key'] for t in translated['target_information']} != {'x86_64-unknown-linux-gnu'}:
        raise ValueError('wrong target')
    functions = [f for f in translated['fun_decls'] if f]
    roots = [f for f in functions if f['item_meta']['name'][-1] == {'Ident': ['proof_execution_lineage', 0]}]
    if len(roots) != 1 or roots[0]['item_meta']['opacity'] != 'Transparent':
        raise ValueError('missing transparent forwarding root')
    calls = []
    def visit(node):
        if isinstance(node, dict):
            if 'Call' in node: calls.append(node['Call']['call'])
            for value in node.values(): visit(value)
        elif isinstance(node, list):
            for value in node: visit(value)
    visit(roots[0]['body'])
    if len(calls) != 1:
        raise ValueError('forwarding root must have exactly one actual call')
    callee_id = calls[0]['func']['Regular']['kind']['Fun']
    callee = next(f for f in functions if f['def_id'] == callee_id)
    if (callee['item_meta']['name'][-1] != {'Ident': ['validate_execution_lineage', 0]}
            or callee['item_meta']['name'][:4] != [
                {'Ident': [n, 0]} for n in ('sandproof_controller_service', 'controller', 'lifecycle_inbox', 'model')]
            or callee['item_meta']['opacity'] != 'Transparent' or 'Structured' not in callee['body']):
        raise ValueError('forwarding root does not call the actual transparent lineage method')
    return dict(root=roots[0], call=calls[0], callee=callee)


def admit_generated(directory):
    translation = json.loads((directory / 'translation.json').read_text())
    functions = translation['functions']
    required = {'sandproof_controller_service.controller.lifecycle_inbox.model.Session.'+name
                for name in ('validate_execution_lineage', 'validate_execution_lineage_loop',
                             'validate_execution_lineage_loop.body',
                             'validate_execution_lineage.closure.Insts.CoreOpsFunctionFnOnceTupleSharedEntryBool.call_once')}
    required |= {'request_validate', 'replay_matches', 'accepted_validate', 'recorded_matches'}
    actual = {f['lean_name'].removeprefix('SandproofStage1RequestRust.') for f in functions
              if f['is_opaque'] is False and f['lean_file'] == 'SandproofStage1Request/Funs.lean'}
    if not required <= actual:
        raise ValueError('actual generated root/body missing: ' + repr(sorted(required - actual)))
    external = {f['lean_name'] for f in functions if f['is_opaque']}
    expected = {'Bool.Insts.CoreCmpPartialEqBool.ne', 'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
                'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.vec.Vec.is_empty',
                'core.option.Option.is_none_or', 'core.result.Result.ok',
                'SharedAVec.Insts.CoreIterTraitsCollectIntoIteratorSharedATIter.into_iter'}
    if external != expected:
        raise ValueError('unreviewed function model or business opacity')
    return translation


def run(output, variant, require_native=False, development_native=None, prepare_only=False):
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='inbox-lineage-formal-development', variant=variant, complete=False,
                  ning_acceptance=False, shared_acceptance=False, full_v3=False,
                  production_refactor=False, borrowed_vec_binding_adopted=False, commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='2',
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
            receipt = native_case('stage1-inbox-lineage', ROOT, None)
            record['native_receipt'] = receipt
            evidence = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])/'commands'/digest(receipt['key'])/'evidence'
            native_result = json.loads((evidence/'result.json').read_text())
        if native_result is not None and (native_result.get('complete') is not True
                or native_result.get('kind') != 'inbox-lineage-native-development'
                or set(native_result.get('variants', {})) != set(native.preparation.VARIANTS)):
            raise ValueError('incomplete native lineage evidence')
        resolver.ROOT, resolver.MODULES = ROOT, ('stage1-protocol',)
        os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(ROOT/'artifacts/inbox-lineage/runtime-cache'))
        runtime = resolver.runtime()
        record['runtime'] = runtime
        inventory = json.loads((ROOT/HERE/'inventory.json').read_text())
        observed = {p.name.removesuffix('.lean.in'): re.findall(
            r'^#audit_controller_proof (\S+)$', p.read_text(), re.M)
            for p in sorted((ROOT/HERE).glob('InboxLineage*.lean.in'))}
        audits = [name for names in observed.values() for name in names]
        if observed != inventory['audits'] or len(audits) != len(set(audits)):
            raise ValueError('missing or changed lineage proof inventory')
        for name, digest in inventory['production_inputs'].items():
            if sha(ROOT/name) != digest:
                raise ValueError('fixed production input differs: '+name)
        record['library_binding'] = library_binding.check(ROOT)
        inputs, libraries = base.snapshot(ROOT), native.registry_module.registry()
        native_admission.admit(record, native_result, inputs, libraries, ROOT,
                               native.registry_module.HERE / 'Cargo.lock')
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
        (project/'.tools').symlink_to(execution.tool_root(ROOT), target_is_directory=True)
        (project/'.lake').mkdir()
        (project/'.lake/packages').symlink_to(execution.package_root(ROOT), target_is_directory=True)
        (project/'artifacts/extract-target').mkdir(parents=True)
        native_variant = 'original' if variant == 'correct' else variant
        changes, mutation = native.preparation.prepare(project, native_variant)
        record.update(business_inputs=changes, mutation=mutation)
        expected.update(changes)
        if native_result is not None:
            for checked_variant in (('original', 'restored') if variant == 'correct' else (native_variant,)):
                if any(native_result['variants'][checked_variant]['inputs'].get(n) != d for n, d in changes.items()):
                    raise ValueError('native and formal business input differs')
            if mutation is not None and native_result['mutations'].get(native_variant) != mutation:
                raise ValueError('native and formal mutation differ')
        wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()), *execution.tool_bind(ROOT),
            '--bind', str(project), str(VIRTUAL), '--chdir', str(VIRTUAL),
            '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL/'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '2',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
        for alias, label in [(False, 'unaliased'), (True, 'aliased')]:
            raw_path = Path('artifacts')/(label+'.llbc')
            opaque = ['rusqlite::error::Error']+(['sandproof_protocol::ContractError'] if alias else [])
            args = [str(VIRTUAL/'.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
                    '--error-on-warnings', '--sysroot', 'default']
            for name in ('sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service'):
                args += ['--include', name]
            for name in opaque:
                args += ['--opaque', name]
            for name in ROOTS:
                args += ['--start-from', name]
            args += ['--dest-file', str(VIRTUAL/raw_path), '--', '--manifest-path', str(PARENT/'Cargo.toml'),
                     '--lib', '--locked', '--offline', '--target', fixed['target']]
            command(project, output, label+'-charon', wrapper+args)
            raw = json.loads((project/raw_path).read_text())
            record.setdefault('llbc', {})[label] = dict(path=str(raw_path), sha256=sha(project/raw_path), binding=admit_ir(raw, opaque))
            if not alias:
                original_enum = binding.contract_error(raw)
                foundations = json.loads((project/'verification/aeneas/llbc/sandproof_foundations.llbc').read_text())
                if original_enum != binding.contract_error(foundations):
                    raise ValueError('shared alias changed the original complete ContractError enum')
                record['contract_error'] = original_enum
        generated = output/'generated'
        generated.mkdir()
        command(project, output, 'aeneas', [str(project/'.tools/aeneas/bin/aeneas'), '-backend', 'lean',
            '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
            '-namespace', 'SandproofStage1RequestRust', '-subdir', 'SandproofStage1Request',
            '-dest', str(generated), str(project/raw_path)])
        record['translation'] = admit_generated(generated)
        command(project, output, 'admission-regressions', [sys.executable, str(HERE/'admission_tests.py'),
            '--llbc', str(raw_path), '--generated', str(generated)])
        record['raw_generated'] = {str(p.relative_to(generated)):sha(p) for p in generated.rglob('*') if p.is_file()}
        for name in ('Types.lean', 'Funs.lean'):
            shutil.copy2(generated/'SandproofStage1Request'/name, project/GENERATED/name)
        shutil.copy2(ROOT/PARENT/'TypesExternal.lean.in', project/GENERATED/'TypesExternal.lean')
        shutil.copy2(ROOT/HERE/'FunsExternal.lean.in', project/GENERATED/'FunsExternal.lean')
        for name in ('Types.lean', 'Funs.lean', 'TypesExternal.lean', 'FunsExternal.lean'):
            expected[str(GENERATED/name)] = sha(project/GENERATED/name)
        # Reuse every arithmetic proof/audit byte-for-byte, changing only the import
        # to the same equality definitions available in this smaller extraction.
        original = (ROOT/HERE.parent/'inbox-append/InboxAppendArithmetic.lean.in').read_text()
        header, rest = original.split('\n', 1)
        if header != 'import SandboxModel.Lifecycle.Protocol.InboxAppendShape':
            raise ValueError('arithmetic reuse header changed')
        destination = Path('SandboxModel/Lifecycle/Protocol/InboxAppendArithmetic.lean')
        (project/destination).write_text('import SandboxModel.Lifecycle.Protocol.ReplayEquality\n'+rest)
        expected[str(destination)] = sha(project/destination)
        record['arithmetic_reuse'] = dict(original_sha256=sha(ROOT/HERE.parent/'inbox-append/InboxAppendArithmetic.lean.in'),
            installed_sha256=expected[str(destination)], body_unchanged=True)
        stems = ['InboxLineageModel', 'InboxLineageSupport', 'InboxLineageValues']
        stems += {'correct': ['InboxLineageStep', 'InboxLineageLoop', 'InboxLineage', 'InboxLineageWitnesses'],
                  'advance-unattached-resume': ['InboxLineageWrongResume'],
                  'repeat-capture': ['InboxLineageWrongCapture'],
                  'skip-final-incarnation': ['InboxLineageWrongFinal']}[variant]
        for stem in stems:
            destination = Path('SandboxModel/Lifecycle/Protocol')/(stem+'.lean')
            source = ROOT/HERE/(stem+'.lean.in')
            shutil.copy2(source, project/destination)
            expected[str(destination)] = sha(project/destination)
        if prepare_only:
            record['prepared'] = True
        else:
            command(project, output, 'build', ['lake', 'build',
                *['SandboxModel.Lifecycle.Protocol.'+stem for stem in stems]])
            files = sorted(resolver.closure(project, [
                *['SandboxModel/Lifecycle/Protocol/'+stem+'.lean' for stem in stems],
                *[str(GENERATED/n) for n in ('Types.lean', 'Funs.lean', 'TypesExternal.lean', 'FunsExternal.lean')]], active_view='lakefile.toml'))
            actual = checker.actual_files(project, files, output, env, command)
            declared = [n for f in files for n in re.findall(r'^#audit_controller_proof (\S+)$', (project/f).read_text(), re.M)]
            if sorted(actual['audits']) != sorted(declared):
                raise ValueError('incomplete audits')
            record['actual'] = actual
        if any(sha(project/n) != d for n, d in expected.items()):
            raise ValueError('checked input changed')
        if inputs != base.snapshot(ROOT) or libraries != native.registry_module.registry() or resolver.runtime() != runtime:
            raise ValueError('source/tool/service/registry changed during checking')
        if record['library_binding'] != library_binding.check(ROOT):
            raise ValueError('borrowed Vec binding inputs changed during checking')
        record['complete'] = not prepare_only
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output/'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=['correct', *native.preparation.MUTATIONS], default='correct')
    parser.add_argument('--require-native', action='store_true')
    parser.add_argument('--development-native', type=Path)
    parser.add_argument('--prepare-only', action='store_true')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.variant, args.require_native, args.development_native, args.prepare_only)
    print(json.dumps({k: result.get(k) for k in ('variant', 'complete', 'prepared', 'seconds', 'error')}))
    raise SystemExit(0 if result['complete'] or result.get('prepared') and 'error' not in result else 1)
