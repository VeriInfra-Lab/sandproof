"""Expose the real ordered record collector and full persisted work-key guard."""

from hashlib import sha256
import json
from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
CURSOR = HERE.with_name('source-save-load-cursor')
sys.path.insert(0, str(CURSOR))
from cursor_inputs import ROOT, SOURCE_FILES, retention, row_provider
from cursor_inputs import configuration as parent_configuration, stage as parent_stage
from capacity_check import verify_artifacts

PARENT = 'ff98242ffb0e9cb6fc3c63c336981cfcc15135dc'
COLLECTOR_REFERENCE = 'c0e7fe698ce1ade6e752bcac8af73e3d9b7a55fc'
COLLECTOR_SOURCE = 'b0d791ccc4d4e058b710dbf8f46766652b2853df'
COLLECTOR_NAMESPACE = 'SandproofSourceLoadBinding'
DRIVER = retention.OLD_PACKAGE / 'source_load_binding_capture.py'
NAMESPACE = 'SandproofSaveLoadJoin'
MODES = ('correct', 'length-only-key')

KEY_GUARD = 'if key.as_slice() == record.work.requirement.work_binding.as_bytes()'
WEAK_GUARD = 'if key.len() == 32'


def original_collector(folder):
    result = json.loads((folder / 'result.json').read_bytes())
    if (result['complete'] is not True or result['source'] != COLLECTOR_SOURCE
            or result['driver'] != sha256(retention.pinned(COLLECTOR_REFERENCE, DRIVER)).hexdigest()
            or result['codec'] != retention.sha(folder / 'source' / retention.CODEC)
            or [(item['label'], item['exit_code'], item['bounded_stop']) for item in result['commands']]
            != [('charon', 0, False), ('aeneas', 0, False)]):
        raise ValueError('the exact original full source-binding capture is required')
    verify_artifacts(folder, result, COLLECTOR_NAMESPACE)
    return result


def configuration(row, reference, bounds_reference, cursor_reference, collector_reference):
    cfg = parent_configuration(row, reference, bounds_reference, cursor_reference)
    cfg['opaque'].remove(row.CODEC_ROOT + '::read_loaded_rows')
    previous = original_collector(collector_reference)['options']
    cfg['include'] = list(dict.fromkeys([*cfg['include'], *previous['include']]))
    cfg['opaque'] = list(dict.fromkeys([*cfg['opaque'], row.CODEC_ROOT + '::checked_source_payload',
                                      row.CODEC_ROOT + '::encode_load_anchor']))
    return cfg


def stage(row, source, mode, *, named=True):
    if mode not in MODES:
        raise ValueError('unknown full key collector view')
    parent_stage(row, source, 'correct', named=named)
    codec = source / retention.CODEC
    text = codec.read_text()
    if text.count(KEY_GUARD) != 1:
        raise ValueError('the unique original complete persisted work-key guard is required')
    if mode == 'length-only-key':
        codec.write_text(text.replace(KEY_GUARD, WEAK_GUARD, 1))
