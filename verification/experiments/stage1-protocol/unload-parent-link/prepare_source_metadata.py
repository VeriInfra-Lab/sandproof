"""Bind the actual source metadata load to the audited source record scan."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
from tempfile import TemporaryDirectory

import source_metadata_capture as capture
from prepare_observation_suffix import replace_once
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact


HERE = Path(__file__).with_name('source-metadata')
ROOTS = {'original': 'SourceMetadata', 'allow-inflated-reserve': 'SourceMetadataNegative'}
PACKAGE = 'controller.lifecycle_inbox.source_work.'
SUPPORT = ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py',
           'scripts/lean-mcp-server.sh')


def metadata(directory, variant):
    report = json.loads((directory / 'result.json').read_bytes())
    if (report['source'], report['tree'], report['variant'], report['options'], report['driver'],
            report['complete']) != (capture.SOURCE, capture.TREE, variant, capture.CFG,
                                   capture.sql.digest(Path(capture.__file__)), True):
        raise ValueError('the completed original metadata capture is required')
    if [(item['label'], item['exit_code'], item['bounded_stop']) for item in report['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('the fixed tools must finish normally')
    read_exact(directory / 'source.tar', capture.ARCHIVE)
    stage = read_exact(directory / 'source' / capture.sql.CODEC, report['codec'])
    with TemporaryDirectory(prefix='source-metadata-') as fresh:
        if stage != capture.reconstruct_codec(Path(fresh), directory / 'source.tar', variant):
            raise ValueError('the staged metadata must reproduce the exact checked Rust input')
    for command in report['commands']:
        read_exact(directory / (command['label'] + '.log'), command['log_sha256'])
    ir, gen = directory / 'actual.llbc', directory / 'generated'
    read_exact(ir, report['ir'])
    expected = {'translation.json', *(f'{capture.NAMESPACE}/{name}.lean' for name in (
        'Types', 'Funs', 'TypesExternal_Template', 'FunsExternal_Template'))}
    if (report['generated'].keys() != expected
            or {str(file.relative_to(gen)) for file in gen.rglob('*') if file.is_file()} != expected):
        raise ValueError('the original capture file inventory differs')
    files = {name: read_exact(gen / name, report['generated'][name]) for name in expected}
    fixed = json.loads((capture.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    if capture.admit(ir, gen, fixed['target']) != report['admission']:
        raise ValueError('the original metadata query or capacity helper selection changed')

    def captured(kind):
        payload = files[f'{capture.NAMESPACE}/{kind}.lean']
        return declarations(payload.removesuffix(b'\n\nend ' + capture.NAMESPACE.encode() + b'\n'))

    types, functions = captured('Types'), captured('Funs')
    codec = PACKAGE + 'codec.'
    schema_callback = codec + 'schema.closure.Insts.CoreOpsFunctionFnOnceTupleSharedRowResultStringError'
    sql_names = (
        'ArrayT1.Insts.RusqliteParamsSealedSealed',
        'ArrayT1.Insts.RusqliteParamsParams',
        'Bool.Insts.RusqliteTypesFrom_sqlFromSql',
        'Shared0T.Insts.RusqliteTypesTo_sqlToSql',
        'Str.Insts.RusqliteTypesTo_sqlToSql',
    )
    constant_names = ('sandproof_protocol.internal.HOST_STORE_FORMAT_VERSION', 'LIFECYCLE_STORE_FORMAT_VERSION',
                      *(codec + name for name in ('TABLE', 'RECORDS_TABLE', 'METADATA_BYTES')))

    def callbacks(prefix):
        return b'\n\n'.join((b'set_option linter.unusedVariables false in\n' if key.endswith('.call_once') else b'')
                              + value for key, value in functions.items()
                              if key.startswith(prefix))

    checked_alias = '-- ACTUAL CHECKED'
    load_alias = '-- ACTUAL LOAD'
    query_names = (
        ('queryVersion', 'closure', 'I64', 'I64', 'SELECT version FROM controller_meta WHERE id=1'),
        ('queryExists', 'closure_1', 'Bool', 'Bool',
         "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name IN ('controller_source_work','controller_source_work_records'))"),
        ('queryCount', 'closure_2', 'I64', 'I64', 'SELECT count(*) FROM controller_source_work'),
        ('queryNumbers', 'closure_3', 'PairI64I64', '(I64 × I64)',
         'SELECT length(state_json),reserve_bytes FROM controller_source_work WHERE id=1'),
        ('queryPayload', 'closure_5', 'PairVecU8VecU8', '(Bytes × Bytes)',
         'SELECT state_json,sha256 FROM controller_source_work WHERE id=1'),
    )
    aliases = []
    actual_load = functions[codec + 'load']
    for name, closure, output, typ, sql in query_names:
        literal = '"' + sql.replace("'", "\\'") + '"'
        if actual_load.count(literal.encode()) != 1:
            raise ValueError('exact metadata SELECT changed: ' + name)
        callback = codec + 'load.' + closure + '.Insts.CoreOpsFunctionFnOnceTupleSharedRowResult' + output + 'Error'
        aliases.append(f'abbrev {name}Sql : Str := toStr {literal}\n\n'
                       f'def {name} (connection : Db) : Result (SqlReply {typ}) :=\n'
                       f'  queryZero {name}Sql\n    {callback} connection')
    fragments = {
        '-- ACTUAL STRUCTURE': types[codec + 'Sources'],
        '-- ACTUAL CLOSURE TYPES': b'\n\n'.join(b'@[reducible]\n' + frag for name, frag in types.items()
                                                if name.startswith(codec) and '.closure' in name),
        '-- ACTUAL CONSTANTS': b'\n\n'.join(functions[name] for name in constant_names),
        '-- ACTUAL VALID': functions['capture.source.SourceObservationConfig.valid'],
        '-- ACTUAL SQL DICTIONARIES': b'\n\n'.join(
            (b'set_option linter.unusedVariables false in\n' if name.endswith('SealedSealed') else b'')
            + functions[name] for name in sql_names),
        '-- ACTUAL TEXT DICTIONARY': functions['alloc.string.String.Insts.RusqliteTypesFrom_sqlFromSql'],
        '-- ACTUAL SCHEMA CALLBACK': b'set_option linter.unusedVariables false in\n' + functions[schema_callback + '.call_once'],
        '-- ACTUAL SCHEMA FN DICTIONARY': functions[schema_callback],
        '-- ACTUAL SCHEMA': functions[codec + 'schema'],
        '-- ACTUAL JSON ERROR MAP': callbacks(codec + 'checked_source_configuration.closure.'),
        checked_alias: functions[codec + 'checked_source_configuration'],
        '-- ACTUAL LOAD CALLBACKS': callbacks(codec + 'load.closure'),
        '-- ACTUAL QUERY ALIASES': '\n\n'.join(aliases).encode(),
        load_alias: actual_load,
    }
    if not all(fragments.values()):
        raise ValueError('a generated declaration group is empty')
    view = (HERE / 'SourceMetadataView.lean.in').read_bytes()
    for marker, content in fragments.items():
        view = replace_once(view, marker.encode() + b'\n', content + b'\n')
    return view, report, {name: sha256(content).hexdigest() for name, content in fragments.items()}


def prepare(variant, actual, prior, audit, output, packages, service):
    view, extracted, fragments = metadata(actual, variant)
    root_name = ROOTS[variant]
    views = ('SourceMetadataView', 'SourceMetadataSpec', root_name)
    original = (prior / 'plan.json').read_bytes()
    plan = json.loads(original)
    receipt = (audit / 'result.json').read_bytes()
    results = json.loads(receipt)
    if (plan['root'], plan['source'], len(plan['files'])) != ('SourceRecordBounds', capture.SOURCE, 52):
        raise ValueError('the original source record bounds view is required')
    if (results['complete'] is not True or results['source_sha256'] != plan['files']
            or results['physical_files'] != 52 or len(results['audited']) != 145
            or any(results[key] for key in ('errors', 'warnings', 'rejected'))):
        raise ValueError('the original complete source record bounds audit is required')
    read_exact(audit / 'service.log', results['service_log_sha256'])
    if closure(prior, ['SourceRecordBounds.lean'], active_view='lakefile.toml') != plan['files']:
        raise ValueError('the original source record bounds proof closure differs')

    output.mkdir(mode=0o700)
    for name, target in (('.lake/packages', packages), ('.tools/lean-mcp', service)):
        link = output / name
        link.parent.mkdir()
        link.symlink_to(target, target_is_directory=True)
    for name, digest in plan['files'].items():
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(read_exact(prior / name, digest))
    root = HERE.parents[4]
    support = {}
    for name in SUPPORT:
        payload = (root / name).read_bytes()
        if (prior / name).read_bytes() != payload:
            raise ValueError('the original tool or service support changed: ' + name)
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(payload)
        support[name] = sha256(payload).hexdigest()
    configuration = (prior / 'lakefile.toml').read_text()
    for name in views:
        configuration += f'\n[[lean_lib]]\nname = "{name}"\n'
        template = view if name == 'SourceMetadataView' else (HERE / (name + '.lean.in')).read_bytes()
        (output / (name + '.lean')).write_bytes(template)
    (output / 'lakefile.toml').write_text(configuration)

    files = closure(output, [f'{root_name}.lean'], active_view='lakefile.toml')
    added = {name + '.lean' for name in views}
    if files.keys() != set(plan['files']) | added or any(files[key] != val for key, val in plan['files'].items()):
        raise ValueError('the materialized closure or an inherited file differs')
    delivered = dict(root=root_name, variant=variant, source=capture.SOURCE, prior=sha256(original).hexdigest(),
                     audit=sha256(receipt).hexdigest(), actual=extracted, fragments=fragments,
                     support=support, files=files, strict_files=sorted(files),
                     modules={name: {'sha256': digest} for name, digest in files.items()},
                     build_order=[name.removesuffix('.lean') for name in files])
    (output / 'plan.json').write_text(json.dumps(delivered, indent=2) + '\n')
    print(root_name, len(files), 'exact Lean inputs')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    names = ('actual', 'prior', 'audit', 'output', 'lake-packages', 'lean-mcp')
    cli.add_argument('--variant', required=True, choices=ROOTS)
    for name in names:
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    prepare(args.variant,
            *(getattr(args, name.replace('-', '_')).resolve(strict=name != 'output') for name in names))
