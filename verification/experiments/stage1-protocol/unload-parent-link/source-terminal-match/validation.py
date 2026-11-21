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

capture = module('source_terminal_match_capture', HERE / 'capture.py')
inherited = module('source_terminal_match_parent_validation', HERE.parent / 'source-prior-end-match/validation.py')
original_inputs = inherited.original_inputs
digest = original_inputs.digest
SUPPORT = original_inputs.SUPPORT
REPO = capture.retire.REPO
ROOT = 'capture.terminated.TerminatedCaptureObservation.matches'
CONSTRUCTOR = 'capture.terminated.TerminatedCaptureWorkRequest.from_source'
TERMINAL_BINDING = 'capture.terminated.TerminatedCaptureWorkRequest.binding_digest'
BINDING = 'capture.work.FullCaptureWorkRequest.binding_digest'
LIBRARIES = capture.LIBRARIES

def parsed(data):
    return original_inputs.parsed(data, capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE / 'capture.py'),
        base_driver=digest(PARENT / 'extract.py'), ports_driver=digest(PARENT / 'ports.py'),
        capture_root=capture.ROOT, direct_original_terminal_match_and_bindings=True, whole_receipt_proved=False,
        parent_driver=digest(capture.PARENT / 'capture.py'),
        exact_library_ports=capture.PORTS,
        mutation=None if variant == 'original' else 'omit original host-slot comparison and discard its now-unused pattern binding',
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
    with TemporaryDirectory(prefix='source-terminal-match-', dir=directory.parent) as temporary:
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
        raise ValueError('only the exact six typed library interfaces are permitted')
    return result, sources


def same_interfaces(correct, wrong):
    for group in ('TypesExternal', 'External'):
        if correct[group] != wrong[group]:
            raise ValueError('the complete typed external outputs must agree: ' + group)
    left_types, right_types = parsed(correct['Types']), parsed(wrong['Types'])
    if len(left_types) != 60 or left_types != right_types:
        raise ValueError('all60 complete canonical type bodies must agree')
    old = correct['Types']
    begin = old.index(b'/-- [sandbox_controller_core::ids::TenantId]')
    end = old.index(b'/-- [sandbox_controller_core::ids::PolicyDigest]', begin)
    block = old[begin:end]
    rest = old[:begin] + old[end:]
    before = rest.index(b'/-- [sandbox_controller_core::maintenance::slots::HostSlotId]')
    if rest[:before] + block + rest[before:] != wrong['Types']:
        raise ValueError('only this exact whole independent alias movement is permitted')
    left, right = parsed(correct['Funs']), parsed(wrong['Funs'])
    removed = {
        'host_contract.HostSlotContext.Insts.CoreCmpPartialEqHostSlotContext.eq',
        'sandbox_controller_core.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq',
        'sandbox_controller_core.maintenance.slots.HostBinding.Insts.CoreCmpPartialEqHostBinding.eq',
        'sandbox_controller_core.maintenance.slots.HostSlotId.Insts.CoreCmpPartialEqHostSlotId.eq',
    }
    if (len(left), len(right)) != (52, 48) or left.keys() - right.keys() != removed or right.keys() - left.keys():
        raise ValueError('only the exact four now-unreachable host comparisons may be pruned')
    if {name for name in right if left[name] != right[name]} != {ROOT}:
        raise ValueError('only the original terminal matcher may change')
    old = '''                if b5
                then
                  let b6 ←
                    host_contract.HostSlotContext.Insts.CoreCmpPartialEqHostSlotContext.eq
                      host_slot work.host_slot
                  ok (core.result.Result.Ok b6)
                else ok (core.result.Result.Ok false)'''.encode()
    new = b'                ok (core.result.Result.Ok b5)'
    pattern = '    worker, host_slot, _, _ ⟩ :='.encode()
    replacement = '    worker, _, _, _ ⟩ :='.encode()
    if left[ROOT].count(old) != 1 or left[ROOT].count(pattern) != 1:
        raise ValueError('the exact original slot guard and pattern must occur once')
    if left[ROOT].replace(old, new, 1).replace(pattern, replacement, 1) != right[ROOT]:
        raise ValueError('only the slot comparison omission/discard is permitted')
    return right


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourcePriorEndMatchValues', 175, 905)
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
    if len(shared['Types']) != 59 or set(added['Types']) != {'capture.terminated.TerminatedCaptureWorkRequest'}:
        raise ValueError('59 same canonical types and only the one real terminal work type are mandatory')
    required = {ROOT, CONSTRUCTOR, TERMINAL_BINDING,
        'capture.launch.CaptureWorkerBinding.Insts.CoreCmpPartialEqCaptureWorkerBinding'}
    if len(shared['Funs']) != 48 or set(added['Funs']) != required:
        raise ValueError('48 verbatim parent bodies and only the four actual additions required')
    if (HERE / 'SourceTerminalMatchLibrary.lean.in').read_bytes() != library():
        raise ValueError('the same exact parent library interpretation is mandatory')
    return plan, dict(parent=evidence, retention=kept_evidence, shared=shared,
        standard_interfaces=sorted(LIBRARIES), parent_library=plan['files']['SourceRequestEndMatchLibrary.lean']), added


def library():
    aliases = (
        ('alloc.string.String.Insts.CoreCmpPartialEqString.eq',
         'SourceRequestEndMatchActual.alloc.string.String.Insts.CoreCmpPartialEqString.eq'),
        ('core.option.Option.Insts.CoreCmpPartialEqOption.eq',
         '@SourceRequestEndMatchActual.core.option.Option.Insts.CoreCmpPartialEqOption.eq'),
    )
    text = 'import SourceTerminalMatchSpec\nopen Aeneas Aeneas.Std\nset_option autoImplicit false\n'
    for namespace in ('SourceTerminalMatchActual', 'SourceTerminalMatchWrongActual'):
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
        if name in (BINDING, TERMINAL_BINDING):
            body += ('\n\nlocal notation "' + name + '" => ' + name + ' io').encode()
        bodies.append(body)
    return b'\n\n'.join(bodies)
