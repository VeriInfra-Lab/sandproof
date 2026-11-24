"""Extend the exact source-save load selection for metadata capacity and reserve."""

from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
LOAD = HERE.with_name('source-save-load')
sys.path.insert(0, str(LOAD))
from load_inputs import (REFERENCE_METADATA, SOURCE_FILES, ROOT, historical_transform, retention,
                         row_provider)
from load_inputs import configuration as original_configuration, stage as original_stage

PARENT = 'c44f460e0b5bb60863662749c66eba8bd2ab8f92'
MODES = ('correct', 'allow-inflated-reserve')
NAMESPACE = 'SandproofSaveLoadJoin'
CAPACITY = 'sandproof_controller_service::controller::capacity::config'


def configuration(row, reference):
    cfg = original_configuration(row, reference)
    sql = row.CODEC_ROOT + '::sql'
    for name in (CAPACITY, sql):
        cfg['opaque'].remove(name)
    cfg['include'].extend((CAPACITY, CAPACITY + '_using', sql))
    return cfg


def stage(row, source, mode, *, named=True):
    if mode not in MODES:
        raise ValueError('unknown payload proof view')
    original_stage(row, source, 'correct', named=named)
    if mode == 'allow-inflated-reserve':
        codec = source / retention.CODEC
        mutate = historical_transform(REFERENCE_METADATA, 'allow_inflated_reserve', {})
        codec.write_text(mutate(codec.read_text()))
