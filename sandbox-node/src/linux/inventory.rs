//! Read-only host inventory; uncertain resources stay occupied.
use super::{
    BTreeMap, BTreeSet, CreateOptions, Digest, DirBuilderExt, File, HostInventory, HostSlotContext,
    LinuxWorker, MetadataExt, NodeResult, ObservedSlot, OsStrExt, Path, PathBuf, RuntimeError,
    RuntimePhysicalObserver, SlotBinding, VmState, check_trusted_path, core, fs,
    helper_observation_pending, host_slots,
};

impl LinuxWorker {
    /// Enumerate slots, VM claims and network handles in this deployment
    /// namespace, including incomplete launches. This is evidence for
    /// reconciliation, not authority to kill or remove anything.
    /// Unexpected/corrupt entries fail closed.
    /// # Errors
    /// Rejects unsafe paths, malformed bindings or mismatched host identities.
    pub fn inventory(&self) -> NodeResult<HostInventory> {
        self.inventory_with(|binding| {
            let config = self.runtime_config(binding.sandbox, binding.vm)?;
            Ok(RuntimePhysicalObserver::for_existing(config.into())?)
        })
    }

    /// Inspect every physical resource using the controller's original launch
    /// inputs, never an unreadable credential or a current template alias.
    /// This grants no worker, initialization, stopping or cleanup capability.
    /// # Errors
    /// Rejects missing/mismatched original bindings and all physical corruption.
    pub fn inventory_with_launch_options(
        &self,
        launches: &BTreeMap<core::SandboxId, (HostSlotContext, CreateOptions)>,
    ) -> NodeResult<HostInventory> {
        let host_slots = self.host_slot_inventory()?;
        self.inventory_with(|binding| {
            let (context, options) = launches
                .get(&binding.sandbox)
                .ok_or("host slot has no original controller launch inputs")?;
            host_slots::require_slot_binding(binding, context.binding)?;
            if !host_slots.iter().any(|(stored, phase)| {
                stored == context && *phase == core::maintenance::slots::HostSlotPhase::Occupied
            }) {
                return Err("original launch has a different physical host context".into());
            }
            options.validate()?;
            let config = self.observation_config(binding.vm, options.template.as_ref())?;
            Ok(RuntimePhysicalObserver::for_existing(config)?)
        })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "closed inventory checks each recorded host resource and orphan claim without silently skipping malformed entries"
    )]
    pub(super) fn inventory_with(
        &self,
        observer: impl Fn(&SlotBinding) -> NodeResult<RuntimePhysicalObserver>,
    ) -> NodeResult<HostInventory> {
        let prefix = format!("slot-{}-", self.config.namespace);
        let mut slots = Vec::new();
        let host_slots = self.host_slot_inventory()?;
        let retiring: BTreeSet<_> = host_slots
            .iter()
            .filter(|(_, phase)| *phase != core::maintenance::slots::HostSlotPhase::Occupied)
            .map(|(context, _)| self.vm_name(context.binding.vm))
            .collect();
        let mut bound = retiring.clone();
        for entry in fs::read_dir(self.config.state_dir.join("slots"))? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "non-UTF8 slot name")?;
            if !name.starts_with(&prefix) {
                continue;
            }
            let number = name
                .strip_prefix(&prefix)
                .and_then(|tail| tail.strip_suffix(".json"))
                .ok_or("invalid slot filename")?
                .parse::<u64>()?;
            let path = entry.path();
            check_trusted_path(&path)?;
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
                return Err("untrusted slot record".into());
            }
            let binding: SlotBinding = serde_json::from_reader(File::open(&path)?)?;
            if binding.slot.0 != number {
                return Err("slot filename or VM identity mismatch".into());
            }
            let vm_name = self.vm_name(binding.vm);
            if retiring.contains(&vm_name) {
                let (context, _) = host_slots
                    .iter()
                    .find(|(context, _)| context.binding.vm == binding.vm)
                    .ok_or("missing retiring context")?;
                host_slots::require_slot_binding(&binding, context.binding)?;
                continue;
            }
            self.require_occupied_binding(&binding)?;
            if !bound.insert(vm_name.clone()) {
                return Err("one VM is bound to multiple network slots".into());
            }
            let position = self.physical_slot_for_vm(binding.vm)?;
            let context = self.runtime_host_slot(position, binding.vm)?;
            let helper_pending = helper_observation_pending(
                context.helper_gate.try_acquire(Some(&context)).map(drop),
            )?;
            let claim = self.config.chroot_base.join(".claims").join(&vm_name);
            let claim_present = match fs::symlink_metadata(&claim) {
                Ok(metadata)
                    if metadata.is_dir()
                        && metadata.uid() == 0
                        && metadata.mode().trailing_zeros() >= 6 =>
                {
                    true
                }
                Ok(_) => return Err("untrusted VM claim".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => return Err(error.into()),
            };
            let runtime = observer(&binding)?;
            let aborted_unlaunched =
                claim_present && !helper_pending && runtime.was_reclaimed_unlaunched(&vm_name)?;
            let failed_prepare_reclaimed = claim_present
                && !helper_pending
                && runtime.was_reclaimed_stopped_prepare(&vm_name)?;
            let process = if claim_present && !helper_pending {
                match runtime.observe(&vm_name) {
                    Ok(physical_record) => Some(physical_record),
                    Err(RuntimeError::MissingRecord) => None,
                    Err(error) => return Err(error.into()),
                }
            } else {
                None
            };
            let network = PathBuf::from(format!("/run/netns/{vm_name}"));
            let network_inode = match fs::symlink_metadata(&network) {
                Ok(metadata) if metadata.is_file() && metadata.uid() == 0 => Some(metadata.ino()),
                Ok(_) => return Err("untrusted network namespace handle".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            // An extant namespace is not evidence that its firewall still
            // enforces the policy. A live (or unobservable) VM must retain
            // the exact pre-launch guard across controller/worker restarts.
            // Incomplete, definitely stopped launches are handled by the
            // separate exact-resource reclamation path instead.
            if process
                .as_ref()
                .is_some_and(|observed| observed.state != VmState::Stopped)
            {
                let observed_inode = runtime.observe_recorded_network(&vm_name)?;
                if network_inode != Some(observed_inode) {
                    return Err("live VM network guard or namespace identity changed".into());
                }
            }
            let disk = self
                .config
                .chroot_base
                .join(
                    self.config
                        .firecracker
                        .file_name()
                        .ok_or("invalid firecracker name")?,
                )
                .join(&vm_name)
                .join("root/rootfs.ext4");
            let jail_observation = if helper_pending {
                None
            } else {
                let observation = runtime.observe_jail(&vm_name)?;
                if matches!(observation, sandbox_runtime::JailObservation::Unregistered) {
                    return Err("VM jail is present without directory authority".into());
                }
                Some(observation)
            };
            let disk_inode = match fs::symlink_metadata(&disk) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    if jail_observation == Some(sandbox_runtime::JailObservation::Absent) {
                        return Err("disk appeared after observing jail absence".into());
                    }
                    Some(metadata.ino())
                }
                Ok(_) => return Err("untrusted VM disk".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            slots.push(ObservedSlot {
                owner: binding.owner,
                sandbox: binding.sandbox,
                generation: binding.generation,
                vm: binding.vm,
                slot: binding.slot,
                claim_present,
                helper_pending,
                aborted_unlaunched,
                failed_prepare_reclaimed,
                process,
                network_inode,
                disk_inode,
                cgroup_inode: runtime.cgroup_inode(&vm_name)?,
            });
        }
        slots.sort_by_key(|slot| slot.slot.0);
        let mut unbound_claims = Vec::new();
        let claims = self.config.chroot_base.join(".claims");
        match fs::read_dir(&claims) {
            Ok(entries) => {
                let vm_prefix = format!("vm-{}-", self.config.namespace);
                for entry in entries {
                    let entry = entry?;
                    let name = entry
                        .file_name()
                        .into_string()
                        .map_err(|_| "non-UTF8 claim name")?;
                    if !name.starts_with(&vm_prefix) {
                        continue;
                    }
                    name.strip_prefix(&vm_prefix)
                        .and_then(|number| number.parse::<u64>().ok())
                        .ok_or("invalid VM claim number")?;
                    let metadata = fs::symlink_metadata(entry.path())?;
                    if !entry.file_type()?.is_dir()
                        || metadata.uid() != 0
                        || metadata.mode().trailing_zeros() < 6
                    {
                        return Err("invalid VM claim name or type".into());
                    }
                    if !bound.contains(&name) {
                        unbound_claims.push(name);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        unbound_claims.sort();
        let mut unbound_networks = Vec::new();
        let networks = Path::new("/run/netns");
        let vm_prefix = format!("vm-{}-", self.config.namespace);
        match fs::read_dir(networks) {
            Ok(entries) => {
                check_trusted_path(networks)?;
                for entry in entries {
                    let entry = entry?;
                    if !entry
                        .file_name()
                        .as_bytes()
                        .starts_with(vm_prefix.as_bytes())
                    {
                        continue;
                    }
                    let name = entry
                        .file_name()
                        .into_string()
                        .map_err(|_| "non-UTF8 network namespace name")?;
                    let number = name
                        .strip_prefix(&vm_prefix)
                        .and_then(|number| number.parse::<u64>().ok())
                        .ok_or("invalid network namespace number")?;
                    let metadata = fs::symlink_metadata(entry.path())?;
                    if name != format!("{vm_prefix}{number}")
                        || !metadata.is_file()
                        || metadata.uid() != 0
                    {
                        return Err("invalid network namespace identity or type".into());
                    }
                    if !bound.contains(&name) {
                        unbound_networks.push(name);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        unbound_networks.sort();
        Ok(HostInventory {
            unbound_cgroups: self.unbound_cgroups(&bound)?,
            slots,
            unbound_claims,
            unbound_networks,
        })
    }

    pub(super) fn unbound_cgroups(&self, bound: &BTreeSet<String>) -> NodeResult<Vec<String>> {
        let mut unbound = Vec::new();
        if let Some(settings) = &self.config.host_resources {
            settings.validate(self.config.memory_mib)?;
            let prefix = format!("vm-{}-", self.config.namespace);
            for entry in fs::read_dir(&settings.cgroup_parent)? {
                let entry = entry?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| "non-UTF8 cgroup name")?;
                if !name.starts_with(&prefix) {
                    continue;
                }
                let number = name
                    .strip_prefix(&prefix)
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or("invalid cgroup VM number")?;
                if name != format!("{prefix}{number}") || !entry.file_type()?.is_dir() {
                    return Err("invalid VM cgroup identity or type".into());
                }
                check_trusted_path(&entry.path())?;
                if !bound.contains(&name) {
                    unbound.push(name);
                }
            }
        }
        unbound.sort();
        Ok(unbound)
    }
}
