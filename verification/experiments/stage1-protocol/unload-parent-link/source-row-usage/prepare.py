"""Insert unchanged generated count/dispatch bodies above the audited #985 view."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
from tempfile import TemporaryDirectory

import extract
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact

HERE = Path(__file__).resolve().parent
ROOTS = {'original': 'SourceRowUsage', 'drop-terminator-byte': 'SourceRowUsageNegative'}


def prepare(actual, prior, audit, output, packages, service):
    result = json.loads((actual / 'result.json').read_bytes())
    variant = result['variant']
    if (result['complete'] is not True or result['source'] != extract.metadata.SOURCE
            or result['tree'] != extract.metadata.TREE or result['options'] != extract.OPTIONS
            or result['driver'] != extract.metadata.sql.digest(Path(extract.__file__)) or variant not in ROOTS):
        raise ValueError('the exact completed row-usage capture is required')
    if [(s['label'], s['exit_code'], s['bounded_stop']) for s in result['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('both fixed tools must finish normally')
    archive = actual / 'source.tar'
    read_exact(archive, extract.metadata.ARCHIVE)
    for command in result['commands']:
        read_exact(actual / (command['label'] + '.log'), command['log_sha256'])
    with TemporaryDirectory(prefix='source-row-usage-') as fresh:
        source = Path(fresh)
        codec = extract.charge.isolate_key_match(extract.metadata.reconstruct_codec(source, archive, 'original').decode())
        (source / extract.metadata.sql.CODEC).write_text(codec)
        capacity = (source / extract.CAPACITY_FILE).read_text()
        if variant == 'drop-terminator-byte':
            capacity = extract.drop_terminator_byte(capacity)
        (source / extract.CAPACITY_FILE).write_text(extract.isolate_row_reader(extract.isolate_value_size(capacity)))
        for name, wanted in result['staged'].items():
            if read_exact(actual / 'source' / name, wanted) != (source / name).read_bytes():
                raise ValueError('the complete staged source differs: ' + name)
    ir = actual / 'actual.llbc'
    read_exact(ir, result['ir'])
    gen = actual / 'generated'
    expected = {'translation.json', *(extract.NAMESPACE + '/' + name + '.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if set(result['generated']) != expected or {str(p.relative_to(gen)) for p in gen.rglob('*') if p.is_file()} != expected:
        raise ValueError('the complete actual generation inventory differs')
    for name, wanted in result['generated'].items():
        read_exact(gen / name, wanted)
    fixed = json.loads((extract.metadata.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    if extract.admit(ir, gen, fixed['target']) != result['admission']:
        raise ValueError('the original function mapping differs')

    def generated(kind):
        return declarations((gen / extract.NAMESPACE / (kind + '.lean')).read_bytes().removesuffix(
            b'\n\nend ' + extract.NAMESPACE.encode() + b'\n'))

    types, functions = generated('Types'), generated('Funs')
    old = json.loads((prior / 'plan.json').read_bytes())
    receipt = (audit / 'result.json').read_bytes()
    completed = json.loads(receipt)
    if (old['root'], old['source'], len(old['files'])) != ('SourceRowCharge', extract.metadata.SOURCE, 55):
        raise ValueError('the exact original #985 parent is required')
    if (completed['complete'] is not True or completed['source_sha256'] != old['files']
            or completed['physical_files'] != 55 or len(completed['audited']) != 172
            or any(completed[name] for name in ('errors', 'warnings', 'rejected'))):
        raise ValueError('the complete original #985 audit is required')
    read_exact(audit / 'service.log', completed['service_log_sha256'])
    if closure(prior, ['SourceRowCharge.lean'], active_view='lakefile.toml') != old['files']:
        raise ValueError('the original parent closure differs')
    parent_view = read_exact(prior / 'SourceRowChargeView.lean', old['files']['SourceRowChargeView.lean'])
    reused = {}
    for name, body in functions.items():
        if name.startswith('controller.lifecycle_inbox.codec.charged_bytes') or name in (
                'controller.lifecycle_inbox.source_work.codec.source_key_is_null',
                'controller.lifecycle_inbox.source_work.codec.charged_bytes'):
            if parent_view.count(body) != 1:
                raise ValueError('every actual callee must equal the audited parent: ' + name)
            reused[name] = sha256(body).hexdigest()
    if len(reused) != 9:
        raise ValueError('all three parent functions and six closure functions are required')
    for name, body in types.items():
        if name.startswith('controller.lifecycle_inbox.codec.charged_bytes.closure'):
            if parent_view.count(body) != 1:
                raise ValueError('the original complete callee closure type differs: ' + name)
    value_file = 'SandproofSourceRowSqlSelect/ReadValueData.lean'
    value_view = read_exact(prior / value_file, old['files'][value_file])
    if value_view.count(types['rusqlite.types.value_ref.ValueRef'].strip()) != 1:
        raise ValueError('all five actual ValueRef constructors and their payloads must be preserved')
    cap = types['controller.capacity.CapacityConfig'].strip()
    if read_exact(prior / 'SourceRecordBoundsView.lean', old['files']['SourceRecordBoundsView.lean']).count(cap) != 1:
        raise ValueError('the complete original capacity type must be preserved')
    internal = 'controller.internal_inbox.codec.charged_bytes'
    own = 'controller.capacity.'
    reader = 'rusqlite.row.Row.Insts.Sandproof_controller_serviceControllerCapacitySourceRowSizeRead'
    fragments = {
        '-- ACTUAL READER TYPE': types[own + 'SourceRowSizeRead'],
        '-- ACTUAL INTERNAL CLOSURE TYPES': b'\n\n'.join(b'@[reducible]\n' + body for name, body in types.items() if name.startswith(internal + '.closure')),
        '-- ACTUAL INTERNAL CALLBACKS': b'\n\n'.join((b'set_option linter.unusedVariables false in\n' if name.endswith('.call_once') else b'') + body
                                                    for name, body in functions.items() if name.startswith(internal + '.closure')),
        '-- ACTUAL INTERNAL CHARGE': functions[internal],
        '-- ACTUAL ADD': functions[own + 'add'],
        '-- ACTUAL DISPATCH': functions[own + 'charged_inbox_bytes'],
        '-- ACTUAL LOOP BODY': functions[own + 'row_bytes_using_loop.body'],
        '-- ACTUAL LOOP': functions[own + 'row_bytes_using_loop'],
        '-- ACTUAL GENERIC COUNT': b'@[reducible]\n' + functions[own + 'row_bytes_using'],
        '-- ACTUAL VALUE SIZE': functions[own + 'source_row_value_size'],
        '-- ACTUAL COLUMN SIZE': functions[own + 'source_row_column_size'],
        '-- ACTUAL ROW READER': functions[reader + '.read_size'],
        '-- ACTUAL ROW DICTIONARY': b'@[reducible]\n' + functions[reader],
        '-- ACTUAL COUNT': functions[own + 'row_bytes'],
    }
    if not all(fragments.values()):
        raise ValueError('every actual closure, reader and function is required')
    view = (HERE / 'SourceRowUsageView.lean.in').read_bytes()
    for marker, body in fragments.items():
        if view.count(marker.encode() + b'\n') != 1:
            raise ValueError('the sole insertion point is required: ' + marker)
        view = view.replace(marker.encode() + b'\n', body + b'\n')
    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', packages), ('.tools/lean-mcp', service)):
        link = output / name
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)
    for name, wanted in old['files'].items():
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(read_exact(prior / name, wanted))
    support = {}
    for name in ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py', 'scripts/lean-mcp-server.sh'):
        payload = (extract.metadata.sql.ROOT / name).read_bytes()
        if payload != (prior / name).read_bytes():
            raise ValueError('the original service support differs')
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(payload)
        support[name] = sha256(payload).hexdigest()
    modules = ('SourceRowUsageView', 'SourceRowUsageSpec', ROOTS[variant])
    lake = (prior / 'lakefile.toml').read_text()
    for module in modules:
        payload = view if module == 'SourceRowUsageView' else (HERE / (module + '.lean.in')).read_bytes()
        (output / (module + '.lean')).write_bytes(payload)
        lake += f'\n[[lean_lib]]\nname = "{module}"\n'
    (output / 'lakefile.toml').write_text(lake)
    files = closure(output, [ROOTS[variant] + '.lean'], active_view='lakefile.toml')
    if files.keys() != set(old['files']) | {m + '.lean' for m in modules} or any(files[n] != h for n, h in old['files'].items()):
        raise ValueError('the exact inherited and new closure is required')
    plan = dict(root=ROOTS[variant], variant=variant, source=extract.metadata.SOURCE, actual=result,
                prior=sha256((prior / 'plan.json').read_bytes()).hexdigest(), audit=sha256(receipt).hexdigest(),
                support=support, reused_functions=reused,
                actual_fragments={n: sha256(b).hexdigest() for n, b in fragments.items()}, files=files,
                strict_files=sorted(files), modules={n: {'sha256': h} for n, h in files.items()},
                build_order=[n.removesuffix('.lean') for n in files])
    (output / 'plan.json').write_text(json.dumps(plan, indent=2) + '\n')
    print(ROOTS[variant], len(files), 'exact physical Lean files')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('actual', 'prior', 'audit', 'output', 'lake-packages', 'lean-mcp'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    prepare(*(getattr(args, n.replace('-', '_')).resolve(strict=n != 'output') for n in
              ('actual', 'prior', 'audit', 'output', 'lake-packages', 'lean-mcp')))
