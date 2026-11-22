"""Strict actual receipt input, current tool and complete parent readback."""
import importlib.util,json,re,sys,tarfile
from pathlib import Path
from tempfile import TemporaryDirectory
HERE=Path(__file__).resolve().parent
PARENT=HERE.parent/'source-remove-retired'
sys.path.insert(0,str(PARENT))
def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path)
 value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
capture=module('source_end_receipt_capture',HERE/'capture.py')
inherited=module('source_end_receipt_parent_validation',HERE.parent/'source-end-validation/validation.py')
original_inputs=inherited.original_inputs
digest=original_inputs.digest
SUPPORT=original_inputs.SUPPORT
REPO=capture.retire.REPO
ROOT='controller.lifecycle_inbox.source_work.full.end.receipt'
LIBRARIES=capture.LIBRARIES

def parsed(data):return original_inputs.parsed(data,capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE/'capture.py'),
        base_driver=digest(PARENT/'extract.py'), ports_driver=digest(PARENT/'ports.py'),
        capture_root=capture.ROOT, direct_original_end_receipt=True, whole_receipt_proved=False,
        original_option_map_and_transpose_preserved=True,
        two_exact_original_end_encode_sites_isolated=True,
        exact_original_sha_primitive=capture.inputs.SHA,
        exact_receipt_encoder=capture.inputs.PORT, original_map_transpose_transparent=True,
        parent_driver=digest(Path(capture.parent.__file__)),staging_driver=digest(HERE/'inputs.py'),
        mutation=None if variant=='original' else 'change only NoRequest receipt tag to Returned',
        complete=True,public_acceptance=False)
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
        raise ValueError('all twelve exact staged files are mandatory')
    with TemporaryDirectory(prefix='source-end-validation-', dir=directory.parent) as temporary:
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
        raise ValueError('only the exact twenty-nine typed library interfaces are permitted')
    return result, sources

def library():return inherited.library().decode().replace('SourceEndValidation','SourceEndReceipt').encode()
def declaration(reference,group,name,body):return inherited.declaration(reference,group,name,body)
def extra_inputs():return inherited.extra_inputs()

def raw_functions(reference,actual_namespace='SourceEndReceiptActual'):
 bodies=[]
 for name,raw in parsed(reference['Funs']).items():
  body=declaration(reference,'Funs',name,raw)
  if name=='controller.lifecycle_inbox.source_work.full.end.material_writer_closed':
   body=b'section\nlocal notation "core.option.Option.is_some" => (@core.option.Option.is_some End)\n\n'+body+b'\nend'
  if name in ('capture.work.FullCaptureWorkRequest.valid','capture.SourceProcessIdentity.Insts.CoreCmpPartialEqSourceProcessIdentity'):
   body+=('\n\nlocal notation "'+name+'" => '+name).encode()
  if name in ('capture.work.FullCaptureWorkRequest.binding_digest',
      'capture.terminated.TerminatedCaptureWorkRequest.binding_digest',
      'capture.CaptureRequestEndObservation.matches',
      'capture.terminated.TerminatedCaptureObservation.matches',
      'controller.lifecycle_inbox.source_work.full.drain.prior_matches',
      'controller.lifecycle_inbox.source_work.full.drain.validate_record',
      'controller.lifecycle_inbox.source_work.full.drain.writer_closed',
      'controller.lifecycle_inbox.source_work.full.end.saved_never_released_matches',
      'controller.lifecycle_inbox.source_work.full.end.saved_worker_ended_matches',
      'controller.lifecycle_inbox.source_work.full.end.validate_record',
      'controller.lifecycle_inbox.source_work.digest'):
   if name=='capture.work.FullCaptureWorkRequest.binding_digest':body+=b'\n\nsection'
   separator='\n' if name=='capture.work.FullCaptureWorkRequest.binding_digest' else '\n\n'
   body+=(separator+'local notation "'+name+'" => '+name+' io').encode()
  if name==ROOT:
   prefix=('end\n'
    +'local notation "capture.work.FullCaptureWorkRequest.binding_digest" => SandboxModel.Stage1Protocol.'+actual_namespace+'.capture.work.FullCaptureWorkRequest.binding_digest (receiptPhase io)\n'
    +'local notation "controller.lifecycle_inbox.source_work.full.end.validate_record" => SandboxModel.Stage1Protocol.'+actual_namespace+'.controller.lifecycle_inbox.source_work.full.end.validate_record io\n'
    +'local notation "controller.lifecycle_inbox.source_work.digest" => SandboxModel.Stage1Protocol.'+actual_namespace+'.controller.lifecycle_inbox.source_work.digest io\n\n')
   body=prefix.encode()+body
  bodies.append(body)
 return b'\n\n'.join(bodies)

