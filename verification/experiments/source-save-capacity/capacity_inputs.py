"""Pinned source for the two save-capacity views."""

from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
ENCODER = HERE.with_name('source-save-encoder')
sys.path.insert(0, str(ENCODER))
from encoder_inputs import CODEC, ROOT, configuration as encoder_configuration, retention, row_provider, stage as stage_encoder

NAMESPACE = 'SandproofSaveCapacityJoin'
MODES = ('correct', 'keep-one-position')
COUNT_PROVIDER = '405b2e346669af18a00f1aeeadea7f54081e6306'


def configuration(row):
    cfg = encoder_configuration(row)
    checkpoint = 'sandproof_controller_service::controller::checkpoint'
    cfg['include'] = list(dict.fromkeys([*cfg['include'],
        row.CODEC_ROOT + '::reserved_records', row.CODEC_ROOT + '::records_per_session',
        checkpoint + '::config::MaintenanceConfig',
        'sandproof_controller_service::controller::capacity::CapacityConfig']))
    cfg['opaque'].remove(row.CODEC_ROOT + '::reserved_records')
    cfg['opaque'].append(checkpoint + '::config')
    return cfg


def stage(row, source, mode):
    if mode not in MODES:
        raise ValueError('unknown capacity view')
    stage_encoder(row, source, 'correct')
    if mode == 'keep-one-position':
        codec = source / retention.CODEC
        body = codec.read_text()
        branch = 'Ok(policy.max_operations_per_sandbox - 2)'
        original = retention.pinned(COUNT_PROVIDER, retention.OLD_PACKAGE / 'source-record-count/extract.py')
        if body.count(branch) != 1 or branch.encode() not in original:
            raise ValueError('exact #989 two-position subtraction and retained error variant required')
        codec.write_text(body.replace(branch, 'Ok(policy.max_operations_per_sandbox - 1)'))
