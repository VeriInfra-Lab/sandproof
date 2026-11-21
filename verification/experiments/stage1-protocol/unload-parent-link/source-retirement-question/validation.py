"""Bind complete current captures and the unchanged complete parent evidence."""
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


capture = module('source_retirement_question_capture', HERE / 'capture.py')
original_inputs = module('source_retirement_question_parent_inputs', PARENT / 'inputs.py')
digest = original_inputs.digest
SUPPORT = original_inputs.SUPPORT
REPO = capture.retire.REPO
ROOT = 'controller.lifecycle_inbox.source_work.retirement.check_question_record'
QUESTION = 'controller.lifecycle_inbox.source_work.retirement.MaterialRetirementQuestion'
EQUALITY = 'capture.retirement.CaptureMaterialRetirementRequest.Insts.CoreCmpPartialEqCaptureMaterialRetirementRequest'
GETTER = 'capture.retirement.CaptureMaterialRetirementRequest.impl.sandbox'
LIBRARIES = {
    'core.option.Option.as_ref', 'core.option.Option.Insts.CoreCloneClone.clone',
    'alloc.string.String.Insts.CoreCloneClone.clone',
}
EQUALITY_LIBRARIES = {
    'Bool.Insts.CoreCmpPartialEqBool.ne',
    'core.option.Option.Insts.CoreCmpPartialEqOption.eq',
    'alloc.string.String.Insts.CoreCmpPartialEqString.eq',
}


def parsed(data):
    return original_inputs.parsed(data, capture.NAMESPACE)