def parent(project,audit,retention,retention_audit,reference):
 plan,evidence=original_inputs.prior(project,audit,'SourceEndValidationValues',201,1115)
 kept,kept_evidence=original_inputs.prior(retention,retention_audit,'RetentionProviders',26,127)
 if (plan['source'],plan['tree'])!=(capture.retire.SOURCE,capture.retire.TREE):raise ValueError('same exact production input required')
 if any(plan['files'].get(name)!=wanted for name,wanted in kept['files'].items()):raise ValueError('all original26/127 retention files required')
 sources={name:(project/name).read_bytes()for name in plan['files']};shared,added={},{}
 for group in ('Types','Funs'):
  shared[group],added[group]={},{}
  for name,body in parsed(reference[group]).items():
   found=[file for file,raw in sources.items()if raw.count(body)==1]
   if found:shared[group][name]=found
   else:added[group][name]=body
 expected_types={'sandbox_controller_core.capture.continuation.'+name for name in ('CaptureWorkEnd','CaptureWorkEndReceipt')}|{'controller.lifecycle_inbox.source_work.full.end.receipt.closure'}
 expected_funs={ROOT,'core.option.Option.map','core.option.OptionResult.transpose',
  'controller.lifecycle_inbox.source_work.full.end.receipt.closure.Insts.CoreOpsFunctionFnOnceTupleSharedWorkActionEffectIdResultCaptureSealEndJournalError.call_once',
  'controller.lifecycle_inbox.source_work.full.end.receipt.closure.Insts.CoreOpsFunctionFnOnceTupleSharedWorkActionEffectIdResultCaptureSealEndJournalError'}
 if len(shared['Types'])!=113 or set(added['Types'])!=expected_types:raise ValueError('113 whole canonical types and exactly three complete additions required')
 if len(shared['Funs'])!=178 or set(added['Funs'])!=expected_funs:raise ValueError('178 verbatim parent functions/dictionaries and exactly five complete additions required')
 for folder,name in [('source-full-work-guard','SourceFullWorkGuardLibrary'),('source-end-validation','SourceEndValidationLibrary')]:
  if (HERE.parent/folder/(name+'.lean.in')).read_bytes()!=(project/(name+'.lean')).read_bytes():raise ValueError('original checked library source required: '+name)
 if (HERE/'SourceEndReceiptLibrary.lean.in').read_bytes()!=library():raise ValueError('original typed library providers required')
 return plan,dict(parent=evidence,retention=kept_evidence,shared=shared,standard_interfaces=sorted(LIBRARIES)),added

def same_interfaces(correct,wrong):
 for group in ('Types','TypesExternal','External'):
  if correct[group]!=wrong[group]:raise ValueError('all complete types/interfaces must agree: '+group)
 if len(parsed(correct['Types']))!=116:raise ValueError('all116 complete types required')
 left,right=parsed(correct['Funs']),parsed(wrong['Funs'])
 if len(left)!=183 or left.keys()!=right.keys():raise ValueError('all183 whole functions required')
 if {name for name in left if left[name]!=right[name]}!={ROOT}:raise ValueError('only actual receipt tag construction may change')
 old=b'            let (creo, typed_end) \xe2\x86\x90\n              match observation.outcome with\n              | capture.CaptureRequestEndOutcome.NoRequest =>\n                ok (capture.CaptureRequestEndOutcome.NoRequest,\n                  sandbox_controller_core.capture.continuation.CaptureWorkEnd.NoRequest\n                  val)\n              | capture.CaptureRequestEndOutcome.Returned _ =>\n                ok (observation.outcome,\n                  sandbox_controller_core.capture.continuation.CaptureWorkEnd.Returned\n                  val)'
 new=b'            let creo \xe2\x86\x90\n              match observation.outcome with\n              | capture.CaptureRequestEndOutcome.NoRequest =>\n                ok capture.CaptureRequestEndOutcome.NoRequest\n              | capture.CaptureRequestEndOutcome.Returned _ =>\n                ok observation.outcome'
 old_output=b'                  { work_binding := a, end_binding := a1, \xc2\xabend\xc2\xbb := typed_end\n                  }))'
 new_output=b'                  {\n                    work_binding := a,\n                    end_binding := a1,\n                    \xc2\xabend\xc2\xbb :=\n                      (sandbox_controller_core.capture.continuation.CaptureWorkEnd.Returned\n                        val)\n                  }))'
 body=left[ROOT]
 if body.count(old)!=1 or body.count(old_output)!=1 or body.replace(old,new,1).replace(old_output,new_output,1)!=right[ROOT]:raise ValueError('exact original NoRequest->Returned constructor lift required; no general normalization')
 return right
