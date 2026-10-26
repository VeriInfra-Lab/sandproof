//! Real restore actions consume the original journal claim, not a fabricated
//! Tokio stub or a manifest found on disk. No snapshot load is retried here.
use super::capture_profile::ProtectedCaptureProfile;
use super::{
    HostObservation, LinuxWorker, NodeResult, ObservedResult, SlotBinding, WorkAction, core,
};
use crate::ProtectedHostEpochSource;
use sandbox_runtime::{LoadedSnapshotObservation, LoadedSnapshotState};
use sandproof_controller_service::capture::{
    RestoreActivationPermit, RestoreCandidateExitObservation, RestoreWorkRequest,
    SourceProcessIdentity,
};

// Called only after checking the original private activation permission. It
// also serves recovery of that same permitted candidate; no startup or load is
// dispatched here, and an observation-only call never enters this function.
fn finish_permitted_activation(
    before: &LoadedSnapshotObservation,
    activate: impl FnOnce() -> NodeResult<LoadedSnapshotObservation>,
    initialize: impl FnOnce() -> NodeResult<()>,
    confirm: impl FnOnce() -> NodeResult<LoadedSnapshotObservation>,
) -> NodeResult<LoadedSnapshotObservation> {
    if before.state == LoadedSnapshotState::Exited {
        return Err("original restore candidate exited".into());
    }
    let activated = activate()?;
    if activated.process != before.process || activated.state != LoadedSnapshotState::Running {
        return Err("restore activation changed its original candidate".into());
    }
    initialize()?;
    let after = confirm()?;
    if after.process != before.process || after.state != LoadedSnapshotState::Running {
        return Err("restore authorization changed its original candidate".into());
    }
    Ok(after)
}

// The original core queues ApplyFilePolicy AFTER ActivateVm. Observe only that
// same running candidate; checking the disk neither activates nor pauses it.
fn observe_running_file_policy(
    before: &LoadedSnapshotObservation,
    disk: impl FnOnce() -> NodeResult<ObservedResult>,
    confirm: impl FnOnce() -> NodeResult<LoadedSnapshotObservation>,
) -> NodeResult<ObservedResult> {
    if before.state != LoadedSnapshotState::Running {
        return Err("restore file policy requires the activated candidate".into());
    }
    let result = disk()?;
    if confirm()? != *before {
        return Err("restore candidate changed while checking disk".into());
    }
    Ok(result)
}

// A readonly probe reports preparation/network facts, not activation authority.
// Once the same loaded candidate runs, those facts remain observable; executing
// load/network work still requires Paused. The caller has checked intent,
// manifest/work binding and original process before reaching this state check.
fn require_loaded_candidate(
    observed: LoadedSnapshotObservation,
    observe_only: bool,
) -> NodeResult<LoadedSnapshotObservation> {
    match (observed.state, observe_only) {
        (LoadedSnapshotState::Paused, _) | (LoadedSnapshotState::Running, true) => Ok(observed),
        _ => Err("restore candidate state does not allow this operation".into()),
    }
}

// Observe only through Runtime's original loaded-record/boot/pidfd checks.
// A physical exit does not establish the service's MayHaveExecuted marker.
fn observe_confirmed_restore_exit(
    mut observe: impl FnMut() -> NodeResult<LoadedSnapshotObservation>,
) -> NodeResult<Option<sandbox_runtime::RuntimeProcessIdentity>> {
    let before = observe()?;
    if before.state != LoadedSnapshotState::Exited {
        return Ok(None);
    }
    before.process.validate_fields()?;
    if observe()? != before {
        return Err("original restore exit changed during observation".into());
    }
    Ok(Some(before.process))
}

fn readonly_restore_prepare(command: &core::Command, observe_only: bool) -> bool {
    observe_only && matches!(command, core::Command::PrepareVm { source: Some(_), .. })
}

fn readonly_prepare_fence_matches(fence: super::Fence, epoch: core::Epoch) -> bool {
    epoch != 0 && (super::decisions::fence_matches(fence, epoch, false)
        || fence.terminal && fence.epoch >= epoch)
}

// This consumes only observations; it cannot load, activate, rewrite a fence,
// acknowledge an outbox action or release a material obligation.
fn observe_prepared_under_fence(
    epoch: core::Epoch,
    mut fence: impl FnMut() -> NodeResult<super::Fence>,
    mut observe: impl FnMut() -> NodeResult<LoadedSnapshotObservation>,
) -> NodeResult<LoadedSnapshotObservation> {
    let original_fence = fence()?;
    if !readonly_prepare_fence_matches(original_fence, epoch) {
        return Err("original restore preparation fence differs".into());
    }
    let original = require_loaded_candidate(observe()?, true)?;
    original.process.validate_fields()?;
    let confirmed = require_loaded_candidate(observe()?, true)?;
    if confirmed != original || fence()? != original_fence {
        return Err("original restore preparation or current fence changed during observation".into());
    }
    Ok(original)
}

