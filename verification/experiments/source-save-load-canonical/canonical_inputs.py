"""Expose the actual specialized record decoder and the two original digest/encoding forwarding calls."""

from pathlib import Path
import sys

HERE = Path(__file__).resolve().parent
COLLECTOR = HERE.with_name('source-save-load-collector')
sys.path.insert(0, str(COLLECTOR))
from collector_inputs import ROOT, SOURCE_FILES, NAMESPACE, retention, row_provider
from collector_inputs import configuration as parent_configuration, stage as parent_stage

PARENT = 'cd6f6b3c8365e1dd05fe350443f0ff848e33c557'
SOURCE_WORK = 'sandproof_controller_service::controller::lifecycle_inbox::source_work'
SHA = 'sandproof_controller_service::controller::lifecycle_inbox::sha256::digest'
DIGEST_CONSTRUCTOR = 'sandbox_protocol::lifecycle::digest::ContentDigest::from_sha256'
MODES = ('correct', 'omit-canonical-byte-check')
CODEC_BODY = 'fn checked<T: serde::de::DeserializeOwned + Serialize>('
PAYLOAD = 'fn checked_source_payload('
BOUNDARIES = ('decode_source_payload', 'encode_source_payload', 'digest_source_payload')


def specialize_checked_record(text, mode):
    if mode not in MODES:
        raise ValueError('unknown canonical record view')
    start = text.index(CODEC_BODY)
    end = text.index('\nfn checked_source_configuration(', start)
    generic = text[start:end].rstrip()
    body = generic[generic.index('    let value: T ='):]
    transformations = (
        ('let value: T = serde_json::from_slice(bytes)',
         'let value: Option<SourceWorkRecord> = decode_source_payload(bytes)'),
        ('Sha256::digest(bytes).as_slice()', 'digest_source_payload(bytes).as_slice()'),
        ('serde_json::to_vec(&value)', 'encode_source_payload(&value)'),
    )
    for old, new in transformations:
        if body.count(old) != 1:
            raise ValueError('the actual generic codec operation differs: ' + old)
        body = body.replace(old, new, 1)
    if mode == 'omit-canonical-byte-check':
        guard = ' || encode_source_payload(&value)? != bytes'
        if body.count(guard) != 1:
            raise ValueError('the actual canonical comparison differs')
        body = body.replace(guard, '', 1)
    start = text.index(PAYLOAD)
    end = text.index('\nfn scan_loaded_records(', start)
    expected = """fn checked_source_payload(
    bytes: &[u8],
    sha: &[u8],
) -> Result<Option<SourceWorkRecord>, JournalError> {
    checked(bytes, sha)
}

fn encode_load_anchor(anchor: &super::SourceAnchor) -> Result<Vec<u8>, JournalError> {
    Ok(serde_json::to_vec(anchor)?)
}
"""
    if text[start:end].rstrip() != expected.rstrip():
        raise ValueError('the actual row decoder boundary differs')
    checked = expected[:expected.index('    checked(bytes, sha)')]
    support = """
fn decode_source_payload(bytes: &[u8]) -> Result<Option<SourceWorkRecord>, serde_json::Error> {
    serde_json::from_slice(bytes)
}
fn encode_source_payload(value: &Option<SourceWorkRecord>) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(value)
}
fn digest_source_payload(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn encode_load_anchor(anchor: &super::SourceAnchor) -> Result<Vec<u8>, JournalError> {
    Ok(encode_anchor_json(anchor)?)
}
"""
    return text[:start] + checked + body + '\n' + support + text[end:]



def configuration(row, reference, bounds_reference, cursor_reference, collector_reference):
    cfg = parent_configuration(row, reference, bounds_reference, cursor_reference, collector_reference)
    exposed = (row.CODEC_ROOT + '::checked_source_payload', row.CODEC_ROOT + '::encode_load_anchor', SOURCE_WORK + '::digest')
    for symbol in exposed:
        cfg['opaque'].remove(symbol)
    cfg['include'] = list(dict.fromkeys([*cfg['include'], *exposed, DIGEST_CONSTRUCTOR]))
    cfg['opaque'] = list(dict.fromkeys([*cfg['opaque'], *(row.CODEC_ROOT + '::' + name for name in BOUNDARIES)]))
    return cfg


def stage(row, source, mode, *, named=True):
    parent_stage(row, source, 'correct', named=named)
    codec = source / retention.CODEC
    original = retention.pinned(retention.SOURCE, retention.CODEC).decode()
    staged = codec.read_text()
    start = original.index(CODEC_BODY)
    end = original.index('\n\npub(super) fn load(', start)
    generic = original[start:end].rstrip()
    begin = staged.index(CODEC_BODY)
    stop = staged.index('\nfn checked_source_configuration(', begin)
    if staged[begin:stop].rstrip() != generic:
        raise ValueError('the actual generic record checker changed before specialization')
    codec.write_text(specialize_checked_record(staged, mode))
