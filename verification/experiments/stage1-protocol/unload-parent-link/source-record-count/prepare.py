"""Join freshly captured count bodies to #988's complete original closure."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
from types import SimpleNamespace

import extract
from prepare_source_record_bounds import bounds as original_bounds
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact

HERE = Path(__file__).resolve().parent
ROOTS = {'original': 'SourceRecordCount', 'keep-one-position': 'SourceRecordCountOnePosition',
         'zero-skips-policy': 'SourceRecordCountZero'}
PARENT_KEYS = {'actual', 'layout_prior', 'layout_audit', 'usage_actual', 'usage_prior', 'usage_audit',
               'charge_prior', 'charge_audit', 'config_actual', 'config_prior', 'config_audit',
               'metadata_prior', 'metadata_audit'}


def parent_prepare():
    directory = HERE.parent / 'source-reservation'
    previous = sys.modules.get('extract')
    sys.modules.pop('extract', None)
    sys.path.insert(0, str(directory))
    try:
        spec = importlib.util.spec_from_file_location('source_reservation_prepare', directory / 'prepare.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path.remove(str(directory))
        sys.modules.pop('extract', None)
        if previous is not None:
            sys.modules['extract'] = previous


def parsed(directory, kind):
    payload = (directory / 'generated' / extract.NAMESPACE / (kind + '.lean')).read_bytes()
    return declarations(payload.removesuffix(b'\n\nend ' + extract.NAMESPACE.encode() + b'\n'))


def prepare(args):
    result = json.loads((args.actual / 'result.json').read_bytes())
    variant = result['variant']
    if (result['complete'] is not True or result['source'] != extract.SOURCE or result['tree'] != extract.TREE
            or result['options'] != extract.OPTIONS or variant not in ROOTS
            or result['driver'] != extract.bounds.sql.digest(Path(extract.__file__))):
        raise ValueError('the exact completed count capture is required')
    if [(r['label'], r['exit_code'], r['bounded_stop']) for r in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed tools must complete normally')
    archive = args.actual / 'source.tar'
    read_exact(archive, extract.ARCHIVE)
    for row in result['commands']:
        read_exact(args.actual / (row['label'] + '.log'), row['log_sha256'])
    with TemporaryDirectory(prefix='source-count-input-') as temporary:
        original = extract.reconstruct(Path(temporary), archive, variant)
        if read_exact(args.actual / 'source' / extract.CODEC, result['codec']) != original:
            raise ValueError('the full original source or sole mutation differs')
    read_exact(args.actual / 'actual.llbc', result['ir'])
    expected = {'translation.json', *(extract.NAMESPACE + '/' + n + '.lean' for n in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    generated = args.actual / 'generated'
    if (set(result['generated']) != expected
            or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != expected):
        raise ValueError('the complete raw generation inventory is required')
    for name, wanted in result['generated'].items():
        read_exact(generated / name, wanted)
    fixed = json.loads((extract.bounds.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    if extract.bounds.admit(args.actual / 'actual.llbc', generated, fixed['target']) != result['admission']:
        raise ValueError('the complete original helper and caller mapping differs')

    parent = parent_prepare()
    old, parent_audit = parent.verify_prior(args.prior, args.audit, 'SourceReservation', 69, 275)
    inputs_data = args.reservation_inputs.read_bytes()
    locations = json.loads(inputs_data)
    if set(locations) != PARENT_KEYS or not all(isinstance(p, str) for p in locations.values()):
        raise ValueError('the exact thirteen original #988 input locations are required')
    with TemporaryDirectory(prefix='source-count-parent-') as temporary:
        fresh = Path(temporary) / 'parent'
        inherited = {n: Path(p).resolve(strict=True) for n, p in locations.items()}
        parent.prepare(SimpleNamespace(**inherited, output=fresh, lake_packages=args.lake_packages, lean_mcp=args.lean_mcp))
        if (fresh / 'plan.json').read_bytes() != (args.prior / 'plan.json').read_bytes():
            raise ValueError('the complete #988 raw input reconstruction differs')
    old_view, original_report, _ = original_bounds(args.bounds_actual, 'original')
    if read_exact(args.prior / 'SourceRecordBoundsView.lean', old['files']['SourceRecordBoundsView.lean']) != old_view:
        raise ValueError('the complete original #978 view differs from the actual caller')
    types, functions = parsed(args.actual, 'Types'), parsed(args.actual, 'Funs')
    if types != parsed(args.bounds_actual, 'Types'):
        raise ValueError('every original type, policy/error field and dictionary must be preserved')
    original_functions = parsed(args.bounds_actual, 'Funs')
    if functions.keys() != original_functions.keys():
        raise ValueError('the full generated function inventory differs')
    prefix = 'controller.lifecycle_inbox.source_work.codec.'
    changed = {name for name in functions if functions[name] != original_functions[name]}
    expected_changed = {'original': set(), 'keep-one-position': {prefix + 'records_per_session'},
                        'zero-skips-policy': {prefix + 'reserved_records'}}[variant]
    if changed != expected_changed:
        raise ValueError('only the selected original count body may differ: ' + repr(changed))
    fragments = {'-- ACTUAL PER SESSION': functions[prefix + 'records_per_session'],
                 '-- ACTUAL RESERVED RECORDS': functions[prefix + 'reserved_records']}
    view = (HERE / 'SourceRecordCountView.lean.in').read_bytes()
    for marker, body in fragments.items():
        if view.count(marker.encode() + b'\n') != 1:
            raise ValueError('one exact original insertion point required: ' + marker)
        view = view.replace(marker.encode() + b'\n', body + b'\n')

    args.output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', args.lake_packages), ('.tools/lean-mcp', args.lean_mcp)):
        link = args.output / name
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)
    for name, wanted in old['files'].items():
        destination = args.output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(read_exact(args.prior / name, wanted))
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        body = (extract.bounds.sql.ROOT / name).read_bytes()
        if body != (args.prior / name).read_bytes():
            raise ValueError('the original fixed tool support differs')
        destination = args.output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(body)
    lake = (args.prior / 'lakefile.toml').read_text()
    modules = ('SourceRecordCountView', 'SourceRecordCountSpec', ROOTS[variant])
    for module in modules:
        body = view if module == 'SourceRecordCountView' else (HERE / (module + '.lean.in')).read_bytes()
        (args.output / (module + '.lean')).write_bytes(body)
        lake += f'\n[[lean_lib]]\nname = "{module}"\n'
    (args.output / 'lakefile.toml').write_text(lake)
    files = closure(args.output, [ROOTS[variant] + '.lean'], active_view='lakefile.toml')
    if files.keys() != set(old['files']) | {m + '.lean' for m in modules} or any(files[n] != h for n, h in old['files'].items()):
        raise ValueError('the entire inherited and new closure is required')
    plan = dict(root=ROOTS[variant], variant=variant, source=extract.SOURCE, actual=result,
                parent_plan=sha256((args.prior / 'plan.json').read_bytes()).hexdigest(), parent_audit=parent_audit,
                parent_locations=sha256(inputs_data).hexdigest(),
                bounds_capture=sha256((args.bounds_actual / 'result.json').read_bytes()).hexdigest(),
                fragments={n: sha256(b).hexdigest() for n, b in fragments.items()},
                files=files, strict_files=sorted(files), modules={n: {'sha256': h} for n, h in files.items()},
                build_order=[n.removesuffix('.lean') for n in files])
    (args.output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(ROOTS[variant], len(files), 'exact physical Lean files')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('actual', 'prior', 'audit', 'bounds-actual', 'reservation-inputs', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    for name, path in vars(args).items():
        setattr(args, name, path.resolve(strict=name != 'output'))
    prepare(args)
