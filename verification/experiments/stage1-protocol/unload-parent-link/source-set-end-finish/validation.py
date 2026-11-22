"""Read exact raw output after the original strict admission has rejected.

This is not capture admission. Keep every external mapping and template;
actual body/dependency proofs and public adoption are separate obligations.
"""
import importlib.util,json,re,sys,tarfile
from pathlib import Path
from tempfile import TemporaryDirectory
HERE=Path(__file__).resolve().parent
PARENT=HERE.parent/'source-remove-retired'
sys.path.insert(0,str(PARENT))
def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path)
 value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
capture=module('source_set_end_capture',HERE/'capture.py')
inherited=module('source_set_end_parent_validation',HERE.parent/'source-end-receipt/validation.py')
original_inputs=inherited.original_inputs
digest=original_inputs.digest
SUPPORT=original_inputs.SUPPORT
REPO=capture.retire.REPO
ROOT='controller.lifecycle_inbox.source_work.full.end.set_end'
LIBRARIES=capture.LIBRARIES

def parsed(data):return original_inputs.parsed(data,capture.NAMESPACE)

def raw(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(Path(capture.base.__file__)),
        base_driver=digest(PARENT/'extract.py'), ports_driver=digest(PARENT/'ports.py'),
        capture_root=capture.ROOT, direct_original_set_end=True, whole_set_end_proved=False,
        complete_record_poststate_required=True, exact_proposal_encoder=capture.inputs.PORT,
        parent_driver=digest(Path(capture.parent.__file__)),staging_driver=digest(HERE.parent/'source-set-end/inputs.py'),
        mutation=None if variant=='original' else 'delete only early completed/NoRequest rejection; retain later receipt validation',
        complete=False,public_acceptance=False)
    if result.get('preflight_only') or any(result.get(name) != wanted for name, wanted in expected.items()):
        raise ValueError('the exact raw original full setter after failed strict admission is required')
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
    with TemporaryDirectory(prefix='source-set-end-', dir=directory.parent) as temporary:
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
    try:
        capture.admit(directory / 'actual.llbc', generated, fixed['target'])
    except ValueError as error:
        if not str(error).startswith('only exact30 existing per-symbol interfaces including original proposal encoding permitted: '):
            raise
    else:
        raise ValueError('this development reader requires the unchanged strict admission refusal')
    if 'admission' in result:
        raise ValueError('a rejected capture cannot carry a successful admission')
    sources = {name: (generated / capture.NAMESPACE / (name + '.lean')).read_bytes()
               for name in ('Types', 'Funs')}
    sources['External'] = (generated / capture.NAMESPACE / 'FunsExternal_Template.lean').read_bytes()
    sources['TypesExternal'] = (generated / capture.NAMESPACE / 'TypesExternal_Template.lean').read_bytes()
    libraries = set(re.findall(r'^axiom\s+([^\s(:]+)', sources['External'].decode(), re.M))
    expected_libraries = LIBRARIES | {"Box.Insts.CoreCmpPartialEqBox.ne"}
    if libraries != expected_libraries or len(libraries) != 31:
        raise ValueError('all31 raw declarations including the unprovided Box method must remain')
    return result, sources


def declaration(reference, group, name, body):
    return inherited.declaration(reference, group, name, body)

