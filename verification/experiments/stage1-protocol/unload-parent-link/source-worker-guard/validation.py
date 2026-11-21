"""Bind exact original worker captures and complete unchanged parent evidence."""
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

capture = module('source_worker_guard_capture', HERE / 'capture.py')
inherited = module('source_worker_guard_parent_validation', HERE.parent / 'source-full-work-guard/validation.py')
original_inputs = inherited.original_inputs
digest = original_inputs.digest
SUPPORT = original_inputs.SUPPORT
REPO = capture.retire.REPO
ROOT = 'capture.launch.CaptureWorkerBinding.valid'
LIBRARIES = capture.LIBRARIES
CALLBACK = ROOT + '.closure'

def parsed(data):
    return original_inputs.parsed(data, capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE / 'capture.py'),
        base_driver=digest(PARENT / 'extract.py'), ports_driver=digest(PARENT / 'ports.py'),
        capture_root=capture.ROOT, direct_original_constructor=True, whole_receipt_proved=False,
        mutation=None if variant == 'original' else 'omit only process.start_time_ticks>0 in actual worker validity',
        complete=True, public_acceptance=False)
    if result.get('preflight_only') or any(result.get(name) != wanted for name, wanted in expected.items()):
        raise ValueError('the exact completed original worker capture is required')
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
        raise ValueError('all six exact staged files are mandatory')
    with TemporaryDirectory(prefix='source-worker-guard-', dir=directory.parent) as temporary:
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
        raise ValueError('only the exact five existing per-symbol standard interfaces are permitted')
    return result, sources

def same_interfaces(correct, wrong):
    for group in ('TypesExternal', 'External'):
        if correct[group] != wrong[group]:
            raise ValueError('all complete existing library outputs must agree: ' + group)
    # Removing the real guard moves only the original callback source line.
    # Compare the raw files with precisely that comment shift; do not edit them.
    old = b"Source: 'controller-service/src/capture/launch.rs', lines 52:21-52:51"
    new = b"Source: 'controller-service/src/capture/launch.rs', lines 51:21-51:51"
    if correct['Types'].count(old) != 1 or correct['Types'].replace(old, new, 1) != wrong['Types']:
        raise ValueError('only the exact one-line original callback source comment may move')
    if parsed(correct['Types']) != parsed(wrong['Types']) or len(parsed(correct['Types'])) != 20:
        raise ValueError('all20 complete actual type bodies must remain identical')
    left, right = parsed(correct['Funs']), parsed(wrong['Funs'])
    if len(left) != 7 or left.keys() != right.keys():
        raise ValueError('both actual seven-function inventories must agree')
    if {name for name in left if left[name] != right[name]} != {ROOT}:
        raise ValueError('only original worker validity may change; constructor/callbacks must remain')
    lines = left[ROOT].splitlines(keepends=True)
    first = lines.index(b'    if self.process.start_time_ticks > 0#u64\n')
    if lines[first + 1] != b'    then\n':
        raise ValueError('the exact positive tick guard must be present')
    last = lines.index(b'    else ok false\n', first + 2)
    middle = lines[first + 2:last]
    if not middle or any(not line.startswith(b'  ') for line in middle):
        raise ValueError('the complete original validity tail is required')
    expected = b''.join((*lines[:first], *(line[2:] for line in middle), *lines[last + 1:]))
    if expected != right[ROOT]:
        raise ValueError('only the start-tick guard and one indent level may disappear')
    return right


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceFullWorkGuardValues', 158, 850)
    kept, kept_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if (plan['source'], plan['tree']) != (capture.retire.SOURCE, capture.retire.TREE):
        raise ValueError('the entire parent must retain the same fixed production source')
    if any(plan['files'].get(name) != wanted for name, wanted in kept['files'].items()):
        raise ValueError('all original retention provider files must remain in the parent')
    pools = {
        'Types': ('SandproofSourceRetentionProviders/Types.lean', 'SourceRetireTypes.lean'),
        'Funs': ('SourceFullWorkGuardView.lean',),
    }
    shared, added = {}, {}
    for group, files in pools.items():
        sources = {name: (project / name).read_bytes() for name in files}
        shared[group], added[group] = {}, {}
        for name, body in parsed(reference[group]).items():
            found = [file for file, raw in sources.items() if raw.count(body) == 1]
            if found:
                shared[group][name] = found
            else:
                added[group][name] = body
    if len(shared['Types']) != 19 or set(added['Types']) != {CALLBACK}:
        raise ValueError('all19 shared canonical types and only the original Unit callback are required')
    if set(shared['Funs']) != {'core.num.U8.is_ascii_graphic'} or len(added['Funs']) != 6:
        raise ValueError('the exact original ASCII body and six actual worker declarations are required')
    expected_library = library()
    if (HERE / 'SourceWorkerGuardLibrary.lean.in').read_bytes() != expected_library:
        raise ValueError('both namespaces must use the same exact five parent call bindings')
    return plan, dict(parent=evidence, retention=kept_evidence, shared=shared,
        standard_interfaces=sorted(LIBRARIES), parent_library=plan['files']['SourceFullWorkGuardLibrary.lean']), added


def library():
    symbols = ('alloc.string.String.Insts.CoreOpsDerefDerefStr.deref',
        'alloc.string.String.is_empty', 'alloc.string.String.len', 'core.str.Str.bytes',
        'core.str.iter.Bytes.Insts.CoreIterTraitsIteratorIteratorU8.all')
    text = 'import SourceWorkerGuardTypes\nopen Aeneas Aeneas.Std\nset_option autoImplicit false\n'
    for namespace in ('SourceWorkerGuardActual', 'SourceWorkerGuardWrongActual'):
        text += '\nnamespace SandboxModel.Stage1Protocol.' + namespace + '\n'
        for name in symbols:
            text += 'def ' + name + ' := ' + ('@' if name.endswith('.all') else '') + 'SourceFullWorkGuardActual.' + name + '\n'
        text += 'end SandboxModel.Stage1Protocol.' + namespace + '\n'
    return text.encode()


def declaration(reference, group, name, body):
    return inherited.declaration(reference, group, name, body)


def raw_functions(reference):
    return b'\n\n'.join(declaration(reference, 'Funs', name, body)
        for name, body in parsed(reference['Funs']).items())
