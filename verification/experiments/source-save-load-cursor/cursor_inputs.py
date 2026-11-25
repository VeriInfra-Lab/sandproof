"""Expose the real ordered cursor and the original checksum-first variant."""

from hashlib import sha256
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
ROWS = HERE.with_name('source-save-load-row-bounds')
sys.path.insert(0, str(ROWS))
from rows_inputs import ROOT, SOURCE_FILES, BOUNDS_REFERENCE, CURSOR, retention, row_provider, historical_operation
from rows_inputs import configuration as parent_configuration, stage as parent_stage
from capacity_check import verify_artifacts

PARENT = '7b49ad7288b910d49deeb1ea80e56ce015c05800'
CURSOR_REFERENCE = 'b56b75ed1f866bfc01cba1cd242d692feebfa020'
CURSOR_SOURCE = 'b0d791ccc4d4e058b710dbf8f46766652b2853df'
CURSOR_NAMESPACE = 'SandproofSourceSqlCursor'
NAMESPACE = 'SandproofSaveLoadJoin'
MODES = ('correct', 'checksum-first')


def original_cursor(folder):
    result = json.loads((folder / 'result.json').read_bytes())
    if (result['complete'] is not True or (result['variant'], result['source'], result['driver']) != (
            'original', CURSOR_SOURCE, sha256(retention.pinned(CURSOR_REFERENCE, CURSOR)).hexdigest())
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in result['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the original current #977 extraction is required')
    verify_artifacts(folder, result, CURSOR_NAMESPACE)
    return result


def configuration(row, reference, bounds_reference, cursor_reference):
    result = parent_configuration(row, reference, bounds_reference)
    result['opaque'].remove(row.CODEC_ROOT + '::scan_loaded_records')
    old = original_cursor(cursor_reference)['options']
    result['opaque'] = list(dict.fromkeys([*result['opaque'],
        *(item for item in old['opaque'] if item.startswith(row.CODEC_ROOT))]))
    # The historic named SQLite MappedRows::next forwarding boundary.
    result['opaque'].append(row.CODEC_ROOT + '::*::next')
    result['include'] = list(dict.fromkeys([*result['include'], *old['include']]))
    result['retained_roots'] = list(dict.fromkeys([*result['retained_roots'], *old['retained_roots']]))
    return result


def stage(row, source, mode, *, named=True):
    if mode not in MODES:
        raise ValueError('unknown cursor capture mode')
    if retention.pinned(BOUNDS_REFERENCE, CURSOR) != retention.pinned(CURSOR_REFERENCE, CURSOR):
        raise ValueError('the inherited cursor projection differs from #977')
    parent_stage(row, source, 'correct', named=named)
    if mode == 'checksum-first':
        codec = source / retention.CODEC
        mutate = historical_operation(CURSOR, 'swap_column_reads', {})
        codec.write_text(mutate(codec.read_text()))
