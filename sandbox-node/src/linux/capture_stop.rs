//! Physical exit of the exact Unload source; SQL permission remains in V5.
use super::capture::require_source_binding;
use super::{HostObservation, LinuxWorker, NodeResult, ObservedResult, Runtime, WorkAction, core};
use crate::ProtectedHostEpochSource;
use sandbox_runtime::{Observation, RuntimeProcessIdentity, VmState};
use sandproof_controller_service::capture::{FullCaptureWorkRequest, SourceObservationRequirement};

impl LinuxWorker {
    pub(crate) fn stop_capture_source(
        &self,
        provider: &ProtectedHostEpochSource,
        action: &WorkAction<core::EffectId>,
        full: &FullCaptureWorkRequest,
        source: &SourceObservationRequirement,
        observe_only: bool,
    ) -> NodeResult<HostObservation> {
        self.validate_action(action)?;
        require_source_binding(&full.pause, source)?;
        if !full.valid() {
            return Err("invalid original Unload Full work".into());
        }
        let core::Command::StopVm {
            sandbox,
            epoch,
            generation,
            vm,
        } = action.command.command()
        else {
            return Err("Unload source exit requires the original StopVm".into());
        };
        if action.identity.resource != source.resource {
            return Err("Unload Stop resource differs from original source".into());
        }
        let context = self.original_source_context(&source.resource)?;
        if context.binding.sandbox != *sandbox
            || context.binding.generation != *generation
            || context.binding.vm != *vm
        {
            return Err("Unload Stop differs from original occupied incarnation".into());
        }
        let slot = self.runtime_host_slot(context.position, *vm)?;
        let _gate = slot.helper_gate.try_acquire(Some(&slot))?;
        self.require_fence(*sandbox, *epoch, false)?;
        provider.observe_original(
            &source.node,
            &source.host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        if self.original_source_context(&source.resource)? != context {
            return Err("Unload Stop context changed".into());
        }
        let runtime = self.runtime(*sandbox, *vm)?;
        let original = crate::source_observation::expected_source(source)?;
        let operation: fn(
            &Runtime,
            &str,
            &RuntimeProcessIdentity,
        ) -> Result<Observation, sandbox_runtime::RuntimeError> = if observe_only {
            Runtime::observe_original_stopped
        } else {
            Runtime::stop_original_vm
        };
        let observed = operation(&runtime, source.resource.id.as_str(), &original)?;
        if observed.state != VmState::Stopped
            || observed.pid != original.pid
            || observed.start_time_ticks != original.start_time_ticks
        {
            return Err("Unload source exit observation differs".into());
        }
        self.require_fence(*sandbox, *epoch, false)?;
        provider.observe_original(
            &source.node,
            &source.host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        if self.original_source_context(&source.resource)? != context {
            return Err("Unload source changed after exit observation".into());
        }
        Ok(HostObservation {
            resource: source.resource.clone(),
            command: action.command.command().clone(),
            result: ObservedResult::Stopped {
                pid: observed.pid,
                start_ticks: observed.start_time_ticks,
            },
        })
    }
}