fn scope(
    action: &WorkAction<core::EffectId>,
    work: &RestoreWorkRequest,
    namespace: &str,
    workspace_vm: Option<core::VmId>,
) -> NodeResult<(core::SandboxId, core::Epoch, core::Generation, core::VmId)> {
    scope_target(action, work.committed().seal_action(), work.operation(), work.generation(), namespace, workspace_vm)
}

// Same source/target checks as the opaque original work; exposed only inside
// this module so deterministic regressions need not fabricate a journal claim.
fn scope_target(
    action: &WorkAction<core::EffectId>,
    seal: &WorkAction<core::EffectId>,
    operation: core::OperationId,
    target_generation: core::Generation,
    namespace: &str,
    workspace_vm: Option<core::VmId>,
) -> NodeResult<(core::SandboxId, core::Epoch, core::Generation, core::VmId)> {
    let source_command = seal.command.command();
    let core::Command::SealSnapshot {
        sandbox: original_sandbox,
        snapshot,
        manifest,
        ..
    } = source_command
    else {
        return Err("restore lacks original sealed native mapping".into());
    };
    let (sandbox, epoch, generation, vm) = match action.command.command() {
        core::Command::PrepareVm {
            sandbox,
            epoch,
            generation,
            vm,
            source: Some(source),
            ..
        } => {
            if source.snapshot != *snapshot || source.manifest != *manifest {
                return Err("restore Prepare differs from original full snapshot mapping".into());
            }
            (*sandbox, *epoch, *generation, *vm)
        }
        core::Command::ApplyFilePolicy {
            sandbox,
            epoch,
            generation,
            ..
        } => (
            *sandbox,
            *epoch,
            *generation,
            workspace_vm.ok_or("restore file policy missing current VM")?,
        ),
        core::Command::ConfigureNetwork {
            sandbox,
            epoch,
            generation,
            vm,
            ..
        }
        | core::Command::ActivateVm {
            sandbox,
            epoch,
            generation,
            vm,
        } => (*sandbox, *epoch, *generation, *vm),
        _ => return Err("action is not the original restore chain".into()),
    };
    if sandbox != *original_sandbox
        || generation != target_generation
        || action.identity.operation.as_str()
            != format!("op-{namespace}-{}-{}", sandbox.0, operation.0)
        || action.identity.owner != seal.identity.owner
    {
        return Err("restore target operation, sandbox or generation differs".into());
    }
    Ok((sandbox, epoch, generation, vm))
}

struct RestoreContext<'a> {
    runtime: sandbox_runtime::Runtime,
    name: String,
    sandbox: core::SandboxId,
    generation: core::Generation,
    vm: core::VmId,
    manifest: [u8; 32],
    work_digest: sandproof_protocol::lifecycle::ContentDigest,
    action: &'a WorkAction<core::EffectId>,
    work: &'a RestoreWorkRequest,
    provider: &'a ProtectedHostEpochSource,
    profile: &'a ProtectedCaptureProfile,
}
impl RestoreContext<'_> {
    fn observe(&self) -> NodeResult<sandbox_runtime::LoadedSnapshotObservation> {
        Ok(self.runtime.observe_loaded_snapshot(
            &self.name,
            self.manifest,
            *self.work_digest.as_bytes(),
        )?)
    }
    fn paused(&self) -> NodeResult<sandbox_runtime::LoadedSnapshotObservation> {
        require_loaded_candidate(self.observe()?, false)
    }
}

