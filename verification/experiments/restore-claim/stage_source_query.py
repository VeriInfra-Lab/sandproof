"""Bind the original three-column source query to the existing SQLite session."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil

from stage_decisions import replace_once
from stage_source_read import CODEC, ERROR_TYPES, HERE, LIBRARY, LIBRARY_FUNCTIONS, MODULE
from stage_source_read import stage_rust as stage_collector


QUERY = 'SandproofRestoreSourceQuery'
SOURCE_SELECT = '"SELECT state_json,sha256,work_key FROM controller_source_work_records ORDER BY id"'
SQLITE = Path('verification/aeneas/create-transaction-link')
SQLITE_PROVIDERS = (
    'SandproofLoadBlob/ReadValueData.lean', 'SandproofLoadBlob/TypesExternal.lean',
    'SandproofSqliteSession/Data.lean', 'SandproofSqliteSession/ReadOps.lean',
)
SQLITE_TYPES = {
    'rusqlite.Connection', 'rusqlite.statement.Statement', 'rusqlite.row.MappedRows',
    'rusqlite.row.Row', 'rusqlite.types.from_sql.FromSqlError',
}
SQLITE_FUNCTIONS = {
    'rusqlite.Connection.prepare',
    'rusqlite.row.MappedRows.Insts.CoreIterTraitsIteratorIteratorResultTError.next',
    'rusqlite.row.Row.get', 'Usize.Insts.RusqliteRowRowIndex.idx',
    'rusqlite.statement.Statement.query_map',
    'alloc.vec.VecU8.Insts.RusqliteTypesFrom_sqlFromSql.column_result',
    'core.option.Option.Insts.RusqliteTypesFrom_sqlFromSql.column_result',
}


def stage_rust(project):
    stage_collector(project)
    target = project / CODEC
    source = target.read_text()
    start = source.index(f'    let mut statement = connection.prepare(\n        {SOURCE_SELECT}')
    end = source.index('\n}', start)
    body = replace_once(source[start:end], 'proof_collect_source_records::<OriginalRowJson, _>',
                        'proof_collect_source_records::<J, _>')
    body = replace_once(body, 'statement.query_map([],', 'statement.query_map(params,')
    body = replace_once(body, f'connection.prepare(\n        {SOURCE_SELECT},\n    )',
                        'connection.prepare(sql)')
    signature = '''

fn proof_query_source_records<J: SourceRowJson, P: rusqlite::Params>(
    connection: &Connection,
    params: P,
    sql: &str,
) -> Result<Vec<SourceWorkRecord>, JournalError> {
'''
    call = f'    proof_query_source_records::<OriginalRowJson, _>(connection, [], {SOURCE_SELECT})'
    target.write_text(source[:start] + call + source[end:] + signature + body + '\n}\n')


def reuse_sqlite(project):
    manifest_file = Path('verification/experiments/stage1-protocol/original-source-rows/execution-inputs.json')
    manifest = (project / manifest_file).read_text()
    entries = json.loads(manifest)['materialize']
    for relative in SQLITE_PROVIDERS:
        target = SQLITE / relative
        entry = entries[target.as_posix()]
        original = project / entry['source']
        if hashlib.sha256(original.read_bytes()).hexdigest() != entry['sha256']:
            raise ValueError(f'original SQLite provider changed: {original}')
        (project / target).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(original, project / target)


def stage_lean(project, generated):
    emitted = generated / QUERY
    for module, expected in (
        ('TypesExternal', ERROR_TYPES | SQLITE_TYPES), ('FunsExternal', LIBRARY_FUNCTIONS | SQLITE_FUNCTIONS),
    ):
        template = (emitted / f'{module}_Template.lean').read_text()
        actual = set(re.findall(r'^axiom\s+(\S+)', template, re.M))
        if actual != expected:
            raise ValueError(f'unexpected {module} symbols: {actual ^ expected}')

    reuse_sqlite(project)
    parent = Path('verification/aeneas/restore-source-query')
    library = project / parent / QUERY
    library.mkdir(parents=True)
    for module in ('Types', 'Funs'):
        shutil.copyfile(emitted / f'{module}.lean', library / f'{module}.lean')
    for module in ('TypesExternal', 'Sqlite'):
        shutil.copyfile(HERE / f'SourceQuery{module}.lean.in', library / f'{module}.lean')
    formulas = (HERE / 'SourceReadFunsExternal.lean.in').read_text().replace(LIBRARY, QUERY)
    (library / 'FunsExternal.lean').write_text(f'import {QUERY}.Sqlite\n' + formulas)

    folder = project / MODULE
    folder.mkdir(parents=True, exist_ok=True)
    for module in ('SourceRead', 'SourceCollect', 'SourceQuery'):
        template = (HERE / f'{module}.lean.in').read_text()
        if module == 'SourceQuery' and f'let sql := {SOURCE_SELECT}' not in template:
            raise ValueError('the Lean statement literal differs from the original Rust SELECT')
        (folder / f'{module}.lean').write_text(template.replace(LIBRARY, QUERY))
    with (project / 'lakefile.toml').open('a') as output:
        for name, directory in ((QUERY, parent), ('SandproofLoadBlob', SQLITE), ('SandproofSqliteSession', SQLITE)):
            output.write(f'\n[[lean_lib]]\nname = "{name}"\nsrcDir = "{directory.as_posix()}"\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('project', type=Path)
    parser.add_argument('--generated', type=Path)
    args = parser.parse_args()
    if args.generated is None:
        stage_rust(args.project)
    else:
        stage_lean(args.project, args.generated)
