//! Original Resume admission and action routing; slow Host work stays outside
//! the shared application lock and original `SQLite` decision transaction.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, NeverCrash, core, map_journal,
};
use crate::capture::{RestoreActivationPermit, RestoreWorkRequest};
use sandproof_protocol::{WorkAction, internal::NodeIdentity, lifecycle::AcceptedLifecycleIntent};

type RestoreExecution = (RestoreWorkRequest, Option<RestoreActivationPermit>);

impl<H: ControllerHost> ControllerApplication<H> {
    /// Current recorded Resume work not already owned by another local driver.
    /// # Errors
    /// Busy application or unreadable original authority refuses the scan.
    pub fn lifecycle_pending_resumes(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<AcceptedLifecycleIntent>, crate::JournalError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| crate::JournalError::ControllerBusy)?;
        let mut pending = inner.journal.lifecycle_pending_resumes(node)?;
        pending.retain(|intent| {
            !inner
                .active_sessions
                .contains(intent.request.binding.session.as_protocol_id().as_str())
        });
        Ok(pending)
    }

    /// Advance a received Resume using the same journal, resource scheduler,
    /// credentials and outbox as initial execution. Replay never re-loads.
    /// # Errors
    /// Unknown or mismatched work retains its original candidate and source pin.
    pub fn advance_recorded_lifecycle_resume(
        &self,
        intent: &AcceptedLifecycleIntent,
        node: &NodeIdentity,
        compute: crate::templates::ComputeLimits,
    ) -> Result<bool, ApplicationError> {
        let context = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            inner
                .journal
                .recorded_lifecycle_context(intent, node)
                .map_err(map_journal)?
        };
        let Some(_session) = self.acquire_lifecycle_session(intent, &context)? else {
            return Ok(false);
        };
        let sandbox = {
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let now_ms = self.clock.now_ms()?;
            let state = inner
                .journal
                .admit_lifecycle_resume(
                    intent,
                    &context,
                    now_ms,
                    self.config.limits,
                    compute,
                    &mut NeverCrash,
                )
                .map_err(map_journal)?;
            let local = state.local.ok_or(ApplicationError::OutcomeUnknown)?;
            inner.revoke_data(local.sandbox);
            local.sandbox
        };
        self.advance_current_workflow(sandbox)
    }

    pub(super) fn restore_execution_context(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<Option<RestoreExecution>, ApplicationError> {
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let Some(work) = inner
            .journal
            .lifecycle_restore_action_work(action)
            .map_err(map_journal)?
        else {
            return Ok(None);
        };
        let context = inner
            .journal
            .recorded_lifecycle_context(work.intent(), &work.intent().request.binding.node)
            .map_err(map_journal)?;
        let state = inner
            .journal
            .lifecycle_work_state(work.intent(), &context)
            .map_err(map_journal)?;
        if !state.current || state.terminal {
            return Err(ApplicationError::OutcomeUnknown);
        }
        let world = inner.journal.world().map_err(map_journal)?;
        let row = world
            .sandbox_record(action.command.command().sandbox())
            .ok_or(ApplicationError::OutcomeUnknown)?;
        if row.terminal
            || row.generation != work.generation()
            || row
                .current_operation
                .is_none_or(|op| op.id != work.operation())
        {
            return Err(ApplicationError::OutcomeUnknown);
        }
        let activation = if matches!(action.command.command(), core::Command::ActivateVm { .. }) {
            Some(
                inner
                    .journal
                    .lifecycle_restore_activation_permit(&work, action, &context)
                    .map_err(map_journal)?,
            )
        } else {
            None
        };
        Ok(Some((work, activation)))
    }
}
