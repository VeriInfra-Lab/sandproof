//! A definite capacity veto is saved in the original Create row before it can
//! be observed. Busy, I/O failure and unknown commit cannot construct this fact.
use super::{ControllerJournal, LifecycleWorkState, authenticated, codec};
use crate::controller::transaction::WorldTransaction;
use crate::{CrashInjector, CrashPoint, JournalError};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CreateRefusalReason, LifecycleOperation,
};

#[cfg(test)]
#[path = "refusal_tests.rs"]
mod tests;

impl ControllerJournal {
    // Only the actual low-physical-disk branch or a definite rolled-back native
    // QuotaExceeded calls this method. No HTTP caller supplies a reason flag.
    pub(crate) fn refuse_lifecycle_create_capacity(
        &mut self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        crashes: &mut impl CrashInjector,
    ) -> Result<LifecycleWorkState, JournalError> {
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        codec::validate(&tx, &tx.world())?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, intent, context)?;
        let (s, e) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
        if !matches!(intent.request.operation, LifecycleOperation::Create { .. }) {
            return Err(JournalError::UnsupportedInput);
        }
        let state = inbox.state(s, e);
        // Destroy/admission may have won while physical sampling was outside
        // the lock. They keep their original responsibility, not a refusal.
        if !state.current || state.terminal || state.local.is_some() || state.refusal.is_some() {
            return Ok(state);
        }
        if inbox.sessions[s]
            .entries
            .iter()
            .any(|entry| entry.local_operation.is_some() || entry.applied_deadline.is_some())
            || super::source_work::retains_session(&tx, &intent.request.binding.session)?
        {
            return Err(JournalError::UnresolvedAction);
        }
        // The original native create/outbox and this local mapping commit
        // together. No mapping or local operation exists in this checked row;
        // later native admission checks this same durable refusal again.
        inbox.sessions[s].refusal = Some(CreateRefusalReason::Capacity);
        codec::save(&tx, &inbox)?;
        let state = inbox.state(s, e);
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(state)
    }
}

pub(super) fn validate(
    connection: &rusqlite::Connection,
    inbox: &super::model::Inbox,
) -> Result<(), JournalError> {
    for session in &inbox.sessions {
        if session.refusal.is_some()
            && super::source_work::retains_session(connection, &session.id)?
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}
