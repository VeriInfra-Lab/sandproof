"""Reinstantiate existing complete policy proofs on the captured current reader."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import sys
from tempfile import TemporaryDirectory
from types import SimpleNamespace

import extract
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact

HERE = Path(__file__).resolve().parent
COUNT_KEYS = {'actual', 'prior', 'audit', 'bounds_actual', 'reservation_inputs'}
HISTORY = {
    'SourcePolicyCapacityBounds': ('inbox-load', 'InboxLoadCapacityBounds'),
    'SourcePolicyCapacityValidation': ('inbox-load', 'InboxLoadCapacityValidation'),
    'SourcePolicyMaintenance': ('inbox-load', 'InboxLoadMaintenanceBounds'),
    'SourcePolicyConfigRead': ('inbox-load', 'InboxLoadConfigRead'),
    'SourcePolicyConfigComposition': ('inbox-load', 'InboxLoadConfigComposition'),
    'SourcePolicyValues': ('inbox-load', 'InboxLoadConfigValues'),
    'SourcePolicyCheckedArithmetic': ('inbox-load', 'InboxLoadCheckedArithmetic'),
    'SourcePolicyNormalValues': ('inbox-load', 'InboxLoadConfigNormalValues'),
}
IMPORTS = {'ShapeIteration': 'SourcePolicyIteration', 'InboxLoadValueLibrary': 'SourcePolicyReadView', **{old: new for new, (_, old) in HISTORY.items()}}
SUPPORT_TYPES = ('core.ops.range.Bound', 'core.ops.range.RangeBounds', 'serde_core.de.Error',
                 'serde_core.de.Visitor', 'serde_core.de.Deserializer', 'serde_core.de.Deserialize')
PREFIX = 'controller.checkpoint.config.'
READ_TYPES = (PREFIX + 'ReadConfigEffects', PREFIX + 'StandardConfigRead',
              PREFIX + 'ReadConfigEffectsStandardConfigRead.query_bytes.closure',
              PREFIX + 'ReadConfigEffectsStandardConfigRead.query_i64.closure')
PURE_PREFIXES = ('core.ops.range.', 'controller.capacity.', PREFIX + 'MaintenanceConfig.validate',
                 'host_contract.', 'journal.')
SKIPPED_DICTIONARIES = {'ArrayShared0P0.Insts.RusqliteParamsSealedSealed',
                       'ArrayShared0P0.Insts.RusqliteParamsParams',
                       'Usize.Insts.RusqliteRowSealedSealed', 'Usize.Insts.RusqliteRowRowIndex',
                       'I64.Insts.RusqliteTypesFrom_sqlFromSql', 'alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql'}


def load_count():
    directory = HERE.parent / 'source-record-count'
    previous = sys.modules.pop('extract', None)
    sys.path.insert(0, str(directory))
    try:
        spec = importlib.util.spec_from_file_location('source_count_prepare', directory / 'prepare.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path.remove(str(directory))
        sys.modules.pop('extract', None)
        if previous is not None:
            sys.modules['extract'] = previous


def parsed(directory, kind):
    data = (directory / 'generated' / extract.NAMESPACE / (kind + '.lean')).read_bytes()
    data = data.removesuffix(b'\n\nend ' + extract.NAMESPACE.encode() + b'\n')
    result = {}
    for block in data.split(b'\n\n/--'):
        match = re.search(rb'^(?:def|impl_def|structure|inductive)\s+([^\s(:]+)', block, re.M)
        if match is None:
            continue
        name = match[1].decode()
        if name in result:
            raise ValueError('duplicate generated declaration: ' + name)
        attribute = block.rfind(b'\n@[', 0, match.start())
        start = attribute + 1 if attribute >= 0 else match.start()
        result[name] = block[start:]
    return result


def captured(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    if (result['source'], result['tree'], result['variant'], result['options'], result['driver'], result['complete']) != (
            extract.SOURCE, extract.TREE, variant, extract.OPTIONS, extract.sql.digest(Path(extract.__file__)), True):
        raise ValueError('the exact completed current read capture is required')
    if [(r['label'], r['exit_code'], r['bounded_stop']) for r in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed tools must complete normally')
    read_exact(directory / 'source.tar', extract.ARCHIVE)
    for command in result['commands']:
        read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    with TemporaryDirectory(prefix='source-policy-input-') as temporary:
        source = Path(temporary)
        extract.reconstruct(directory / 'source.tar', source, variant)
        if set(result['staged']) != {str(p) for p in (extract.CONFIG, extract.CHECKPOINT, extract.LIB)}:
            raise ValueError('the complete three-file staging inventory is required')
        for name, wanted in result['staged'].items():
            if read_exact(directory / 'source' / name, wanted) != (source / name).read_bytes():
                raise ValueError('original body, sole mutation, roots or naming-only metadata differs')
    read_exact(directory / 'actual.llbc', result['ir'])
    generated = directory / 'generated'
    expected = {'translation.json', *(extract.NAMESPACE + '/' + n + '.lean' for n in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if set(result['generated']) != expected or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != expected:
        raise ValueError('the complete generation inventory is required')
    for name, wanted in result['generated'].items():
        read_exact(generated / name, wanted)
    fixed = json.loads((extract.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    if extract.admit(directory / 'actual.llbc', generated, fixed['target']) != result['admission']:
        raise ValueError('the complete original read/validator/forwarding mapping differs')
    return result, parsed(directory, 'Types'), parsed(directory, 'Funs')


def option_model():
    path = extract.sql.ROOT / 'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean'
    body = path.read_bytes()
    marker = b'def core.option.Option.as_ref '
    if body.count(marker) != 1:
        raise ValueError('the single existing V0 Option model is required')
    start = body.index(marker)
    return body[start:body.index(b'\n\n', start)]


def historic(module, record):
    directory, original = HISTORY[module]
    path = HERE.parent.parent / directory / (original + '.lean.in')
    body = path.read_bytes()
    record[str(path.relative_to(extract.sql.ROOT))] = sha256(body).hexdigest()
    text = body.decode().replace('import SandproofStage1Request.Funs\n', 'import SourcePolicyReadView\n')
    text = re.sub(r'^import SandboxModel.Lifecycle.Protocol.(\w+)$',
                  lambda m: 'import ' + IMPORTS[m[1]], text, flags=re.M)
    if module == 'SourcePolicyConfigComposition':
        first = text.index('def capacityReadView ')
        end = text.index('-- Shared full-value contract', first)
        text = text[:first] + text[end:]
        text = text.replace('#audit_controller_proof SandboxModel.Stage1Protocol.InboxLoadConfig.capacity_read_refines\n', '')
    return text.encode()


def bind_success(history):
    path = HERE.parent.parent / 'inbox-load/InboxLoadSuccess.lean.in'
    body = path.read_bytes()
    history[str(path.relative_to(extract.sql.ROOT))] = sha256(body).hexdigest()
    first = body.index(b'theorem bind_success_value ')
    end = body.index(b'\ntheorem attempt_success_value ', first)
    return body[first:end]


def iteration(record):
    path = HERE.parent.parent / 'session-shape/ShapeIteration.lean.in'
    body = path.read_bytes()
    record[str(path.relative_to(extract.sql.ROOT))] = sha256(body).hexdigest()
    text = body.decode()
    first = text.index('theorem index_add_one ')
    end = text.index('\n\nend\nend SandboxModel.', first)
    return ('import SourcePolicyReadView\n\nopen Aeneas Aeneas.Std\n'
            'namespace SandboxModel.Stage1Protocol.SessionShape\nnoncomputable section\n'
            'open scoped Classical\n\n' + text[first:end] +
            '\n\nend\nend SandboxModel.Stage1Protocol.SessionShape\n\n'
            '#audit_controller_proof SandboxModel.Stage1Protocol.SessionShape.index_add_one\n'
            '#audit_controller_proof SandboxModel.Stage1Protocol.SessionShape.indexed_any_loop\n').encode()


def prepare(args):
    reference, types, functions = captured(args.reference, 'original')
    selected = json.loads((args.actual / 'result.json').read_bytes())['variant']
    actual, actual_types, actual_functions = captured(args.actual, selected)
    if actual_types != types or actual_functions.keys() != functions.keys():
        raise ValueError('all current types, fields and function names must remain the same')
    changed = {n for n in functions if functions[n] != actual_functions[n]}
    if changed != (set() if selected == 'original' else {PREFIX + 'read_using'}):
        raise ValueError('only the original decoded-policy validation call may differ')
    count = load_count()
    old, prior_audit = count.parent_prepare().verify_prior(args.prior, args.audit, 'SourceRecordCount', 72, 298)
    locations_bytes = args.count_inputs.read_bytes()
    locations = json.loads(locations_bytes)
    if set(locations) != COUNT_KEYS or not all(isinstance(p, str) for p in locations.values()):
        raise ValueError('the exact five original count input locations are required')
    with TemporaryDirectory(prefix='source-policy-parent-') as temporary:
        fresh = Path(temporary) / 'count'
        count.prepare(SimpleNamespace(**{n: Path(p).resolve(strict=True) for n, p in locations.items()},
            output=fresh, lake_packages=args.lake_packages, lean_mcp=args.lean_mcp))
        if (fresh / 'plan.json').read_bytes() != (args.prior / 'plan.json').read_bytes():
            raise ValueError('the complete original count closure does not reconstruct')
    old_capture = Path(locations['actual'])
    old_types_data = (old_capture / 'generated' / count.extract.NAMESPACE / 'Types.lean').read_bytes()
    old_types = declarations(old_types_data.removesuffix(b'\n\nend ' + count.extract.NAMESPACE.encode() + b'\n'))
    common = set(types) & set(old_types)
    if not {'controller.capacity.CapacityConfig', PREFIX + 'MaintenanceConfig', 'JournalError'} <= common or any(
            declarations(types[n])[n] != old_types[n] for n in common):
        raise ValueError('every shared generated type and complete policy/capacity/error field must agree')
    prior_bodies = [read_exact(args.prior / n, h) for n, h in old['files'].items()]
    dictionaries = sorted(set(functions) & SKIPPED_DICTIONARIES)
    for name in dictionaries:
        body = declarations(functions[name])[name].rstrip()
        if not any(body in raw for raw in prior_bodies):
            raise ValueError('the exact existing generated dictionary is required: ' + name)
    io_blocks = []
    standard = PREFIX + 'StandardConfigRead.Insts.Sandproof_controller_serviceControllerCheckpointConfigReadConfigEffects'
    for name, body in functions.items():
        if name.startswith(PURE_PREFIXES) or name in SKIPPED_DICTIONARIES:
            continue
        io_blocks.append(body)
        arguments = None
        if name == PREFIX + 'MaintenanceConfig.Insts.Serde_coreDeDeserialize':
            arguments = 'deserialize'
        elif name in {standard + '.decode_maintenance', standard, PREFIX + 'read',
                      'controller.checkpoint.config', 'controller.checkpoint.proof_source_policy_read'}:
            arguments = 'deserialize fromSlice'
        if arguments:
            io_blocks.append(('local notation \"' + name + '\" => ' + name + ' ' + arguments + '\n').encode())
    fragments = {
        '-- EXISTING OPTION MODEL': option_model(),
        '-- ACTUAL SUPPORT TYPES': b'\n\n'.join(types[n] for n in SUPPORT_TYPES),
        '-- ACTUAL RANGE FUNCTIONS': b'\n\n'.join(b for n, b in functions.items() if n.startswith('core.ops.range.')),
        '-- ACTUAL READ TYPES': b'\n\n'.join(types[n] for n in READ_TYPES),
        '-- ACTUAL PURE FUNCTIONS': b'\n\n'.join(b for n, b in functions.items() if n.startswith(PURE_PREFIXES[1:])),
        '-- ACTUAL IO FUNCTIONS': b'\n\n'.join(io_blocks),
    }
    view = (HERE / 'SourcePolicyReadView.lean.in').read_bytes()
    for marker, body in fragments.items():
        if view.count(marker.encode() + b'\n') != 1:
            raise ValueError('one exact insertion marker is required: ' + marker)
        view = view.replace(marker.encode() + b'\n', body + b'\n')
    args.output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', args.lake_packages), ('.tools/lean-mcp', args.lean_mcp)):
        link = args.output / name
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)
    for name, wanted in old['files'].items():
        path = args.output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(read_exact(args.prior / name, wanted))
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        body = (extract.sql.ROOT / name).read_bytes()
        if body != (args.prior / name).read_bytes():
            raise ValueError('fixed tool support differs from the original count project')
        path = args.output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)
    history = {}
    modules = {'SourcePolicyReadView': view, 'SourcePolicyIteration': iteration(history)}
    modules.update({m: historic(m, history) for m in HISTORY})
    root = 'SourcePolicyRead' if selected == 'original' else 'SourcePolicyReadNegative'
    for module in ('SourcePolicyReadSpec', root):
        body = (HERE / (module + '.lean.in')).read_bytes()
        if module == 'SourcePolicyReadSpec':
            marker = b'-- EXISTING BIND SUCCESS LEMMA\n'
            if body.count(marker) != 1:
                raise ValueError('one exact old generic success lemma insertion is required')
            body = body.replace(marker, bind_success(history) + b'\n')
        modules[module] = body
    if selected != 'original':
        wrong = (HERE / 'SourcePolicyReadWrongView.lean.in').read_bytes()
        body = actual_functions[PREFIX + 'read_using']
        if wrong.count(b'-- ACTUAL WRONG READ\n') != 1:
            raise ValueError('one exact wrong read insertion is required')
        wrong = wrong.replace(b'-- ACTUAL WRONG READ\n', body + b'\n')
        modules['SourcePolicyReadWrongView'] = wrong
    lake = (args.prior / 'lakefile.toml').read_text()
    for module, body in modules.items():
        (args.output / (module + '.lean')).write_bytes(body)
        lake += f'\n[[lean_lib]]\nname = "{module}"\n'
    (args.output / 'lakefile.toml').write_text(lake)
    files = closure(args.output, [root + '.lean'], active_view='lakefile.toml')
    if set(files) != set(old['files']) | {m + '.lean' for m in modules} or any(files[n] != h for n,h in old['files'].items()):
        raise ValueError('all inherited and re-instantiated physical proof files are required')
    plan = dict(root=root, variant=selected, source=extract.SOURCE, reference=reference, actual=actual,
        parent_plan=sha256((args.prior/'plan.json').read_bytes()).hexdigest(), parent_audit=prior_audit,
        count_locations=sha256(locations_bytes).hexdigest(), historical_proofs=history, shared_types=sorted(common), shared_dictionaries=dictionaries,
        library_bindings={name: sha256((extract.sql.ROOT / name).read_bytes()).hexdigest() for name in (
            'verification/aeneas/foundations/SandproofFoundations/FunsExternal.lean',
            'verification/aeneas/foundations/SandproofFoundations/TypesExternal.lean')},
        fragments={n:sha256(b).hexdigest() for n,b in fragments.items()}, files=files, strict_files=sorted(files),
        modules={n:{'sha256':h} for n,h in files.items()}, build_order=[n.removesuffix('.lean') for n in files])
    (args.output/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')
    print(root,len(files),'exact physical Lean files')


if __name__ == '__main__':
    cli=argparse.ArgumentParser(description=__doc__)
    for name in ('actual','reference','prior','audit','count-inputs','output','lake-packages','lean-mcp'):
        cli.add_argument('--'+name,type=Path,required=True)
    args=cli.parse_args()
    for n,p in vars(args).items():setattr(args,n,p.resolve(strict=n!='output'))
    prepare(args)
