"""Bind the original source capacity guards to the audited SQLite cursor."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
from tempfile import TemporaryDirectory

import source_record_bounds_capture as capture
from prepare_observation_suffix import replace_once
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact


HERE = Path(__file__).with_name('source-record-bounds')
ROOTS = {'original': 'SourceRecordBounds', 'deployment-cap-for-length': 'SourceRecordBoundsNegative'}
PACKAGE = 'controller.lifecycle_inbox.source_work.'
SUPPORT = ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py',
           'scripts/lean-mcp-server.sh')


def bounds(directory, variant):
    report = json.loads((directory / 'result.json').read_bytes())
    if (report['source'], report['tree'], report['variant'], report['options'], report['driver'],
            report['complete']) != (capture.SOURCE, capture.TREE, variant, capture.CFG,
                                   capture.sql.digest(Path(capture.__file__)), True):
        raise ValueError('the completed original bounds capture is required')
    if [(item['label'], item['exit_code'], item['bounded_stop']) for item in report['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('the fixed tools must finish normally')
    read_exact(directory / 'source.tar', capture.ARCHIVE)
    stage = read_exact(directory / 'source' / capture.sql.CODEC, report['codec'])
    with TemporaryDirectory(prefix='source-record-bounds-') as fresh:
        if stage != capture.reconstruct_codec(Path(fresh), directory / 'source.tar', variant):
            raise ValueError('the staged bounds must reproduce the exact checked Rust input')
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
        raise ValueError('the original bounds query or capacity helper selection changed')

    def captured(kind):
        payload = files[f'{capture.NAMESPACE}/{kind}.lean']
        return declarations(payload.removesuffix(b'\n\nend ' + capture.NAMESPACE.encode() + b'\n'))

    types, functions = captured('Types'), captured('Funs')
    codec = PACKAGE + 'codec.'
    retirement = 'capture.retirement.MATERIAL_'
    mapper = codec + 'load_records.closure.Insts.'
    aggregate = mapper + 'CoreOpsFunctionFnOnceTupleSharedRowResultTupleI64I64I64I64I64I64Error'
    length = codec + 'load_records.closure_1.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError'
    sql = codec + 'sql.closure.Insts.CoreOpsFunctionFnOnceTupleTryFromIntErrorJournalError'
    type_names = {
        'SOURCE CONFIG': 'capture.source.SourceObservationConfig',
        'CONFIGURATION': codec + 'Configuration',
        'CAPACITY': 'controller.capacity.CapacityConfig',
        'POLICY': 'controller.checkpoint.config.MaintenanceConfig',
        'SQL CLOSURE TYPE': codec + 'sql.closure',
        'LENGTH CLOSURE TYPE': codec + 'load_records.closure_1',
        'AGGREGATE CLOSURE TYPE': codec + 'load_records.closure',
    }
    function_names = {
        'EMPTY SEALED': 'ArrayShared0P0.Insts.RusqliteParamsSealedSealed',
        'EMPTY DICTIONARY': 'ArrayShared0P0.Insts.RusqliteParamsParams',
        'PLAN LIMIT': retirement + 'RETIREMENT_PLAN_MAX_BYTES',
        'RETIREMENT LIMIT': retirement + 'RETIREMENT_RECORD_MAX_BYTES',
        'RELEASE LIMIT': retirement + 'RELEASE_RECORD_MAX_BYTES',
        'VALUE LIMIT': codec + 'VALUE_LIMIT',
        'RESERVATION': 'capture.source.SourceObservationConfig.required_bytes',
        'SQL CLOSURE': sql + '.call_once',
        'SQL DICTIONARY': sql,
        'SQL CONVERSION': codec + 'sql',
        'LENGTH CLOSURE': length + '.call_once',
        'LENGTH DICTIONARY': length,
        'AGGREGATE CLOSURE': aggregate + '.call_once',
        'AGGREGATE DICTIONARY': aggregate,
        'PER SESSION': codec + 'records_per_session',
        'RESERVED RECORDS': codec + 'reserved_records',
        'LOADER': codec + 'load_records',
    }
    fragments = {f'-- ACTUAL {marker}': types[name] for marker, name in type_names.items()}
    fragments.update({f'-- ACTUAL {marker}': functions[name] for marker, name in function_names.items()})
    literal = b'"' + capture.AGGREGATE_SQL.encode() + b'"'
    view = (HERE / 'SourceRecordBoundsView.lean.in').read_bytes()
    if fragments['-- ACTUAL LOADER'].count(literal) != 1 or view.count(literal) != 1:
        raise ValueError('the caller and query view must use the exact original aggregate SQL')
    for marker, content in fragments.items():
        view = replace_once(view, marker.encode() + b'\n', content + b'\n')
    return view, report, {name: sha256(content).hexdigest() for name, content in fragments.items()}


def prepare(variant, actual, prior, audit, output, packages, service):
    view, extracted, fragments = bounds(actual, variant)
    root_name = ROOTS[variant]
    views = ('SourceRecordBoundsView', 'SourceRecordBoundsContract', root_name)
    original = (prior / 'plan.json').read_bytes()
    plan = json.loads(original)
    receipt = (audit / 'result.json').read_bytes()
    results = json.loads(receipt)
    if (plan['root'], plan['source'], len(plan['files'])) != ('SourceSqlCursor', capture.SOURCE, 49):
        raise ValueError('the original SQLite cursor view is required')
    if (results['complete'] is not True or results['source_sha256'] != plan['files']
            or results['physical_files'] != 49 or len(results['audited']) != 128
            or any(results[key] for key in ('errors', 'warnings', 'rejected'))):
        raise ValueError('the original complete SQLite cursor audit is required')
    read_exact(audit / 'service.log', results['service_log_sha256'])
    if closure(prior, ['SourceSqlCursor.lean'], active_view='lakefile.toml') != plan['files']:
        raise ValueError('the original SQLite cursor proof closure differs')

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
        template = view if name == 'SourceRecordBoundsView' else (HERE / (name + '.lean.in')).read_bytes()
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
