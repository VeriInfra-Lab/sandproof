"""Strict original end_reservation capture and exact fixed tool/source readback."""
import importlib.util,json,re,sys,tarfile
from pathlib import Path
from tempfile import TemporaryDirectory
HERE=Path(__file__).resolve().parent
PARENT=HERE.parent/'source-remove-retired'
sys.path.insert(0,str(PARENT))
def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path)
 value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
capture=module('source_end_reservation_capture',HERE/'capture.py')
inherited=module('source_end_reservation_parent_validation',HERE.parent/'source-end-receipt/validation.py')
original_inputs=inherited.original_inputs
digest=original_inputs.digest
SUPPORT=original_inputs.SUPPORT
REPO=capture.retire.REPO
ROOT='controller.lifecycle_inbox.source_work.full.end.reserved_record_bytes'
LIBRARIES=capture.LIBRARIES

def parsed(data):return original_inputs.parsed(data,capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE/'capture.py'),
        base_driver=digest(PARENT/'extract.py'), ports_driver=digest(PARENT/'ports.py'),
        capture_root=capture.ROOT, direct_original_reserved_record_bytes=True, whole_reservation_proved=False,
        complete_capture_encoding_preserved=True, exact_capture_encoder=capture.inputs.PORT,
        parent_driver=digest(Path(capture.parent.__file__)),staging_driver=digest(HERE/'inputs.py'),
        mutation=None if variant=='original' else 'change only absent-End reservation16384 to0',
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
    with TemporaryDirectory(prefix='source-end-reservation-', dir=directory.parent) as temporary:
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
        raise ValueError('only the exact thirty typed library interfaces are permitted')
    return result, sources

def library():
 return (HERE.parent/'source-end-receipt/SourceEndReceiptLibrary.lean.in').read_bytes().replace(b'SourceEndReceipt',b'SourceEndReservation')
def declaration(reference,group,name,body):return inherited.declaration(reference,group,name,body)
def extra_inputs():return inherited.extra_inputs()


def parent(project,audit,retention,retention_audit,reference):
 plan,evidence=original_inputs.prior(project,audit,'SourceEndReceiptValues',207,1175)
 kept,kept_evidence=original_inputs.prior(retention,retention_audit,'RetentionProviders',26,127)
 if (plan['source'],plan['tree'])!=(capture.retire.SOURCE,capture.retire.TREE):raise ValueError('same exact production input required')
 if any(plan['files'].get(n)!=wanted for n,wanted in kept['files'].items()):raise ValueError('all original26/127 retention files required')
 sources={n:(project/n).read_bytes()for n in plan['files']};shared={};added={}
 for group in ('Types','Funs'):
  shared[group]={};added[group]={}
  for n,body in parsed(reference[group]).items():
   found=[file for file,raw in sources.items()if raw.count(body)==1]
   if found:shared[group][n]=found
   else:added[group][n]=body
 if len(shared['Types'])!=116 or added['Types']:raise ValueError('all116 whole canonical types must be reused verbatim')
 if len(shared['Funs'])!=181 or set(added['Funs'])!={ROOT}:raise ValueError('181 whole functions/dictionaries reused; only original reservation addition permitted')
 return plan,dict(parent=evidence,retention=kept_evidence,shared=shared,standard_interfaces=sorted(LIBRARIES)),added

def same_interfaces(correct,wrong):
 for key in ('Types','External','TypesExternal'):
  if correct[key]!=wrong[key]:raise ValueError('complete actual types/interfaces must match')
 c=parsed(correct['Funs']);w=parsed(wrong['Funs'])
 if c.keys()!=w.keys()or[n for n in c if c[n]!=w[n]]!=[ROOT]:raise ValueError('only entire actual reservation root may change')
 old="""    let b := core.option.Option.is_none capture.end
    let i ←
      if b
      then ok controller.lifecycle_inbox.source_work.full.end.END_RECORD_BYTES
      else ok 0#usize
    let o ← lift (Usize.checked_add bytes i)""".encode()
 new='    let o ← lift (Usize.checked_add bytes 0#usize)'.encode()
 if c[ROOT].count(old)!=1 or c[ROOT].replace(old,new,1)!=w[ROOT]:raise ValueError('only exact observed pure constant hoist and missing budget pair permitted')
 return w[ROOT]
