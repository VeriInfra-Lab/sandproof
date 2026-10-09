//! Read-only evidence collection; never dispatch execution.
use super::{
    FailureCode, HostObservation, LinuxWorker, MetadataExt, NodeResult, ObservedResult, Path,
    VmState, WorkAction, WorkOutcome, WorkReceipt, core, fs, require_absent,
};

impl LinuxWorker {
    /// Reconstruct a receipt only from current host state. This path never
    /// launches, initializes, stops, or cleans a VM. A durable aborted-launch
    /// marker can prove a definitive failure; otherwise an observation error
    /// leaves the dispatched outcome unknown.
    /// # Errors
    /// Returns the exact failed observation check for operator diagnosis.
    #[expect(
        clippy::too_many_lines,
        reason = "each V1 command needs its own read-only physical observation without sharing mutating worker paths"
    )]
    pub fn observe_action(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> NodeResult<WorkReceipt<HostObservation>> {
        self.validate_action(action)?;
        let command = action.command.command();
        let result = match command {
            core::Command::InstallFence {
                sandbox,
                epoch,
                terminal,
            } => {
                let observed = self.fence(*sandbox)?;
                if observed.epoch != *epoch || observed.terminal != *terminal {
                    return Err("host fence does not prove this action".into());
                }
                ObservedResult::Fence(observed)
            }
            core::Command::PrepareVm {
                sandbox,
                generation,
                vm,
                slot,
                ..
            } => {
                self.require_slot(*sandbox, *generation, *vm, *slot)?;
                if self
                    .runtime(*sandbox, *vm)?
                    .was_reclaimed_unlaunched(&self.vm_name(*vm))?
                    || self
                        .runtime(*sandbox, *vm)?
                        .was_reclaimed_stopped_prepare(&self.vm_name(*vm))?
                {
                    return Ok(WorkReceipt {
                        identity: action.identity.clone(),
                        outcome: WorkOutcome::DefinitiveFailure {
                            // The never-reusable VM resource was consumed by
                            // this aborted launch; retry needs a new resource.
                            code: FailureCode::InvalidResource,
                            retryable: false,
                            partial: None,
                        },
                    });
                }
                let observed = self.running(*sandbox, *vm)?;
                ObservedResult::Prepared {
                    pid: observed.pid,
                    start_ticks: observed.start_time_ticks,
                }
            }
            core::Command::ConfigureNetwork {
                sandbox,
                epoch,
                generation,
                vm,
                slot,
                policy,
            } => {
                if self
                    .runtime(*sandbox, *vm)?
                    .was_reclaimed_ipv4_uplink(&self.vm_name(*vm), policy)?
                {
                    return Ok(WorkReceipt {
                        identity: action.identity.clone(),
                        outcome: WorkOutcome::DefinitiveFailure {
                            code: FailureCode::InvalidResource,
                            retryable: false,
                            partial: None,
                        },
                    });
                }
                self.observe_network(*sandbox, *epoch, *generation, *vm, *slot, policy)?
            }
            core::Command::ApplyFilePolicy {
                sandbox,
                epoch,
                generation,
                policy,
                ..
            } => self.observe_file_policy(*sandbox, *epoch, *generation, *policy)?,
            core::Command::CleanupSlot {
                sandbox,
                generation,
                slot,
            } => self.observe_closed_slot(*sandbox, *generation, *slot)?,
            core::Command::ActivateVm {
                sandbox,
                generation,
                vm,
                ..
            } => {
                if self.vm_for_workspace(*sandbox, *generation)? != *vm {
                    return Err("activation VM binding mismatch".into());
                }
                let observed = self.running(*sandbox, *vm)?;
                let token = self.secret(*sandbox)?.token;
                if self.guest_file_status(&self.vm_name(*vm), &token)? != "200" {
                    return Err("guest authentication not observed".into());
                }
                ObservedResult::Activated {
                    pid: observed.pid,
                    start_ticks: observed.start_time_ticks,
                }
            }
            core::Command::StopVm {
                sandbox,
                epoch,
                generation,
                vm,
            } => {
                self.require_fence(*sandbox, *epoch, true)?;
                if self.vm_for_workspace(*sandbox, *generation)? != *vm {
                    return Err("stop VM binding mismatch".into());
                }
                let observed = self.runtime(*sandbox, *vm)?.observe(&self.vm_name(*vm))?;
                if observed.state != VmState::Stopped {
                    return Err("exact VM stop not observed".into());
                }
                ObservedResult::Stopped {
                    pid: observed.pid,
                    start_ticks: observed.start_time_ticks,
                }
            }
            core::Command::AbortPrepare {
                sandbox,
                epoch,
                generation,
                vm,
                slot,
            } => self.observe_aborted_prepare(*sandbox, *epoch, *generation, *vm, *slot)?,
            core::Command::CleanupVm {
                sandbox,
                generation,
                vm,
            } => {
                if !self.fence(*sandbox)?.terminal
                    || self.vm_for_workspace(*sandbox, *generation)? != *vm
                {
                    return Err("cleanup VM binding mismatch".into());
                }
                let name = self.vm_name(*vm);
                let runtime = self.runtime(*sandbox, *vm)?;
                if runtime.observe(&name)?.state != VmState::Stopped {
                    return Err("exact VM is still live".into());
                }
                let jail = self
                    .config
                    .chroot_base
                    .join(
                        self.config
                            .firecracker
                            .file_name()
                            .ok_or("invalid firecracker name")?,
                    )
                    .join(&name);
                require_absent(&jail)?;
                runtime.require_cgroup_absent(&name)?;
                require_absent(Path::new(&format!("/run/netns/{name}")))?;
                ObservedResult::VmCleaned
            }
            _ => return Err("command is outside the V1 host observer".into()),
        };
        Ok(WorkReceipt {
            identity: action.identity.clone(),
            outcome: WorkOutcome::ObservedSuccess(HostObservation {
                resource: action.identity.resource.clone(),
                command: command.clone(),
                result,
            }),
        })
    }

    pub(super) fn observe_aborted_prepare(
        &self,
        sandbox: core::SandboxId,
        epoch: core::Epoch,
        generation: core::Generation,
        vm: core::VmId,
        slot: core::NetSlotId,
    ) -> NodeResult<ObservedResult> {
        self.require_fence(sandbox, epoch, true)?;
        self.require_slot(sandbox, generation, vm, slot)?;
        let name = self.vm_name(vm);
        let runtime = self.runtime(sandbox, vm)?;
        if !runtime.was_reclaimed_unlaunched(&name)?
            && !runtime.was_reclaimed_stopped_prepare(&name)?
        {
            return Err("exact aborted prepare marker not observed".into());
        }
        require_absent(Path::new(&format!("/run/netns/{name}")))?;
        Ok(ObservedResult::PrepareAborted)
    }

    pub(super) fn observe_network(
        &self,
        sandbox: core::SandboxId,
        epoch: core::Epoch,
        generation: core::Generation,
        vm: core::VmId,
        slot: core::NetSlotId,
        policy: &core::NetworkPolicy,
    ) -> NodeResult<ObservedResult> {
        self.require_fence(sandbox, epoch, false)?;
        self.require_slot(sandbox, generation, vm, slot)?;
        self.running(sandbox, vm)?;
        let name = self.vm_name(vm);
        let runtime = self.runtime(sandbox, vm)?;
        let namespace_inode = match policy {
            core::NetworkPolicy::DenyAll => runtime.observe_private_network(&name)?,
            core::NetworkPolicy::Ipv4 { .. } => match runtime.observe_ipv4_uplink(&name, policy)? {
                sandbox_runtime::UplinkState::Active(inode) => inode,
                sandbox_runtime::UplinkState::Staged => {
                    return Err("IPv4 uplink has not been activated".into());
                }
            },
            core::NetworkPolicy::StaticAllowlist(_) => {
                return Err("unsupported network policy".into());
            }
        };
        let (gate, mut command) = self.namespace_command(&name)?;
        command.args([
            "netns",
            "exec",
            &name,
            "/usr/bin/ip",
            "-4",
            "route",
            "show",
            "default",
        ]);
        let output = self
            .spawn_namespace_command(&name, &gate, &mut command)?
            .wait_output()?;
        gate.finish_tracked()?;
        if !output.status.success()
            || (matches!(policy, core::NetworkPolicy::DenyAll) && !output.stdout.is_empty())
            || (matches!(policy, core::NetworkPolicy::Ipv4 { .. }) && output.stdout.is_empty())
        {
            return Err("namespace route does not match configured policy".into());
        }
        Ok(ObservedResult::Network { namespace_inode })
    }

    pub(super) fn observe_file_policy(
        &self,
        sandbox: core::SandboxId,
        epoch: core::Epoch,
        generation: core::Generation,
        policy: core::FilePolicyRef,
    ) -> NodeResult<ObservedResult> {
        self.require_fence(sandbox, epoch, false)?;
        if policy.digest != self.config.file_policy_digest {
            return Err("file policy digest is not the deployed V1 policy".into());
        }
        let vm = self.vm_for_workspace(sandbox, generation)?;
        self.running(sandbox, vm)?;
        let disk = self
            .config
            .chroot_base
            .join(
                self.config
                    .firecracker
                    .file_name()
                    .ok_or("invalid firecracker name")?,
            )
            .join(self.vm_name(vm))
            .join("root/rootfs.ext4");
        let metadata = fs::symlink_metadata(&disk)?;
        let uid = self.vm_uid(vm)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.mode() & 0o777 != 0o600
            || metadata.uid() != uid
            || metadata.ino() == 0
        {
            return Err("VM private disk policy not observed".into());
        }
        Ok(ObservedResult::FilePolicy {
            disk_inode: metadata.ino(),
        })
    }

    pub(super) fn observe_closed_slot(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        slot: core::NetSlotId,
    ) -> NodeResult<ObservedResult> {
        if !self.fence(sandbox)?.terminal {
            return Err("slot cleanup requires terminal fence".into());
        }
        let binding = self.slot(slot)?;
        if binding.sandbox != sandbox || binding.generation != generation {
            return Err("slot cleanup binding mismatch".into());
        }
        require_absent(Path::new(&format!(
            "/run/netns/{}",
            self.vm_name(binding.vm)
        )))?;
        self.runtime(sandbox, binding.vm)?
            .require_cgroup_absent(&self.vm_name(binding.vm))?;
        if !self
            .runtime(sandbox, binding.vm)?
            .uplink_resources_absent(&self.vm_name(binding.vm))?
        {
            return Err("slot uplink resources remain after cleanup".into());
        }
        if self
            .runtime(sandbox, binding.vm)?
            .observe(&self.vm_name(binding.vm))?
            .state
            != VmState::Stopped
        {
            return Err("slot VM remains alive".into());
        }
        Ok(ObservedResult::SlotClosed)
    }

    pub(super) fn running(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
    ) -> NodeResult<sandbox_runtime::Observation> {
        let observation = self.runtime(sandbox, vm)?.observe(&self.vm_name(vm))?;
        if observation.state != VmState::VmmRunning
            || observation.pid == 0
            || observation.start_time_ticks == 0
        {
            return Err("exact VM is not running".into());
        }
        Ok(observation)
    }
}
