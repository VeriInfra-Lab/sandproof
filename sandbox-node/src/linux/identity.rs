//! Observe original bindings before comparing pure identity values.
use super::{
    Fence, File, LinuxWorker, NodeResult, PathBuf, ResourceEvidence, Runtime, RuntimeConfig,
    RuntimeObservationConfig, SlotBinding, TemplateBinding, WorkAction, check_trusted_path, core,
    decisions, fs, host_slots, launch_observation,
};

impl LinuxWorker {
    pub(super) fn vm_name(&self, vm: core::VmId) -> String {
        format!("vm-{}-{}", self.config.namespace, vm.0)
    }

    pub(super) fn slot_path(&self, slot: core::NetSlotId) -> PathBuf {
        self.config
            .state_dir
            .join("slots")
            .join(format!("slot-{}-{}.json", self.config.namespace, slot.0))
    }

    pub(super) fn fence_path(&self, sandbox: core::SandboxId) -> PathBuf {
        self.config
            .state_dir
            .join("fences")
            .join(format!("sb-{}-{}.json", self.config.namespace, sandbox.0))
    }

    pub(super) fn runtime(&self, sandbox: core::SandboxId, vm: core::VmId) -> NodeResult<Runtime> {
        Ok(Runtime::for_existing(self.runtime_config(sandbox, vm)?)?)
    }

    pub(super) fn runtime_config(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
    ) -> NodeResult<RuntimeConfig> {
        let secret = self.secret(sandbox)?;
        let observed = self.observation_config(vm, secret.template.as_ref())?;
        Ok(RuntimeConfig {
            console_policy: observed.console_policy,
            jailer: observed.jailer,
            firecracker: observed.firecracker,
            chroot_base: observed.chroot_base,
            kernel: observed.kernel,
            rootfs: observed.rootfs,
            vm_uid: observed.vm_uid,
            vm_gid: observed.vm_gid,
            vcpu_count: observed.vcpu_count,
            memory_mib: observed.memory_mib,
            host_resources: observed.host_resources,
            private_netns: observed.private_netns,
            access_token_hash: Some(secret.sha512),
            host_slot: observed.host_slot,
        })
    }

    pub(super) fn observation_config(
        &self,
        vm: core::VmId,
        template: Option<&TemplateBinding>,
    ) -> NodeResult<RuntimeObservationConfig> {
        let id = self.vm_uid(vm)?;
        let vm_name = self.vm_name(vm);
        launch_observation::bind_template(
            RuntimeObservationConfig {
                console_policy: self.config.bounded_host.console_policy.map(
                    |policy| match policy {
                        sandproof_controller_service::ConsolePolicy::Discard => {
                            sandbox_runtime::ConsolePolicy::Discard
                        }
                    },
                ),
                jailer: self.config.jailer.clone(),
                firecracker: self.config.firecracker.clone(),
                chroot_base: self.config.chroot_base.clone(),
                kernel: self.config.kernel.clone(),
                rootfs: self.config.rootfs.clone(),
                vm_uid: id,
                vm_gid: id,
                vcpu_count: self.config.vcpu_count,
                memory_mib: self.config.memory_mib,
                host_resources: self.config.host_resources.clone(),
                private_netns: Some(PathBuf::from(format!("/run/netns/{vm_name}"))),
                host_slot: Some(self.runtime_host_slot(self.physical_slot_for_vm(vm)?, vm)?),
            },
            template,
        )
    }

    pub(super) fn vm_uid(&self, vm: core::VmId) -> NodeResult<u32> {
        host_slots::position_uid(&self.config.bounded_host, self.physical_slot_for_vm(vm)?)
    }

    pub(super) fn slot(&self, slot: core::NetSlotId) -> NodeResult<SlotBinding> {
        let path = self.slot_path(slot);
        check_trusted_path(&path)?;
        let value: SlotBinding = serde_json::from_reader(File::open(&path)?)?;
        File::open(path.parent().ok_or("slot parent")?)?.sync_all()?;
        if value.slot != slot {
            return Err("slot mapping identity mismatch".into());
        }
        Ok(value)
    }

    pub(super) fn binding_for_workspace(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> NodeResult<SlotBinding> {
        let mut found = None;
        for entry in fs::read_dir(self.config.state_dir.join("slots"))? {
            let entry = entry?;
            let binding: SlotBinding = serde_json::from_reader(File::open(entry.path())?)?;
            if binding.sandbox == sandbox && binding.generation == generation {
                if found.is_some() {
                    return Err("ambiguous VM for workspace".into());
                }
                found = Some(binding);
            }
        }
        found.ok_or_else(|| "no VM for workspace".into())
    }

    pub(super) fn vm_for_workspace(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> NodeResult<core::VmId> {
        Ok(self.binding_for_workspace(sandbox, generation)?.vm)
    }

    pub(super) fn fence(&self, sandbox: core::SandboxId) -> NodeResult<Fence> {
        let path = self.fence_path(sandbox);
        check_trusted_path(&path)?;
        let value = serde_json::from_reader(File::open(&path)?)?;
        File::open(path.parent().ok_or("fence parent")?)?.sync_all()?;
        Ok(value)
    }

    pub(super) fn require_fence(
        &self,
        sandbox: core::SandboxId,
        epoch: core::Epoch,
        terminal: bool,
    ) -> NodeResult<()> {
        let fence = self.fence(sandbox)?;
        if !decisions::fence_matches(fence, epoch, terminal) {
            return Err("host fence does not match command".into());
        }
        Ok(())
    }

    pub(super) fn validate_action(&self, action: &WorkAction<core::EffectId>) -> NodeResult<()> {
        action.identity.validate_version()?;
        self.require_occupied_action(action)?;
        let command = action.command.command();
        let generation = decisions::command_generation(command);
        if !decisions::action_fields_match(action, &self.config.namespace)
            || action.identity.resource
                != decisions::command_resource(command, &self.config.namespace)?
        {
            return Err("host action identity mismatch".into());
        }
        if let Some(generation) = generation {
            // PrepareVm creates the first slot binding. All later actions,
            // including old cleanup and read-only observations, must carry
            // the same owner as that durable host binding.
            let bound_owner = match command {
                core::Command::PrepareVm { slot, .. }
                | core::Command::AbortPrepare { slot, .. } => {
                    match fs::symlink_metadata(self.slot_path(*slot)) {
                        Ok(_) => Some(self.slot(*slot)?.owner),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                        Err(error) => return Err(error.into()),
                    }
                }
                _ => Some(
                    self.binding_for_workspace(command.sandbox(), generation)?
                        .owner,
                ),
            };
            if bound_owner.is_some_and(|owner| owner != action.identity.owner) {
                return Err("host action owner differs from resource binding".into());
            }
        }
        Ok(())
    }

    pub(super) fn require_slot(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        vm: core::VmId,
        slot: core::NetSlotId,
    ) -> NodeResult<()> {
        let binding = self.slot(slot)?;
        self.require_occupied_binding(&binding)?;
        if !decisions::slot_coordinates_match(&binding, sandbox, generation, vm, slot) {
            return Err("slot binding mismatch".into());
        }
        Ok(())
    }
}
