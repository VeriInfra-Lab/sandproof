"""Capture real row-byte counting, exact table dispatch and source charges."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'source-metadata-json'))
from capture import run
import source_metadata_capture as metadata
from prepare_transaction_capture import checked_translation
from revision_capture import declaration, regular_calls
import importlib.util
_charge_spec = importlib.util.spec_from_file_location('source_row_charge_capture', HERE.parent / 'source-row-charge/extract.py')
charge = importlib.util.module_from_spec(_charge_spec)
_charge_spec.loader.exec_module(charge)

NAMESPACE = 'SandproofSourceRowUsage'
CAPACITY = 'sandproof_controller_service::controller::capacity'
ROOT = CAPACITY + '::row_bytes'
DISPATCH = CAPACITY + '::charged_inbox_bytes'
ADD = CAPACITY + '::add'
CAPACITY_FILE = Path('controller-service/src/controller/capacity.rs')
INTERNAL = 'sandproof_controller_service::controller::internal_inbox::codec::charged_bytes'
INTERNAL_FILE = Path('controller-service/src/controller/internal_inbox/codec.rs')
OPTIONS = dict(roots=[ROOT], retained_roots=[DISPATCH], include=[
    ROOT, DISPATCH, ADD, CAPACITY + '::source_row_value_size', CAPACITY + '::source_row_column_size', CAPACITY + '::row_bytes_using', CAPACITY + '::SourceRowSizeRead', CAPACITY + '::*::read_size', INTERNAL, charge.ROOT, charge.BASE,
    metadata.sql.CODEC_ROOT + '::source_key_is_null', CAPACITY + '::CapacityConfig',
    'sandproof_controller_service::JournalError', 'rusqlite::row::RowIndex',
    'rusqlite::types::value_ref::ValueRef', 'rusqlite::types::from_sql::FromSql',
], opaque=['sandproof_controller_service', 'sandbox_controller_core', 'sandbox_protocol', 'rusqlite'])
VARIANTS = ('original', 'drop-terminator-byte')


def drop_terminator_byte(text):
    start = text.index('fn row_bytes(')
    stop = text.index('\nfn publication_bytes(', start)
    body = text[start:stop]
    anchor = 'value.len() as u64'
    if body.count(anchor) != 1:
        raise ValueError('the original complete TEXT/BLOB length must be unique')
    changed = body.replace(anchor, 'value.len().saturating_sub(1) as u64', 1)
    return text[:start] + changed + text[stop:]


def isolate_value_size(text):
    first = text.index('fn row_bytes(')
    start = text.index('        let size = match row.get_ref(index)? {', first)
    end = text.index('\n        };', start) + len('\n        };')
    block = text[start:end]
    arms = block.split('match row.get_ref(index)? {', 1)[1].removesuffix('\n        };')
    if (block.count('rusqlite::types::ValueRef::Null => 0,') != 1
            or block.count('rusqlite::types::ValueRef::Integer(_) | rusqlite::types::ValueRef::Real(_) => 8,') != 1
            or block.count('rusqlite::types::ValueRef::Text(value) | rusqlite::types::ValueRef::Blob(value) => {') != 1
            or sum(block.count(value) for value in ('value.len() as u64', 'value.len().saturating_sub(1) as u64')) != 1):
        raise ValueError('the complete original five-way byte-size match is required')
    helper = "\nfn source_row_value_size(value: rusqlite::types::ValueRef<'_>) -> u64 {\n    match value {" + arms + "\n    }\n}\n"
    return text[:start] + '        let size = source_row_column_size(row, index)?;' + text[end:] + helper + "\nfn source_row_column_size(row: &rusqlite::Row<'_>, index: usize) -> Result<u64, JournalError> {\n    Ok(source_row_value_size(row.get_ref(index)?))\n}\n"


def isolate_row_reader(text):
    start = text.index('fn row_bytes(')
    stop = text.index('\nfn publication_bytes(', start)
    original = text[start:stop]
    at = original.index('    let mut bytes = 0;')
    body = original[at:]
    call = 'source_row_column_size(row, index)?'
    if body.count(call) != 1 or body.count('for index in 0..columns {') != 1:
        raise ValueError('the complete original row loop and its sole indexed read are required')
    # Desugar the original for-range without replacing its indexed body. The
    # only opaque helper delegates one next call to the same std::ops::Range.
    # Its A6 value contract must be reviewed separately from the business loop.
    body = body.replace('for index in 0..columns {',
                        'let mut indices = 0..columns;\n    while let Some(index) = source_row_next(&mut indices) {', 1)
    generic = """
