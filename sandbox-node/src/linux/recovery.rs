//! Compensation after uncertain results; original bindings precede mutations.
use super::*;

impl LinuxWorker {
    /// Finish only an unlaunched `PrepareVm` whose slot binding has already
    /// been checked against the durable operation plan by the caller. This
    /// does not retry `PrepareVm` or infer that a live/unknown VM has stopped.
    /// # Errors
    /// Refuses a mismatched binding or any possible live VM identity.
    pub fn reclaim_aborted_prepare(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        vm: core::VmId,
        slot: core::NetSlotId,
    ) -> NodeResult<()> {
        self.require_slot(sandbox, generation, vm, slot)?;
        let name = self.vm_name(vm);
        let runtime = self.runtime(sandbox, vm)?;
        runtime.reclaim_unlaunched(&name)?;
        if !runtime.was_reclaimed_unlaunched(&name)? {
            return Err("aborted launch marker is not durable".into());
        }
        require_absent(Path::new(&format!("/run/netns/{name}")))?;
        Ok(())
    }

    /// Reclaim a `PrepareVm` whose recorded process has definitively exited.
    /// # Errors
    /// Refuses a mismatched slot or any still-live/uncertain VM process.
    pub fn reclaim_stopped_prepare(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        vm: core::VmId,
        slot: core::NetSlotId,
    ) -> NodeResult<()> {
        self.require_slot(sandbox, generation, vm, slot)?;
        let name = self.vm_name(vm);
        let runtime = self.runtime(sandbox, vm)?;
        if runtime.observe(&name)?.state != VmState::Stopped {
            return Err("unknown or live VM cannot be reclaimed".into());
        }
        runtime.reclaim_stopped_prepare(&name)?;
        if !runtime.was_reclaimed_stopped_prepare(&name)? {
            return Err("failed PrepareVm marker is not durable".into());
        }
        Ok(())
    }