prefix = module('source_set_end_finish_prefix_validation', HERE.parent/'source-set-end/validation-prefix.py')
PRIOR = 'controller.lifecycle_inbox.source_work.full.drain.PriorCaptureEnd.Insts.CoreCmpPartialEqPriorCaptureEnd'
END = 'controller.lifecycle_inbox.source_work.full.end.CaptureEndRecord.Insts.CoreCmpPartialEqCaptureEndRecord'
OBSERVATION = 'capture.CaptureRequestEndObservation.Insts.CoreCmpPartialEqCaptureRequestEndObservation'
NEW = {OBSERVATION, ROOT}


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceSetEndPrefix', 216, 1208)
    kept, kept_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if (plan['source'], plan['tree']) != (capture.retire.SOURCE, capture.retire.TREE):
        raise ValueError('same exact production source/tree required')
    if any(plan['files'].get(n) != h for n, h in kept['files'].items()):
        raise ValueError('all retained26/127 files remain required')
    sources = {name: (project/name).read_bytes() for name in plan['files']}
    shared, added = {}, {}
    for group, count, total in [('Types', 117, 117), ('Funs', 199, 201)]:
        shared[group], added[group] = {}, {}
        rows = parsed(reference[group])
        if len(rows) != total:
            raise ValueError('all original complete declarations required: '+group)
        for name, body in rows.items():
            owners = [file for file, raw in sources.items() if raw.count(body) == 1]
            (shared[group] if owners else added[group])[name] = owners if owners else body
        if len(shared[group]) != count or set(added[group]) != (NEW if group == 'Funs' else set()):
            raise ValueError('all parent bodies and only two original additions required: '+group)
    return plan, dict(parent=evidence, retention=kept_evidence, shared=shared), added


def same_interfaces(reference, wrong):
    old_closure = (b"    Source: 'controller-service/src/controller/lifecycle_inbox/source_work/full/end.rs', "
                   b'lines 420:52-420:77 -/')
    new_closure = old_closure.replace(b'420:52-420:77', b'417:52-417:77')
    if (reference['Types'].count(old_closure) != 1
            or reference['Types'].replace(old_closure, new_closure, 1) != wrong['Types']
            or parsed(reference['Types']) != parsed(wrong['Types'])
            or reference['TypesExternal'] != wrong['TypesExternal']):
        raise ValueError('all117 canonical types and only the exact closure annotation shift required')
    old = b"Source: 'controller-service/src/controller/lifecycle_inbox/source_work/full/end.rs', lines 426:0-428:1"
    new = old.replace(b'426:0-428:1', b'423:0-425:1')
    if reference['External'].count(old) != 1 or reference['External'].replace(old, new, 1) != wrong['External']:
        raise ValueError('only the exact encoder source span moves by the deleted three-line guard')
    left, right = parsed(reference['Funs']), parsed(wrong['Funs'])
    proposal = 'controller.lifecycle_inbox.source_work.full.end.propose_end'
    if set(left) != set(right) or {n for n in left if left[n] != right[n]} != {proposal}:
        raise ValueError('only the actual early guard can differ; all201 original bodies remain')
    original = (b'              let b := core.option.Option.is_some val.completed\n'
        b'              let b1 \xe2\x86\x90\n'
        b'                controller.lifecycle_inbox.source_work.full.end.saved_completed_no_request\n'
        b'                  b observation1\n'
        b'              if b1\n'
        b'              then ok (core.result.Result.Err JournalError.InvalidHostEvidence)\n'
        b'              else\n'
        b'                ok (core.result.Result.Ok\n'
        b'                  (controller.lifecycle_inbox.source_work.full.end.CaptureEndRecord.WorkerEnded\n'
        b'                  observation1))')
    replacement = (b'              ok (core.result.Result.Ok\n'
        b'                (controller.lifecycle_inbox.source_work.full.end.CaptureEndRecord.WorkerEnded\n'
        b'                observation1))')
    if left[proposal].count(original) != 1 or left[proposal].replace(original, replacement, 1) != right[proposal]:
        raise ValueError('the actual complete wrong function must remove exactly the original early guard')
    return dict(changed_function=proposal, complete_other_functions=200,
        complete_canonical_types_identical=True, exact_closure_and_encoder_source_span_shifts=True,
        original_admission_failed=True, property_counterexample_requires_Lean=True)


def extra_inputs():
    return sorted(set(prefix.extra_inputs()) | {
        str((HERE.parent/'source-set-end/validation.py').relative_to(REPO)),
        str((HERE/'capture.py').relative_to(REPO)),
        str((HERE/'validation.py').relative_to(REPO))})