trait SourceRowSizeRead {
    fn read_size(&self, index: usize) -> Result<u64, JournalError>;
}
impl SourceRowSizeRead for rusqlite::Row<'_> {
    fn read_size(&self, index: usize) -> Result<u64, JournalError> {
        source_row_column_size(self, index)
    }
}
fn row_bytes_using<R: SourceRowSizeRead>(row: &R, columns: usize, cap: &CapacityConfig) -> Result<u64, JournalError> {
""" + body.replace(call, 'row.read_size(index)?', 1)
    wrapper = original[:at] + '    row_bytes_using(row, columns, cap)\n}\n'
    adapter = """
fn source_row_next(indices: &mut std::ops::Range<usize>) -> Option<usize> {
    indices.next()
}
"""
    return text[:start] + wrapper + text[stop:] + generic + adapter


def admit(ir, generated, target):
    translated = checked_translation(ir, OPTIONS, target, ROOT, DISPATCH)
    mapping = json.loads((generated / 'translation.json').read_bytes())['functions']
    result = {}
    for name, filename in ((ROOT, str(CAPACITY_FILE)), (DISPATCH, str(CAPACITY_FILE)),
                           (ADD, str(CAPACITY_FILE)), (CAPACITY + '::source_row_value_size', str(CAPACITY_FILE)),
                           (CAPACITY + '::source_row_column_size', str(CAPACITY_FILE)),
                           (CAPACITY + '::row_bytes_using', str(CAPACITY_FILE)),
                           (INTERNAL, str(INTERNAL_FILE)),
                           (charge.ROOT, str(metadata.sql.CODEC)), (charge.BASE, charge.BASE_FILE),
                           (metadata.sql.CODEC_ROOT + '::source_key_is_null', str(metadata.sql.CODEC))):
        actual = declaration(translated['fun_decls'], name)
        entries = [row for row in mapping if row['def_id'] == actual['def_id']]
        expected = 3 if name == CAPACITY + '::row_bytes_using' else 1
        if (len(entries) != expected or 'Structured' not in actual['body'] or
                any(row['is_opaque'] or row['source']['file'] != filename for row in entries)):
            raise ValueError('every original count, dispatch and charge body must be transparent: ' + name)
        result[name] = dict(def_id=actual['def_id'], generated_functions=len(entries))
    prefix = [{'Ident': [part, 0]} for part in CAPACITY.split('::')]
    readers = [row for row in translated['fun_decls'] if row and
               row['item_meta']['name'][:-2] == prefix and
               row['item_meta']['name'][-1] == {'Ident': ['read_size', 0]}]
    if len(readers) != 1:
        raise ValueError('the one actual Row trait implementation is required')
    reader = readers[0]
    entries = [row for row in mapping if row['def_id'] == reader['def_id']]
    if ('Structured' not in reader['body'] or len(entries) != 1 or entries[0]['is_opaque'] or
            entries[0]['source']['file'] != str(CAPACITY_FILE) or
            list(regular_calls(reader['body'])) != [result[CAPACITY + '::source_row_column_size']['def_id']]):
        raise ValueError('the actual Row reader must call the original indexed read once')
    result['row_reader'] = dict(def_id=reader['def_id'], generated_functions=1)
    adapter = declaration(translated['fun_decls'], CAPACITY + '::source_row_next')
    entries = [row for row in mapping if row['def_id'] == adapter['def_id']]
    if (adapter['body'] != 'Opaque' or len(entries) != 1 or not entries[0]['is_opaque'] or
            entries[0]['source']['file'] != str(CAPACITY_FILE)):
        raise ValueError('only the separately reviewed standard Range next adapter may be opaque')
    result['range_next_adapter'] = dict(def_id=adapter['def_id'], generated_functions=1)
    return result


def capture(output, tools, lock, variant):
    if os.environ.get('SANDPROOF_ACCEPTANCE_ROLE') != 'formal':
        raise ValueError('the existing formal role is required')
    os.umask(0o077)
    os.sched_setaffinity(0, set(sorted(os.sched_getaffinity(0))[:4]))
    fixed = json.loads((metadata.sql.ROOT / 'verification/foundations-environment.json').read_bytes())
    charon, aeneas = tools / 'charon/bin/charon', tools / 'bin/aeneas'
    for name, binary in (('charon', charon), ('aeneas', aeneas)):
        if metadata.sql.digest(binary) != fixed[name]['binary_sha256']:
            raise ValueError('the original fixed tool is required: ' + name)
    output.mkdir(mode=0o700)
    result = dict(source=metadata.SOURCE, tree=metadata.TREE, variant=variant, options=OPTIONS,
                  driver=metadata.sql.digest(Path(__file__)), commands=[], complete=False)
    try:
        archive = output / 'source.tar'
        with archive.open('xb') as stream:
            subprocess.run(['git', 'archive', metadata.SOURCE], cwd=metadata.sql.ROOT, stdout=stream, check=True)
        if metadata.sql.digest(archive) != metadata.ARCHIVE:
            raise ValueError('the exact parent production archive is required')
        source = output / 'source'
        source.mkdir(mode=0o700)
        codec = charge.isolate_key_match(metadata.reconstruct_codec(source, archive, 'original').decode())
        (source / metadata.sql.CODEC).write_text(codec)
        capacity = (source / CAPACITY_FILE).read_text()
        if variant == 'drop-terminator-byte':
            capacity = drop_terminator_byte(capacity)
        (source / CAPACITY_FILE).write_text(isolate_row_reader(isolate_value_size(capacity)))
        result['staged'] = {str(path): metadata.sql.digest(source / path)
                            for path in (metadata.sql.CODEC, CAPACITY_FILE, Path(charge.BASE_FILE), INTERNAL_FILE)}
        env = dict(os.environ, CARGO_NET_OFFLINE='true', CARGO_BUILD_JOBS='4',
                   CARGO_INCREMENTAL='0', CARGO_TARGET_DIR=str(output / 'target'))
        for name in ('RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS'):
            env.pop(name, None)
        selectors = [part for flag, group in (('--start-from', 'roots'), ('--start-from-if-exists', 'retained_roots'), ('--include', 'include'),
                    ('--opaque', 'opaque')) for pattern in OPTIONS[group] for part in (flag, pattern)]
        ir, generated = output / 'actual.llbc', output / 'generated'
        commands = [
            ('charon', [charon, 'cargo', '--preset=aeneas', '--error-on-warnings',
                        '--sysroot', 'default', *selectors, '--dest-file', ir, '--',
                        '--manifest-path', 'controller-service/Cargo.toml', '--lib',
                        '--locked', '--offline', '--target', fixed['target']], 240),
            ('aeneas', [aeneas, '-backend', 'lean', '-abort-on-error', '-warnings-as-errors',
                        '-split-files', '-emit-json', '-namespace', NAMESPACE, '-subdir', NAMESPACE,
                        '-dest', generated, ir], 120),
        ]
        with lock.open('r+') as guard:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
            for name, argv, seconds in commands:
                status = run(name, argv, seconds, source, output, env)
                result['commands'].append(status)
                if status['exit_code'] or status['bounded_stop']:
                    raise ValueError('the complete fixed ' + name + ' output is required')
        result['ir'] = metadata.sql.digest(ir)
        result['generated'] = {str(p.relative_to(generated)): metadata.sql.digest(p)
                               for p in sorted(generated.rglob('*')) if p.is_file()}
        result['admission'] = admit(ir, generated, fixed['target'])
        result['complete'] = True
    finally:
        (output / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
    print(variant, 'complete actual row-byte count and dispatch')


if __name__ == '__main__':
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--variant', required=True, choices=VARIANTS)
    for name in ('output', 'tools', 'lock'):
        cli.add_argument('--' + name, required=True, type=Path)
    args = cli.parse_args()
    capture(args.output.resolve(), args.tools.resolve(strict=True), args.lock.resolve(strict=True), args.variant)
