"""Bind the actual prepare and three-column mapper to the audited row consumer."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
from tempfile import TemporaryDirectory

import source_sql_cursor_capture as capture
from prepare_observation_suffix import replace_once
from prepare_retention_providers import declarations
from prepare_world_begin import closure, read_exact


HERE = Path(__file__).with_name('source-sql-cursor')
ROOTS = {'original': 'SourceSqlCursor', 'checksum-first': 'SourceSqlCursorNegative'}
PACKAGE = 'controller.lifecycle_inbox.source_work.'
SUPPORT = ('lean-toolchain', 'lake-manifest.json', 'tools/lean_session.py',
           'scripts/lean-mcp-server.sh')


def cursor(directory, variant):
    report = json.loads((directory / 'result.json').read_bytes())
    if (report['source'], report['tree'], report['variant'], report['options'], report['driver'],
            report['complete']) != (capture.SOURCE, capture.TREE, variant, capture.CFG,
                                   capture.sql.digest(Path(capture.__file__)), True):
        raise ValueError('the completed original cursor capture is required')
    if [(item['label'], item['exit_code'], item['bounded_stop']) for item in report['commands']] != [
            ('charon', 0, False), ('aeneas', 0, False)]:
        raise ValueError('the fixed tools must finish normally')
    read_exact(directory / 'source.tar', capture.ARCHIVE)
    stage = read_exact(directory / 'source' / capture.sql.CODEC, report['codec'])
    with TemporaryDirectory(prefix='source-sql-cursor-') as fresh:
        if stage != capture.reconstruct_codec(Path(fresh), directory / 'source.tar', variant):
            raise ValueError('the staged cursor must reproduce the exact checked Rust input')
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
        raise ValueError('the original query, mapper or consumer selection changed')

    function_bytes = files[f'{capture.NAMESPACE}/Funs.lean']
    funs = declarations(function_bytes.removesuffix(b'\n\nend ' + capture.NAMESPACE.encode() + b'\n'))
    marker_names = {
        '-- ACTUAL BLOB DICTIONARY': 'alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql',
        '-- ACTUAL OPTION DICTIONARY': 'core.option.Option.Insts.RusqliteTypesFrom_sqlFromSql',
        '-- ACTUAL ITERATOR DICTIONARY': PACKAGE + 'codec.SourceRows.Insts.'
            'CoreIterTraitsIteratorIteratorResultTupleVecU8VecU8OptionVecU8Error',
        '-- ACTUAL MAPPER': PACKAGE + 'codec.read_source_columns',
        '-- ACTUAL SCAN': PACKAGE + 'codec.scan_loaded_records',
    }
    fragments = {marker: funs[name] for marker, name in marker_names.items()}
    if fragments['-- ACTUAL SCAN'].count(b'"' + capture.SELECT.encode() + b'"') != 1:
        raise ValueError('the generated caller must use the actual ordered SELECT')
    view = (HERE / 'SourceSqlCursorView.lean.in').read_bytes()
    for marker, content in fragments.items():
        view = replace_once(view, marker.encode() + b'\n', content + b'\n')
    return view, report, {name: sha256(content).hexdigest() for name, content in fragments.items()}


def prepare(variant, actual, prior, audit, output, packages, service):
    view, extracted, fragments = cursor(actual, variant)
    root_name = ROOTS[variant]
    views = ('SourceSqlCursorView', 'SourceSqlCursorWitness', root_name)
    original = (prior / 'plan.json').read_bytes()
    plan = json.loads(original)
    receipt = (audit / 'result.json').read_bytes()
    results = json.loads(receipt)
    if (plan['root'], plan['source'], len(plan['files'])) != ('SourceLoadBinding', capture.SOURCE, 46):
        raise ValueError('the original row consumer view is required')
    if (results['complete'] is not True or results['source_sha256'] != plan['files']
            or results['physical_files'] != 46 or len(results['audited']) != 115
            or any(results[key] for key in ('errors', 'warnings', 'rejected'))):
        raise ValueError('the original complete row consumer audit is required')
    read_exact(audit / 'service.log', results['service_log_sha256'])
    if closure(prior, ['SourceLoadBinding.lean'], active_view='lakefile.toml') != plan['files']:
        raise ValueError('the original loader proof closure differs')

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
        template = view if name == 'SourceSqlCursorView' else (HERE / (name + '.lean.in')).read_bytes()
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
