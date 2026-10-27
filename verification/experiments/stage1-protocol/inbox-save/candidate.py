"""Review-only narrow save refactor, applied solely inside disposable sources.

The generic body retains the original guards. Encoding, digest and SQL calls are
parameters with explicit mutable state, not assumed-correct business operations.
The standard provider/adoption and same-transaction semantics remain obligations.
"""
from pathlib import Path
import hashlib,importlib.util,subprocess
SOURCE='controller-service/src/controller/lifecycle_inbox/codec.rs'
MODEL='controller-service/src/controller/lifecycle_inbox/model.rs'

def candidate(project):
 path=project/'verification/experiments/stage1-protocol/inbox-validation/prepare.py'
 spec=importlib.util.spec_from_file_location('save_validation_candidate',path)
 previous=importlib.util.module_from_spec(spec);spec.loader.exec_module(previous)
 changes,metadata=previous.prepare(project,'candidate')
 p=project/MODEL;s=p.read_text();old='    fn validate_using<';assert s.count(old)==1
 s=s.replace(old,'    pub(super) fn validate_using<')
 assert s.count('struct StandardTerminalEncoding;')==1
 p.write_text(s.replace('struct StandardTerminalEncoding;','pub(super) struct StandardTerminalEncoding;'))
 p=project/SOURCE;s=p.read_text()
 signature='pub(super) fn save(connection: &Connection, inbox: &Inbox) -> Result<(), JournalError> {'
 assert s.count(signature)==1
 start=s.index(signature);end=s.index('\npub(in crate::controller) fn validate(',start)
 original=s[start:end]
 body=original[len(signature):]
 replacements=[('    inbox.validate()?;', '    inbox.validate_using::<R, T, N, U, B>()?;'),
  ('    let bytes = serde_json::to_vec(inbox)?;', '    let bytes = effects.encode(inbox)?;'),
  ('    if connection.execute(', '    let digest = effects.digest(&bytes);\n    if effects.update('),
  ('        rusqlite::params![bytes, Sha256::digest(&bytes).as_slice()],', '        &bytes,\n        &digest,')]
 for old,new in replacements:
  assert body.count(old)==1,(old,body.count(old));body=body.replace(old,new)
 declaration='''pub(super) trait InboxSaveEffects {
    fn encode(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError>;
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32];
    fn update(&mut self, statement: &str, bytes: &[u8], digest: &[u8]) -> Result<usize, JournalError>;
}

struct StandardInboxSave<'a> {
    connection: &'a Connection,
}

impl InboxSaveEffects for StandardInboxSave<'_> {
    fn encode(&mut self, inbox: &Inbox) -> Result<Vec<u8>, JournalError> {
        Ok(serde_json::to_vec(inbox)?)
    }
    fn digest(&mut self, bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }
    fn update(&mut self, statement: &str, bytes: &[u8], digest: &[u8]) -> Result<usize, JournalError> {
        Ok(self.connection.execute(statement, rusqlite::params![bytes, digest])?)
    }
}

pub(super) fn save(connection: &Connection, inbox: &Inbox) -> Result<(), JournalError> {
    let mut effects = StandardInboxSave { connection };
    save_using::<
        super::native_retirement::StandardRetirementEncoding,
        super::model::StandardTerminalEncoding,
        super::renewal_confirmation::StandardRenewalEncoding,
        super::unload_resolution::StandardUnloadEncoding,
        crate::capture::StandardGraphicBytes,
        _,
    >(inbox, &mut effects)
}

pub(super) fn save_using<
    R: super::native_retirement::RetirementEncoding,
    T: super::model::TerminalEncoding,
    N: super::renewal_confirmation::RenewalEncoding,
    U: super::unload_resolution::UnloadEncoding,
    B: crate::capture::GraphicBytes,
    E: InboxSaveEffects,
>(inbox: &Inbox, effects: &mut E) -> Result<(), JournalError> {'''
 p.write_text(s[:start]+declaration+body+s[end:])
 subprocess.run(['rustfmt','--edition','2024',MODEL,SOURCE],cwd=project,check=True)
 sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
 changes.update({n:sha(project/n)for n in [MODEL,SOURCE]})
 metadata.update(save_candidate=True,save_business_body_from_original=True,
                 opaque_business_functions=[],standard_save_provider_adopted=False,
                 transaction_correctness_proved=False)
 return changes,metadata
