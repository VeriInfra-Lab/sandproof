"""Bind original prior-end captures and complete unchanged parent evidence."""
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

capture = module('source_prior_end_match_capture', HERE / 'capture.py')
inherited = module('source_prior_end_match_parent_validation', HERE.parent / 'source-request-end-match/validation.py')
original_inputs = inherited.original_inputs
digest = original_inputs.digest
SUPPORT = original_inputs.SUPPORT
REPO = capture.retire.REPO
ROOT = 'controller.lifecycle_inbox.source_work.full.drain.prior_matches'
MATCHER = 'capture.CaptureRequestEndObservation.matches'
GUARD = 'controller.lifecycle_inbox.source_work.full.drain.prior_completed_no_request'
BINDING = 'capture.work.FullCaptureWorkRequest.binding_digest'
LIBRARIES = capture.LIBRARIES

def parsed(data):
    return original_inputs.parsed(data, capture.NAMESPACE)

def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE / 'capture.py'),
        base_driver=digest(PARENT / 'extract.py'), ports_driver=digest(PARENT / 'ports.py'),
        capture_root=capture.ROOT, direct_original_caller_with_transparent_guard=True, whole_receipt_proved=False,
        parent_driver=digest(capture.PARENT / 'capture.py'),
        original_completed_no_request_clause_moved_to_transparent_helper_with_original_bool_argument=(variant == 'original'),
        mutation=None if variant == 'original' else 'omit only completed/NoRequest rejection after actual request-end match',
        complete=True, public_acceptance=False)
    if result.get('preflight_only') or any(result.get(name) != wanted for name, wanted in expected.items()):
        raise ValueError('the exact completed original prior-end capture is required')
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
    with TemporaryDirectory(prefix='source-prior-end-match-', dir=directory.parent) as temporary:
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
        raise ValueError('only the exact seven typed library interfaces are permitted')
    return result, sources

def same_interfaces(correct, wrong):
    for group in ('Types', 'TypesExternal', 'External'):
        if correct[group] != wrong[group]:
            raise ValueError('all complete type/library outputs must agree: ' + group)
    if len(parsed(correct['Types'])) != 109:
        raise ValueError('all109 canonical type bodies are mandatory')
    left, right = parsed(correct['Funs']), parsed(wrong['Funs'])
    if (len(left), len(right)) != (51, 50) or left.keys() - right.keys() != {GUARD} or right.keys() - left.keys():
        raise ValueError('only the original late-guard helper may be pruned')
    if {n for n in right if left[n] != right[n]} != {ROOT}:
        raise ValueError('only original prior_matches may change')
    old = '''            if val3
            then
              let b := core.option.Option.is_some val.completed
              let b1 ←
                controller.lifecycle_inbox.source_work.full.drain.prior_completed_no_request
                  b observation
              ok (core.result.Result.Ok (¬ b1))
            else ok (core.result.Result.Ok false)
'''.encode()
    new = b'            ok (core.result.Result.Ok val3)\n'
    if left[ROOT].count(old) != 1:
        raise ValueError('the exact original post-match gate is required')
    expected = left[ROOT].replace(old, new, 1)
    # Aeneas renames exactly one branch-local temporary after pruning the gate.
    # Normalize this exact pair in memory only; preserve both original bodies.
    before = 'let b3 ←\n                core.option.Option.Insts.CoreCmpPartialEqOption.eq'.encode()
    after = 'let val2 ←\n                core.option.Option.Insts.CoreCmpPartialEqOption.eq'.encode()
    returned = b'ok (core.result.Result.Ok b3)\n'
    if expected.count(before) != 1 or expected.count(returned) != 1:
        raise ValueError('one exact optional-action temporary rename is required')
    expected = expected.replace(before, after, 1).replace(returned, b'ok (core.result.Result.Ok val2)\n', 1)
    if expected != right[ROOT]:
        raise ValueError('only gate removal and its exact branch-local alpha rename are permitted')
    return right


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceRequestEndMatchValues', 170, 885)
    kept, kept_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if (plan['source'], plan['tree']) != (capture.retire.SOURCE, capture.retire.TREE):
        raise ValueError('complete parent must retain the same fixed source')
    if any(plan['files'].get(n) != wanted for n, wanted in kept['files'].items()):
        raise ValueError('all original retention files must remain in the parent')
    pools = {
        'Types': ('SandproofSourceRetentionProviders/Types.lean', 'SourceRetireTypes.lean',
                  'SourceFullWorkGuardTypes.lean', 'SourceWorkerGuardTypes.lean'),
        'Funs': ('SourceRetireEqualityView.lean', 'SourceRetireView.lean',
                 'SourceRequestEndMatchView.lean', 'SourceFullWorkGuardView.lean'),
    }
    shared, added = {}, {}
    for group, files in pools.items():
        sources = {n: (project / n).read_bytes() for n in files}
        shared[group], added[group] = {}, {}
        for name, body in parsed(reference[group]).items():
            found = [file for file, raw in sources.items() if raw.count(body) == 1]
            if found: shared[group][name] = found
            else: added[group][name] = body
    if len(shared['Types']) != 109 or added['Types']:
        raise ValueError('all109 original canonical parent types must be reused')
    required = {ROOT, GUARD, 'controller.lifecycle_inbox.source_work.SourceWork.impl.requirement',
                'sandproof_protocol.WorkAction.Insts.CoreCmpPartialEqWorkAction'}
    if len(shared['Funs']) != 47 or set(added['Funs']) != required:
        raise ValueError('47 exact parent bodies and only four original additions required')
    if (HERE / 'SourcePriorEndMatchLibrary.lean.in').read_bytes() != library():
        raise ValueError('exact same parent per-symbol library bindings required')
    return plan, dict(parent=evidence, retention=kept_evidence, shared=shared,
        standard_interfaces=sorted(LIBRARIES), parent_library=plan['files']['SourceRequestEndMatchLibrary.lean'],
        parent_option_deref=plan['files']['SourceMetadataView.lean']), added


def library():
    aliases = (
        ('alloc.string.String.Insts.CoreCmpPartialEqString.eq', 'SourceRequestEndMatchActual.alloc.string.String.Insts.CoreCmpPartialEqString.eq'),
        ('core.option.Option.Insts.CoreCmpPartialEqOption.eq', '@SourceRequestEndMatchActual.core.option.Option.Insts.CoreCmpPartialEqOption.eq'),
        ('core.ops.range.RangeInclusive.contains', '@SourceRequestEndMatchActual.core.ops.range.RangeInclusive.contains'),
        ('core.option.Option.as_ref', '@SourceRetireLibrary.optionRef'),
        ('core.option.Option.as_deref', '@SourceMetadataView.optionalDeref'),
    )
    text = 'import SourcePriorEndMatchSpec\nopen Aeneas Aeneas.Std\nset_option autoImplicit false\n'
    for namespace in ('SourcePriorEndMatchActual', 'SourcePriorEndMatchWrongActual'):
        text += '\nnamespace SandboxModel.Stage1Protocol.' + namespace + '\n'
        for name, target in aliases: text += 'def ' + name + ' := ' + target + '\n'
        text += 'end SandboxModel.Stage1Protocol.' + namespace + '\n'
    return text.encode()


def declaration(reference, group, name, body):
    return inherited.declaration(reference, group, name, body)


def raw_functions(reference):
    bodies = []
    for name, raw in parsed(reference['Funs']).items():
        body = declaration(reference, 'Funs', name, raw)
        if name in (BINDING, MATCHER):
            body += ('\n\nlocal notation "' + name + '" => ' + name + ' io').encode()
        bodies.append(body)
    return b'\n\n'.join(bodies)
