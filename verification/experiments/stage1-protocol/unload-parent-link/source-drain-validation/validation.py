"""Exact current source/tool/interface and canonical parent evidence readback."""
import importlib.util,json,re,sys,tarfile
from pathlib import Path
from tempfile import TemporaryDirectory
HERE=Path(__file__).resolve().parent
PARENT=HERE.parent/'source-remove-retired'
sys.path.insert(0,str(PARENT))
def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path)
 value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
capture=module('source_drain_validation_capture',HERE/'capture.py')
inherited=module('source_drain_validation_parent_validation',HERE.parent/'source-saved-question/validation.py')
original_inputs=inherited.original_inputs
digest=original_inputs.digest
SUPPORT=original_inputs.SUPPORT
REPO=capture.retire.REPO
ROOT='controller.lifecycle_inbox.source_work.full.drain.validate_record'
WRITER='controller.lifecycle_inbox.source_work.full.drain.writer_closed'
LIBRARIES=capture.LIBRARIES

def parsed(data):return original_inputs.parsed(data,capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE / 'capture.py'),
        base_driver=digest(PARENT / 'extract.py'), ports_driver=digest(PARENT / 'ports.py'),
        capture_root=capture.ROOT, direct_original_drain_validation=True, whole_receipt_proved=False,
        original_saved_end_option_selection_expanded_to_nested_matches=True,
        parent_driver=digest(Path(capture.inputs.parent.__file__)), staging_driver=digest(HERE / 'inputs.py'),
        terminal_driver=digest(Path(capture.inputs.terminal.__file__)),
        prior_driver=digest(Path(capture.inputs.prior.__file__)),
        mutation=None if variant == 'original' else 'omit only CorruptStore return for an actual mismatched saved prior',
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
        raise ValueError('all eleven exact staged files are mandatory')
    with TemporaryDirectory(prefix='source-drain-validation-', dir=directory.parent) as temporary:
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
        raise ValueError('only the exact twenty-six typed library interfaces are permitted')
    return result, sources



def library():
 text=inherited.library().decode().replace('SourceSavedQuestion','SourceDrainValidation')
 for namespace in ('SourceDrainValidationActual','SourceDrainValidationWrongActual'):
  marker='end SandboxModel.Stage1Protocol.'+namespace+'\n'
  if text.count(marker)!=1:raise ValueError('exact original library namespace required')
  text=text.replace(marker,'def core.option.Option.as_deref := @SourcePriorEndMatchActual.core.option.Option.as_deref\n'+marker,1)
 return text.encode()

def declaration(reference,group,name,body):return inherited.declaration(reference,group,name,body)
def raw_functions(reference):
 bodies=[]
 for name,raw in parsed(reference['Funs']).items():
  body=declaration(reference,'Funs',name,raw)
  if name in ('capture.work.FullCaptureWorkRequest.valid','capture.SourceProcessIdentity.Insts.CoreCmpPartialEqSourceProcessIdentity'):
   body+=('\n\nlocal notation "'+name+'" => '+name).encode()
  if name in ('capture.work.FullCaptureWorkRequest.binding_digest',
      'capture.terminated.TerminatedCaptureWorkRequest.binding_digest',
      'capture.CaptureRequestEndObservation.matches',
      'capture.terminated.TerminatedCaptureObservation.matches',
      'controller.lifecycle_inbox.source_work.full.drain.prior_matches',
      'controller.lifecycle_inbox.source_work.full.drain.validate_record'):
   body+=('\n\nlocal notation "'+name+'" => '+name+' io').encode()
  bodies.append(body)
 return b'\n\n'.join(bodies)
def extra_inputs():return inherited.extra_inputs()


def parent(project,audit,retention,retention_audit,reference):
 plan,evidence=original_inputs.prior(project,audit,'SourceSavedQuestionValues',191,1014)
 kept,kept_evidence=original_inputs.prior(retention,retention_audit,'RetentionProviders',26,127)
 if (plan['source'],plan['tree'])!=(capture.retire.SOURCE,capture.retire.TREE):raise ValueError('same exact production source required')
 if any(plan['files'].get(name)!=wanted for name,wanted in kept['files'].items()):raise ValueError('all original26/127 retention files required')
 sources={name:(project/name).read_bytes() for name in plan['files']};shared,added={},{}
 for group in ('Types','Funs'):
  shared[group],added[group]={},{}
  for name,body in parsed(reference[group]).items():
   found=[file for file,raw in sources.items() if raw.count(body)==1]
   if found:shared[group][name]=found
   else:added[group][name]=body
 if len(shared['Types'])!=112 or added['Types']:raise ValueError('all112 complete canonical parent types must remain')
 if len(shared['Funs'])!=167 or set(added['Funs'])!={ROOT,WRITER}:raise ValueError('167 verbatim parent bodies and exactly the original validate/writer additions required')
 for name in ('SourceFullWorkGuardLibrary','SourceSavedQuestionLibrary'):
  template=HERE.parent/('source-full-work-guard' if name=='SourceFullWorkGuardLibrary' else 'source-saved-question')/(name+'.lean.in')
  if template.read_bytes()!=(project/(name+'.lean')).read_bytes():raise ValueError('exact original checked library required: '+name)
 if (HERE/'SourceDrainValidationLibrary.lean.in').read_bytes()!=library():raise ValueError('exact26 inherited typed providers required')
 return plan,dict(parent=evidence,retention=kept_evidence,shared=shared,standard_interfaces=sorted(LIBRARIES)),added


def same_interfaces(correct,wrong):
 for group in ('Types','TypesExternal','External'):
  if correct[group]!=wrong[group]:raise ValueError('all complete types and typed interfaces must agree: '+group)
 if len(parsed(correct['Types']))!=112:raise ValueError('all112 canonical complete types required')
 left,right=parsed(correct['Funs']),parsed(wrong['Funs'])
 if len(left)!=169 or left.keys()!=right.keys():raise ValueError('all169 complete functions/dictionaries required')
 if {name for name in left if left[name]!=right[name]}!={ROOT}:raise ValueError('only original validator may change')
 old=b'else ok (core.result.Result.Err JournalError.CorruptStore)'
 new=b'else ok (core.result.Result.Ok ())'
 # The earlier observation mismatch return has different indentation but the
 # same suffix; the exact original late branch must be selected, not replaced globally.
 marker=b'                  '+old
 replacement=b'                  '+new
 if left[ROOT].count(marker)!=1 or left[ROOT].replace(marker,replacement,1)!=right[ROOT]:raise ValueError('only the one original late prior rejection return may be omitted')
 return right
