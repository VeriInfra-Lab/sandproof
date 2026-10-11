//! An original application pre-admission capacity veto, not an I/O failure fact.
use super::{super::transaction::WorldTransaction, codec};
use crate::{ControllerJournal, CrashInjector, CrashPoint, JournalError};
use sandproof_protocol::internal::lifecycle::{
    Intent, LifecycleFact, LifecycleRequest, LifecycleResponse,
};

impl ControllerJournal {
    /// Called ONLY by the original application while retaining its host/writer
    /// mutex, after a definitive pre-admission capacity veto. No caller or remote
    /// status may invoke this transition. Atomic native attachment guarantees
    /// that a current original Create without local binding admitted no VM.
    pub(crate) fn reject_internal_capacity(
        &mut self,
        request: &LifecycleRequest,
        crashes: &mut impl CrashInjector,
    ) -> Result<LifecycleResponse, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        codec::validate(&tx, &tx.world())?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let index = inbox
            .entries
            .iter()
            .position(|entry| entry.request == *request)
            .ok_or(JournalError::Conflict)?;
        let entry = &inbox.entries[index];
        if request.intent != Intent::Create
            || entry.local.is_some()
            || inbox.entries.iter().any(|other| {
                other.request.identity.sandbox_id == request.identity.sandbox_id
                    && (other.local.is_some()
                        || other.request.identity.intent_version.get()
                            > request.identity.intent_version.get())
            })
        {
            return Err(JournalError::Conflict);
        }
        if entry.response.fact == LifecycleFact::NotAccepted {
            return Ok(entry.response.clone());
        }
        if entry.response.fact != LifecycleFact::Recorded {
            return Err(JournalError::Conflict);
        }
        let response = LifecycleResponse {
            identity: request.identity.clone(),
            observation_version: inbox
                .observation
                .checked_next()
                .map_err(|_| JournalError::IdentifierExhausted)?,
            fact: LifecycleFact::NotAccepted,
        };
        response
            .validate_for(request)
            .map_err(|_| JournalError::CorruptStore)?;
        inbox.observation = response.observation_version;
        inbox.entries[index].response = response.clone();
        inbox.entries[index].confirmed = None;
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(response)
    }
}
