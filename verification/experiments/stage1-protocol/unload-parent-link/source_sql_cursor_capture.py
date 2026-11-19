"""Capture the original three-column source cursor and its actual caller."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

import source_row_sql_composition as sql
from source_load_binding_capture import SELECT, READER, staged_codec as stage_consumer
from prepare_transaction_capture import checked_translation
from probe_prepare_iterators import GIB, run
from revision_capture import declaration
from source_save_row_list_capture import ARCHIVE, SOURCE, STAGED, TREE


NAMESPACE = 'SandproofSourceSqlCursor'
SCAN = sql.CODEC_ROOT + '::scan_loaded_records'
MAPPER = sql.CODEC_ROOT + '::read_source_columns'
QUERY = sql.CODEC_ROOT + '::query_source_rows'
VARIANTS = ('original', 'checksum-first')
CFG = dict(roots=[SCAN], retained_roots=[MAPPER], include=[
    SCAN, MAPPER, 'sandproof_controller_service::JournalError',
    'rusqlite::params::Params', 'rusqlite::types::from_sql::FromSql',
    'rusqlite::row::RowIndex', 'core::ops::function::FnMut',
    'rusqlite::types::value_ref::ValueRef',
], opaque=['sandproof_controller_service', 'sandbox_controller_core', 'sandbox_protocol',
           'serde_json', 'rusqlite', READER, QUERY, sql.CODEC_ROOT + '::SourceRows'])


def swap_column_reads(text):
    original = ('        Ok((\n'
                '            row.get::<_, Vec<u8>>(0)?,\n'
                '            row.get::<_, Vec<u8>>(1)?,')
    if text.count(original) != 1:
        raise ValueError('the original first two column reads must occur once')
    altered = ('        let first_checksum = row.get::<_, Vec<u8>>(1)?;\n'
               '        Ok((\n'
               '            row.get::<_, Vec<u8>>(0)?,\n'
               '            first_checksum,')
    return text.replace(original, altered, 1)


def staged_codec(text, variant):
    text = stage_consumer(text)
    initial = text.index('fn load_records(')
    start = text.index('    let mut statement = connection.prepare(', initial)
    end = text.index('\n}', start)
    tail = text[start:end]
    closure_start = tail.index('|row| {')
    closure_end = tail.index('})?;', closure_start) + 1
    closure = tail[closure_start:closure_end]
    if (tail.count(SELECT) != 1 or tail.count('read_loaded_rows(rows)') != 1
            or tuple(part.strip() for part in closure.splitlines()[2:5]) != (
                'row.get::<_, Vec<u8>>(0)?,', 'row.get::<_, Vec<u8>>(1)?,',
                'row.get::<_, Option<Vec<u8>>>(2)?,') or closure.count('row.get') != 3):
        raise ValueError('the original ordered cursor and exact three column reads are required')
    query_start = tail.index('statement.query_map(')
    query_stop = closure_end + len(')?')
    original_query = tail[query_start:closure_start] + 'read_source_columns as SourceMapper)'
    captured = tail[:query_start] + 'query_source_rows(&mut statement)?' + tail[query_stop:]
    scan = ('\nfn scan_loaded_records(connection: &Connection) -> Result<Vec<SourceWorkRecord>, JournalError> {\n'
            + captured + '\n}\n')
    if variant == 'checksum-first':
        closure = swap_column_reads(closure)
    mapper = ("\nfn read_source_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<SourceColumns> "
              + closure.removeprefix('|row| ') + '\n')
    cursor = """
type SourceMapper = for<'row, 'statement> fn(&'row rusqlite::Row<'statement>) -> rusqlite::Result<SourceColumns>;
struct SourceRows<'statement>(rusqlite::MappedRows<'statement, SourceMapper>);

impl Iterator for SourceRows<'_> {
    type Item = rusqlite::Result<SourceColumns>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}

fn query_source_rows<'statement>(
    statement: &'statement mut rusqlite::Statement<'_>,
) -> rusqlite::Result<SourceRows<'statement>> {
    """ + original_query + """.map(SourceRows)
}
"""
    return text[:start] + '    scan_loaded_records(connection)' + text[end:] + scan + mapper + cursor


def admit(ir, generated, target):
    translated = checked_translation(ir, CFG, target, SCAN, MAPPER)
    functions = translated['fun_decls']
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    result = {}
    for name, opaque in ((SCAN, False), (MAPPER, False), (READER, True), (QUERY, True)):
        actual = declaration(functions, name)
        bodies = [item for item in mapping if item['def_id'] == actual['def_id']]
        if (len(bodies) != 1 or bodies[0]['is_opaque'] is not opaque
                or bodies[0]['source']['file'] != str(sql.CODEC)):
            raise ValueError('the original cursor code or consumer type changed: ' + name)
        result[name] = actual['def_id']
    return result


def reconstruct_codec(source, archive, variant):
    with tarfile.open(archive) as inputs:
        inputs.extractall(source, filter='data')
    if sql.stage(source, 'core') != STAGED:
        raise ValueError('the original reviewed save projection differs')
    return staged_codec((source / sql.CODEC).read_text(), variant).encode()


def capture(output, tools, lock, variant):
    if (os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal'
            or min(shutil.disk_usage(path).free for path in (output.parent, sql.ROOT)) < 80 * GIB):
        raise ValueError('the original formal role and capture budget are required')
    os.sched_setaffinity(0, sorted(os.sched_getaffinity(0))[:4])
    os.umask(0o077)
    fixed = json.loads((sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    if any(sql.digest(binary) != fixed[name]['binary_sha256']
           for name, binary in (('charon', charon), ('aeneas', aeneas))):
        raise ValueError('the original tool binaries are required')
    if subprocess.check_output(['git', 'rev-parse', SOURCE + '^{tree}'], cwd=sql.ROOT, text=True).strip() != TREE:
        raise ValueError('the fixed production tree is required')
    output.mkdir()
    source = output / 'source'
    source.mkdir()
    report = dict(source=SOURCE, tree=TREE, variant=variant, options=CFG, driver=sql.digest(Path(__file__)),
                  commands=[], complete=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', SOURCE], cwd=sql.ROOT, stdout=stream, check=True)
        if sql.digest(archive) != ARCHIVE:
            raise ValueError('the fixed production archive is required')
        codec = source / sql.CODEC
        codec.write_bytes(reconstruct_codec(source, archive, variant))
        report['codec'] = sql.digest(codec)

        selectors = [item for flag, group in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'),
                                               ('--include', 'include'),
                                               ('--opaque', 'opaque'))
                     for pattern in CFG[group] for item in (flag, pattern)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                   CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(output / 'target'))
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name, None)
        commands = (
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings', '--sysroot', 'default',
                        *selectors, '--dest-file', ir, '--', '--manifest-path', 'controller-service/Cargo.toml',
                        '--lib', '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors', '-split-files',
                        '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE, '-dest', generated, ir], 120),
        )
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name, argv, timeout in commands:
                verdict = run(name, argv, timeout, source, output, env)
                report['commands'].append(verdict)
                if verdict['exit_code'] or verdict['bounded_stop']:
                    raise ValueError(f'the fixed {name} did not finish')
        report['ir'] = sql.digest(ir)
        report['generated'] = {str(path.relative_to(generated)): sql.digest(path)
                               for path in sorted(generated.rglob('*')) if path.is_file()}
        report['admission'] = admit(ir, generated, fixed['target'])
        report['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    print(report['admission'])


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, required=True, type=Path)
    cli.add_argument('--variant', required=True, choices=VARIANTS)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
