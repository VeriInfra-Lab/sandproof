//! One original recorded Unload, shared owner lock, and lock-free host I/O.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, JournalError, NeverCrash, core,
    map_journal,
};
use crate::capture::FullCaptureResult;
use sandproof_protocol::{
    WorkAction, WorkReceipt,
    internal::NodeIdentity,
    lifecycle::{AcceptedLifecycleIntent, AuthenticatedCenterContext},
};

impl<H: ControllerHost> ControllerApplication<H> {
    /// Advance only this already recorded canonical Unload.
    /// # Errors
    /// Unknown work stays in the original SQL/outbox and is never reissued.
    pub fn advance_recorded_lifecycle_unload(
        &self,
        intent: &AcceptedLifecycleIntent,
        node: &NodeIdentity,
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
        let record = self
            .prepare_lifecycle_source_work(intent, &context)
            .map_err(map_journal)?;
        let record = if record.observation.is_none() {
            self.observe_lifecycle_source_work(&record.work, &context)
                .map_err(map_journal)?
        } else {
            record
        };
        if record.full_capture().is_none() {
            let host = {
                let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
                std::sync::Arc::clone(&inner.host)
            };
            let full = host.propose_lifecycle_full(
                &record.work.pause_request().map_err(map_journal)?,
                record.work.requirement(),
            )?;
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let now = self.clock.now_ms()?;
            inner
                .journal
                .admit_lifecycle_full_capture::<H::Evidence>(
                    &record.work,
                    &context,
                    &full,
                    now,
                    &mut NeverCrash,
                )
                .map_err(map_journal)?;
        }
        let sandbox = self
            .lifecycle_work_state(intent, &context)
            .map_err(map_journal)?
            .local
            .ok_or(ApplicationError::OutcomeUnknown)?
            .sandbox;
        self.advance_current_workflow(sandbox)
    }

    pub(super) fn execute_capture_action(
        &self,
        host: &H,
        action: &WorkAction<core::EffectId>,
    ) -> Result<Result<WorkReceipt<H::Evidence>, crate::WorkerTransportError>, ApplicationError>
    {
        let sandbox = action.command.command().sandbox();
        let (record, expected) = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            (
                inner
                    .journal
                    .lifecycle_capture_record(sandbox)
                    .map_err(map_journal)?,
                inner
                    .journal
                    .sealed_capture_expectation(sandbox)
                    .map_err(map_journal)?,
            )
        };
        let Some(record) = record else {
            return Ok(host.execute(action));
        };
        let Some(full) = record.full_capture() else {
            return Ok(host.execute(action));
        };
        if matches!(action.command.command(), core::Command::SealSnapshot { .. }) {
            let worker =
                host.prepare_lifecycle_capture_worker(action, full, record.work.requirement())?;
            let binding = worker.binding();
            let permit = {
                let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
                let context = inner
                    .journal
                    .recorded_lifecycle_context(record.work.request(), &full.pause.node)
                    .map_err(map_journal)?;
                let now = self.clock.now_ms()?;
                inner
                    .journal
                    .authorize_lifecycle_full_capture(
                        &record.work,
                        &context,
                        action,
                        binding,
                        now,
                        &mut NeverCrash,
                    )
                    .map_err(map_journal)?
            };
            return Ok(match worker.release(permit) {
                FullCaptureResult::Captured(receipt) => Ok(receipt),
                FullCaptureResult::UnknownCapture => Err(crate::WorkerTransportError::ReplyLost),
            });
        }
        if matches!(
            action.command.command(),
            core::Command::FreezeVm { .. }
                | core::Command::CommitSnapshot { .. }
                | core::Command::StopVm { .. }
        ) {
            return Ok(host.execute_lifecycle_capture_action(
                action,
                full,
                record.work.requirement(),
                record.capture_worker(),
                expected.as_ref(),
            ));
        }
        Ok(host.execute(action))
    }
    pub(super) fn observe_capture_action(
        &self,
        host: &H,
        action: &WorkAction<core::EffectId>,
    ) -> Result<Option<WorkReceipt<H::Evidence>>, ApplicationError> {
        let sandbox = action.command.command().sandbox();
        let (record, expected) = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            (
                inner
                    .journal
                    .lifecycle_capture_record(sandbox)
                    .map_err(map_journal)?,
                inner
                    .journal
                    .sealed_capture_expectation(sandbox)
                    .map_err(map_journal)?,
            )
        };
        if let Some(record) = &record
            && let Some(full) = record.full_capture()
            && matches!(
                action.command.command(),
                core::Command::FreezeVm { .. }
                    | core::Command::SealSnapshot { .. }
                    | core::Command::CommitSnapshot { .. }
                    | core::Command::StopVm { .. }
            )
        {
            return Ok(host.observe_lifecycle_capture_action(
                action,
                full,
                record.work.requirement(),
                record.capture_worker(),
                expected.as_ref(),
            ));
        }
        Ok(host.observe(action))
    }
}

impl<H: ControllerHost> ControllerApplication<H> {
    /// Original authorized read; it cannot advance, rebuild or fabricate capture.
    /// # Errors
    /// Unknown clock/storage/current identity leaves the caller without success.
    pub fn observe_lifecycle_unloaded(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
    ) -> Result<sandproof_protocol::lifecycle::UnloadObservation, JournalError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let now = self
            .clock
            .now_ms()
            .map_err(|_| JournalError::UnresolvedAction)?;
        Ok(sandproof_protocol::lifecycle::UnloadObservation {
            request: intent.clone(),
            unloaded: inner.journal.lifecycle_unloaded(intent, context, now)?,
        })
    }
}

impl<H: ControllerHost> ControllerApplication<H> {
    pub(super) fn prepare_unload_drain(
        &self,
        sandbox: core::SandboxId,
        host: &H,
    ) -> Result<(), ApplicationError> {
        let context = {
            let mut inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            let world = inner.journal.world().map_err(map_journal)?;
            if world
                .sandbox_record(sandbox)
                .is_none_or(|r| r.phase != core::Phase::Pausing)
            {
                return Ok(());
            }
            inner.revoke_data(sandbox);
            if !super::execution::local_routes_dropped(&inner, sandbox) {
                return Err(ApplicationError::Busy);
            }
            inner
                .journal
                .host_slot_context(sandbox)
                .map_err(map_journal)?
                .ok_or(ApplicationError::OutcomeUnknown)?
        };
        if !host.data_gate_drained(&context)? {
            return Err(ApplicationError::Busy);
        }
        Ok(())
    }
}

impl<H: ControllerHost> ControllerApplication<H> {
    /// # Errors
    /// Original protected node/authority must remain readable.
    pub fn lifecycle_pending_unloads(
        &self,
        node: &NodeIdentity,
    ) -> Result<Vec<AcceptedLifecycleIntent>, JournalError> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| JournalError::ControllerBusy)?;
        let mut pending = inner.journal.lifecycle_pending_unloads(node)?;
        pending.retain(|i| {
            !inner
                .active_sessions
                .contains(i.request.binding.session.as_protocol_id().as_str())
        });
        Ok(pending)
    }
}
