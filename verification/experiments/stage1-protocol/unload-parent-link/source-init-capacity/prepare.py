"""Retain the complete captured initializer beside the existing audited closure."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import re
import sys
import textwrap
from tempfile import TemporaryDirectory
from types import SimpleNamespace

import extract
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact

HERE = Path(__file__).resolve().parent
CODEC = 'controller.lifecycle_inbox.source_work.codec.'
PURE = tuple(CODEC + n for n in ('TABLE', 'RECORDS_TABLE', 'METADATA_BYTES', 'VALUE_LIMIT',
                                'initial_source_bytes', 'ensure_source_capacity'))
NEW_TYPES = ('sandproof_protocol.internal.PositiveCounter', 'sandproof_protocol.NodeId',
    'sandproof_protocol.DeploymentId', 'sandproof_protocol.internal.NodeIdentity',
    'sandproof_protocol.lifecycle.authority.CenterId', 'sandproof_protocol.lifecycle.authority.CenterIdentity',
    'sandproof_protocol.lifecycle.digest.ContentDigest', 'capture.HostEpochRegistration',
    'controller.lifecycle_inbox.model.LifecycleInboxConfig', CODEC + 'Configuration', CODEC + 'sql.closure')
EFFECT = ('TupleABC.Insts.RusqliteParamsSealedSealed', 'TupleABCD.Insts.RusqliteParamsSealedSealed',
    'TupleABC.Insts.RusqliteParamsParams', 'TupleABCD.Insts.RusqliteParamsParams',
    'I64.Insts.RusqliteTypesTo_sqlToSql', 'SliceU8.Insts.RusqliteTypesTo_sqlToSql',
    'sandproof_protocol.internal.PositiveCounter.Insts.CoreCloneClone.clone',
    'sandproof_protocol.internal.NodeIdentity.Insts.CoreCloneClone.clone',
    CODEC + 'records_per_session', CODEC + 'reserved_records',
    CODEC + 'sql.closure.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError.call_once',
    CODEC + 'sql.closure.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError', CODEC + 'sql',
    CODEC + 'insert_initial_source_row', CODEC + 'write_initial_source_rows_loop.body',
    CODEC + 'write_initial_source_rows_loop', CODEC + 'write_initial_source_rows', CODEC + 'initialize',
    'controller.lifecycle_inbox.source_work.initialize')


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
        result[name] = block[attribute + 1 if attribute >= 0 else match.start():]
    return result


def load_policy():
    directory = HERE.parent / 'source-policy-read'
    previous = sys.modules.pop('extract', None)
    sys.path.insert(0, str(directory))
    try:
        spec = importlib.util.spec_from_file_location('source_policy_prepare', directory / 'prepare.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path.remove(str(directory))
        sys.modules.pop('extract', None)
        if previous is not None:
            sys.modules['extract'] = previous


def captured(directory, variant):
    result = json.loads((directory / 'result.json').read_bytes())
    if (result['source'], result['tree'], result['variant'], result['options'], result['driver'], result['complete']) != (
            extract.SOURCE, extract.TREE, variant, extract.OPTIONS, extract.sql.digest(Path(extract.__file__)), True):
        raise ValueError('the exact completed initializer capture is required')
    if [(r['label'], r['exit_code'], r['bounded_stop']) for r in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed tools must complete normally')
    read_exact(directory / 'source.tar', extract.ARCHIVE)
    for command in result['commands']:
        read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    with TemporaryDirectory(prefix='source-init-input-') as temporary:
        source = Path(temporary)
        extract.reconstruct(directory / 'source.tar', source, variant)
        if set(result['staged']) != {str(p) for p in (extract.CODEC, extract.LIB)}:
            raise ValueError('the exact staging inventory is required')
        for name, wanted in result['staged'].items():
            if read_exact(directory / 'source' / name, wanted) != (source / name).read_bytes():
                raise ValueError('the exact original isolation and sole mutation differ')
    read_exact(directory / 'actual.llbc', result['ir'])
    generated = directory / 'generated'
    expected = {'translation.json', *(extract.NAMESPACE + '/' + n + '.lean' for n in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if set(result['generated']) != expected or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != expected:
        raise ValueError('the complete generated inventory is required')
    for name, wanted in result['generated'].items():
        read_exact(generated / name, wanted)
    fixed = json.loads((extract.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    if extract.admit(directory / 'actual.llbc', generated, fixed['target']) != result['admission']:
        raise ValueError('the complete initializer/caller mapping differs')
    return result, parsed(directory, 'Types'), parsed(directory, 'Funs')


def prepare(args):
    reference, types, functions = captured(args.reference, 'original')
    selected = json.loads((args.actual / 'result.json').read_bytes())['variant']
    actual, actual_types, actual_functions = captured(args.actual, selected)
    if actual_types != types or actual_functions.keys() != functions.keys():
        raise ValueError('all current types, fields and function names must remain identical')
    changed = {n for n in functions if functions[n] != actual_functions[n]}
    if changed != (set() if selected == 'original' else {CODEC + 'initial_source_bytes'}):
        raise ValueError('only the original budget overhead may differ')
    policy = load_policy()
    old, prior_audit = policy.load_count().parent_prepare().verify_prior(args.prior, args.audit, 'SourcePolicyRead', 84, 357)
    locations_bytes = args.policy_inputs.read_bytes()
    locations = json.loads(locations_bytes)
    if set(locations) != {'actual', 'reference', 'prior', 'audit', 'count_inputs'}:
        raise ValueError('the exact five original policy input locations are required')
    with TemporaryDirectory(prefix='source-init-parent-') as temporary:
        fresh = Path(temporary) / 'policy'
        policy.prepare(SimpleNamespace(**{n: Path(p).resolve(strict=True) for n,p in locations.items()},
            output=fresh, lake_packages=args.lake_packages, lean_mcp=args.lean_mcp))
        if (fresh / 'plan.json').read_bytes() != (args.prior / 'plan.json').read_bytes():
            raise ValueError('the complete original policy closure must reconstruct')
    # The source config/count/read implementations are reused only after the
    # generated declaration bodies have been compared with their exact inputs.
    _, old_types, old_functions = policy.captured(Path(locations['reference']), 'original')
    common = sorted(set(types) & set(old_types))
    if any(declarations(types[n])[n] != declarations(old_types[n])[n] for n in common):
        raise ValueError('every complete shared generated type must agree')
    reused = sorted(set(functions) & set(old_functions))
    if any(declarations(functions[n])[n] != declarations(old_functions[n])[n] for n in reused):
        raise ValueError('all reused generated readers and validators must agree')
    prior_bodies = [read_exact(args.prior / name, digest) for name,digest in old['files'].items()]
    reused_value_functions = ['capture.source.SourceObservationConfig.valid', 'capture.source.SourceObservationConfig.required_bytes',
        'capture.retirement.MATERIAL_RETIREMENT_PLAN_MAX_BYTES', 'capture.retirement.MATERIAL_RETIREMENT_RECORD_MAX_BYTES',
        'capture.retirement.MATERIAL_RELEASE_RECORD_MAX_BYTES', 'Shared0T.Insts.RusqliteTypesTo_sqlToSql']
    for name in reused_value_functions:
        if not any(declarations(functions[name])[name].rstrip() in body for body in prior_bodies):
            raise ValueError('the exact existing value function is required: ' + name)
    for name in ('capture.source.SourceObservationConfig', 'controller.capacity.CapacityConfig', 'JournalError'):
        if not any(declarations(types[name])[name].rstrip() in body for body in prior_bodies):
            raise ValueError('the full actual shared value type is required: ' + name)
    fragments = {'-- ACTUAL VALUE TYPES': b'\n\n'.join(types[n] for n in NEW_TYPES),
                 '-- ACTUAL PURE FUNCTIONS': b'\n\n'.join(functions[n] for n in PURE)}
    blocks = []
    for name in EFFECT:
        if name == CODEC + 'initialize':
            body = declarations(actual_functions[name])[name].decode()
            first = '              let ni ←\n'
            last = '\n            | core.ops.control_flow.ControlFlow.Break residual =>'
            if body.count(first) != 1 or body.count(last) != 1:
                raise ValueError('one exact complete post-capacity continuation is required')
            suffix = body[body.index(first):body.index(last)]
            suffix = textwrap.indent(textwrap.dedent(suffix), '  ')
            blocks.append(('def afterAcceptedCapacity (connection : rusqlite.Connection)\n'
                '  (config : capture.source.SourceObservationConfig)\n'
                '  (inbox : controller.lifecycle_inbox.model.LifecycleInboxConfig)\n'
                '  (val reserve : U64) : Result (core.result.Result Unit JournalError) := do\n' + suffix + '\n').encode())
        blocks.append(actual_functions[name])
        arguments = None
        if name.startswith('TupleABC') and name.endswith('RusqliteParamsParams'):
            arguments = 'io'
        elif name in {'I64.Insts.RusqliteTypesTo_sqlToSql', 'SliceU8.Insts.RusqliteTypesTo_sqlToSql'}:
            arguments = 'io'
        elif name.endswith('NodeIdentity.Insts.CoreCloneClone.clone'):
            arguments = 'io'
        elif name == CODEC + 'records_per_session' or name == CODEC + 'reserved_records':
            arguments = 'policy'
        elif name.startswith(CODEC + 'write_initial_source_rows') or name in {
                CODEC + 'insert_initial_source_row', CODEC + 'initialize', 'controller.lifecycle_inbox.source_work.initialize'}:
            arguments = ('io policy capacityRead' if name.endswith('initialize') else 'io')
        if arguments:
            blocks.append(('local notation "' + name + '" => ' + name + ' ' + arguments + '\n').encode())
    fragments['-- ACTUAL PURE FUNCTIONS'] = b'\n\n'.join(actual_functions[n] for n in PURE)
    fragments['-- ACTUAL EFFECT FUNCTIONS'] = b'\n\n'.join(blocks)
    slice_path = extract.sql.ROOT / 'verification/aeneas/storage/SandproofStorage/FunsExternal.lean'
    slice_body = slice_path.read_bytes()
    marker = b'def alloc.vec.Vec.as_slice '
    if slice_body.count(marker) != 1:
        raise ValueError('the existing unique Vec slice model is required')
    first = slice_body.index(marker)
    fragments['-- EXISTING VEC SLICE MODEL'] = slice_body[first:slice_body.index(b'\n\n', first)]
    view = (HERE / 'SourceInitCapacityView.lean.in').read_bytes()
    # These aliases use existing, exact generated functions rather than new
    # algorithms. The additional current count/config comparison is checked below.
    aliases = ('abbrev capture.source.SourceObservationConfig.valid := SourceConfigView.valid\n'
        'abbrev capture.source.SourceObservationConfig.required_bytes := SourceReservationView.reservation\n')
    view = view.replace(b'-- ACTUAL PURE FUNCTIONS\n', aliases.encode() + b'-- ACTUAL PURE FUNCTIONS\n')
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
            raise ValueError('the fixed support differs from the original project')
        path = args.output / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)
    read_view = (HERE / 'SourceInitCapacityReadView.lean.in').read_bytes()
    for marker, name in (('-- ACTUAL CAPACITY EFFECTS', 'controller.capacity.config_using'),
                         ('-- ACTUAL CAPACITY STANDARD', 'controller.capacity.config')):
        if read_view.count(marker.encode() + b'\n') != 1:
            raise ValueError('one exact original capacity reader insertion is required')
        read_view = read_view.replace(marker.encode() + b'\n', actual_functions[name] + b'\n')
    modules = {'SourceInitCapacityView': view, 'SourceInitCapacityReadView': read_view}
    root = 'SourceInitCapacity' if selected == 'original' else 'SourceInitCapacityNegative'
    for module in ('SourceInitCapacitySpec', 'SourceInitCapacityRead', 'SourceInitCapacityParent', root):
        path = HERE / (module + '.lean.in')
        modules[module] = path.read_bytes()
    lake = (args.prior / 'lakefile.toml').read_text()
    for module, body in modules.items():
        (args.output / (module + '.lean')).write_bytes(body)
        lake += f'\n[[lean_lib]]\nname = "{module}"\n'
    (args.output / 'lakefile.toml').write_text(lake)
    files = closure(args.output, [root + '.lean'], active_view='lakefile.toml')
    if set(files) != set(old['files']) | {m + '.lean' for m in modules} or any(files[n] != h for n,h in old['files'].items()):
        raise ValueError('the entire inherited and new physical closure must be retained')
    plan = dict(root=root, variant=selected, source=extract.SOURCE, reference=reference, actual=actual,
        parent_plan=sha256((args.prior/'plan.json').read_bytes()).hexdigest(), parent_audit=prior_audit,
        policy_locations=sha256(locations_bytes).hexdigest(), shared_types=common, shared_functions=reused,
        reused_value_functions=reused_value_functions, vec_slice_model=sha256(slice_path.read_bytes()).hexdigest(),
        fragments={n:sha256(b).hexdigest() for n,b in fragments.items()}, files=files, strict_files=sorted(files),
        modules={n:{'sha256':h} for n,h in files.items()}, build_order=[n.removesuffix('.lean') for n in files])
    (args.output/'plan.json').write_text(json.dumps(plan,indent=2)+'\n')
    print(root,len(files),'exact physical Lean files')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('actual','reference','prior','audit','policy-inputs','output','lake-packages','lean-mcp'):
        cli.add_argument('--'+name,type=Path,required=True)
    args = cli.parse_args()
    for n,p in vars(args).items():
        setattr(args,n,p.resolve(strict=n!='output'))
    prepare(args)
