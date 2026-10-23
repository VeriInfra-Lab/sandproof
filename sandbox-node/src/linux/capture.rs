//! Original claimed Pause and its readonly recovery. No capture/stop authority.
use super::{
    HostObservation, LinuxWorker, NodeResult, ObservedResult, Runtime, RuntimeError, WorkAction,
    core,
};
use crate::ProtectedHostEpochSource;
use sandbox_runtime::{OriginalPauseObservation, PausedOriginalVm, RuntimeProcessIdentity};
use sandproof_controller_service::capture::{PauseWorkRequest, SourceObservationRequirement};

impl LinuxWorker {
    pub(crate) fn pause_original(
        &self,
        protected: &ProtectedHostEpochSource,
        action: &WorkAction<core::EffectId>,
        request: &PauseWorkRequest,
        original_source: &SourceObservationRequirement,
    ) -> NodeResult<HostObservation> {
        self.with_original_pause(
            protected,
            action,
            request,
            original_source,
            |runtime, id, source| runtime.pause_original_vm(id, source),
        )
    }

    pub(crate) fn observe_pause_original(
        &self,
        protected: &ProtectedHostEpochSource,
        action: &WorkAction<core::EffectId>,
        request: &PauseWorkRequest,
        original_source: &SourceObservationRequirement,
    ) -> NodeResult<HostObservation> {
        self.with_original_pause(
            protected,
            action,
            request,
            original_source,
            |runtime, id, source| match runtime.observe_original_pause_state(id, source)? {
                OriginalPauseObservation::Paused(paused) => Ok(paused),
                OriginalPauseObservation::Running | OriginalPauseObservation::Exited => Err(
                    RuntimeError::IdentityMismatch("original Pause outcome not confirmed"),
                ),
            },
        )
    }

    fn with_original_pause(
        &self,
        protected: &ProtectedHostEpochSource,
        action: &WorkAction<core::EffectId>,
        request: &PauseWorkRequest,
        original_source: &SourceObservationRequirement,
        operation: impl FnOnce(
            &Runtime,
            &str,
            &RuntimeProcessIdentity,
        ) -> Result<PausedOriginalVm, RuntimeError>,
    ) -> NodeResult<HostObservation> {
        self.validate_action(action)?;
        require_source_binding(request, original_source)?;
        let core::Command::FreezeVm {
            sandbox,
            epoch,
            generation,
            vm,
        } = action.command.command()
        else {
            return Err("original Pause requires its FreezeVm action".into());
        };
        if action.identity.resource != request.source_resource {
            return Err("Pause action resource differs from original source work".into());
        }
        let context = self.original_source_context(&request.source_resource)?;
        if context.binding.sandbox != *sandbox
            || context.binding.generation != *generation
            || context.binding.vm != *vm
        {
            return Err("Pause action differs from original occupied source".into());
        }
        let slot = self.runtime_host_slot(context.position, *vm)?;
        let _guard = slot.helper_gate.try_acquire(Some(&slot))?;
        self.require_fence(*sandbox, *epoch, false)?;
        if self.original_source_context(&request.source_resource)? != context {
            return Err("Pause source context changed before execution".into());
        }
        let before = crate::source_observation::observe(self, Some(protected), original_source)?;
        let runtime = self.runtime(*sandbox, *vm)?;
        let expected = crate::source_observation::expected_source(original_source)?;
        let paused = operation(&runtime, request.source_resource.id.as_str(), &expected)?;
        if paused.resource_id() != request.source_resource.id.as_str()
            || paused.source() != &expected
        {
            return Err("Pause observation differs from original source".into());
        }
        let after = crate::source_observation::observe(self, Some(protected), original_source)?;
        if before != after || self.original_source_context(&request.source_resource)? != context {
            return Err("original Pause configuration or context changed".into());
        }
        runtime.confirm_original_paused(&paused)?;
        self.require_fence(*sandbox, *epoch, false)?;
        Ok(HostObservation {
            resource: request.source_resource.clone(),
            command: action.command.command().clone(),
            result: ObservedResult::Frozen {
                pid: paused.source().pid,
                start_ticks: paused.source().start_time_ticks,
                boot_id: paused.source().boot_id.clone(),
            },
        })
    }
}

// The service supplies this requirement from the SAME saved source-work row,
// not by reconstructing a registration from the incoming Pause DTO. Equality
// does not attest the external operation -> native action mapping or current
// lifetime: those checks remain in the original durable claim and completion.
pub(super) fn require_source_binding(
    request: &PauseWorkRequest,
    original: &SourceObservationRequirement,
) -> NodeResult<()> {
    if !request.valid()
        || !original.limits.valid()
        || request.source_work_binding != original.work_binding
        || request.node != original.node
        || request.source_resource != original.resource
        || request.source != original.source
    {
        return Err("Pause differs from the independent original source work".into());
    }
    crate::source_observation::expected_source(original)?;
    Ok(())
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
