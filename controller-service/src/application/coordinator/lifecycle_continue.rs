//! Same original application owner; short decisions and lock-free physical calls.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, NeverCrash, core, map_journal,
};
use crate::capture::{OriginalContinuePermit, OriginalContinueWorkRequest};
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, AuthenticatedCenterContext};
use sandproof_protocol::{WorkAction, WorkOutcome, WorkReceipt};

type ContinueExecution = (OriginalContinueWorkRequest, OriginalContinuePermit);
impl<H: ControllerHost> ControllerApplication<H> {
    pub(super) fn advance_closed_unload(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<Option<bool>, ApplicationError> {
        let origin = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            inner
                .journal
                .lifecycle_unload_before_admission_origin(intent, context)
                .map_err(map_journal)?
        };
        let Some(origin) = origin else {
            return Ok(None);
        };
        let ready = self
            .observe_lifecycle_execution(&origin, context)
            .map_err(map_journal)?;
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        Ok(Some(
            inner
                .journal
                .publish_pre_admission_original_continued(
                    &ready,
                    context,
                    self.clock.now_ms()?,
                    &mut NeverCrash,
                )
                .map_err(map_journal)?
                .is_some(),
        ))
    }

    pub(super) fn close_failed_unadmitted_unload(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        error: ApplicationError,
    ) -> Result<bool, ApplicationError> {
        if !matches!(
            error,
            ApplicationError::Invalid | ApplicationError::Failed | ApplicationError::QuotaExceeded
        ) {
            return Err(error);
        }
        {
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            inner
                .journal
                .close_lifecycle_unload_before_admission(
                    intent,
                    context,
                    self.clock.now_ms()?,
                    &mut NeverCrash,
                )
                .map_err(map_journal)?;
        }
        Ok(self
            .advance_closed_unload(intent, context)?
            .unwrap_or(false))
    }
    pub(super) fn original_continue_execution_context(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<Option<ContinueExecution>, ApplicationError> {
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let Some(work) = inner
            .journal
            .lifecycle_original_continue_action_work(action)
            .map_err(map_journal)?
        else {
            return Ok(None);
        };
        let record = inner
            .journal
            .lifecycle_capture_action_record(action)
            .map_err(map_journal)?
            .ok_or(ApplicationError::OutcomeUnknown)?;
        let context = inner
            .journal
            .recorded_lifecycle_context(record.work.request(), &work.source().node)
            .map_err(map_journal)?;
        let permit = inner
            .journal
            .lifecycle_original_continue_permit(&work, action, &context, self.clock.now_ms()?)
            .map_err(map_journal)?;
        Ok(Some((work, permit)))
    }

    pub(super) fn continue_original_activation(
        &self,
        host: &H,
        action: &WorkAction<core::EffectId>,
    ) -> Result<Option<WorkReceipt<H::Evidence>>, ApplicationError> {
        {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let world = inner.journal.world().map_err(map_journal)?;
            if !core::capture::continuation::continuation_activation_pending(
                &world,
                action.command.command(),
            ) {
                return Ok(None);
            }
        }
        let Some((work, permit)) = self.original_continue_execution_context(action)? else {
            return Ok(None);
        };
        match host.execute_lifecycle_original_continue(action, &work, &permit) {
            Ok(receipt) if matches!(receipt.outcome, WorkOutcome::ObservedSuccess(_)) => {
                Ok(Some(receipt))
            }
            Ok(_) | Err(_) => Ok(None),
        }
    }

    // Only called by the owner of ActiveSandbox after a failed/blocked turn.
    // Native Freeze/Commit uncertainty remains a barrier, never inferred away.
    pub(super) fn prepare_original_continue_if_ended(
        &self,
        sandbox: core::SandboxId,
        host: &H,
    ) -> Result<bool, ApplicationError> {
        let (record, context) = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let Some(record) = inner
                .journal
                .lifecycle_capture_record(sandbox)
                .map_err(map_journal)?
            else {
                return Ok(false);
            };
            let world = inner.journal.world().map_err(map_journal)?;
            let Some(row) = world.sandbox_record(sandbox) else {
                return Ok(false);
            };
            if row.terminal
                || row.phase != core::Phase::Pausing
                || row
                    .current_operation
                    .is_none_or(|op| Some(op.id) != record.capture_operation())
            {
                return Ok(false);
            }
            if record.capture_action().is_none()
                && row
                    .current_operation
                    .is_some_and(|op| op.status == core::OperationStatus::Pending)
            {
                return Ok(false);
            }
            let key = core::OperationKey {
                sandbox,
                operation: record
                    .capture_operation()
                    .ok_or(ApplicationError::OutcomeUnknown)?,
            };
            let capture = world
                .operation_plan(key)
                .and_then(|p| p.capture)
                .ok_or(ApplicationError::OutcomeUnknown)?;
            if capture.original_continuation().is_some()
                || capture.commit() != core::capture::CaptureCommit::NotRegistered
                || record.captured_manifest().is_some()
                    && row.current_operation.is_none_or(|op| {
                        !matches!(op.status, core::OperationStatus::Compensating(_))
                    })
            {
                return Ok(false);
            }
            let context = inner
                .journal
                .recorded_lifecycle_context(record.work.request(), &record.work.requirement().node)
                .map_err(map_journal)?;
            (record, context)
        };
        let Some(full) = record.full_capture() else {
            return Ok(false);
        };
        let end = if let Some(worker) = record.capture_worker() {
            match record.capture_api_end().cloned().or_else(|| {
                host.observe_lifecycle_capture_api_end(
                    record.capture_action()?,
                    full,
                    record.work.requirement(),
                    worker,
                )
            }) {
                Some(end) => Some(end),
                None => return Ok(false),
            }
        } else {
            None
        };
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        match inner.journal.prepare_lifecycle_original_continue(
            &record.work,
            &context,
            end,
            self.clock.now_ms()?,
            &mut NeverCrash,
        ) {
            Ok(_) => Ok(true),
            Err(
                crate::JournalError::ControllerRejected(core::ControllerReject::Capture(_))
                | crate::JournalError::Conflict
                | crate::JournalError::UnresolvedAction,
            ) => Ok(false),
            Err(error) => Err(map_journal(error)),
        }
    }

    pub(super) fn publish_original_continue_if_ready(
        &self,
        sandbox: core::SandboxId,
        host: &H,
    ) -> Result<Option<bool>, ApplicationError> {
        let candidate = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            inner
                .journal
                .lifecycle_original_continue_activation(sandbox)
                .map_err(map_journal)?
        };
        let Some((action, work)) = candidate else {
            return Ok(None);
        };
        let Some(receipt) = host.observe_lifecycle_original_continue(&action, &work) else {
            return Ok(Some(false));
        };
        if !matches!(&receipt.outcome, WorkOutcome::ObservedSuccess(_)) {
            return Ok(Some(false));
        }
        let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let record = inner
            .journal
            .lifecycle_capture_action_record(&action)
            .map_err(map_journal)?
            .ok_or(ApplicationError::OutcomeUnknown)?;
        let context = inner
            .journal
            .recorded_lifecycle_context(record.work.request(), &work.source().node)
            .map_err(map_journal)?;
        inner
            .journal
            .publish_lifecycle_original_continued(
                &work,
                &action,
                &receipt,
                &context,
                self.clock.now_ms()?,
                &mut NeverCrash,
            )
            .map_err(map_journal)?;
        Ok(Some(true))
    }
}
