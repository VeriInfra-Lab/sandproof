"""Two candidate thin interfaces for the unchanged six-field binding call."""
from pathlib import Path

SOURCE = Path('controller-service/src/capture/continuation.rs')
MODULE = 'sandproof_controller_service::capture::continuation'
PORTS = (MODULE + '::encode_continue_work_binding', MODULE + '::digest_continue_work_binding')
VARIANTS = ('original', 'network-policy-as-deny-all', 'file-policy-as-zero')


def region(text):
    start = text.index('    pub fn binding_digest(&self) -> serde_json::Result<ContentDigest> {')
    stop = text.index('\n}\n', start)
    return start, stop


def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('one exact original call expression required: ' + old)
    return text.replace(old, new, 1)


def mutate(text, variant):
    if variant not in VARIANTS:
        raise ValueError('unknown complete continuation binding variant')
    start, stop = region(text)
    body = text[start:stop]
    old = '                &self.network_policy,\n'
    if body.count(old) != 1:
        raise ValueError('the complete original network-policy field is required')
    if variant == 'network-policy-as-deny-all':
        body = once(body, old, '                &sandbox_controller_core::NetworkPolicy::DenyAll,\n')
    elif variant == 'file-policy-as-zero':
        body = once(body, '                self.file_policy,\n',
            '                sandbox_controller_core::FilePolicyRef { digest: sandbox_controller_core::PolicyDigest(0) },\n')
    return text[:start] + body + text[stop:]


def stage(source):
    path = source / SOURCE
    text = path.read_text()
    start, stop = region(text)
    body = text[start:stop]
    body = once(body, 'serde_json::to_vec(&(', 'encode_continue_work_binding(&(')
    body = once(body, 'Sha256::digest(encode_continue_work_binding(&(',
                'digest_continue_work_binding(&encode_continue_work_binding(&(')
    body = once(body, '            ))?)\n            .into(),\n', '            ))?),\n')
    # Keep the original tuple, question-mark, result and call order verbatim.
    text = text[:start] + body + text[stop:]
    text += '''
fn encode_continue_work_binding(value: &(
    OriginalContinueIdentity,
    &FullCaptureWorkRequest,
    &SourceObservationRequirement,
    CaptureWorkEndReceipt,
    &sandbox_controller_core::NetworkPolicy,
    sandbox_controller_core::FilePolicyRef,
)) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(value)
}
fn digest_continue_work_binding(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}
'''
    path.write_text(text)
    return SOURCE
