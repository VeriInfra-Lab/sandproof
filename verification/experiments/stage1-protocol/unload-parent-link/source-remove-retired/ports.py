"""Specialize only original JSON/SHA library calls in the isolated probe."""
from pathlib import Path

CODEC = Path('controller-service/src/controller/lifecycle_inbox/source_work/codec.rs')
RETIREMENT = CODEC.parent / 'retirement.rs'
CLOSURE = CODEC.parent / 'retirement/closure.rs'
CAPTURE = Path('controller-service/src/capture/retirement.rs')
WORK = 'sandproof_controller_service::controller::lifecycle_inbox::source_work'
PORTS = [
    WORK + '::codec::' + name for name in (
        'decode_retired_record', 'encode_retired_record', 'encode_retired_optional_record',
        'digest_retired_record')
] + [WORK + '::retirement::encode_retirement_state',
     WORK + '::retirement::closure::encode_retirement_release',
     WORK + '::retirement::closure::encode_retirement_restore',
     'sandproof_controller_service::capture::retirement::encode_retirement_binding',
     'sandproof_controller_service::capture::retirement::digest_retirement_binding']


def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('exact single original library call required: ' + old)
    return text.replace(old, new, 1)


def stage(source):
    codec = source / CODEC
    original = codec.read_text()
    begin = original.index('fn clear_retired_source_record(')
    end = original.index('pub(in crate::controller) fn remove_retired(', begin)
    helper = original[begin:end]
    for old, new in (
            ('checked(&bytes, &sha)?', 'checked_retired_record(&bytes, &sha)?'),
            ('serde_json::to_vec(record)?', 'encode_retired_record(record)?'),
            ('Sha256::digest(empty).as_slice()', 'digest_retired_record(empty).as_slice()')):
        helper = once(helper, old, new)
    helper = once(helper,
        'params![empty.as_slice(),digest_retired_record(empty).as_slice(),id,bytes,sha,key.as_bytes().as_slice()]',
        '(empty.as_slice(),digest_retired_record(empty).as_slice(),id,bytes.as_slice(),sha.as_slice(),key.as_bytes().as_slice())')
    start = original.index('    let value: T = serde_json::from_slice(bytes)')
    stop = original.index('\npub(super) fn load(', start)
    checked = original[start:stop].rstrip()
    for old, new in (
            ('let value: T = serde_json::from_slice(bytes)',
             'let value: Option<SourceWorkRecord> = decode_retired_record(bytes)'),
            ('Sha256::digest(bytes).as_slice()', 'digest_retired_record(bytes).as_slice()'),
            ('serde_json::to_vec(&value)', 'encode_retired_optional_record(&value)')):
        checked = once(checked, old, new)
    ports = '''fn decode_retired_record(bytes: &[u8]) -> Result<Option<SourceWorkRecord>, serde_json::Error> {
    serde_json::from_slice(bytes)
}
fn encode_retired_record(record: &SourceWorkRecord) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(record)
}
fn encode_retired_optional_record(record: &Option<SourceWorkRecord>) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(record)
}
fn digest_retired_record(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn checked_retired_record(bytes: &[u8], sha: &[u8]) -> Result<Option<SourceWorkRecord>, JournalError> {
'''
    codec.write_text(original[:begin] + ports + checked + '\n\n' + helper + original[end:])

    path = source / RETIREMENT
    text = path.read_text()
    text = once(text, 'serde_json::to_vec(state)?.len() as u64',
                'encode_retirement_state(state)?.len() as u64')
    text += '''
fn encode_retirement_state(state: &MaterialRetirementState) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(state)
}
'''
    path.write_text(text)

    path = source / CLOSURE
    text = path.read_text()
    old = '''serde_json::to_vec(&(
                &claim.intent,
                claim.local,
                claim.generation,
                &claim.activation,
            ))'''
    new = '''encode_retirement_restore(
                &claim.intent,
                claim.local,
                claim.generation,
                &claim.activation,
            )'''
    text = once(text, old, new)
    # Two existing uses of the exact same release value, in separate helpers.
    old = 'serde_json::to_vec(&record.material_release)'
    if text.count(old) != 2:
        raise ValueError('both original release-binding call sites are required')
    text = text.replace(old, 'encode_retirement_release(&record.material_release)')
    text += '''
fn encode_retirement_release(value: &super::MaterialReleaseFacts) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(value)
}
fn encode_retirement_restore(
    intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    local: crate::InternalLocalBinding,
    generation: core::Generation,
    activation: &Option<sandproof_protocol::WorkAction<core::EffectId>>,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&(intent, local, generation, activation))
}
'''
    path.write_text(text)

    path = source / CAPTURE
    text = path.read_text()
    old = '''serde_json::to_vec(&(
                self.sandbox,
                self.generation,
                &self.full,
                &self.source,
                &self.seal_action,
                &self.worker,
                self.manifest_digest,
            ))'''
    new = '''encode_retirement_binding(
                self.sandbox,
                self.generation,
                &self.full,
                &self.source,
                &self.seal_action,
                &self.worker,
                self.manifest_digest,
            )'''
    text = once(text, old, new)
    digest_old = '''Sha256::digest(encode_retirement_binding(
                self.sandbox,
                self.generation,
                &self.full,
                &self.source,
                &self.seal_action,
                &self.worker,
                self.manifest_digest,
            )?)
            .into()'''
    digest_new = '''digest_retirement_binding(&encode_retirement_binding(
                self.sandbox,
                self.generation,
                &self.full,
                &self.source,
                &self.seal_action,
                &self.worker,
                self.manifest_digest,
            )?)'''
    text = once(text, digest_old, digest_new)
    text += '''
fn digest_retirement_binding(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
fn encode_retirement_binding(
    sandbox: SandboxId,
    generation: Generation,
    full: &FullCaptureWorkRequest,
    source: &SourceObservationRequirement,
    seal_action: &Option<WorkAction<EffectId>>,
    worker: &Option<CaptureWorkerBinding>,
    manifest_digest: Option<ContentDigest>,
) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(&(sandbox, generation, full, source, seal_action, worker, manifest_digest))
}
'''
    path.write_text(text)
    return [CODEC, RETIREMENT, CLOSURE, CAPTURE]
