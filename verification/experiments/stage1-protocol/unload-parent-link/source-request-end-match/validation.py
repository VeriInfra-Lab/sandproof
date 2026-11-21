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

capture = module('source_request_end_match_capture', HERE / 'capture.py')
reuse = module('source_request_end_match_proof_reuse', HERE / 'reuse.py')
inherited = module('source_request_end_match_parent_validation', HERE.parent / 'source-worker-guard/validation.py')
original_inputs = inherited.original_inputs
digest = original_inputs.digest
SUPPORT = original_inputs.SUPPORT
REPO = capture.retire.REPO
ROOT = 'capture.CaptureRequestEndObservation.matches'
BINDING = 'capture.work.FullCaptureWorkRequest.binding_digest'
LIBRARIES = capture.LIBRARIES

def parsed(data):
    return original_inputs.parsed(data, capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE / 'capture.py'),
        base_driver=digest(PARENT / 'extract.py'), ports_driver=digest(PARENT / 'ports.py'),
        capture_root=capture.ROOT, direct_original_match_and_binding=True, whole_receipt_proved=False,
        exact_library_ports=capture.PORTS,
        mutation=None if variant == 'original' else 'replace only complete worker comparison by original process getter comparison',
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
        raise ValueError('all seven exact staged files are mandatory')
    with TemporaryDirectory(prefix='source-request-end-match-', dir=directory.parent) as temporary:
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
        raise ValueError('only the exact five typed library interfaces are permitted')
    return result, sources

def same_interfaces(correct, wrong):
    for group in ('Types', 'TypesExternal', 'External'):
        if correct[group] != wrong[group]:
            raise ValueError('all complete type/library outputs must agree: ' + group)
    if len(parsed(correct['Types'])) != 51:
        raise ValueError('all51 complete original type bodies are mandatory')
    left, right = parsed(correct['Funs']), parsed(wrong['Funs'])
    removed = {'capture.launch.CaptureWorkerBinding.Insts.CoreCmpPartialEqCaptureWorkerBinding.eq'}
    added = {'capture.SourceProcessIdentity.Insts.CoreCmpPartialEqSourceProcessIdentity',
             'capture.launch.CaptureWorkerBinding.impl.process'}
    if (len(left), len(right)) != (45, 46) or left.keys() - right.keys() != removed or right.keys() - left.keys() != added:
        raise ValueError('only omitted worker equality and original getter/process dictionary may change')
    if {name for name in left.keys() & right.keys() if left[name] != right[name]} != {ROOT}:
        raise ValueError('only original request-end matches may change')
    old = '''        let b2 ←
          capture.launch.CaptureWorkerBinding.Insts.CoreCmpPartialEqCaptureWorkerBinding.eq
            self.worker worker
'''.encode()
    new = '''        let spi ←
          capture.launch.CaptureWorkerBinding.impl.process self.worker
        let spi1 ← capture.launch.CaptureWorkerBinding.impl.process worker
        let b2 ←
          capture.SourceProcessIdentity.Insts.CoreCmpPartialEqSourceProcessIdentity.eq
            spi spi1
'''.encode()
    if left[ROOT].count(old) != 1 or left[ROOT].replace(old, new, 1) != right[ROOT]:
        raise ValueError('only complete worker equality may become its original process getter comparison')
    if right['capture.launch.CaptureWorkerBinding.impl.process'] != b'''def capture.launch.CaptureWorkerBinding.impl.process
  (self : capture.launch.CaptureWorkerBinding) :
  Result capture.SourceProcessIdentity
  := do
  ok self.process''':
        raise ValueError('the original complete getter body is required')
    return right


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceWorkerGuardValues', 164, 867)
    kept, kept_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if (plan['source'], plan['tree']) != (capture.retire.SOURCE, capture.retire.TREE):
        raise ValueError('the complete parent must retain the same fixed source')
    if any(plan['files'].get(name) != wanted for name, wanted in kept['files'].items()):
        raise ValueError('all original retention files must remain in the parent')
    pools = {
        'Types': ('SandproofSourceRetentionProviders/Types.lean', 'SourceRetireTypes.lean',
                  'SourceFullWorkGuardTypes.lean', 'SourceWorkerGuardTypes.lean'),
        'Funs': ('SourceRetireEqualityView.lean', 'SourceRetireView.lean',
                 'SourceFullWorkGuardView.lean', 'SourceWorkerGuardView.lean'),
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
    if len(shared['Types']) != 51 or added['Types']:
        raise ValueError('all51 complete canonical parent types must be reused, without new protocol types')
    if len(parsed(reference['Funs'])) != 45 or len(shared['Funs']) != 5:
        raise ValueError('all45 complete original functions, including five verbatim parent bodies, are mandatory')
    if (HERE / 'SourceRequestEndMatchLibrary.lean.in').read_bytes() != library():
        raise ValueError('both namespaces must use the exact same parent standard bindings')
    return plan, dict(parent=evidence, retention=kept_evidence, shared=shared,
        standard_interfaces=sorted(LIBRARIES), parent_library=plan['files']['SourceFullWorkGuardLibrary.lean']), added


def library():
    aliases = (
        ('alloc.string.String.Insts.CoreCmpPartialEqString.eq',
         'SandproofSourceRetentionProviders.alloc.string.String.Insts.CoreCmpPartialEqString.eq'),
        ('core.option.Option.Insts.CoreCmpPartialEqOption.eq',
         '@SandproofSourceRetentionProviders.core.option.Option.Insts.CoreCmpPartialEqOption.eq'),
        ('core.ops.range.RangeInclusive.contains',
         '@SourceFullWorkGuardActual.core.ops.range.RangeInclusive.contains'),
    )
    text = 'import SourceRequestEndMatchSpec\nopen Aeneas Aeneas.Std\nset_option autoImplicit false\n'
    for namespace in ('SourceRequestEndMatchActual', 'SourceRequestEndMatchWrongActual'):
        text += '\nnamespace SandboxModel.Stage1Protocol.' + namespace + '\n'
        for name, target in aliases:
            text += 'def ' + name + ' := ' + target + '\n'
        text += 'end SandboxModel.Stage1Protocol.' + namespace + '\n'
    return text.encode()


def declaration(reference, group, name, body):
    return inherited.declaration(reference, group, name, body)


def raw_functions(reference):
    bodies = []
    for name, raw in parsed(reference['Funs']).items():
        body = declaration(reference, 'Funs', name, raw)
        if name == BINDING:
            body += ('\n\nlocal notation "' + name + '" => ' + name + ' io').encode()
        bodies.append(body)
    return b'\n\n'.join(bodies)
