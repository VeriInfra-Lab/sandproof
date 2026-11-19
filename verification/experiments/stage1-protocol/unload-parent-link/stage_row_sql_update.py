"""Stage reviewed source UPDATE variants on the fixed integration archive."""
from pathlib import Path

from source_row_write import CODEC, CODEC_DIGESTS, digest, stage as stage_row


HERE = Path(__file__).resolve().parent / 'row-sql-update'
MODES = ('core', 'swap-key-digest')
STATEMENT = 'UPDATE controller_source_work_records SET state_json=?1,sha256=?2,work_key=?3 WHERE id=?4 AND (work_key IS NULL OR work_key=?3)'
STAGED = {
    'legacy': CODEC_DIGESTS['core'],
    'core': 'f71ad9cd9e01c043347d37306878cc1a078fc6198a06cbc20f015bff3524a8f5',
    'swap-key-digest': '8ff8f57f682d518fc6cbc96a72ac601e2f64844b6f748625ce4b54b33276bf03',
}
PROBE_SOURCE = '''
#[expect(dead_code, reason = "selected by the fixed Charon")]
fn proof_source_row_sql_update(
    connection: &Connection,
    bytes: &[u8],
    sha: &[u8; 32],
    key: &[u8; 32],
    id: i64,
) -> Result<usize, JournalError> {
    SqliteSourceRow::update(connection, bytes, sha, key, id)
}
'''


def stage(source, mode):
    if mode not in STAGED:
        raise ValueError('unknown source UPDATE view')
    original = stage_row(source, 'core')
    if mode == 'legacy':
        return original
    path = source / CODEC
    text = path.read_text()
    start = text.index('    fn update(\n        connection: &Connection,')
    end = text.index('\n    fn capacity()', start)
    old = text[start:end]
    actual = (HERE / 'adapter.rs.in').read_text()
    if (old.count(STATEMENT) != 1 or actual.count(STATEMENT) != 1
            or old.count('params![bytes, sha.as_slice(), key.as_slice(), id]') != 1):
        raise ValueError('the exact original SqliteSourceRow UPDATE is required')
    if mode == 'swap-key-digest':
        left = '                Borrowed(ValueRef::Blob(sha.as_slice())),\n'
        right = '                Borrowed(ValueRef::Blob(key.as_slice())),\n'
        if actual.count(left + right) != 1:
            raise ValueError('expected the adjacent live digest and work key slots')
        actual = actual.replace(left + right, right + left)
    path.write_text(text[:start] + actual + text[end:] + PROBE_SOURCE)
    if digest(path) != STAGED[mode]:
        raise ValueError('actual prepared four-parameter source differs')
    return STAGED[mode]


def native(source, mode):
    staged = stage(source, mode)
    with (source / CODEC).open('ab') as file:
        file.write(b'\n' + (HERE / 'native-update.rs.in').read_bytes())
    return staged
