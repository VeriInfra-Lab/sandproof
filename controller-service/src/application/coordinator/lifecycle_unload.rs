//! One original recorded Unload, shared owner lock, and lock-free host I/O.
use super::{
    ApplicationError, ControllerApplication, ControllerHost, JournalError, NeverCrash, core,
    map_journal,
};
use crate::SourceWorkRecord;
use crate::capture::{CommittedCaptureExpectation, FullCaptureResult};

type CaptureActionContext = (SourceWorkRecord, Option<CommittedCaptureExpectation>);
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
        if let Some((work, permit)) = self.restore_execution_context(action)? {
            return Ok(host.execute_lifecycle_restore_action(action, &work, permit.as_ref()));
        }
        let Some((record, expected)) = self.capture_action_context(action)? else {
            return Ok(host.execute(action));
        };
        let full = record
            .full_capture()
            .ok_or(ApplicationError::OutcomeUnknown)?;
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
                | core::Command::CleanupVm { .. }
                | core::Command::CleanupSlot { .. }
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
        let restore = {
            let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
            inner
                .journal
                .lifecycle_restore_action_work(action)
                .map_err(map_journal)?
        };
        if let Some(work) = restore {
            return Ok(host.observe_lifecycle_restore_action(action, &work));
        }
        let Some((record, expected)) = self.capture_action_context(action)? else {
            return Ok(host.observe(action));
        };
        let full = record
            .full_capture()
            .ok_or(ApplicationError::OutcomeUnknown)?;
        Ok(host.observe_lifecycle_capture_action(
            action,
            full,
            record.work.requirement(),
            record.capture_worker(),
            expected.as_ref(),
        ))
    }

    // Read routing data under the original short lock, then perform host I/O
    // only after this guard is dropped. Historical capture never captures a
    // later terminal Stop/Cleanup: those retain the original terminal fence.
    fn capture_action_context(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<Option<CaptureActionContext>, ApplicationError> {
        let inner = self.inner.lock().map_err(|_| ApplicationError::Busy)?;
        let Some(record) = inner
            .journal
            .lifecycle_capture_action_record(action)
            .map_err(map_journal)?
        else {
            return Ok(None);
        };
        let world = inner.journal.world().map_err(map_journal)?;
        if !capture_action_matches(&world, &record, action) {
            return Ok(None);
        }
        let expected = if matches!(
            action.command.command(),
            core::Command::StopVm { .. }
                | core::Command::CleanupVm { .. }
                | core::Command::CleanupSlot { .. }
        ) {
            inner
                .journal
                .committed_capture_expectation(&record.work)
                .map_err(map_journal)?
        } else {
            inner
                .journal
                .sealed_capture_expectation(&record.work)
                .map_err(map_journal)?
        };
        Ok(Some((record, expected)))
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
        let outcome = inner
            .journal
            .lifecycle_unloaded(intent, context, now)?
            .map_or(
                sandproof_protocol::lifecycle::UnloadOutcome::Pending,
                |facts| sandproof_protocol::lifecycle::UnloadOutcome::Unloaded { facts },
            );
        Ok(sandproof_protocol::lifecycle::UnloadObservation {
            request: intent.clone(),
            outcome,
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

// CleanupVm deliberately has Maintenance origin in the original core (no
// epoch on that command). Bind it instead to the current nonterminal Paused
// operation and its original planned VM, never merely to a sandbox ID.
fn capture_action_matches(
    world: &core::World,
    record: &SourceWorkRecord,
    action: &WorkAction<core::EffectId>,
) -> bool {
    let Some(operation) = record.capture_operation() else {
        return false;
    };
    if record.full_capture().is_none() {
        return false;
    }
    let command = action.command.command();
    let sandbox = command.sandbox();
    let key = core::OperationKey { sandbox, operation };
    let Some(logical) = world.logical_effect(&action.command.logical) else {
        return false;
    };
    if logical.current_attempt != action.command.attempt {
        return false;
    }
    match *command {
        core::Command::FreezeVm { .. }
        | core::Command::SealSnapshot { .. }
        | core::Command::CommitSnapshot { .. }
        | core::Command::StopVm { .. } => logical.origin == core::EffectOrigin::Operation(key),
        core::Command::CleanupVm { generation, vm, .. } => {
            cleanup_matches(world, key, logical.origin, generation)
                && world
                    .operation_plan(key)
                    .is_some_and(|plan| plan.vm == Some(vm) && plan.stop_vms.contains(&vm))
        }
        core::Command::CleanupSlot {
            generation, slot, ..
        } => {
            cleanup_matches(world, key, logical.origin, generation)
                && world
                    .operation_plan(key)
                    .is_some_and(|plan| plan.close_slots.contains(&slot))
        }
        _ => false,
    }
}

fn cleanup_matches(
    world: &core::World,
    key: core::OperationKey,
    origin: core::EffectOrigin,
    generation: core::Generation,
) -> bool {
    origin == core::EffectOrigin::Maintenance(key.sandbox)
        && world.sandbox_record(key.sandbox).is_some_and(|row| {
            !row.terminal
                && row.phase == core::Phase::Paused
                && row
                    .current_operation
                    .is_some_and(|op| op.id == key.operation)
                && row.generation == generation
        })
        && world
            .operation_plan(key)
            .is_some_and(|plan| plan.key == key && plan.generation == generation)
}