impl LinuxWorker {
    pub(crate) fn observe_restore_exit(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        action: &WorkAction<core::EffectId>,
        work: &RestoreWorkRequest,
    ) -> NodeResult<Option<RestoreCandidateExitObservation>> {
        if !matches!(action.command.command(), core::Command::ActivateVm { .. }) {
            return Err("restore exit requires the original activation action".into());
        }
        self.validate_action(action)?;
        let (sandbox, epoch, _, vm) = scope(action, work, &self.config.namespace, None)?;
        super::capture::require_source_binding(&work.committed().work().pause, work.source())?;
        profile.require_original_source(work.source())?;
        provider.observe_original(
            &work.source().node,
            &work.source().host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        self.require_fence(sandbox, epoch, false)?;
        let runtime = self.runtime(sandbox, vm)?;
        let name = self.vm_name(vm);
        let work_binding = work.binding_digest()?;
        let process = observe_confirmed_restore_exit(|| {
            runtime
                .observe_loaded_snapshot(
                    &name,
                    *work.committed().manifest_digest().as_bytes(),
                    *work_binding.as_bytes(),
                )
                .map_err(Into::into)
        })?;
        self.require_fence(sandbox, epoch, false)?;
        self.validate_action(action)?;
        provider.observe_original(
            &work.source().node,
            &work.source().host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        Ok(process.map(|process| RestoreCandidateExitObservation {
            action: action.clone(),
            work_binding,
            process: SourceProcessIdentity {
                pid: process.pid,
                start_time_ticks: process.start_time_ticks,
                boot_id: process.boot_id,
            },
        }))
    }

    pub(crate) fn restore_action(
        &self,
        provider: &ProtectedHostEpochSource,
        profile: &ProtectedCaptureProfile,
        action: &WorkAction<core::EffectId>,
        work: &RestoreWorkRequest,
        activation: Option<&RestoreActivationPermit>,
        observe_only: bool,
    ) -> NodeResult<HostObservation> {
        self.validate_action(action)?;
        let workspace_vm = if let core::Command::ApplyFilePolicy {
            sandbox,
            generation,
            ..
        } = action.command.command()
        {
            Some(self.vm_for_workspace(*sandbox, *generation)?)
        } else {
            None
        };
        let (sandbox, epoch, generation, vm) =
            scope(action, work, &self.config.namespace, workspace_vm)?;
        super::capture::require_source_binding(&work.committed().work().pause, work.source())?;
        profile.require_original_source(work.source())?;
        provider.observe_original(
            &work.source().node,
            &work.source().host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        if !readonly_restore_prepare(action.command.command(), observe_only) {
            self.require_fence(sandbox, epoch, false)?;
        }
        let context = RestoreContext {
            runtime: self.runtime(sandbox, vm)?,
            name: self.vm_name(vm),
            sandbox,
            generation,
            vm,
            manifest: *work.committed().manifest_digest().as_bytes(),
            work_digest: work.binding_digest()?,
            action,
            work,
            provider,
            profile,
        };
        let result = match action.command.command() {
            core::Command::PrepareVm { slot, .. } => {
                self.restore_prepare(&context, *slot, observe_only)?
            }
            core::Command::ConfigureNetwork { slot, policy, .. } => {
                self.restore_network(&context, *slot, policy, observe_only)?
            }
            core::Command::ApplyFilePolicy { policy, .. } => {
                self.restore_file_policy(&context, *policy)?
            }
            core::Command::ActivateVm { .. } => {
                self.restore_activate(&context, activation, observe_only)?
            }
            _ => return Err("unsupported restore action".into()),
        };
        if !readonly_restore_prepare(action.command.command(), observe_only) {
            self.require_fence(sandbox, epoch, false)?;
        }
        self.validate_action(action)?;
        provider.observe_original(
            &work.source().node,
            &work.source().host_bootstrap,
            self.host_bootstrap_inputs(),
        )?;
        Ok(HostObservation {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
            result,
        })
    }

    fn restore_prepare(
        &self,
        context: &RestoreContext<'_>,
        slot: core::NetSlotId,
        observe_only: bool,
    ) -> NodeResult<ObservedResult> {
        let observed = if observe_only {
            self.require_slot(context.sandbox, context.generation, context.vm, slot)?;
            observe_prepared_under_fence(
                context.action.identity.epoch,
                || self.fence(context.sandbox),
                || context.observe(),
            )?
        } else {
            let template = self
                .secret(context.sandbox, context.generation)?
                .template
                .ok_or("restore lacks original sealed template")?;
            self.validate_template(&template)?;
            let source = context.work.source();
            let expected = super::full_capture::expected_compatibility(
                &template,
                context.runtime.boot_plan_digest()?,
                context.provider.original_tool_digest(
                    &source.node,
                    &source.host_bootstrap,
                    self.host_bootstrap_inputs(),
                )?,
                source.host_bootstrap.policy,
            )?;
            if context.work.committed().work().compatibility != expected {
                return Err("restore full profile differs from protected current runtime".into());
            }
            let stored = super::full_capture::observe_committed_capture(
                context.profile,
                context.work.committed(),
            )?;
            self.write_node_json(
                &self.slot_path(slot),
                &SlotBinding {
                    owner: context.action.identity.owner.clone(),
                    sandbox: context.sandbox,
                    generation: context.generation,
                    vm: context.vm,
                    slot,
                },
            )?;
            context.runtime.load_paused_snapshot(
                &context.name,
                stored.bundle(),
                &context.profile.layout,
                context.manifest,
                *context.work_digest.as_bytes(),
                context.profile.api_timeout,
            )?
        };
        let observed = require_loaded_candidate(observed, observe_only)?;
        Ok(ObservedResult::Prepared {
            pid: observed.process.pid,
            start_ticks: observed.process.start_time_ticks,
            boot_id: observed.process.boot_id,
        })
    }

    fn restore_network(
        &self,
        context: &RestoreContext<'_>,
        slot: core::NetSlotId,
        policy: &core::NetworkPolicy,
        observe_only: bool,
    ) -> NodeResult<ObservedResult> {
        let before = if observe_only {
            require_loaded_candidate(context.observe()?, true)?
        } else {
            context.paused()?
        };
        if !observe_only && !matches!(policy, core::NetworkPolicy::DenyAll) {
            context
                .runtime
                .configure_ipv4_uplink(&context.name, policy)?;
        }
        let result = self.observe_network_after_candidate(
            context.sandbox,
            context.generation,
            context.vm,
            slot,
            policy,
        )?;
        if context.observe()? != before {
            return Err("restore candidate changed while configuring network".into());
        }
        Ok(result)
    }

    fn restore_file_policy(
        &self,
        context: &RestoreContext<'_>,
        policy: core::FilePolicyRef,
    ) -> NodeResult<ObservedResult> {
        if policy.digest != self.config.file_policy_digest {
            return Err("restore file policy differs".into());
        }
        let before = context.observe()?;
        observe_running_file_policy(
            &before,
            || self.observe_private_disk_after_candidate(context.sandbox, context.generation),
            || context.observe(),
        )
    }

    fn restore_activate(
        &self,
        context: &RestoreContext<'_>,
        activation: Option<&RestoreActivationPermit>,
        observe_only: bool,
    ) -> NodeResult<ObservedResult> {
        let before = context.observe()?;
        let secret = self.secret(context.sandbox, context.generation)?;
        let confirm = || {
            let after = context.observe()?;
            if after.process != before.process
                || after.state != LoadedSnapshotState::Running
                || self.guest_file_status(&context.name, &secret.token)? != "200"
            {
                return Err("restore activation/current guest credential not observed".into());
            }
            Ok(after)
        };
        let after = if observe_only {
            confirm()?
        } else {
            let permit =
                activation.ok_or("restore activation lacks durable MayHaveExecuted permit")?;
            if permit.action() != context.action || permit.work_binding() != &context.work_digest {
                return Err("restore activation permit differs from original action/work".into());
            }
            finish_permitted_activation(
                &before,
                || {
                    context
                        .runtime
                        .activate_loaded_snapshot(
                            &context.name,
                            context.manifest,
                            *context.work_digest.as_bytes(),
                            &before.process,
                        )
                        .map_err(Into::into)
                },
                // RAM already contains the template process. Only rotate
                // authorization; never dispatch startup a second time.
                || self.initialize_guest_authorization(&context.name, &secret),
                confirm,
            )?
        };
        #[cfg(debug_assertions)]
        if !observe_only && Self::restore_activation_gate(context, &after, &secret.token)? {
            confirm()?;
        }
        Ok(ObservedResult::Activated {
            pid: after.process.pid,
            start_ticks: after.process.start_time_ticks,
            boot_id: after.process.boot_id,
        })
    }

    #[cfg(debug_assertions)]
    fn restore_activation_gate(
        context: &RestoreContext<'_>,
        after: &LoadedSnapshotObservation,
        token: &str,
    ) -> NodeResult<bool> {
        use std::{fs::File, os::unix::fs::MetadataExt};
        let Some(gate) = crate::adapter::faults::RestoreActivationGate::configured(context.action)?
        else {
            return Ok(false);
        };
        let path = std::path::Path::new("/run/netns").join(&context.name);
        sandbox_runtime::check_trusted_path(&path)?;
        let expected_inode = context.runtime.observe_recorded_network(&context.name)?;
        let namespace = File::from(rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )?);
        let actual = namespace.metadata()?;
        let process_namespace =
            File::open(format!("/proc/{}/ns/net", after.process.pid))?.metadata()?;
        if actual.ino() != expected_inode
            || (actual.dev(), actual.ino()) != (process_namespace.dev(), process_namespace.ino())
            || context.observe()?.process != after.process
        {
            return Err("restore gate candidate or original namespace differs".into());
        }
        gate.wait(
            context.action,
            &context.name,
            &after.process,
            &namespace,
            token,
        )?;
        let current = std::fs::symlink_metadata(path)?;
        if (current.dev(), current.ino()) != (actual.dev(), actual.ino()) {
            return Err("restore gate namespace changed while waiting".into());
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "restore_activation_tests.rs"]
mod activation_tests;

#[cfg(test)]
#[path = "restore_prepare_observation_tests.rs"]
mod prepare_observation_tests;
