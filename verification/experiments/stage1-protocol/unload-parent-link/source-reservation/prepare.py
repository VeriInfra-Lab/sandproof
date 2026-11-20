"""Join exact #987/#983 inputs and the newly captured original reservation body."""
import argparse
from hashlib import sha256
import importlib.util
import json
from pathlib import Path
import sys
from tempfile import TemporaryDirectory

import extract
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact

HERE = Path(__file__).resolve().parent
ROOTS = {'original': 'SourceReservation', 'omit-release-budget': 'SourceReservationNegative'}


def load_prepare(name):
    # Each historical materializer imports its own exact extract.py.
    source = HERE.parent / name
    previous = sys.modules.get('extract')
    sys.modules.pop('extract', None)
    sys.path.insert(0, str(source))
    try:
        spec = importlib.util.spec_from_file_location(name.replace('-', '_') + '_prepare', source / 'prepare.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module
    finally:
        sys.path.remove(str(source))
        sys.modules.pop('extract', None)
        if previous is not None:
            sys.modules['extract'] = previous


def verify_prior(project, audit, root, count, audits):
    old = json.loads((project / 'plan.json').read_bytes())
    receipt = (audit / 'result.json').read_bytes()
    result = json.loads(receipt)
    if (old['root'], old['source'], len(old['files'])) != (root, extract.SOURCE, count):
        raise ValueError('the exact original parent is required: ' + root)
    if (result['complete'] is not True or result['source_sha256'] != old['files']
            or result['physical_files'] != count or len(result['audited']) != audits
            or any(result[n] for n in ('errors', 'warnings', 'rejected'))):
        raise ValueError('the complete parent service audit is required: ' + root)
    read_exact(audit / 'service.log', result['service_log_sha256'])
    if closure(project, [root + '.lean'], active_view='lakefile.toml') != old['files']:
        raise ValueError('the complete parent closure differs: ' + root)
    return old, sha256(receipt).hexdigest()


def prepare(args):
    actual, output = args.actual, args.output
    result = json.loads((actual / 'result.json').read_bytes())
    variant = result['variant']
    if (result['complete'] is not True or result['source'] != extract.SOURCE or result['tree'] != extract.TREE
            or result['options'] != extract.OPTIONS or variant not in ROOTS
            or result['driver'] != extract.sql.digest(Path(extract.__file__))):
        raise ValueError('the exact complete reservation capture is required')
    if [(r['label'], r['exit_code'], r['bounded_stop']) for r in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed tools must complete normally')
    archive = actual / 'source.tar'
    read_exact(archive, extract.ARCHIVE)
    for row in result['commands']:
        read_exact(actual / (row['label'] + '.log'), row['log_sha256'])
    with TemporaryDirectory(prefix='source-reservation-capture-') as temporary:
        expected = extract.reconstruct(archive, Path(temporary), variant)
        if read_exact(actual / 'source' / extract.CONFIG, result['staged'][str(extract.CONFIG)]) != expected:
            raise ValueError('the original complete source or sole omission differs')
    read_exact(actual / 'actual.llbc', result['ir'])
    generated = actual / 'generated'
    expected_names = {'translation.json', *(extract.NAMESPACE + '/' + n + '.lean' for n in ('Types', 'Funs'))}
    if (set(result['generated']) != expected_names
            or {str(p.relative_to(generated)) for p in generated.rglob('*') if p.is_file()} != expected_names):
        raise ValueError('the entire original generation inventory is required')
    for name, wanted in result['generated'].items():
        read_exact(generated / name, wanted)
    fixed = json.loads((extract.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    if extract.admit(actual / 'actual.llbc', generated, fixed['target']) != result['admission']:
        raise ValueError('the exact actual mapping differs')

    layout, layout_audit = verify_prior(args.layout_prior, args.layout_audit, 'SourceRowLayout', 60, 223)
    config, config_audit = verify_prior(args.config_prior, args.config_audit, 'SourceConfigGuards', 58, 175)
    with TemporaryDirectory(prefix='source-reservation-parents-') as temporary:
        fresh = Path(temporary)
        load_prepare('source-row-layout').prepare(args.usage_actual, args.usage_prior, args.usage_audit,
            args.charge_prior, args.charge_audit, fresh / 'layout', args.lake_packages, args.lean_mcp)
        load_prepare('source-config-guards').prepare(args.config_actual, args.metadata_prior, args.metadata_audit,
            fresh / 'config', args.lake_packages, args.lean_mcp)
        for name, parent in [('layout', args.layout_prior), ('config', args.config_prior)]:
            if (fresh / name / 'plan.json').read_bytes() != (parent / 'plan.json').read_bytes():
                raise ValueError('the full raw parent reconstruction differs: ' + name)

    parents = dict(layout['files'])
    for name, wanted in config['files'].items():
        if name in parents and parents[name] != wanted:
            raise ValueError('two parents disagree on the original representation: ' + name)
        parents[name] = wanted
    if len(parents) != 66:
        raise ValueError('the complete shared representation and both closures are required')

    def parsed(kind):
        data = (generated / extract.NAMESPACE / (kind + '.lean')).read_bytes()
        return declarations(data.removesuffix(b'\n\nend ' + extract.NAMESPACE.encode() + b'\n'))
    types, functions = parsed('Types'), parsed('Funs')
    old_view = read_exact(args.layout_prior / 'SourceRecordBoundsView.lean', parents['SourceRecordBoundsView.lean'])
    if old_view.count(types['capture.source.SourceObservationConfig'].strip()) != 1:
        raise ValueError('all six original configuration fields must match')
    fragments = {
        '-- ACTUAL PLAN LIMIT': functions['capture.retirement.MATERIAL_RETIREMENT_PLAN_MAX_BYTES'],
        '-- ACTUAL RETIREMENT LIMIT': functions['capture.retirement.MATERIAL_RETIREMENT_RECORD_MAX_BYTES'],
        '-- ACTUAL RELEASE LIMIT': functions['capture.retirement.MATERIAL_RELEASE_RECORD_MAX_BYTES'],
        '-- ACTUAL RESERVATION': functions['capture.source.SourceObservationConfig.required_bytes'],
    }
    for marker, body in fragments.items():
        if marker != '-- ACTUAL RESERVATION' or variant == 'original':
            if old_view.count(body.strip()) != 1:
                raise ValueError('the exact original function/constant must match its caller: ' + marker)
    view = (HERE / 'SourceReservationView.lean.in').read_bytes()
    for marker, body in fragments.items():
        if view.count(marker.encode() + b'\n') != 1:
            raise ValueError('one original insertion point required: ' + marker)
        view = view.replace(marker.encode() + b'\n', body + b'\n')

    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', args.lake_packages), ('.tools/lean-mcp', args.lean_mcp)):
        link = output / name
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)
    for name, wanted in parents.items():
        source = args.layout_prior if name in layout['files'] else args.config_prior
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(read_exact(source / name, wanted))
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        body = (extract.sql.ROOT / name).read_bytes()
        if body != (args.layout_prior / name).read_bytes() or body != (args.config_prior / name).read_bytes():
            raise ValueError('the original fixed tool support differs')
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(body)
    lake = (args.layout_prior / 'lakefile.toml').read_text()
    for name in sorted(set(config['files']) - set(layout['files'])):
        lake += f'\n[[lean_lib]]\nname = "{name.removesuffix(".lean")}"\n'
    modules = ('SourceReservationView', 'SourceReservationSpec', ROOTS[variant])
    for module in modules:
        body = view if module == 'SourceReservationView' else (HERE / (module + '.lean.in')).read_bytes()
        (output / (module + '.lean')).write_bytes(body)
        lake += f'\n[[lean_lib]]\nname = "{module}"\n'
    (output / 'lakefile.toml').write_text(lake)
    files = closure(output, [ROOTS[variant] + '.lean'], active_view='lakefile.toml')
    if files.keys() != set(parents) | {m + '.lean' for m in modules} or any(files[n] != h for n, h in parents.items()):
        raise ValueError('the complete inherited and new source closure is required')
    plan = dict(root=ROOTS[variant], variant=variant, source=extract.SOURCE, actual=result,
                parents={n: sha256((p / 'plan.json').read_bytes()).hexdigest() for n, p in (
                    ('layout', args.layout_prior), ('config', args.config_prior))},
                audits=dict(layout=layout_audit, config=config_audit),
                actual_fragments={n: sha256(b).hexdigest() for n, b in fragments.items()},
                files=files, strict_files=sorted(files), modules={n: {'sha256': h} for n, h in files.items()},
                build_order=[n.removesuffix('.lean') for n in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(ROOTS[variant], len(files), 'exact physical Lean files')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('actual', 'layout-prior', 'layout-audit', 'usage-actual', 'usage-prior', 'usage-audit',
                 'charge-prior', 'charge-audit', 'config-actual', 'config-prior', 'config-audit',
                 'metadata-prior', 'metadata-audit', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, type=Path, required=True)
    args = cli.parse_args()
    for name, path in vars(args).items():
        setattr(args, name, path.resolve(strict=name != 'output'))
    prepare(args)
