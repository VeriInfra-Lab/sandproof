"""The exact source shared by the encoder's formal and native views."""

from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
JOINT = HERE.with_name('source-save-retention-rows')
sys.path.insert(0, str(JOINT))
from joint_inputs import ROOT, configuration as parent_configuration, retention, row_provider, stage as stage_parent

NAMESPACE = 'SandproofSaveEncoderJoin'
MODES = ('correct', 'zero-row-key')
CODEC = 'controller.lifecycle_inbox.source_work.codec.'
ENCODER_PROVIDER = '36f551f005e29b960cf1ac0a2638e21c47f4fa2a'
STAGED_CODEC = retention.OLD_PACKAGE / 'source_key_encode_capture.py'


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('expected one original source occurrence: ' + old)
    return text.replace(old, new)


def stage(row, source, mode):
    if mode not in MODES:
        raise ValueError('unknown encoder view')
    stage_parent(row, source, 'parent')
    codec = source / retention.CODEC
    text = codec.read_text()
    start = text.index('fn encode_candidate_records(')
    end = text.index('fn retains_saved_records(', start)
    body = text[start:end]
    body = replace_once(body, 'serde_json::to_vec(&record.work.anchor)', 'encode_anchor_json(&record.work.anchor)')
    body = replace_once(body, 'serde_json::to_vec(record)', 'encode_record_json(record)')
    # Reuse the two #974 monomorphic Rust forwards verbatim.
    original = retention.pinned(ENCODER_PROVIDER, STAGED_CODEC).decode()
    wrappers = original.split("text[stop:] + '''", 1)[1].split("'''", 1)[0]
    text = text[:start] + body + text[end:] + wrappers
    # Avoid the Aeneas name collision between two From dictionaries while
    # returning the identical JSON error constructor and payload.
    conversion = '''impl From<serde_json::Error> for JournalError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}'''
    if (source / retention.LIB).read_text().count(conversion) != 1:
        raise ValueError('the original JSON error conversion is required')
    for call in ('encode_anchor_json(&record.work.anchor)', 'encode_record_json(record)'):
        text = replace_once(text, call + '?', 'match ' + call + ' {\n'
            '                Ok(bytes) => bytes,\n'
            '                Err(error) => return Err(JournalError::Json(error)),\n'
            '            }')
    if mode == 'zero-row-key':
        text = replace_once(text, 'encoded.push((key, bytes));', 'encoded.push(([0_u8; 32], bytes));')
    codec.write_text(text)


def configuration(row):
    cfg = parent_configuration(row, 'parent')
    cfg['opaque'].remove(row.CODEC_ROOT + '::encode_candidate_records')
    cfg['opaque'] += [row.CODEC_ROOT + '::' + name for name in ('encode_anchor_json', 'encode_record_json')]
    cfg['opaque'].append(row.CODEC_ROOT.rsplit('::', 1)[0] + '::digest')
    return cfg
