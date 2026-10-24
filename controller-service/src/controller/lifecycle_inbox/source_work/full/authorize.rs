use super::super::{ControllerJournal, SourceWork, authenticated};
use super::{CaptureWorkerBinding, JournalError, WorkAction, codec, core, matches_action, save};
use crate::{CrashInjector, CrashPoint, capture::FullCaptureLaunchPermit};
use rusqlite::OptionalExtension;
use sandproof_protocol::lifecycle::AuthenticatedCenterContext;

impl ControllerJournal {
    /// Save the exact prepared worker before returning its single release permit.
    /// # Errors
    /// Unknown commit returns no permit. A saved worker is never reauthorized.
    pub fn authorize_lifecycle_full_capture(
        &mut self,
        work: &SourceWork,
        context: &AuthenticatedCenterContext,
        action: &WorkAction<core::EffectId>,
        worker: CaptureWorkerBinding,
        now_ms: i64,
        crashes: &mut impl CrashInjector,
    ) -> Result<FullCaptureLaunchPermit, JournalError> {
        if !worker.valid() {
            return Err(JournalError::InvalidHostEvidence);
        }
        let tx = crate::controller::transaction::WorldTransaction::begin(
            &mut self.journal.connection,
            &self.committed_world,
        )?;
        let inbox = super::super::super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, work.request(), context)?;
        let (s, e) = inbox.find(work.request())?.ok_or(JournalError::Conflict)?;
        let state = inbox.state(s, e);
        if !state.current || state.terminal {
            return Err(JournalError::Conflict);
        }
        let world = tx.world();
        let lifetime = crate::controller::admission::read_lifetime(
            &tx,
            &self.namespace,
            work.anchor.local.sandbox,
            work.anchor.local.owner,
        )
        .map_err(|_| JournalError::CorruptStore)?
        .ok_or(JournalError::CorruptStore)?;
        if now_ms < 0 || now_ms >= lifetime.deadline_ms {
            return Err(JournalError::UnresolvedAction);
        }
        let mut sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let record = sources
            .records
            .iter_mut()
            .find(|r| r.work == *work)
            .ok_or(JournalError::Conflict)?;
        if !matches_action(record, action, &world) {
            return Err(JournalError::Conflict);
        }
        let capture = record.capture.as_mut().ok_or(JournalError::Conflict)?;
        if capture.action.as_ref() != Some(action)
            || capture.worker.is_some()
            || capture.completed.is_some()
            || world.effect_status(&action.command) != Some(core::EffectStatus::Dispatched)
        {
            return Err(JournalError::Conflict);
        }
        let stored: Option<Vec<u8>> = tx.query_row(
            "SELECT action_json FROM outbox WHERE sandbox_id=?1 AND logical_action_id=?2 AND attempt=?3 AND dispatch_state='dispatched'",
            rusqlite::params![action.identity.sandbox.as_str(),action.identity.logical_action.as_str(),i64::try_from(action.identity.attempt).map_err(|_|JournalError::Conflict)?], |r|r.get(0),
        ).optional()?;
        if stored.as_deref() != Some(serde_json::to_vec(action)?.as_slice()) {
            return Err(JournalError::Conflict);
        }
        let binding = capture.full.binding_digest()?;
        capture.worker = Some(worker.clone());
        save(&tx, &sources)?;
        crashes.hit(CrashPoint::BeforeCommit)?;
        tx.commit()?;
        crashes.hit(CrashPoint::AfterCommit)?;
        Ok(FullCaptureLaunchPermit::after_commit(
            action.clone(),
            binding,
            worker,
        ))
    }
}
