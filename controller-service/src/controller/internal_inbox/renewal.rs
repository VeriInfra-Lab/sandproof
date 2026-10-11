//! Apply a fixed authorized deadline and its fact in the original transaction.
use super::super::{admission, transaction::WorldTransaction};
use super::codec;
use crate::{AdmissionError, ControllerJournal, CrashInjector, CrashPoint, JournalError};
use sandbox_controller_core as core;
use sandproof_protocol::internal::lifecycle::{
    Intent, LifecycleFact, LifecycleRequest, LifecycleResponse,
};

impl ControllerJournal {
    /// Apply an already recorded current renewal, without reinterpreting its
    /// duration or allocating a new native operation. A cached historical fact
    /// replays even after expiry; an unapplied renewal never revives expiry.
    /// # Errors
    /// Original owner/state/clock/expiry checks, invalid exact bounds, stale or
    /// unrecorded identity, storage failure and uncertain commit stay explicit.
    pub fn apply_internal_renew(
        &mut self,
        request: &LifecycleRequest,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<LifecycleResponse, AdmissionError> {
        let Intent::Renew { end_at_ms } = request.intent else {
            return Err(JournalError::Conflict.into());
        };
        let tx = WorldTransaction::begin(&mut self.journal.connection, &self.committed_world)?;
        let mut inbox = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let index = inbox
            .entries
            .iter()
            .position(|entry| entry.request == *request)
            .ok_or(JournalError::Conflict)?;
        let previous = inbox.entries[index].response.clone();
        if matches!(previous.fact, LifecycleFact::Renewed { .. })
            || inbox.entries.iter().any(|entry| {
                entry.request.identity.sandbox_id == request.identity.sandbox_id
                    && entry.request.identity.intent_version.get()
                        > request.identity.intent_version.get()
            })
        {
            return Ok(previous);
        }
        let local = inbox
            .entries
            .iter()
            .find(|entry| entry.request.identity.sandbox_id == request.identity.sandbox_id)
            .and_then(|entry| entry.local)
            .ok_or(AdmissionError::NotRunning)?;
        let lifetime = admission::lifetime_for_update(
            &tx,
            &self.namespace,
            &core::Actor::Tenant(local.owner),
            local.sandbox,
            now_ms,
        )?;
        admission::write_lifetime_deadline(
            &tx,
            &self.namespace,
            local.sandbox,
            lifetime,
            i64::try_from(end_at_ms).map_err(|_| AdmissionError::TimeOverflow)?,
        )?;
        let response = LifecycleResponse {
            identity: request.identity.clone(),
            observation_version: inbox
                .observation
                .checked_next()
                .map_err(|_| JournalError::IdentifierExhausted)?,
            fact: LifecycleFact::Renewed { end_at_ms },
        };
        response
            .validate_for(request)
            .map_err(|_| JournalError::CorruptStore)?;
        inbox.observation = response.observation_version;
        inbox.entries[index].response = response.clone();
        codec::save(&tx, &inbox)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(response)
    }
}
