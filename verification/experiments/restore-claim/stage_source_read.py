"""Expose the original source-row decision and traversal in a private extraction copy."""
import argparse
from pathlib import Path
import shutil
from textwrap import dedent, indent

from stage_decisions import replace_once
from stage_lean import HERE, MODULE, fixed_source


CODEC = Path('controller-service/src/controller/lifecycle_inbox/source_work/codec.rs')
SERVICE = Path('controller-service/src/lib.rs')
LIBRARY = 'SandproofRestoreSourceRead'
ERROR_TYPES = {'core.io.error.Error', 'rusqlite.error.Error', 'serde_json.error.Error'}
LIBRARY_FUNCTIONS = {
    'Slice.Insts.CoreCmpPartialEqArray.eq', 'Slice.Insts.CoreCmpPartialEqArray.ne',
    'alloc.vec.Vec.as_slice', 'alloc.vec.Vec.Insts.CoreCmpPartialEqShared0Slice.ne',
}


def stage_rust(project):
    source = fixed_source(project, CODEC, '87561cb050f40186d20be73b7eeaf91a93be2248d017316fcb886864a062a453')
    lib = fixed_source(project, SERVICE, 'bc15a0989bca7001b2927e6c912601832bf01601cf83e3cfac34351e4fbd6f73')

    collect_start = source.index('    let mut records = Vec::new();')
    query_start = source.index('statement.query_map([], |row| {', collect_start)
    query_end = source.index('\n    })?', query_start) + len('\n    })?')
    row_start = source.index('        let (bytes, sha, key) = row?;', query_end)
    row_end = source.index('\n    }\n    Ok(records)', row_start)
    collect_end = source.index('\n}', row_end)
    query = source[query_start:query_end]
    row = dedent(source[row_start:row_end])
    row = replace_once(row, 'checked(&bytes, &sha)?', 'proof_checked_source_record::<J>(&bytes, &sha)?')
    row = replace_once(row, 'serde_json::to_vec(&record.work.anchor)?', 'J::encode_anchor(&record.work.anchor)?')
    row = replace_once(row, 'super::digest(', 'J::anchor_digest(')
    row = replace_once(row, 'records.push(record);', 'return Ok(Some(record));')

    checked_start = source.index('    let value: T = serde_json::from_slice(bytes)')
    checked_end = source.index('\n}', checked_start)
    checked = replace_once(source[checked_start:checked_end], 'let value: T = serde_json::from_slice(bytes)',
                           'let value = J::decode_record(bytes)')
    checked = replace_once(checked, 'serde_json::to_vec(&value)?', 'J::encode_record(&value)?')
    checked = replace_once(checked, 'Sha256::digest(bytes).as_slice()', 'J::digest_slice(&J::row_digest(bytes))')

    # The original query and row callback remain intact, including all three columns.
    actual_query = f'    let rows = {query};\n    proof_collect_source_records::<OriginalRowJson, _>(rows)'
    source = source[:collect_start] + actual_query + source[collect_end:]
    source += '''

fn proof_decode_source_record<J: SourceRowJson>(
    row: Result<(Vec<u8>, Vec<u8>, Option<Vec<u8>>), rusqlite::Error>,
) -> Result<Option<SourceWorkRecord>, JournalError> {
''' + indent(row, '    ') + '''
    Ok(None)
}

fn proof_checked_source_record<J: SourceRowJson>(
    bytes: &[u8],
    sha: &[u8],
) -> Result<Option<SourceWorkRecord>, JournalError> {
''' + checked + '''
}

trait SourceRowJson {
    type RowDigest;
    fn decode_record(bytes: &[u8]) -> Result<Option<SourceWorkRecord>, serde_json::Error>;
    fn encode_record(value: &Option<SourceWorkRecord>) -> Result<Vec<u8>, serde_json::Error>;
    fn encode_anchor(value: &super::SourceAnchor) -> Result<Vec<u8>, serde_json::Error>;
    fn row_digest(bytes: &[u8]) -> Self::RowDigest;
    fn digest_slice(output: &Self::RowDigest) -> &[u8];
    fn anchor_digest(bytes: &[u8]) -> sandproof_protocol::lifecycle::ContentDigest;
}

struct OriginalRowJson;

impl SourceRowJson for OriginalRowJson {
    type RowDigest = sha2::digest::Output<Sha256>;

    fn decode_record(bytes: &[u8]) -> Result<Option<SourceWorkRecord>, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    fn encode_record(value: &Option<SourceWorkRecord>) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(value)
    }

    fn encode_anchor(value: &super::SourceAnchor) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(value)
    }

    fn row_digest(bytes: &[u8]) -> Self::RowDigest {
        Sha256::digest(bytes)
    }

    fn digest_slice(output: &Self::RowDigest) -> &[u8] {
        output.as_slice()
    }

    fn anchor_digest(bytes: &[u8]) -> sandproof_protocol::lifecycle::ContentDigest {
        super::digest(bytes)
    }
}


fn proof_collect_source_records<J, I>(rows: I) -> Result<Vec<SourceWorkRecord>, JournalError>
where
    J: SourceRowJson,
    I: Iterator<Item = Result<(Vec<u8>, Vec<u8>, Option<Vec<u8>>), rusqlite::Error>>,
{
    let mut records = Vec::new();
    for row in rows {
        if let Some(record) = proof_decode_source_record::<J>(row)? {
            records.push(record);
        }
    }
    Ok(records)
}
'''
    (project / CODEC).write_text(source)

    # Only extraction names change. Both original error conversion bodies remain byte-identical.
    conversion = 'impl From<SqliteError> for JournalError {'
    tagged = replace_once(lib, conversion, '#[charon::rename("SqliteSourceRowFromError")]\n' + conversion)
    (project / SERVICE).write_text('#![feature(register_tool)]\n#![register_tool(charon)]\n' + tagged)


def stage_lean(project, generated):
    emitted = generated / LIBRARY
    for module, expected in (('TypesExternal', ERROR_TYPES), ('FunsExternal', LIBRARY_FUNCTIONS)):
        lines = (emitted / f'{module}_Template.lean').read_text().splitlines()
        actual = {line.split()[1] for line in lines if line.startswith('axiom ')}
        if actual != expected:
            raise ValueError(f'unexpected {module} symbols: {actual}')

    parent = Path('verification/aeneas/restore-source-read')
    library = project / parent / LIBRARY
    library.mkdir(parents=True)
    for module in ('Types', 'Funs'):
        shutil.copyfile(emitted / f'{module}.lean', library / f'{module}.lean')
        shutil.copyfile(HERE / f'SourceRead{module}External.lean.in', library / f'{module}External.lean')

    folder = project / MODULE
    folder.mkdir(parents=True)
    for module in ('SourceRead', 'SourceCollect'):
        shutil.copyfile(HERE / f'{module}.lean.in', folder / f'{module}.lean')
    with (project / 'lakefile.toml').open('a') as output:
        output.write(f'\n[[lean_lib]]\nname = "{LIBRARY}"\nsrcDir = "{parent.as_posix()}"\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('project', type=Path)
    parser.add_argument('--generated', type=Path)
    args = parser.parse_args()
    if args.generated is None:
        stage_rust(args.project)
    else:
        stage_lean(args.project, args.generated)
