//! Confirmation is a durable center-to-host acknowledgement, NOT success from
//! caller input. Match the original saved result before discarding older renews.
use super::{super::transaction::WorldTransaction, codec};
use crate::{ControllerJournal, CrashInjector, CrashPoint, JournalError};
use sandproof_protocol::internal::lifecycle::{Intent, LifecycleFact, LifecycleResponse};

impl ControllerJournal {
    /// Keep the create mapping and intent frontier; a delayed lower version
    /// remains rejected by original inbox admission after its result is removed.
    /// Higher pending requests survive a late acknowledgement. Native details
    /// stay pinned until the separate two-end WINDOW retirement protocol.
    /// # Errors
    /// Wrong full identity/result, non-final facts and uncertain commits fail.
    pub fn confirm_internal_result(
        &mut self,
        result: &LifecycleResponse,
        crashes: &mut impl CrashInjector,
    ) -> Result<LifecycleResponse, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        result
            .identity
            .validate_node(&inbox.config.node)
            .map_err(|_| JournalError::Conflict)?;
        if matches!(
            result.fact,
            LifecycleFact::Recorded | LifecycleFact::Stopped
        ) || !inbox.entries.iter().any(|entry| entry.response == *result)
        {
            return Err(JournalError::Conflict);
        }
        let entry = inbox
            .entries
            .iter_mut()
            .find(|entry| entry.response == *result)
            .ok_or(JournalError::Conflict)?;
        entry.confirmed = Some(result.clone());
        inbox.entries.retain(|entry| {
            entry.request.identity.sandbox_id != result.identity.sandbox_id
                || entry.request.intent == Intent::Create
                || entry.request.identity.intent_version.get()
                    >= result.identity.intent_version.get()
        });
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(result.clone())
    }
}
