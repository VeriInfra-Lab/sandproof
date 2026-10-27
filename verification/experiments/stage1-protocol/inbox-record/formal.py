"""Re-extract retained replay and full fresh-record state contracts and real Rust counterexamples."""
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
HERE = Path('verification/experiments/stage1-protocol/inbox-record')
VIRTUAL = Path('/home/zn/u333-5tuoxof8/std-mutex-source')
MODEL = 'controller-service/src/controller/lifecycle_inbox/model.rs'
GENERATED = Path('verification/aeneas/stage1-protocol/request/SandproofStage1Request')
ROOTS = ['sandproof_controller_service::controller::lifecycle_inbox::model::proof_record',
         *['crate::' + name for name in ('request_validate', 'replay_matches', 'accepted_validate', 'recorded_matches')]]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


base = module('record_formal_sources', ROOT / HERE.parent / 'schema-leaves/formal.py')
checker = module('record_actual_checker', ROOT / HERE.parent / 'formal.py')
binding = module('record_existing_enum_binding', ROOT / HERE.parent / 'envelope-writer/binding.py')
native = module('record_native_inputs', ROOT / HERE / 'native.py')
resolver = module('record_existing_closure', ROOT / 'tools/proof_batch.py')
sha = base.sha
journal_binding = module('current_journal_binding', ROOT / HERE.parent / 'inbox-lookup/journal_binding.py')
native_admission = module('inbox_native_admission', ROOT / HERE.parent / 'inbox-lookup/native_admission.py')


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
    roots = [f for f in functions if f['item_meta']['name'][-1] == {'Ident': ['proof_record', 0]}]
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
    if (callee['item_meta']['name'][-1] != {'Ident': ['record', 0]}
            or callee['item_meta']['name'][:4] != [
                {'Ident': [n, 0]} for n in ('sandproof_controller_service', 'controller', 'lifecycle_inbox', 'model')]
            or callee['item_meta']['opacity'] != 'Transparent' or 'Structured' not in callee['body']):
        raise ValueError('forwarding root does not call the actual transparent record caller')
    return dict(root=roots[0], call=calls[0], callee=callee)


def admit_generated(directory):
    translation = json.loads((directory / 'translation.json').read_text())
    functions = translation['functions']
    required = {'sandproof_controller_service.controller.lifecycle_inbox.model.Inbox.' + n
                for n in ('record', 'record_fresh', 'record_existing', 'find', 'session_index',
                          'retention_lane_index', 'retention_slot_used', 'recently_retired_identity')}
    required |= {'sandproof_controller_service.controller.lifecycle_inbox.model.Session.' + n
                 for n in ('find_entry', 'last_entry', 'can_follow_running')}
    required |= {'request_validate', 'replay_matches', 'accepted_validate', 'recorded_matches'}
    actual = {f['lean_name'].removeprefix('SandproofStage1RequestRust.') for f in functions
              if f['is_opaque'] is False and f['lean_file'] == 'SandproofStage1Request/Funs.lean'}
    if not required <= actual:
        raise ValueError('actual generated root/body missing: ' + repr(sorted(required - actual)))
    external = {f['lean_name'] for f in functions if f['is_opaque']}
    expected = {'Bool.Insts.CoreCmpPartialEqBool.ne', 'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
                'alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'alloc.vec.Vec.is_empty',
                'core.option.Option.is_some_and', 'core.option.Option.is_none_or', 'core.option.Option.as_ref',
                'core.result.Result.ok', 'alloc.string.String.Insts.CoreCloneClone.clone'}
    if external != expected:
        raise ValueError('unreviewed function model or business opacity')
    return translation


