"""Bind original request-end captures and complete unchanged parent evidence."""
import importlib.util
import json
from pathlib import Path
import re
import sys
import tarfile
from tempfile import TemporaryDirectory

HERE = Path(__file__).resolve().parent
PARENT = HERE.parent / 'source-remove-retired'
sys.path.insert(0, str(PARENT))

def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value

capture = module('source_saved_question_capture', HERE / 'capture.py')
inherited = module('source_saved_question_parent_validation', HERE.parent / 'source-full-binding/validation.py')
original_inputs = inherited.original_inputs
digest = original_inputs.digest
SUPPORT = original_inputs.SUPPORT
REPO = capture.retire.REPO
ROOT = 'controller.lifecycle_inbox.source_work.full.drain.question.from_saved'
ASSEMBLY = 'controller.lifecycle_inbox.source_work.full.drain.question.assemble'
VALID = 'capture.work.FullCaptureWorkRequest.valid'
LIBRARIES = capture.LIBRARIES

def parsed(data):
    return original_inputs.parsed(data, capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE / 'capture.py'),
        base_driver=digest(PARENT / 'extract.py'), ports_driver=digest(PARENT / 'ports.py'),
        capture_root=capture.ROOT, direct_original_saved_question=True, whole_receipt_proved=False,
        parent_driver=digest(capture.inputs.PARENT / 'capture.py'), staging_driver=digest(HERE / 'inputs.py'),
        mutation=None if variant == 'original' else 'omit only original strict stop epoch comparison',
        complete=True, public_acceptance=False)
    if result.get('preflight_only') or any(result.get(name) != wanted for name, wanted in expected.items()):
        raise ValueError('the exact completed original request-end capture is required')
    if [(c['label'], c['exit_code'], c['bounded_stop']) for c in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both original fixed tools must actually finish successfully')
    fixed = json.loads((REPO / 'verification/foundations-environment.json').read_bytes())
    ir, generated = directory / 'actual.llbc', directory / 'generated'
    selectors = [part for flag, group in (('--start-from', 'roots'),
        ('--start-from-if-exists', 'retained_roots'), ('--include', 'include'), ('--opaque', 'opaque'))
        for pattern in capture.OPTIONS[group] for part in (flag, pattern)]
    tails = [
        ['cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default', *selectors,
         '--dest-file', str(ir), '--', '--manifest-path', 'controller-service/Cargo.toml',
         '--lib', '--locked', '--offline', '--target', fixed['target']],
        ['-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files', '-emit-json',
         '-namespace', capture.NAMESPACE, '-subdir', capture.NAMESPACE, '-dest', str(generated), str(ir)],
    ]
    for command, tail in zip(result['commands'], tails, strict=True):
        binary = Path(command['argv'][0])
        if (command['argv'][1:] != tail or not binary.is_absolute()
                or digest(binary) != fixed[command['label']]['binary_sha256']):
            raise ValueError('the complete exact fixed tool command and binary are required')
    original_inputs.read_exact(directory / 'source.tar', capture.retire.ARCHIVE)
    for command in result['commands']:
        original_inputs.read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    if set(result['staged']) != {str(path) for path in capture.STAGED}:
        raise ValueError('all eight exact staged files are mandatory')
    with TemporaryDirectory(prefix='source-full-binding-', dir=directory.parent) as temporary:
        source = Path(temporary)
        capture.reconstruct(directory / 'source.tar', source, variant)
        with tarfile.open(directory / 'source.tar') as archive:
            members = {m.name: m for m in archive.getmembers() if m.isfile()}
        if len(members) != 4906:
            raise ValueError('all4906 complete fixed production files are mandatory')
        paths = list((directory / 'source').rglob('*'))
        if any(path.is_symlink() for path in paths):
            raise ValueError('the full captured source cannot contain symlinks')
        if {str(p.relative_to(directory / 'source')) for p in paths if p.is_file()} != set(members):
            raise ValueError('the entire fixed production source inventory must remain present')
        for name, member in members.items():
            path = directory / 'source' / name
            if (digest(path) != digest(source / name)
                    or bool(path.stat().st_mode & 0o111) != bool(member.mode & 0o111)):
                raise ValueError('captured source differs from exact reconstruction: ' + name)
        for name, wanted in result['staged'].items():
            original_inputs.read_exact(directory / 'source' / name, wanted)
    original_inputs.read_exact(directory / 'actual.llbc', result['ir'])
    generated = directory / 'generated'
    names = {'translation.json', *(capture.NAMESPACE + '/' + name + '.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if (set(result['generated']) != names
            or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != names):
        raise ValueError('all actual generated files are required')
    for name, wanted in result['generated'].items():
        original_inputs.read_exact(generated / name, wanted)
    if capture.admit(directory / 'actual.llbc', generated, fixed['target']) != result['admission']:
        raise ValueError('the original transparent business/library mapping changed')
    sources = {name: (generated / capture.NAMESPACE / (name + '.lean')).read_bytes()
               for name in ('Types', 'Funs')}
    sources['External'] = (generated / capture.NAMESPACE / 'FunsExternal_Template.lean').read_bytes()
    sources['TypesExternal'] = (generated / capture.NAMESPACE / 'TypesExternal_Template.lean').read_bytes()
    libraries = set(re.findall(r'^axiom\s+([^\s(:]+)', sources['External'].decode(), re.M))
    expected_libraries = LIBRARIES
    if libraries != expected_libraries or sorted(libraries) != result['admission']['libraries']:
        raise ValueError('only the exact twenty-one typed library interfaces are permitted')
    return result, sources


def same_interfaces(correct, wrong):
    for group in ('Types', 'TypesExternal', 'External'):
        if correct[group] != wrong[group]:
            raise ValueError('all complete types and typed standard interfaces must agree: ' + group)
    if len(parsed(correct['Types'])) != 112:
        raise ValueError('all112 complete canonical types are mandatory')
    left, right = parsed(correct['Funs']), parsed(wrong['Funs'])
    if len(left) != 145 or left.keys() != right.keys():
        raise ValueError('all145 original complete functions/dictionaries must remain')
    if {name for name in right if left[name] != right[name]} != {ASSEMBLY}:
        raise ValueError('only the original assembly body may change')
    # Pin both complete generated suffixes. Besides the missing guard, the fixed
    # formatter deindents and rewraps the surviving lines. No broad whitespace,
    # alpha normalization or generated-file rewriting is permitted.
    old = '''              if stop.epoch <= val.epoch
              then ok (core.result.Result.Err JournalError.CorruptStore)
              else
                if stop_action.command.attempt != stop.attempt
                then ok (core.result.Result.Err JournalError.CorruptStore)
                else
                  if stop_action.identity.attempt != stop.attempt
                  then ok (core.result.Result.Err JournalError.CorruptStore)
                  else
                    if stop_action.identity.epoch != stop.epoch
                    then ok (core.result.Result.Err JournalError.CorruptStore)
                    else
                      if stop_action.identity.generation != val.generation
                      then
                        ok (core.result.Result.Err JournalError.CorruptStore)
                      else
                        let b3 ←
                          core.cmp.PartialEq.ne.trait_default
                            sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId
                            stop_action.identity.owner val2.identity.owner
                        if b3
                        then
                          ok (core.result.Result.Err JournalError.CorruptStore)
                        else
                          let b4 ←
                            core.cmp.PartialEq.ne.trait_default
                              sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId
                              stop_action.identity.sandbox
                              val2.identity.sandbox
                          if b4
                          then
                            ok (core.result.Result.Err
                              JournalError.CorruptStore)
                          else
                            let b5 ←
                              core.cmp.PartialEq.ne.trait_default
                                sandbox_controller_core.ids.TenantId.Insts.CoreCmpPartialEqTenantId
                                host_slot.binding.owner
                                record.work.anchor.local.owner
                            if b5
                            then
                              ok (core.result.Result.Err
                                JournalError.CorruptStore)
                            else
                              let b6 ←
                                core.cmp.PartialEq.ne.trait_default
                                  sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
                                  host_slot.binding.sandbox val.key.sandbox
                              if b6
                              then
                                ok (core.result.Result.Err
                                  JournalError.CorruptStore)
                              else
                                if host_slot.binding.generation !=
                                  val.generation
                                then
                                  ok (core.result.Result.Err
                                    JournalError.CorruptStore)
                                else
                                  let b7 ←
                                    core.cmp.PartialEq.ne.trait_default
                                      sandbox_controller_core.ids.VmId.Insts.CoreCmpPartialEqVmId
                                      host_slot.binding.vm val.vm
                                  if b7
                                  then
                                    ok (core.result.Result.Err
                                      JournalError.CorruptStore)
                                  else
                                    let wa ←
                                      sandproof_protocol.WorkAction.Insts.CoreCloneClone.clone
                                        sandbox_controller_core.model.EffectId.Insts.CoreCloneClone
                                        val2
                                    let fcwr ←
                                      capture.work.FullCaptureWorkRequest.Insts.CoreCloneClone.clone
                                        val1.full
                                    let sor ←
                                      controller.lifecycle_inbox.source_work.SourceWork.impl.requirement
                                        record.work
                                    let sor1 ←
                                      capture.source.SourceObservationRequirement.Insts.CoreCloneClone.clone
                                        sor
                                    let o2 ←
                                      core.option.Option.Insts.CoreCloneClone.clone
                                        capture.launch.CaptureWorkerBinding.Insts.CoreCloneClone
                                        val1.worker
                                    let wa1 ←
                                      sandproof_protocol.WorkAction.Insts.CoreCloneClone.clone
                                        sandbox_controller_core.model.EffectId.Insts.CoreCloneClone
                                        stop_action
                                    let tcwr ←
                                      capture.terminated.TerminatedCaptureWorkRequest.from_source
                                        val wa fcwr sor1 o2 (wa1, stop)
                                        host_slot
                                    ok (core.result.Result.Ok tcwr)
      | core.ops.control_flow.ControlFlow.Break residual =>
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual
          capture.terminated.TerminatedCaptureWorkRequest
          (core.convert.FromSame JournalError) residual
    | core.ops.control_flow.ControlFlow.Break residual =>
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual
        capture.terminated.TerminatedCaptureWorkRequest (core.convert.FromSame
        JournalError) residual
  | core.ops.control_flow.ControlFlow.Break residual =>
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual
      capture.terminated.TerminatedCaptureWorkRequest (core.convert.FromSame
      JournalError) residual'''.encode()
    new = '''              if stop_action.command.attempt != stop.attempt
              then ok (core.result.Result.Err JournalError.CorruptStore)
              else
                if stop_action.identity.attempt != stop.attempt
                then ok (core.result.Result.Err JournalError.CorruptStore)
                else
                  if stop_action.identity.epoch != stop.epoch
                  then ok (core.result.Result.Err JournalError.CorruptStore)
                  else
                    if stop_action.identity.generation != val.generation
                    then ok (core.result.Result.Err JournalError.CorruptStore)
                    else
                      let b3 ←
                        core.cmp.PartialEq.ne.trait_default
                          sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId
                          stop_action.identity.owner val2.identity.owner
                      if b3
                      then
                        ok (core.result.Result.Err JournalError.CorruptStore)
                      else
                        let b4 ←
                          core.cmp.PartialEq.ne.trait_default
                            sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId
                            stop_action.identity.sandbox val2.identity.sandbox
                        if b4
                        then
                          ok (core.result.Result.Err JournalError.CorruptStore)
                        else
                          let b5 ←
                            core.cmp.PartialEq.ne.trait_default
                              sandbox_controller_core.ids.TenantId.Insts.CoreCmpPartialEqTenantId
                              host_slot.binding.owner
                              record.work.anchor.local.owner
                          if b5
                          then
                            ok (core.result.Result.Err
                              JournalError.CorruptStore)
                          else
                            let b6 ←
                              core.cmp.PartialEq.ne.trait_default
                                sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
                                host_slot.binding.sandbox val.key.sandbox
                            if b6
                            then
                              ok (core.result.Result.Err
                                JournalError.CorruptStore)
                            else
                              if host_slot.binding.generation != val.generation
                              then
                                ok (core.result.Result.Err
                                  JournalError.CorruptStore)
                              else
                                let b7 ←
                                  core.cmp.PartialEq.ne.trait_default
                                    sandbox_controller_core.ids.VmId.Insts.CoreCmpPartialEqVmId
                                    host_slot.binding.vm val.vm
                                if b7
                                then
                                  ok (core.result.Result.Err
                                    JournalError.CorruptStore)
                                else
                                  let wa ←
                                    sandproof_protocol.WorkAction.Insts.CoreCloneClone.clone
                                      sandbox_controller_core.model.EffectId.Insts.CoreCloneClone
                                      val2
                                  let fcwr ←
                                    capture.work.FullCaptureWorkRequest.Insts.CoreCloneClone.clone
                                      val1.full
                                  let sor ←
                                    controller.lifecycle_inbox.source_work.SourceWork.impl.requirement
                                      record.work
                                  let sor1 ←
                                    capture.source.SourceObservationRequirement.Insts.CoreCloneClone.clone
                                      sor
                                  let o2 ←
                                    core.option.Option.Insts.CoreCloneClone.clone
                                      capture.launch.CaptureWorkerBinding.Insts.CoreCloneClone
                                      val1.worker
                                  let wa1 ←
                                    sandproof_protocol.WorkAction.Insts.CoreCloneClone.clone
                                      sandbox_controller_core.model.EffectId.Insts.CoreCloneClone
                                      stop_action
                                  let tcwr ←
                                    capture.terminated.TerminatedCaptureWorkRequest.from_source
                                      val wa fcwr sor1 o2 (wa1, stop) host_slot
                                  ok (core.result.Result.Ok tcwr)
      | core.ops.control_flow.ControlFlow.Break residual =>
        core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual
          capture.terminated.TerminatedCaptureWorkRequest
          (core.convert.FromSame JournalError) residual
    | core.ops.control_flow.ControlFlow.Break residual =>
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual
        capture.terminated.TerminatedCaptureWorkRequest (core.convert.FromSame
        JournalError) residual
  | core.ops.control_flow.ControlFlow.Break residual =>
    core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual
      capture.terminated.TerminatedCaptureWorkRequest (core.convert.FromSame
      JournalError) residual'''.encode()
    if left[ASSEMBLY].count(old) != 1 or left[ASSEMBLY].replace(old, new, 1) != right[ASSEMBLY]:
        raise ValueError('only the exact reviewed original epoch omission and formatter layout are permitted')
    return right


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceFullBindingValues', 186, 958)
    kept, kept_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if (plan['source'], plan['tree']) != (capture.retire.SOURCE, capture.retire.TREE):
        raise ValueError('the complete parent must retain the same fixed source')
    if any(plan['files'].get(name) != wanted for name, wanted in kept['files'].items()):
        raise ValueError('all original retention files must remain in the parent')
    sources = {name: (project / name).read_bytes() for name in plan['files']}
    shared, added = {}, {}
    for group in ('Types', 'Funs'):
        shared[group], added[group] = {}, {}
        for name, body in parsed(reference[group]).items():
            found = [file for file, raw in sources.items() if raw.count(body) == 1]
            if found:
                shared[group][name] = found
            else:
                added[group][name] = body
    if len(shared['Types']) != 112 or added['Types']:
        raise ValueError('all112 complete canonical parent types must be reused')
    required = {ROOT, ASSEMBLY,
        'controller.lifecycle_inbox.source_work.full.key',
        'controller.lifecycle_inbox.source_work.full.drain.question.original_identity',
        'sandbox_controller_core.ids.TenantId.Insts.CoreCmpPartialEqTenantId',
        'sandbox_controller_core.model.Command.Insts.CoreCmpPartialEqCommand',
        'sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId',
        'sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId'}
    if len(shared['Funs']) != 137 or set(added['Funs']) != required:
        raise ValueError('137 verbatim parent bodies and only eight actual additions are required')
    if (HERE.parent / 'source-full-work-guard/SourceFullWorkGuardLibrary.lean.in').read_bytes() != (project / 'SourceFullWorkGuardLibrary.lean').read_bytes():
        raise ValueError('the original parent literal/library template must match its checked physical file')
    if (HERE / 'SourceSavedQuestionLibrary.lean.in').read_bytes() != library():
        raise ValueError('the exact21 existing per-symbol standard providers are mandatory')
    return plan, dict(parent=evidence, retention=kept_evidence, shared=shared,
        standard_interfaces=sorted(LIBRARIES), parent_library=plan['files']['SourceFullWorkGuardLibrary.lean']), added


def library():
    text = 'import SourceSavedQuestionSpec\nopen Aeneas Aeneas.Std\nset_option autoImplicit false\n'
    source = (HERE.parent / 'source-full-work-guard/SourceFullWorkGuardLibrary.lean.in').read_text()
    start = source.index('def toStr (text : String)')
    stop = source.index('\ndef I64.', start)
    literal = source[start:stop]
    expected = """def toStr (text : String)
    (bounded : text.toByteArray.size ≤ U32.max := by rw [U32.max_eq]; decide) : Str :=
  SandproofSourceRowSqlSelect.toStr text bounded
"""
    if literal != expected:
        raise ValueError('the exact parent kernel-bounded literal adapter is mandatory')
    foundation = (REPO / 'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean').read_text()
    start = foundation.index('@[simp, rust_fun "core::option::{core::ops::try_trait::Try')
    stop = foundation.index('@[simp, rust_fun "core::result::', start)
    selected = foundation[start:stop]
    if selected.count('\ndef ') != 2 or 'axiom' in selected:
        raise ValueError('only the two original fully defined A6 Option Try models may be selected')
    providers = {name: '@SourceFullBindingActual.' + name for name in capture.parent.LIBRARIES}
    providers.update({
        'alloc.string.String.Insts.CoreCloneClone.clone': '@SourceRetireLibrary.stringClone',
        'core.option.Option.Insts.CoreCloneClone.clone': '@SourceRetireLibrary.optionClone',
        'core.option.Option.as_ref': '@SourceRetireLibrary.optionRef',
        'Bool.Insts.CoreCmpPartialEqBool.ne': '@SandproofSourceRetentionProviders.Bool.Insts.CoreCmpPartialEqBool.ne',
        'core.option.Option.Insts.CoreCmpPartialEqOption.eq': '@SourceRequestEndMatchActual.core.option.Option.Insts.CoreCmpPartialEqOption.eq'})
    if set(providers) | {'core.option.Option.Insts.CoreOpsTry_traitTry.branch',
            'core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual'} != LIBRARIES:
        raise ValueError('each of the exact21 original interfaces requires its existing typed provider')
    for namespace in ('SourceSavedQuestionActual', 'SourceSavedQuestionWrongActual'):
        text += '\nnamespace SandboxModel.Stage1Protocol.' + namespace + '\n' + literal + '\n'
        for name, value in sorted(providers.items()):
            text += 'def ' + name + ' := ' + value + '\n'
        text += '\n' + selected + 'end SandboxModel.Stage1Protocol.' + namespace + '\n'
    return text.encode()


def declaration(reference, group, name, body):
    return inherited.declaration(reference, group, name, body)


def raw_functions(reference):
    bodies = []
    for name, raw in parsed(reference['Funs']).items():
        body = declaration(reference, 'Funs', name, raw)
        # The original Unload parameter is named capture. Keep its body verbatim;
        # resolve these two complete callee names before that local binder exists.
        if name in (VALID, 'capture.SourceProcessIdentity.Insts.CoreCmpPartialEqSourceProcessIdentity'):
            body += ('\n\nlocal notation "' + name + '" => ' + name).encode()
        bodies.append(body)
    return b'\n\n'.join(bodies)


def extra_inputs():
    return [str((HERE.parent / 'source-full-work-guard/SourceFullWorkGuardLibrary.lean.in').relative_to(REPO)),
            'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean']