    /// Recover only an already-dispatched `PrepareVm` selected by the
    /// controller's durable plan. This never launches or signals a VM. An
    /// absent first slot can be tombstoned only when its claim, jail and
    /// namespace are all absent; otherwise identity remains unknown.
    /// # Errors
    /// Rejects mismatched ownership or any ambiguous physical resource.
    pub fn reclaim_incomplete_prepare(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> NodeResult<()> {
        self.validate_action(action)?;
        let core::Command::PrepareVm {
            sandbox,
            generation,
            vm,
            slot,
            ..
        } = action.command.command()
        else {
            return Err("reclaim requires a PrepareVm action".into());
        };
        self.ensure_prepare_slot(&action.identity.owner, *sandbox, *generation, *vm, *slot)?;
        let name = self.vm_name(*vm);
        match self.runtime(*sandbox, *vm)?.observe(&name) {
            Err(RuntimeError::MissingRecord) => {
                self.reclaim_aborted_prepare(*sandbox, *generation, *vm, *slot)
            }
            Ok(Observation {
                state: VmState::Stopped,
                ..
            }) => self.reclaim_stopped_prepare(*sandbox, *generation, *vm, *slot),
            Ok(_) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Complete only the original guest initialization for the exact live VM.
    /// The controller first proves that this outbox action still belongs to
    /// the current create; this host check binds it to the open fence,
    /// workspace, persisted credential/config and recorded VMM process.
    /// Sending the same restricted `/init` body again is convergent for the
    /// pinned envd version. An inconclusive request remains unknown.
    /// # Errors
    /// Rejects changed physical identity or an unobservable guest.
    pub fn reclaim_incomplete_activation(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> NodeResult<()> {
        self.validate_action(action)?;
        let core::Command::ActivateVm {
            sandbox,
            epoch,
            generation,
            vm,
        } = action.command.command()
        else {
            return Err("reclaim requires an ActivateVm action".into());
        };
        self.complete_guest_activation(*sandbox, *epoch, *generation, *vm)?;
        Ok(())
    }

    pub(super) fn ensure_prepare_slot(
        &self,
        owner: &TenantId,
        sandbox: core::SandboxId,
        generation: core::Generation,
        vm: core::VmId,
        slot: core::NetSlotId,
    ) -> NodeResult<()> {
        let slot_path = self.slot_path(slot);
        match fs::symlink_metadata(&slot_path) {
            Ok(_) => {
                self.sync_node_record(&slot_path)?;
                self.require_slot(sandbox, generation, vm, slot)?;
                if self.slot(slot)?.owner != *owner {
                    return Err("prepare slot owner mismatch".into());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let inventory = self.inventory()?;
                if !inventory.unbound_claims.is_empty()
                    || !inventory.unbound_networks.is_empty()
                    || !inventory.unbound_cgroups.is_empty()
                    || inventory
                        .slots
                        .iter()
                        .any(|known| known.vm == vm || known.slot == slot)
                {
                    return Err("missing slot has an ambiguous host resource".into());
                }
                let name = self.vm_name(vm);
                require_absent(&self.config.chroot_base.join(".claims").join(&name))?;
                require_absent(
                    &self
                        .config
                        .chroot_base
                        .join(
                            self.config
                                .firecracker
                                .file_name()
                                .ok_or("firecracker name")?,
                        )
                        .join(&name),
                )?;
                require_absent(Path::new(&format!("/run/netns/{name}")))?;
                self.runtime(sandbox, vm)?.require_cgroup_absent(&name)?;
                self.write_node_json(
                    &slot_path,
                    &SlotBinding {
                        owner: owner.clone(),
                        sandbox,
                        generation,
                        vm,
                        slot,
                    },
                )?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }

    /// Resume a dispatched startup abort by exact identity. It never retries
    /// `PrepareVm` and repeats only the same idempotent stop/reclaim operation.
    /// # Errors
    /// Refuses a changed process or incomplete physical cleanup.
    pub fn reclaim_incomplete_abort_prepare(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> NodeResult<()> {
        if !matches!(action.command.command(), core::Command::AbortPrepare { .. }) {
            return Err("reclaim requires AbortPrepare".into());
        }
        if !matches!(self.perform(action)?.result, ObservedResult::PrepareAborted) {
            return Err("aborted prepare not observed".into());
        }
        Ok(())
    }

    /// Complete only a dispatched cleanup whose operation was matched by A.
    /// Runtime cleanup is idempotent but still verifies the persisted process
    /// identity, stopped state, jail type and namespace inode on every call.
    /// # Errors
    /// A live, changed or unobservable VM is never cleaned by guesswork.
    pub fn reclaim_incomplete_cleanup(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> NodeResult<()> {
        self.validate_action(action)?;
        let core::Command::CleanupVm {
            sandbox,
            generation,
            vm,
        } = action.command.command()
        else {
            return Err("reclaim requires a CleanupVm action".into());
        };
        if !self.fence(*sandbox)?.terminal || self.vm_for_workspace(*sandbox, *generation)? != *vm {
            return Err("cleanup requires terminal fence and exact VM binding".into());
        }
        let name = self.vm_name(*vm);
        let runtime = self.runtime(*sandbox, *vm)?;
        if runtime.observe(&name)?.state != VmState::Stopped {
            return Err("uncertain or live VM cannot be cleaned".into());
        }
        runtime.cleanup(&name)?;
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
        Ok(())
    }

    /// Reconcile only an already dispatched, current `ConfigureNetwork`. A
    /// staged exact policy may be completed; pre-prepared partial resources
    /// are removed under the original deny gate before final failure.
    /// # Errors
    /// Invalid identity or uncertain host effects remain unresolved.
    pub fn reclaim_incomplete_network(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> NodeResult<()> {
        self.validate_action(action)?;
        let core::Command::ConfigureNetwork {
            sandbox,
            epoch,
            generation,
            vm,
            slot,
            policy,
        } = action.command.command()
        else {
            return Err("reclaim requires ConfigureNetwork".into());
        };
        self.require_fence(*sandbox, *epoch, false)?;
        self.require_slot(*sandbox, *generation, *vm, *slot)?;
        self.running(*sandbox, *vm)?;
        if matches!(policy, core::NetworkPolicy::Ipv4 { .. }) {
            self.runtime(*sandbox, *vm)?
                .reconcile_incomplete_ipv4_uplink(&self.vm_name(*vm), policy)?;
        }
        Ok(())
    }

    /// Reapply only a current terminal fence whose first receipt is unknown.
    /// Setting the same or stricter fence is idempotent; the normal host
    /// transition rejects any attempt to reopen or move backwards.
    /// # Errors
    /// A stale or unobservable fence remains unresolved.
    pub fn reclaim_incomplete_terminal_fence(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> NodeResult<()> {
        if !matches!(
            action.command.command(),
            core::Command::InstallFence { terminal: true, .. }
        ) {
            return Err("reclaim requires terminal fence".into());
        }
        let observation = self.perform(action)?;
        if !matches!(
            observation.result,
            ObservedResult::Fence(Fence { terminal: true, .. })
        ) {
            return Err("terminal fence not observed".into());
        }
        Ok(())
    }

    /// Reapply only an opening fence that the controller matched to the
    /// current nonterminal create. `fence_can_advance` prevents reopening a
    /// terminal host fence, including after a stale receipt arrives.
    /// # Errors
    /// Rejects any different action or a terminal/stale host fence.
    pub fn reclaim_incomplete_open_fence(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> NodeResult<()> {
        if !matches!(
            action.command.command(),
            core::Command::InstallFence {
                terminal: false,
                ..
            }
        ) {
            return Err("reclaim requires opening fence".into());
        }
        let observation = self.perform(action)?;
        if !matches!(
            observation.result,
            ObservedResult::Fence(Fence {
                terminal: false,
                ..
            })
        ) {
            return Err("opening fence not observed".into());
        }
        Ok(())
    }

    /// Complete the one already-dispatched stop selected by the controller.
    /// `perform` checks the terminal fence and workspace binding, then the
    /// runtime signals only the process verified by its durable identity and
    /// pidfd. A repeated stop of that same process is harmless.
    /// # Errors
    /// A changed identity or unconfirmed process exit remains unresolved.
    pub fn reclaim_incomplete_stop(&self, action: &WorkAction<core::EffectId>) -> NodeResult<()> {
        if !matches!(action.command.command(), core::Command::StopVm { .. }) {
            return Err("reclaim requires StopVm".into());
        }
        let observation = self.perform(action)?;
        if !matches!(observation.result, ObservedResult::Stopped { .. }) {
            return Err("exact VM stop not observed".into());
        }
        Ok(())
    }
}