def run(output, variant, require_native=False, development_native=None):
    output.mkdir(parents=True, mode=0o700)
    start = time.monotonic()
    record = dict(kind='inbox-record-formal-development', variant=variant, complete=False,
                  production_adopted=True, ning_acceptance=False, shared_acceptance=False,
                  full_v3=False, commands=[])
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', LEAN_NUM_THREADS='2',
               LEAN_STACK_SIZE_KB='65536', CARGO_NET_OFFLINE='true')

    def command(project, folder, label, argv, expected_code=0):
        begin = time.monotonic()
        log = folder / (label + '.log')
        with log.open('xb') as stream:
            done = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                  stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        record['commands'].append(dict(label=label, argv=argv, cwd=str(project),
            exit_code=done.returncode, seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output / 'progress.json').write_text(json.dumps(record, indent=2)+'\n')
        if expected_code is not None and done.returncode != expected_code:
            raise ValueError('command failed: ' + str(log))
        return done.returncode

    try:
        native_result = None
        if development_native is not None:
            if require_native:
                raise ValueError('development evidence cannot substitute for a ning receipt')
            native_result = json.loads(development_native.read_text())
            if (native_result.get('complete') is not True or native_result.get('kind') != 'inbox-record-native-development'
                    or set(native_result.get('variants', {})) != set(native.preparation.VARIANTS)):
                raise ValueError('incomplete development native evidence')
            record['development_native'] = dict(path=str(development_native), sha256=sha(development_native))
        if require_native:
            sys.path.insert(0, str(ROOT))
            from tools.acceptance_receipts import validate_native, native_case, digest
            validate_native(Path(os.environ['SANDPROOF_NATIVE_EVIDENCE']))
            native_case('functional', ROOT, None)
            receipt = native_case('stage1-inbox-record', ROOT, None)
            record['native_receipt'] = receipt
            evidence = Path(os.environ['SANDPROOF_NATIVE_EVIDENCE'])/'commands'/digest(receipt['key'])/'evidence'
            native_result = json.loads((evidence/'result.json').read_text())
            if (native_result.get('complete') is not True or set(native_result.get('variants',{})) !=
                    set(native.preparation.VARIANTS)):
                raise ValueError('native record case has incomplete variants')
        # Reuse the original runtime checker for this component; no Writer namespace is executed.
        resolver.ROOT, resolver.MODULES = ROOT, ('stage1-protocol',)
        os.environ.setdefault('SANDPROOF_FORMAL_CACHE', str(ROOT/'artifacts/inbox-record/runtime-cache'))
        runtime = resolver.runtime()
        record['runtime'] = runtime
        inventory = json.loads((ROOT/HERE/'inventory.json').read_text())
        if sha(ROOT/HERE/'production-candidate.patch') != inventory['candidate_patch_sha256']:
            raise ValueError('reviewed source candidate differs')
        if {p.stem.removesuffix('.lean') for p in (ROOT/HERE).glob('InboxRecord*.lean.in')} != set(inventory['audits']):
            raise ValueError('unregistered or missing record proof template')
        for name,digest in inventory['production_inputs'].items():
            if sha(ROOT/name) != digest: raise ValueError('fixed original source differs: '+name)
        for stem,audits in inventory['audits'].items():
            actual = re.findall(r'^#audit_controller_proof (\S+)$', (ROOT/HERE/(stem+'.lean.in')).read_text(), re.M)
            if actual != audits or len(actual) != len(set(actual)):
                raise ValueError('missing/extra/duplicate candidate audit: '+stem)
        clone_name = 'alloc.string.String.Insts.CoreCloneClone.clone'
        clone_pattern = r'def ' + re.escape(clone_name) + r' \(value : String\) : Result String :=\s*\.ok value'
        registered_clone = ROOT/'verification/aeneas/network/SandproofNetwork/FunsExternal.lean'
        current_clone = ROOT/HERE/'FunsExternal.lean.in'
        if not all(re.search(clone_pattern, path.read_text()) for path in (registered_clone, current_clone)):
            raise ValueError('String clone binding differs from the registered normal-value model')
        record['registered_string_clone'] = dict(symbol=clone_name, path=str(registered_clone.relative_to(ROOT)),
            sha256=sha(registered_clone), binding_sha256=sha(current_clone))
        inputs = base.snapshot(ROOT)
        libraries = native.registry_module.registry()
        native_admission.admit(record, native_result, inputs, libraries, ROOT,
                               native.registry_module.HERE / 'Cargo.lock')
        record.update(inputs=inputs, registry=libraries, source_commit=subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip())
        fixed = json.loads((ROOT / 'verification/foundations-environment.json').read_text())
        for name, suffix in [('charon', 'charon/bin/charon'), ('aeneas', 'bin/aeneas')]:
            if sha(ROOT / '.tools/aeneas' / suffix) != fixed[name]['binary_sha256']:
                raise ValueError('fixed tool differs: ' + name)
        record['tools'] = fixed
        project = output / 'source'
        project.mkdir(mode=0o700)
        expected = dict(inputs)
        for name, digest in inputs.items():
            path = project / name
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, path)
            if sha(path) != digest: raise ValueError('input changed while copying: ' + name)
        (project / '.tools').symlink_to(ROOT / '.tools', target_is_directory=True)
        (project / '.lake').mkdir()
        (project / '.lake/packages').symlink_to(ROOT / '.lake/packages', target_is_directory=True)
        (project / 'artifacts/extract-target').mkdir(parents=True)
        wrapper = ['bwrap', '--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc',
            '--tmpfs', '/tmp', '--tmpfs', '/home', '--ro-bind', str(Path.home()), str(Path.home()),
            '--bind', str(project), str(VIRTUAL), '--chdir', str(VIRTUAL),
            '--setenv', 'CARGO_TARGET_DIR', str(VIRTUAL / 'artifacts/extract-target'),
            '--setenv', 'CARGO_NET_OFFLINE', 'true', '--setenv', 'CARGO_BUILD_JOBS', '2',
            '--unsetenv', 'RUSTFLAGS', '--unsetenv', 'CARGO_ENCODED_RUSTFLAGS', '--']
        original_model = (project / MODEL).read_bytes()
        sdk = native.preparation.SDK
        original_sdk = (project / sdk).read_bytes()
        changes, _ = native.preparation.prepare(project, 'original')
        if native_result is not None:
            for v in ('original',):
                for name, digest in changes.items():
                    if native_result['variants'][v]['inputs'].get(name) != digest:
                        raise ValueError('native original/restored source differs from formal source')

        def extract(label, alias):
            raw_path = Path('artifacts') / (label + '.llbc')
            if (project / raw_path).exists(): raise ValueError('stale LLBC destination')
            opaque = ['rusqlite::error::Error'] + (['sandproof_protocol::ContractError'] if alias else [])
            args = [str(VIRTUAL / '.tools/aeneas/charon/bin/charon'), 'cargo', '--preset=aeneas',
                    '--error-on-warnings', '--sysroot', 'default']
            for n in ('sandproof_protocol', 'sandbox_controller_core', 'sandproof_controller_service'):
                args += ['--include', n]
            for n in opaque: args += ['--opaque', n]
            for n in ROOTS: args += ['--start-from', n]
            args += ['--dest-file', str(VIRTUAL / raw_path), '--', '--manifest-path', str(HERE / 'Cargo.toml'),
                     '--lib', '--locked', '--offline', '--target', fixed['target']]
            command(project, output, label+'-charon', wrapper+args)
            raw = json.loads((project / raw_path).read_text())
            admitted = admit_ir(raw, opaque)
            record.setdefault('llbc', {})[label] = dict(path=str(raw_path), sha256=sha(project/raw_path), binding=admitted)
            return raw, raw_path

        raw, original_path = extract('original', False)
        original_enum = binding.contract_error(raw)
        foundations = json.loads((project / 'verification/aeneas/llbc/sandproof_foundations.llbc').read_text())
        if original_enum != binding.contract_error(foundations):
            raise ValueError('shared ContractError alias changed the original complete enum')
        record['contract_error'] = original_enum
        # Preserve original Charon/type-alias evidence, then apply the explicitly
        # reviewed private source candidate. Production adoption remains pending.
        (project / MODEL).write_bytes(original_model)
        (project / sdk).write_bytes(original_sdk)
        native_variant = 'candidate' if variant == 'correct' else variant
        changes, mutation = native.preparation.prepare(project, native_variant)
        expected.update(changes)
        record['business_inputs'] = changes
        record['mutation'] = mutation
        if native_result is not None:
            matching_variants = ('candidate', 'restored') if variant == 'correct' else (native_variant,)
            for checked_variant in matching_variants:
                for name, digest in changes.items():
                    if native_result['variants'][checked_variant]['inputs'].get(name) != digest:
                        raise ValueError('native candidate/mutant/restored source differs from formal source')
            if mutation is not None and native_result['mutations'].get(native_variant) != mutation:
                raise ValueError('native and formal mutation evidence differs')
        raw, raw_path = extract('candidate', True)
        generated = output / 'generated'
        generated.mkdir()
        command(project, output, 'candidate-aeneas', [str(project/'.tools/aeneas/bin/aeneas'),
            '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
            '-namespace', 'SandproofStage1RequestRust', '-subdir', 'SandproofStage1Request',
            '-dest', str(generated), str(project/raw_path)])
        record['translation'] = admit_generated(generated)
        command(project, output, 'admission-regressions', [sys.executable, str(HERE/'admission_tests.py'),
            '--llbc', str(raw_path), '--generated', str(generated)])
        record['raw_generated'] = {str(p.relative_to(generated)):sha(p) for p in generated.rglob('*') if p.is_file()}
        for name in ('Types.lean', 'Funs.lean'):
            shutil.copy2(generated/'SandproofStage1Request'/name, project/GENERATED/name)
        shutil.copy2(ROOT/HERE/'TypesExternal.lean.in', project/GENERATED/'TypesExternal.lean')
        shutil.copy2(ROOT/HERE/'FunsExternal.lean.in', project/GENERATED/'FunsExternal.lean')
        for name in ('Types.lean', 'Funs.lean', 'TypesExternal.lean', 'FunsExternal.lean'):
            expected[str(GENERATED/name)] = sha(project/GENERATED/name)
        target = {'correct': 'InboxRecordNewSession', 'skip-retention-window': 'InboxRecordWrongWindow',
                  'wrong-recorded-version': 'InboxRecordWrongVersion'}[variant]
        for stem in ('InboxLookupModel', 'InboxLookupSupport', 'InboxLookupValues', 'InboxLookup'):
            destination = Path('SandboxModel/Lifecycle/Protocol')/(stem+'.lean')
            shutil.copy2(ROOT/HERE.parent/'inbox-lookup'/(stem+'.lean.in'), project/destination)
            expected[str(destination)] = sha(project/destination)
        stems = ['InboxRecord'+n for n in ('Cloning','Selection','Predicates','Replay','FreshModel',
                 'FreshSupport','NewSessionModel','Values','WitnessSupport')]
        if variant != 'skip-retention-window': stems += ['InboxRecordWindowComparison']
        if variant == 'correct': stems += ['InboxRecordFresh']
        stems += [target]
        for stem in stems:
            destination = Path('SandboxModel/Lifecycle/Protocol')/(stem+'.lean')
            shutil.copy2(ROOT/HERE/(stem+'.lean.in'), project/destination)
            expected[str(destination)] = sha(project/destination)
        command(project, output, 'build', ['lake', 'build',
            *['SandboxModel.Lifecycle.Protocol.'+stem for stem in stems]])
        files = sorted(resolver.closure(project, [
            *['SandboxModel/Lifecycle/Protocol/'+stem+'.lean' for stem in stems],
            *[str(GENERATED/n) for n in ('Types.lean','Funs.lean','TypesExternal.lean','FunsExternal.lean')]], active_view='lakefile.toml'))
        actual = checker.actual_files(project, files, output, env, command)
        declared = [n for f in files for n in re.findall(r'^#audit_controller_proof (\S+)$', (project/f).read_text(), re.M)]
        if sorted(actual['audits']) != sorted(declared): raise ValueError('incomplete audits')
        record['actual'] = actual
        for name,digest in expected.items():
            if sha(project/name) != digest: raise ValueError('checked input changed: '+name)
        if inputs != base.snapshot(ROOT) or libraries != native.registry_module.registry():
            raise ValueError('original source/dependencies changed during checking')
        if resolver.runtime() != runtime:
            raise ValueError('tool/service/backend/library inputs changed during checking')
        record['complete'] = True
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record['seconds'] = time.monotonic()-start
    (output/'result.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--variant', choices=['correct','skip-retention-window','wrong-recorded-version'], default='correct')
    parser.add_argument('--require-native', action='store_true', help='require current full native seal and this registered case')
    parser.add_argument('--development-native', type=Path, help='bind same-input local development result, without a ning seal')
    args = parser.parse_args()
    os.umask(0o077)
    result = run(args.output.absolute(), args.variant, args.require_native, args.development_native)
    print(json.dumps({k:result.get(k) for k in ('variant','complete','seconds','error')}))
    raise SystemExit(0 if result['complete'] else 1)