def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.OPTIONS, driver=digest(HERE / 'capture.py'),
        base_driver=digest(PARENT / 'extract.py'), ports_driver=digest(PARENT / 'ports.py'),
        capture_root=capture.ROOT, original_find_proved=False,
        tail_transformation='verbatim original request-check/return tail with optional sandbox-only mutation',
        mutation=None if variant == 'original' else 'compare only sandbox instead of the complete request',
        complete=True, public_acceptance=False)
    if any(result.get(name) != wanted for name, wanted in expected.items()):
        raise ValueError('the exact completed selected-record capture is required')
    if [(c['label'], c['exit_code'], c['bounded_stop']) for c in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both original fixed tools must actually finish successfully')
    original_inputs.read_exact(directory / 'source.tar', capture.retire.ARCHIVE)
    for command in result['commands']:
        original_inputs.read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    if set(result['staged']) != {str(path) for path in capture.retire.STAGED}:
        raise ValueError('all five exact staged files are mandatory')
    with TemporaryDirectory(prefix='source-retirement-question-', dir=directory.parent) as temporary:
        source = Path(temporary)
        capture.reconstruct(directory / 'source.tar', source, variant)
        with tarfile.open(directory / 'source.tar') as archive:
            members = {m.name: m for m in archive.getmembers() if m.isfile()}
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
    fixed = json.loads((REPO / 'verification/foundations-environment.json').read_bytes())
    if capture.admit(directory / 'actual.llbc', generated, fixed['target']) != result['admission']:
        raise ValueError('the original transparent business/library mapping changed')
    sources = {name: (generated / capture.NAMESPACE / (name + '.lean')).read_bytes()
               for name in ('Types', 'Funs')}
    sources['External'] = (generated / capture.NAMESPACE / 'FunsExternal_Template.lean').read_bytes()
    sources['TypesExternal'] = (generated / capture.NAMESPACE / 'TypesExternal_Template.lean').read_bytes()
    libraries = set(re.findall(r'^axiom\s+([^\s(:]+)', sources['External'].decode(), re.M))
    expected_libraries = LIBRARIES | (EQUALITY_LIBRARIES if variant == 'original' else set())
    if libraries != expected_libraries or sorted(libraries) != result['admission']['libraries']:
        raise ValueError('only the original exact String/Option/Bool interfaces are permitted')
    return result, sources


def same_interfaces(correct, wrong):
    if correct['TypesExternal'] != wrong['TypesExternal']:
        raise ValueError('the original external type declarations must remain byte-identical')
    left_types, right_types = parsed(correct['Types']), parsed(wrong['Types'])
    if len(left_types) != 111 or left_types != right_types:
        raise ValueError('all 111 complete actual type declarations must remain identical')
    # The backend's dependency ordering moves only these two unchanged scalar
    # aliases after ResultDigest when the full request comparison is pruned.
    policy = b'/-- [sandbox_controller_core::ids::PolicyDigest]\n'
    tenant = b'/-- [sandbox_controller_core::ids::TenantId]\n'
    slot = b'/-- [sandbox_controller_core::maintenance::slots::HostSlotId]\n'
    if any(correct['Types'].count(anchor) != 1 for anchor in (policy, tenant, slot)):
        raise ValueError('the original scalar-alias ordering anchors are required')
    first, last = correct['Types'].index(policy), correct['Types'].index(tenant)
    aliases = correct['Types'][first:last]
    reordered = correct['Types'][:first] + correct['Types'][last:]
    if reordered.replace(slot, aliases + slot, 1) != wrong['Types']:
        raise ValueError('only the two unchanged digest aliases may move in the type output')
    left, right = parsed(correct['Funs']), parsed(wrong['Funs'])
    shared = set(left) & set(right)
    if (len(left), len(right), len(shared)) != (125, 60, 59):
        raise ValueError('the complete actual function inventories are required')
    if {n for n in shared if left[n] != right[n]} != {ROOT} or set(right) - set(left) != {GETTER}:
        raise ValueError('only the selected root changes and the original sandbox getter becomes reachable')
    original = '''    let b ←
      core.cmp.PartialEq.ne.trait_default
        capture.retirement.CaptureMaterialRetirementRequest.Insts.CoreCmpPartialEqCaptureMaterialRetirementRequest
        val question.request'''.encode()
    changed = '''    let si ←
      capture.retirement.CaptureMaterialRetirementRequest.impl.sandbox val
    let si1 ←
      capture.retirement.CaptureMaterialRetirementRequest.impl.sandbox
        question.request
    let b ←
      core.cmp.PartialEq.ne.trait_default
        sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
        si si1'''.encode()
    if left[ROOT].count(original) != 1 or left[ROOT].replace(original, changed, 1) != right[ROOT]:
        raise ValueError('the actual wrong root must change only complete request equality to sandbox equality')
    # Removing the only use of request equality prunes its exact dependency
    # closure and emitted dictionaries, rather than preserving a fictitious
    # unchanged function inventory.
    pattern = re.compile(r'(?<![\w.])(' + '|'.join(map(re.escape,
        sorted(left, key=len, reverse=True))) + r')(?![\w.])')
    seen, pending = set(), [EQUALITY]
    while pending:
        name = pending.pop()
        if name in seen:
            continue
        seen.add(name)
        body = left[name].decode()
        pending.extend(n for n in pattern.findall(body) if n not in seen)
        if name.endswith('.eq') and name[:-3] in left:
            pending.append(name[:-3])
        if 'core.option.Option.Insts.CoreCmpPartialEqOption.eq' in body:
            pending.append('core.option.Option.Insts.CoreCmpPartialEqOption')
    missing = set(left) - set(right)
    if len(missing) != 66 or missing != seen - set(right):
        raise ValueError('every pruned declaration must belong to complete request equality and its dictionaries')
    return dict(root=right[ROOT], getter=right[GETTER], pruned=sorted(missing))


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceMaterialChargeCases', 133, 752)
    retained, retained_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if (plan['source'], plan['tree']) != (capture.retire.SOURCE, capture.retire.TREE):
        raise ValueError('the fixed production identity must agree with the entire parent')
    if any(plan['files'].get(name) != wanted for name, wanted in retained['files'].items()):
        raise ValueError('the entire retention provider project must remain in its parent')
    pools = {
        'Types': ('SandproofSourceRetentionProviders/Types.lean', 'SourceRetireTypes.lean'),
        'Funs': ('SandproofSourceRetentionProviders/Funs.lean', 'SourceRetireEqualityView.lean', 'SourceRetireView.lean'),
    }
    expected_new = {'Types': {QUESTION}, 'Funs': {EQUALITY, EQUALITY + '.eq', ROOT}}
    matched, additional = {}, {}
    for group, files in pools.items():
        data = parsed(reference[group])
        sources = {name: (project / name).read_bytes() for name in files}
        matched[group] = {}
        additional[group] = {}
        for name, body in data.items():
            # The namespace views preserve each complete raw declaration
            # verbatim. Their exact enclosing modules and original whole-file
            # service evidence have already been checked above.
            found = [file for file, source in sources.items() if source.count(body) == 1]
            if found:
                matched[group][name] = found
            else:
                additional[group][name] = body
        if (len(matched[group]) != (110 if group == 'Types' else 122)
                or set(additional[group]) != expected_new[group]):
            raise ValueError('all common complete generated declarations must be unchanged: ' + group)
    return plan, dict(parent=evidence, retention=retained_evidence, shared=matched), additional


def declaration(reference, group, name, body):
    raw = reference[group]
    if raw.count(body) != 1:
        raise ValueError('one complete original generated declaration is required: ' + name)
    at = raw.index(body)
    prefix = raw[raw.rfind(b'\n\n/--', 0, at):at]
    return (b'@[reducible]\n' if re.search(rb'@\[reducible\]\s*$', prefix) else b'') + body
