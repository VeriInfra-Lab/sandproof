"""Exact capture, parent views and per-symbol evidence for material charging."""
import importlib.util
import json
from pathlib import Path
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


capture = module('source_material_charge_capture', HERE / 'capture.py')
original_inputs = module('source_material_charge_original_inputs', PARENT / 'inputs.py')
original_prepare = module('source_material_charge_original_prepare', PARENT / 'prepare.py')
digest = original_inputs.digest
SUPPORT = original_inputs.SUPPORT
REPO = capture.retire.REPO
ROOT = 'controller.lifecycle_inbox.source_work.retirement.material_charge'


def current(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    expected = dict(source=capture.retire.SOURCE, tree=capture.retire.TREE, variant=variant,
        options=capture.retire.OPTIONS, driver=digest(HERE / 'capture.py'),
        base_driver=digest(PARENT / 'extract.py'), ports_driver=digest(PARENT / 'ports.py'),
        capture_root=capture.retire.ROOT, selected_business=capture.SELECTED,
        mutation=None if variant == 'original' else 'omit settled released-material byte equality only',
        complete=True, public_acceptance=False)
    if any(result.get(name) != wanted for name, wanted in expected.items()):
        raise ValueError('the exact completed current material-charge capture is required')
    if [(c['label'], c['exit_code'], c['bounded_stop']) for c in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both original fixed tools must actually finish successfully')
    original_inputs.read_exact(directory / 'source.tar', capture.retire.ARCHIVE)
    for command in result['commands']:
        original_inputs.read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    if set(result['staged']) != {str(path) for path in capture.retire.STAGED}:
        raise ValueError('all five exact staging inputs are mandatory')
    with TemporaryDirectory(prefix='source-material-charge-', dir=directory.parent) as temporary:
        source = Path(temporary)
        capture.retire.reconstruct(directory / 'source.tar', source, 'original')
        target = source / capture.retire.ports.RETIREMENT
        target.write_text(capture.mutate(target.read_text(), variant))
        with tarfile.open(directory / 'source.tar') as archive:
            members = {m.name: m for m in archive.getmembers() if m.isfile()}
        paths = list((directory / 'source').rglob('*'))
        if any(path.is_symlink() for path in paths):
            raise ValueError('the exact captured source cannot contain symlinks')
        actual = {str(p.relative_to(directory / 'source')) for p in paths if p.is_file()}
        if actual != set(members):
            raise ValueError('the entire captured source inventory must remain present')
        for name, member in members.items():
            captured = directory / 'source' / name
            if (digest(captured) != digest(source / name)
                    or bool(captured.stat().st_mode & 0o111) != bool(member.mode & 0o111)):
                raise ValueError('current full source differs from exact reconstruction: ' + name)
        for name, wanted in result['staged'].items():
            original_inputs.read_exact(directory / 'source' / name, wanted)
    original_inputs.read_exact(directory / 'actual.llbc', result['ir'])
    generated = directory / 'generated'
    namespace = capture.retire.NAMESPACE
    names = {'translation.json', *(namespace + '/' + name + '.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if (set(result['generated']) != names
            or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != names):
        raise ValueError('all current generated files are required')
    for name, wanted in result['generated'].items():
        original_inputs.read_exact(generated / name, wanted)
    fixed = json.loads((REPO / 'verification/foundations-environment.json').read_bytes())
    if capture.retire.admit(directory / 'actual.llbc', generated, fixed['target']) != result['admission']:
        raise ValueError('all actual transparent business and exact typed library calls are mandatory')
    sources = {name: (generated / namespace / (name + '.lean')).read_bytes() for name in ('Types', 'Funs')}
    sources['External'] = (generated / namespace / 'FunsExternal_Template.lean').read_bytes()
    sources['TypesExternal'] = (generated / namespace / 'TypesExternal_Template.lean').read_bytes()
    return result, sources


def same_interfaces(correct, wrong):
    if any(correct[name] != wrong[name] for name in ('Types', 'TypesExternal')):
        raise ValueError('the complete actual type inventory must remain byte-identical')
    before = b"Source: 'controller-service/src/controller/lifecycle_inbox/source_work/retirement.rs', lines 551:0-553:1"
    after = b"Source: 'controller-service/src/controller/lifecycle_inbox/source_work/retirement.rs', lines 550:0-552:1"
    if (correct['External'].count(before) != 1
            or correct['External'].replace(before, after, 1) != wrong['External']):
        raise ValueError('only the state encoder source comment may move after deleting one Rust line')
    left = original_inputs.parsed(correct['Funs'], capture.retire.NAMESPACE)
    right = original_inputs.parsed(wrong['Funs'], capture.retire.NAMESPACE)
    if left.keys() != right.keys() or {n for n in left if left[n] != right[n]} != {ROOT}:
        raise ValueError('only the original material_charge body may differ')
    original = b'''                            else
                              if settlement.released_material_bytes !=
                                capture.reserved_material_bytes
                              then
                                ok (core.result.Result.Err
                                  JournalError.CorruptStore)
                              else ok (core.result.Result.Ok 0#u64)'''
    changed = b'''                            else ok (core.result.Result.Ok 0#u64)'''
    if left[ROOT].count(original) != 1 or left[ROOT].replace(original, changed, 1) != right[ROOT]:
        raise ValueError('the actual wrong body must omit exactly the settled-byte comparison')
    return right[ROOT]


def parent(project, audit, retention, retention_audit, reference):
    plan, evidence = original_inputs.prior(project, audit, 'SourceRetireCases', 130, 675)
    retained, retained_evidence = original_inputs.prior(retention, retention_audit, 'RetentionProviders', 26, 127)
    if any(plan['files'].get(name) != wanted for name, wanted in retained['files'].items()):
        raise ValueError('the complete original retention project must be preserved in its parent')
    selected = original_inputs.select(reference, retention)
    expected = original_prepare.views(reference, reference, selected, False)
    for name, body in expected.items():
        if (project / (name + '.lean')).read_bytes() != body:
            raise ValueError('the full original parent view must reproduce from current capture: ' + name)
    return plan, dict(parent=evidence, retention=retained_evidence,
        shared=selected['shared'], exact_parent_views=sorted(expected))
