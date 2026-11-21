"""Bind complete current Full/Pause captures and exact unchanged proof inputs."""
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

capture = module('source_full_work_capture', HERE / 'capture.py')
inherited = module('source_full_work_parent_validation', HERE.parent / 'source-terminal-end/validation.py')
original_inputs = inherited.original_inputs
reuse = module('source_full_work_exact_reuse', HERE / 'reuse.py')
digest = original_inputs.digest
SUPPORT = original_inputs.SUPPORT
REPO = capture.retire.REPO
ROOT = 'capture.work.FullCaptureWorkRequest.valid'
NEW_TYPES = {'sandproof_protocol.lifecycle.manifest.ManifestError',
             'capture.work.PauseWorkRequest.valid.closure'}
LIBRARIES = capture.LIBRARIES

def parsed(data):
    return original_inputs.parsed(data, capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE / 'capture.py'),
        base_driver=digest(PARENT / 'extract.py'), ports_driver=digest(PARENT / 'ports.py'),
        capture_root=capture.ROOT, direct_original_roots=True, whole_receipt_proved=False,
        mutation=None if variant == 'original' else 'omit only state_bytes>0 in actual Full validity',
        complete=True, public_acceptance=False)
    if result.get('preflight_only') or any(result.get(name) != wanted for name, wanted in expected.items()):
        raise ValueError('the exact completed original Full/Pause capture is required')
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
    with TemporaryDirectory(prefix='source-full-work-guard-', dir=directory.parent) as temporary:
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
        raise ValueError('only the exact14 existing per-symbol standard interfaces are permitted')
    return result, sources

def same_interfaces(correct, wrong):
    for group in ('Types', 'TypesExternal', 'External'):
        if correct[group] != wrong[group]:
            raise ValueError('all exact complete type/library outputs must agree: ' + group)
    if len(parsed(correct['Types'])) != 26:
        raise ValueError('all26 complete actual types are mandatory')
    left, right = parsed(correct['Funs']), parsed(wrong['Funs'])
    if len(left) != 29 or left.keys() != right.keys():
        raise ValueError('both actual29 function/dictionary inventories must agree')
    if {name for name in left if left[name] != right[name]} != {ROOT}:
        raise ValueError('only the complete original Full validity root may differ')
    lines = left[ROOT].splitlines(keepends=True)
    first = lines.index(b'            if self.state_bytes > 0#u64\n')
    if lines[first + 1] != b'            then\n':
        raise ValueError('the exact original state-byte branch is required')
    last = lines.index(b'            else ok false\n', first + 2)
    middle = lines[first + 2:last]
    if not middle or any(not line.startswith(b'  ') for line in middle):
        raise ValueError('the complete unchanged original branch body is required')
    expected = b''.join((*lines[:first], *(line[2:] for line in middle), *lines[last + 1:]))
    if expected != right[ROOT]:
        raise ValueError('only the positive state guard and one generated indent level may disappear')
    return right[ROOT]


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceTerminalEndProof', 145, 804)
    retained, retained_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if (plan['source'], plan['tree']) != (capture.retire.SOURCE, capture.retire.TREE):
        raise ValueError('the fixed production source must agree with the complete parent')
    if any(plan['files'].get(name) != wanted for name, wanted in retained['files'].items()):
        raise ValueError('all exact original retention providers must remain in the parent')
    pools = {
        'Types': ('SandproofSourceRetentionProviders/Types.lean', 'SourceRetireTypes.lean'),
        'Funs': ('SandproofSourceRetentionProviders/Funs.lean', 'SourceRetireEqualityView.lean', 'SourceRetireView.lean'),
    }
    shared, added = {}, {}
    for group, files in pools.items():
        sources = {name: (project / name).read_bytes() for name in files}
        shared[group], added[group] = {}, {}
        for name, body in parsed(reference[group]).items():
            found = [file for file, source in sources.items() if source.count(body) == 1]
            if found:
                shared[group][name] = found
            else:
                added[group][name] = body
    if len(shared['Types']) != 24 or set(added['Types']) != NEW_TYPES:
        raise ValueError('all24 canonical parent types must match, with exactly two new actual types')
    if len(shared['Funs']) != 6 or len(added['Funs']) != 23:
        raise ValueError('the exact six shared/23 additional actual declarations are required')
    historical = {}
    for group in ('Types', 'Funs'):
        path = REPO / 'verification/aeneas/stage1-protocol/manifest/SandproofStage1Manifest' / (group + '.lean')
        old = original_inputs.parsed(path.read_bytes(), 'SandproofStage1ManifestRust')
        now = parsed(reference[group])
        common = old.keys() & now.keys()
        if len(common) != (20 if group == 'Types' else 19) or any(old[name] != now[name] for name in common):
            raise ValueError('all exact historical Manifest declarations must remain applicable: ' + group)
        historical[group] = dict(source=str(path.relative_to(REPO)), sha256=digest(path),
            unchanged_complete_declarations=sorted(common))
    return plan, dict(parent=evidence, retention=retained_evidence,
        shared=shared, historical=historical), added


def declaration(reference, group, name, body):
    return inherited.declaration(reference, group, name, body)


def raw_functions(reference):
    return b'\n\n'.join(declaration(reference, 'Funs', name, body)
        for name, body in parsed(reference['Funs']).items())


def extra_inputs():
    return [*reuse.input_paths(), *(str(Path('verification/aeneas/stage1-protocol/manifest/SandproofStage1Manifest') /
        (name + '.lean')) for name in ('Types', 'Funs'))]
