//! Cleanup of original running components after durable material adoption.
//! This never removes the separate immutable material store or its references.
use super::capture::require_source_binding;
use super::capture_profile::ProtectedCaptureProfile;
use super::{
    HostObservation, LinuxWorker, NodeResult, ObservedResult, WorkAction, core, require_absent,
};
use crate::ProtectedHostEpochSource;
use sandproof_controller_service::capture::{
    CommittedCaptureExpectation, SourceObservationRequirement,
};
use std::path::Path;

fn scope(
    action: &WorkAction<core::EffectId>,
    committed: &CommittedCaptureExpectation,
) -> NodeResult<(core::SandboxId, core::Epoch, core::Generation, core::VmId)> {
    let core::Command::SealSnapshot {
        sandbox,
        epoch,
        generation,
        vm,
        ..
    } = committed.seal_action().command.command()
    else {
        return Err("Unload cleanup has no original sealed work".into());
    };
    let matches = match action.command.command() {
        core::Command::CleanupVm {
            sandbox: actual,
            generation: actual_generation,
            vm: actual_vm,
        } => actual == sandbox && actual_generation == generation && actual_vm == vm,
        core::Command::CleanupSlot {
            sandbox: actual,
            generation: actual_generation,
            ..
        } => actual == sandbox && actual_generation == generation,
        _ => false,
    };
    if !matches {
        return Err("Unload cleanup differs from original captured source".into());
    }
    Ok((*sandbox, *epoch, *generation, *vm))
}

impl LinuxWorker {
    pub(crate) fn cleanup_capture_source(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        action: &WorkAction<core::EffectId>,
        source: &SourceObservationRequirement,
        committed: &CommittedCaptureExpectation,
        observe_only: bool,
    ) -> NodeResult<HostObservation> {
        self.validate_action(action)?;
        let full = committed.work();
        require_source_binding(&full.pause, source)?;
        let (sandbox, epoch, generation, vm) = scope(action, committed)?;
        if !full.valid() {
            return Err("invalid original Full cleanup work".into());
        }
        let context = self.original_source_context(&source.resource)?;
        if context.binding.sandbox != sandbox
            || context.binding.generation != generation
            || context.binding.vm != vm
        {
            return Err("Unload cleanup original position changed".into());
        }
        let vm_cleanup = match action.command.command() {
            core::Command::CleanupVm { .. } if action.identity.resource == source.resource => true,
            core::Command::CleanupSlot { slot, .. } if *slot == context.binding.network => false,
            _ => return Err("Unload cleanup resource binding differs".into()),
        };
        let slot = self.runtime_host_slot(context.position, vm)?;
        let gate = slot.helper_gate.try_acquire(Some(&slot))?;
        self.require_fence(sandbox, epoch, false)?;
        provider.observe_original(
            &source.node,
            &source.host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        // The original journal dispatch separately checks Registered, completed
        // Full and this exact current native Stop/Cleanup action authority.
        super::full_capture::observe_committed_capture(profile, committed)?;
        let runtime = self.runtime(sandbox, vm)?;
        let original = crate::source_observation::expected_source(source)?;
        let name = self.vm_name(vm);
        runtime.observe_original_stopped(&name, &original)?;
        if vm_cleanup && !observe_only {
            runtime.cleanup_under_gate(&name, &gate)?;
        }
        runtime.observe_original_stopped(&name, &original)?;
        runtime.require_cgroup_absent(&name)?;
        let jail = self
            .config
            .chroot_base
            .join(
                self.config
                    .firecracker
                    .file_name()
                    .ok_or("firecracker name")?,
            )
            .join(&name);
        require_absent(&jail)?;
        require_absent(Path::new(&format!("/run/netns/{name}")))?;
        if !runtime.uplink_resources_absent(&name)? {
            return Err("Unload cleanup uplink remains".into());
        }
        self.require_fence(sandbox, epoch, false)?;
        if self.original_source_context(&source.resource)? != context {
            return Err("Unload cleanup original context changed after observation".into());
        }
        super::full_capture::observe_committed_capture(profile, committed)?;
        provider.observe_original(
            &source.node,
            &source.host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        Ok(HostObservation {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
            result: if vm_cleanup {
                ObservedResult::VmCleaned
            } else {
                ObservedResult::SlotClosed
            },
        })
    }
}
