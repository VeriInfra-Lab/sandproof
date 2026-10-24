//! Apply an already recorded absolute deadline in the original transaction.
use super::{ControllerJournal, LifecycleWorkState, authenticated, codec};
use crate::controller::{admission, transaction::WorldTransaction};
use crate::{AdmissionError, CrashInjector, CrashPoint, JournalError};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleOperation,
};

#[cfg(test)]
#[path = "renewal_tests.rs"]
mod tests;

impl ControllerJournal {
    /// Returns the saved result and its actual current-order flags. A replay
    /// never recomputes a relative duration and is not fresh liveness evidence.
    /// # Errors
    /// Wrong input/incarnation, expired native lifetime and stale unapplied work
    /// refuse; no mutation can bypass original owner and maximum-total checks.
    pub fn apply_lifecycle_renew(
        &mut self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<(LifecycleWorkState, i64), AdmissionError> {
        let LifecycleOperation::Renew {
            incarnation,
            end_at_ms,
            extend_only,
        } = intent.request.operation
        else {
            return Err(JournalError::Conflict.into());
        };
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, intent, context)?;
        let (s, e) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
        let state = inbox.state(s, e);
        if let Some(end) = inbox.sessions[s].entries[e].applied_deadline {
            return Ok((state, end));
        }
        if !state.current || state.terminal || inbox.sessions[s].incarnation != Some(incarnation) {
            return Err(JournalError::Conflict.into());
        }
        let local = state.local.ok_or(AdmissionError::NotRunning)?;
        let lifetime = admission::lifetime_for_update(
            &tx,
            &self.namespace,
            &sandbox_controller_core::Actor::Tenant(local.owner),
            local.sandbox,
            now_ms,
        )?;
        let end = if extend_only {
            lifetime.deadline_ms.max(end_at_ms)
        } else {
            end_at_ms
        };
        admission::write_lifetime_deadline(&tx, &self.namespace, local.sandbox, lifetime, end)?;
        inbox.sessions[s].entries[e].previous_effective_end_at_ms = Some(lifetime.deadline_ms);
        inbox.sessions[s].entries[e].applied_deadline = Some(end);
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok((state, end))
    }
}
