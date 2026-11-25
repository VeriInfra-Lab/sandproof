"""Pin the original six-column guard and the historical one-comparison variant."""

import ast
from hashlib import sha256
import json
from pathlib import Path
import sys
import tempfile

HERE = Path(__file__).resolve().parent
PAYLOAD = HERE.with_name('source-save-load-payload')
sys.path.insert(0, str(PAYLOAD))
from payload_inputs import ROOT, SOURCE_FILES, retention, row_provider
from payload_inputs import configuration as parent_configuration, stage as parent_stage
from capacity_check import verify_artifacts

PARENT = '62b7bc39e7f26c23e88a00bcb08c95530907e598'
BOUNDS_REFERENCE = 'e3e24a6fe7a7a3566b34c84aa27ac7001924474a'
BOUNDS_SOURCE = 'b0d791ccc4d4e058b710dbf8f46766652b2853df'
BOUNDS_NAMESPACE = 'SandproofSourceRecordBounds'
DRIVER = retention.OLD_PACKAGE / 'source_record_bounds_capture.py'
CONSUMER = retention.OLD_PACKAGE / 'source_load_binding_capture.py'
CURSOR = retention.OLD_PACKAGE / 'source_sql_cursor_capture.py'
LOADER_TEMPLATE = retention.OLD_PACKAGE / 'source-load-binding/loader.rs.in'
NAMESPACE = 'SandproofSaveLoadJoin'
MODES = ('correct', 'deployment-cap-for-length')


def original_bounds(folder):
    report = json.loads((folder / 'result.json').read_bytes())
    if (report['complete'] is not True or (report['variant'], report['source'], report['driver']) != (
            'original', BOUNDS_SOURCE, sha256(retention.pinned(BOUNDS_REFERENCE, DRIVER)).hexdigest())
            or report['codec'] != retention.sha(folder / 'source' / retention.CODEC)
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in report['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the original complete #978 extraction is required')
    verify_artifacts(folder, report, BOUNDS_NAMESPACE)
    return report


def configuration(row, reference, bounds_reference):
    cfg = parent_configuration(row, reference)
    cfg['opaque'].remove(row.CODEC_ROOT + '::load_records')
    cfg['opaque'].append(row.CODEC_ROOT + '::scan_loaded_records')
    cfg['include'] = list(dict.fromkeys([*cfg['include'], *original_bounds(bounds_reference)['options']['include']]))
    return cfg


def historical_operation(path, symbol, dependencies):
    source = retention.pinned(BOUNDS_REFERENCE, path)
    node, = (item for item in ast.parse(source, filename=str(path)).body
             if isinstance(item, ast.FunctionDef) and item.name == symbol)
    scope = {'__builtins__': {'ValueError': ValueError, 'len': len, 'tuple': tuple}, **dependencies}
    exec(compile(ast.Module(body=[node], type_ignores=[]), str(path), 'exec'), scope)
    return scope[symbol]


def stage(row, source, mode, *, named=True):
    if mode not in MODES:
        raise ValueError('unknown record-bound view')
    if (source / retention.CODEC).read_bytes() != retention.pinned(BOUNDS_SOURCE, retention.CODEC):
        raise ValueError('the current production record guard differs from #978')
    parent_stage(row, source, 'correct', named=named)
    codec = source / retention.CODEC
    text = codec.read_text()
    select = 'SELECT state_json,sha256,work_key FROM controller_source_work_records ORDER BY id'
    with tempfile.TemporaryDirectory(dir=source.parent, prefix='record-guard-projection-') as temporary:
        template = Path(temporary)
        (template / 'loader.rs.in').write_bytes(retention.pinned(BOUNDS_REFERENCE, LOADER_TEMPLATE))
        reader = historical_operation(CONSUMER, 'staged_codec', {'HERE': template, 'SELECT': select})
        cursor = historical_operation(CURSOR, 'staged_codec', {'stage_consumer': reader, 'SELECT': select})
        staged = cursor(text, 'original')
    if mode == 'deployment-cap-for-length':
        mutate = historical_operation(DRIVER, 'substitute_length_limit', {})
        staged = mutate(staged)
    codec.write_text(staged)
