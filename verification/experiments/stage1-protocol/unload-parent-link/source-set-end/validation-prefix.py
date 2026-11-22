"""Strict original set_end capture and exact fixed tool/source readback."""
import importlib.util,json,re,sys,tarfile
from pathlib import Path
from tempfile import TemporaryDirectory
HERE=Path(__file__).resolve().parent
PARENT=HERE.parent/'source-remove-retired'
sys.path.insert(0,str(PARENT))
def module(name,path):
 spec=importlib.util.spec_from_file_location(name,path)
 value=importlib.util.module_from_spec(spec);spec.loader.exec_module(value);return value
capture=module('source_set_end_capture',HERE/'capture-prefix.py')
inherited=module('source_set_end_parent_validation',HERE.parent/'source-end-receipt/validation.py')
original_inputs=inherited.original_inputs
digest=original_inputs.digest
SUPPORT=original_inputs.SUPPORT
REPO=capture.retire.REPO
ROOT='controller.lifecycle_inbox.source_work.full.end.propose_end'
LIBRARIES=capture.LIBRARIES

def parsed(data):return original_inputs.parsed(data,capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE/'capture-prefix.py'),
        base_driver=digest(PARENT/'extract.py'), ports_driver=digest(PARENT/'ports.py'),
        capture_root=capture.ROOT, transparent_original_prefix=True, whole_set_end_proved=False,
        full_setter_admission_unchanged=True,
        parent_driver=digest(Path(capture.parent.__file__)),staging_driver=digest(HERE/'inputs.py'),
        mutation=None if variant=='original' else 'delete only early completed/NoRequest rejection; later setter still separately open',
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


def declaration(reference,group,name,body):return inherited.declaration(reference,group,name,body)


CLOSURE = ROOT + '.closure'
CALL = CLOSURE + '.Insts.CoreOpsFunctionFnOnceTupleWorkActionEffectIdBoxWorkActionEffectId'
NEW = {'Types': {CLOSURE}, 'Funs': {ROOT, CALL, CALL + '.call_once'}}


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceEndReceiptValues', 207, 1175)
    kept, kept_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if (plan['source'], plan['tree']) != (capture.retire.SOURCE, capture.retire.TREE):
        raise ValueError('the exact original production identity must agree with the entire parent')
    if any(plan['files'].get(name) != wanted for name, wanted in kept['files'].items()):
        raise ValueError('the entire retained26/127 provider must remain present')
    sources = {name: (project / name).read_bytes() for name in plan['files']}
    shared, added = {}, {}
    for group, count in (('Types', 116), ('Funs', 182)):
        shared[group], added[group] = {}, {}
        for name, raw in parsed(reference[group]).items():
            found = [file for file, source in sources.items() if source.count(raw) == 1]
            if found:
                shared[group][name] = found
            else:
                added[group][name] = raw
        if len(shared[group]) != count or set(added[group]) != NEW[group]:
            raise ValueError('all complete parent declarations and only original prefix additions required: ' + group)
    return plan, dict(parent=evidence, retention=kept_evidence, shared=shared), added


def same_interfaces(reference, wrong):
    for key in ('External', 'TypesExternal'):
        if reference[key] != wrong[key]:
            raise ValueError('the exact same complete per-symbol interfaces are mandatory')
    # Removing the three-line guard moves only this closure's source annotation.
    # Compare that exact reviewed metadata delta, not a general comment eraser.
    old_span = (b"    Source: 'controller-service/src/controller/lifecycle_inbox/source_work/full/end.rs', "
        b'lines 420:52-420:77 -/')
    new_span = (b"    Source: 'controller-service/src/controller/lifecycle_inbox/source_work/full/end.rs', "
        b'lines 417:52-417:77 -/')
    if (reference['Types'].count(old_span) != 1
            or reference['Types'].replace(old_span, new_span, 1) != wrong['Types']
            or parsed(reference['Types']) != parsed(wrong['Types'])
            or len(parsed(reference['Types'])) != 117):
        raise ValueError('all117 complete canonical types and only the exact three-line closure span shift required')
    left, right = parsed(reference['Funs']), parsed(wrong['Funs'])
    if (set(left) != set(right) or [name for name in left if left[name] != right[name]] != [ROOT]):
        raise ValueError('only the original completed/NoRequest proposal guard may differ')
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
    if left[ROOT].count(original) != 1 or left[ROOT].replace(original, replacement, 1) != right[ROOT]:
        raise ValueError('the actual complete wrong function must remove exactly the original early guard')
    return dict(changed_function=ROOT, actual_guard_only_difference=True)


def raw_functions(reference):
    return b'\n\n'.join(declaration(reference, 'Funs', name, body)
        for name, body in parsed(reference['Funs']).items() if name in NEW['Funs'])


def extra_inputs():
    # Trace loaded repository Python modules, including the spec-loaded parent
    # chain. Normal imported helper modules are in sys.modules; custom parents
    # are reached through their module globals. No host path enters the plan.
    from types import ModuleType
    pending = [sys.modules[__name__]] if __name__ in sys.modules else []
    pending += [capture, inherited, original_inputs, *tuple(sys.modules.values())]
    visited, files = set(), set(inherited.extra_inputs())
    files.add(str((PARENT / 'check.py').relative_to(REPO)))
    while pending:
        value = pending.pop()
        if not isinstance(value, ModuleType) or id(value) in visited:
            continue
        visited.add(id(value))
        filename = getattr(value, '__file__', None)
        if not filename or str(filename).startswith('<'):
            continue
        path = Path(filename).resolve()
        if not path.is_relative_to(REPO):
            continue
        if path.suffix != '.py' or not path.is_file():
            raise ValueError('one complete repository Python source is required')
        files.add(str(path.relative_to(REPO)))
        pending.extend(item for item in vars(value).values() if isinstance(item, ModuleType))
    return sorted(files)
