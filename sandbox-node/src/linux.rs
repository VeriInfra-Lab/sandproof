use rustix::process::geteuid;
use sandbox_controller_core as core;
use sandbox_runtime::{
    Observation, Runtime, RuntimeConfig, RuntimeError, VmState, check_trusted_path,
};
use sandproof_controller_service::{HostEvidence, WorkerTransportError};
use sandproof_protocol::{
    FailureCode, ResourceEvidence, ResourceId, ResourceKind, ResourceRef, UnknownReason,
    WorkAction, WorkOutcome, WorkReceipt,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::error::Error;
use std::fs::{self, DirBuilder, File};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::thread;
use std::time::{Duration, Instant};

type NodeResult<T> = Result<T, Box<dyn Error>>;
const ENVD_URL: &str = "http://172.16.0.2:49983";

/// Deployment inputs are fixed, root-owned host paths, not API request data.
#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub namespace: String,
    pub state_dir: PathBuf,
    pub jailer: PathBuf,
    pub firecracker: PathBuf,
    pub chroot_base: PathBuf,
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
    pub uid_base: u32,
    pub max_vm_id: u64,
    pub vcpu_count: u8,
    pub memory_mib: u32,
    /// V1 has one deployment-controlled managed-disk policy, not arbitrary
    /// user-supplied host paths or a directory ACL implementation.
    pub file_policy_digest: core::PolicyDigest,
}

/// Read-only host inventory. A claim without a process record is explicitly
/// inconclusive; it must not be treated as a stopped VM or deleted by name.
#[derive(Clone, Debug)]
pub struct HostInventory {
    pub slots: Vec<ObservedSlot>,
    pub unbound_claims: Vec<String>,
    pub unbound_networks: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ObservedSlot {
    pub sandbox: core::SandboxId,
    pub generation: core::Generation,
    pub vm: core::VmId,
    pub slot: core::NetSlotId,
    pub claim_present: bool,
    pub aborted_unlaunched: bool,
    pub failed_prepare_reclaimed: bool,
    pub process: Option<Observation>,
    pub network_inode: Option<u64>,
    pub disk_inode: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestSecret {
    token: String,
    sha512: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotBinding {
    sandbox: core::SandboxId,
    generation: core::Generation,
    vm: core::VmId,
    slot: core::NetSlotId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fence {
    epoch: core::Epoch,
    terminal: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ObservedResult {
    Fence(Fence),
    Prepared { pid: u32, start_ticks: u64 },
    Network { namespace_inode: u64 },
    Activated { pid: u32, start_ticks: u64 },
    FilePolicy { disk_inode: u64 },
    Stopped { pid: u32, start_ticks: u64 },
    VmCleaned,
    SlotClosed,
}

/// Only the trusted worker constructs this from host checks. Serialization
/// stores the observation, but deserializing a guest reply is never authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostObservation {
    resource: ResourceRef,
    command: core::Command,
    result: ObservedResult,
}

impl ResourceEvidence for HostObservation {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}

impl HostEvidence for HostObservation {
    fn validates(&self, command: &core::Command) -> bool {
        if &self.command != command {
            return false;
        }
        match (command, &self.result) {
            (
                core::Command::InstallFence {
                    epoch, terminal, ..
                },
                ObservedResult::Fence(fence),
            ) => fence.epoch == *epoch && fence.terminal == *terminal,
            (core::Command::PrepareVm { .. }, ObservedResult::Prepared { pid, start_ticks })
            | (core::Command::ActivateVm { .. }, ObservedResult::Activated { pid, start_ticks })
            | (core::Command::StopVm { .. }, ObservedResult::Stopped { pid, start_ticks }) => {
                *pid != 0 && *start_ticks != 0
            }
            (
                core::Command::ConfigureNetwork { .. },
                ObservedResult::Network { namespace_inode },
            )
            | (
                core::Command::ApplyFilePolicy { .. },
                ObservedResult::FilePolicy {
                    disk_inode: namespace_inode,
                },
            ) => *namespace_inode != 0,
            (core::Command::CleanupVm { .. }, ObservedResult::VmCleaned)
            | (core::Command::CleanupSlot { .. }, ObservedResult::SlotClosed) => true,
            _ => false,
        }
    }
}

pub struct LinuxWorker {
    config: NodeConfig,
    last_error: Option<String>,
}

impl LinuxWorker {
    /// Enumerate slots, VM claims and network handles in this deployment
    /// namespace, including incomplete launches. This is evidence for
    /// reconciliation, not authority to kill or remove anything.
    /// Unexpected/corrupt entries fail closed.
    /// # Errors
    /// Rejects unsafe paths, malformed bindings or mismatched host identities.
    #[expect(
        clippy::too_many_lines,
        reason = "closed inventory checks each recorded host resource and orphan claim without silently skipping malformed entries"
    )]
    pub fn inventory(&self) -> NodeResult<HostInventory> {
        let prefix = format!("slot-{}-", self.config.namespace);
        let mut slots = Vec::new();
        let mut bound = BTreeSet::new();
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
            if binding.slot.0 != number || binding.vm.0 > self.config.max_vm_id {
                return Err("slot filename or VM identity mismatch".into());
            }
            let vm_name = self.vm_name(binding.vm);
            if !bound.insert(vm_name.clone()) {
                return Err("one VM is bound to multiple network slots".into());
            }
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
            let aborted_unlaunched = claim_present
                && self
                    .runtime(binding.sandbox, binding.vm)?
                    .was_reclaimed_unlaunched(&vm_name)?;
            let failed_prepare_reclaimed = claim_present
                && self
                    .runtime(binding.sandbox, binding.vm)?
                    .was_reclaimed_stopped_prepare(&vm_name)?;
            let process = if claim_present {
                match self.runtime(binding.sandbox, binding.vm)?.observe(&vm_name) {
                    Ok(observed) => Some(observed),
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
            let disk_inode = match fs::symlink_metadata(&disk) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    Some(metadata.ino())
                }
                Ok(_) => return Err("untrusted VM disk".into()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            slots.push(ObservedSlot {
                sandbox: binding.sandbox,
                generation: binding.generation,
                vm: binding.vm,
                slot: binding.slot,
                claim_present,
                aborted_unlaunched,
                failed_prepare_reclaimed,
                process,
                network_inode,
                disk_inode,
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
                    let number = name
                        .strip_prefix(&vm_prefix)
                        .and_then(|number| number.parse::<u64>().ok())
                        .ok_or("invalid VM claim number")?;
                    let metadata = fs::symlink_metadata(entry.path())?;
                    if number > self.config.max_vm_id
                        || !entry.file_type()?.is_dir()
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
                        || number > self.config.max_vm_id
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
            slots,
            unbound_claims,
            unbound_networks,
        })
    }

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

    /// Check allocatable bytes on the same filesystem that stores the VM's
    /// private disk. This is an admission observation, not a reservation
    /// against unrelated host processes.
    /// # Errors
    /// Rejects an unavailable or untrusted disk location.
    pub fn available_disk_bytes(&self) -> NodeResult<u64> {
        check_trusted_path(&self.config.chroot_base)?;
        let status = rustix::fs::statvfs(&self.config.chroot_base)?;
        Ok(status
            .f_bavail
            .checked_mul(status.f_frsize)
            .ok_or("disk size overflow")?)
    }

    /// The state directory must already be private and root-owned. Secrets,
    /// slot bindings and fences are persisted below it with exact IDs.
    /// # Errors
    /// Rejects unsafe deployment paths and an unavailable host runtime.
    pub fn new(config: NodeConfig) -> NodeResult<Self> {
        if geteuid().as_raw() != 0 {
            return Err("node worker requires root on the isolated KVM host".into());
        }
        if config.namespace.len() != 16
            || !config
                .namespace
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("invalid deployment namespace".into());
        }
        for path in [
            &config.state_dir,
            &config.jailer,
            &config.firecracker,
            &config.chroot_base,
            &config.kernel,
            &config.rootfs,
        ] {
            check_trusted_path(path)?;
        }
        let metadata = fs::metadata(&config.state_dir)?;
        if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
            return Err("node state directory must be root-only".into());
        }
        if config.uid_base == 0
            || config.max_vm_id == 0
            || config.vcpu_count == 0
            || config.memory_mib < 128
        {
            return Err("invalid node resource limits".into());
        }
        let longest_socket = config
            .chroot_base
            .join(
                config
                    .firecracker
                    .file_name()
                    .ok_or("invalid firecracker name")?,
            )
            .join(format!("vm-{}-{}", config.namespace, config.max_vm_id))
            .join("root/api.socket");
        if longest_socket.as_os_str().as_bytes().len() >= 108 {
            return Err("Firecracker API socket path exceeds Linux limit".into());
        }
        for subdir in ["secrets", "fences", "slots"] {
            let path = config.state_dir.join(subdir);
            if !path.exists() {
                DirBuilder::new().mode(0o700).create(&path)?;
            }
            check_trusted_path(&path)?;
            let metadata = fs::metadata(path)?;
            if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
                return Err("node state subdirectory must be root-only".into());
            }
        }
        for binary in ["/usr/bin/ip", "/usr/bin/curl", "/usr/bin/sha512sum"] {
            check_trusted_path(Path::new(binary))?;
        }
        Ok(Self {
            config,
            last_error: None,
        })
    }

    /// Create a one-time root-only credential before accepting a sandbox.
    /// The caller must use a CSPRNG and not log or pass the token in argv.
    /// # Errors
    /// Rejects reuse, malformed tokens, or a hash computation failure.
    pub fn put_secret(&self, sandbox: core::SandboxId, token: &str) -> NodeResult<()> {
        if token.len() < 32
            || token.len() > 128
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err("invalid guest access token shape".into());
        }
        let sha512 = hash_token(token)?;
        let value = GuestSecret {
            token: token.to_owned(),
            sha512,
        };
        let path = self.secret_path(sandbox);
        write_new_json(&path, &value)
    }

    /// Read a credential only for the authenticated API response or worker.
    /// # Errors
    /// Rejects a missing or malformed stored secret.
    pub fn token_for(&self, sandbox: core::SandboxId) -> NodeResult<String> {
        Ok(self.secret(sandbox)?.token)
    }

    /// A missing secret is distinguishable from a malformed or untrusted one.
    /// # Errors
    /// Refuses to treat a corrupt credential as safe to replace.
    pub fn has_secret(&self, sandbox: core::SandboxId) -> NodeResult<bool> {
        let path = self.secret_path(sandbox);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
            Ok(_) => {
                self.secret(sandbox)?;
                Ok(true)
            }
        }
    }

    /// Execute one already claimed action. Any physical uncertainty becomes an
    /// Unknown receipt, never a fabricated success or an automatic retry.
    /// # Errors
    /// The current in-process worker transport is infallible; the result type
    /// remains aligned with the controller's fallible worker interface.
    pub fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<HostObservation>, WorkerTransportError> {
        let result = self.perform(action);
        let outcome = match result {
            Ok(evidence) => {
                self.last_error = None;
                WorkOutcome::ObservedSuccess(evidence)
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                WorkOutcome::Unknown {
                    reason: UnknownReason::ObservationUnavailable,
                    last_observation: None,
                }
            }
        };
        Ok(WorkReceipt {
            identity: action.identity.clone(),
            outcome,
        })
    }

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
            } => self.observe_network(*sandbox, *epoch, *generation, *vm, *slot, policy)?,
            core::Command::ApplyFilePolicy {
                sandbox,
                epoch,
                generation,
                policy,
                ..
            } => self.observe_file_policy(*sandbox, *epoch, *generation, policy)?,
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
                if Self::guest_file_status(&self.vm_name(*vm), &token)? != "200" {
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
                if self.runtime(*sandbox, *vm)?.observe(&name)?.state != VmState::Stopped {
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

    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    fn secret_path(&self, sandbox: core::SandboxId) -> PathBuf {
        self.config
            .state_dir
            .join("secrets")
            .join(format!("sb-{}-{}", self.config.namespace, sandbox.0))
    }

    fn secret(&self, sandbox: core::SandboxId) -> NodeResult<GuestSecret> {
        let path = self.secret_path(sandbox);
        check_trusted_path(&path)?;
        let value: GuestSecret = serde_json::from_reader(File::open(path)?)?;
        if value.token.len() < 32
            || value.sha512.len() != 128
            || hash_token(&value.token)? != value.sha512
        {
            return Err("stored guest secret hash mismatch".into());
        }
        Ok(value)
    }

    fn vm_name(&self, vm: core::VmId) -> String {
        format!("vm-{}-{}", self.config.namespace, vm.0)
    }

    fn slot_path(&self, slot: core::NetSlotId) -> PathBuf {
        self.config
            .state_dir
            .join("slots")
            .join(format!("slot-{}-{}.json", self.config.namespace, slot.0))
    }

    fn fence_path(&self, sandbox: core::SandboxId) -> PathBuf {
        self.config
            .state_dir
            .join("fences")
            .join(format!("sb-{}-{}.json", self.config.namespace, sandbox.0))
    }

    fn runtime(&self, sandbox: core::SandboxId, vm: core::VmId) -> NodeResult<Runtime> {
        let secret = self.secret(sandbox)?;
        let id = u32::try_from(vm.0)?;
        if vm.0 > self.config.max_vm_id {
            return Err("node VM ID exceeds allocated UID range".into());
        }
        let uid = self
            .config
            .uid_base
            .checked_add(id)
            .ok_or("VM uid overflow")?;
        let vm_name = self.vm_name(vm);
        Ok(Runtime::new(RuntimeConfig {
            jailer: self.config.jailer.clone(),
            firecracker: self.config.firecracker.clone(),
            chroot_base: self.config.chroot_base.clone(),
            kernel: self.config.kernel.clone(),
            rootfs: self.config.rootfs.clone(),
            vm_uid: uid,
            vm_gid: uid,
            vcpu_count: self.config.vcpu_count,
            memory_mib: self.config.memory_mib,
            private_netns: Some(PathBuf::from(format!("/run/netns/{vm_name}"))),
            access_token_hash: Some(secret.sha512),
        })?)
    }

    fn slot(&self, slot: core::NetSlotId) -> NodeResult<SlotBinding> {
        let path = self.slot_path(slot);
        check_trusted_path(&path)?;
        let value: SlotBinding = serde_json::from_reader(File::open(path)?)?;
        if value.slot != slot {
            return Err("slot mapping identity mismatch".into());
        }
        Ok(value)
    }

    fn vm_for_workspace(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> NodeResult<core::VmId> {
        let mut found = None;
        for entry in fs::read_dir(self.config.state_dir.join("slots"))? {
            let entry = entry?;
            let binding: SlotBinding = serde_json::from_reader(File::open(entry.path())?)?;
            if binding.sandbox == sandbox && binding.generation == generation {
                if found.is_some() {
                    return Err("ambiguous VM for workspace".into());
                }
                found = Some(binding.vm);
            }
        }
        found.ok_or_else(|| "no VM for workspace".into())
    }

    fn fence(&self, sandbox: core::SandboxId) -> NodeResult<Fence> {
        let path = self.fence_path(sandbox);
        check_trusted_path(&path)?;
        Ok(serde_json::from_reader(File::open(path)?)?)
    }

    fn require_fence(
        &self,
        sandbox: core::SandboxId,
        epoch: core::Epoch,
        terminal: bool,
    ) -> NodeResult<()> {
        let fence = self.fence(sandbox)?;
        if fence.epoch != epoch || fence.terminal != terminal {
            return Err("host fence does not match command".into());
        }
        Ok(())
    }

    fn validate_action(&self, action: &WorkAction<core::EffectId>) -> NodeResult<()> {
        action.identity.validate_version()?;
        let command = action.command.command();
        let generation = match command {
            core::Command::PrepareVm { generation, .. }
            | core::Command::ConfigureNetwork { generation, .. }
            | core::Command::ActivateVm { generation, .. }
            | core::Command::ApplyFilePolicy { generation, .. }
            | core::Command::StopVm { generation, .. }
            | core::Command::CleanupVm { generation, .. }
            | core::Command::CleanupSlot { generation, .. } => Some(*generation),
            _ => None,
        };
        if action.identity.attempt != action.command.attempt
            || action.identity.epoch != command.epoch().unwrap_or(action.identity.epoch)
            || generation.is_some_and(|value| value != action.identity.generation)
            || action.identity.sandbox.as_str()
                != format!("sb-{}-{}", self.config.namespace, command.sandbox().0)
            || action.identity.resource != self.resource(command)?
        {
            return Err("host action identity mismatch".into());
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "closed V1 command dispatch keeps each physical precondition adjacent to its exact observation"
    )]
    fn perform(&self, action: &WorkAction<core::EffectId>) -> NodeResult<HostObservation> {
        self.validate_action(action)?;
        let command = action.command.command();
        let result = match command {
            core::Command::InstallFence {
                sandbox,
                epoch,
                terminal,
            } => {
                let path = self.fence_path(*sandbox);
                let next = Fence {
                    epoch: *epoch,
                    terminal: *terminal,
                };
                if path.exists() {
                    let prior = self.fence(*sandbox)?;
                    if prior.epoch > *epoch
                        || (prior.epoch == *epoch && prior.terminal != *terminal)
                        || (prior.terminal && !terminal)
                    {
                        return Err("stale or reopened host fence".into());
                    }
                    if prior != next {
                        replace_json(&path, &next)?;
                    }
                } else {
                    write_new_json(&path, &next)?;
                }
                ObservedResult::Fence(self.fence(*sandbox)?)
            }
            core::Command::PrepareVm {
                sandbox,
                epoch,
                generation,
                vm,
                slot,
                source,
            } => {
                if source.is_some() {
                    return Err("snapshot restore is outside V1".into());
                }
                self.require_fence(*sandbox, *epoch, false)?;
                let binding = SlotBinding {
                    sandbox: *sandbox,
                    generation: *generation,
                    vm: *vm,
                    slot: *slot,
                };
                write_new_json(&self.slot_path(*slot), &binding)?;
                let observation = self.runtime(*sandbox, *vm)?.launch(&self.vm_name(*vm))?;
                if observation.state != VmState::VmmRunning
                    || observation.pid == 0
                    || observation.start_time_ticks == 0
                {
                    return Err("VM has not entered running state".into());
                }
                ObservedResult::Prepared {
                    pid: observation.pid,
                    start_ticks: observation.start_time_ticks,
                }
            }
            core::Command::ConfigureNetwork {
                sandbox,
                epoch,
                generation,
                vm,
                slot,
                policy,
            } => self.observe_network(*sandbox, *epoch, *generation, *vm, *slot, policy)?,
            core::Command::ApplyFilePolicy {
                sandbox,
                epoch,
                generation,
                policy,
                ..
            } => self.observe_file_policy(*sandbox, *epoch, *generation, policy)?,
            core::Command::ActivateVm {
                sandbox,
                epoch,
                generation,
                vm,
            } => {
                self.require_fence(*sandbox, *epoch, false)?;
                if self.vm_for_workspace(*sandbox, *generation)? != *vm {
                    return Err("activation VM binding mismatch".into());
                }
                let observation = self.running(*sandbox, *vm)?;
                self.initialize_guest(*sandbox, *vm)?;
                let after = self.running(*sandbox, *vm)?;
                if after.pid != observation.pid
                    || after.start_time_ticks != observation.start_time_ticks
                {
                    return Err("VM changed while initializing guest service".into());
                }
                ObservedResult::Activated {
                    pid: after.pid,
                    start_ticks: after.start_time_ticks,
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
                let observation = self.runtime(*sandbox, *vm)?.stop(&self.vm_name(*vm))?;
                if observation.state != VmState::Stopped {
                    return Err("VM stop not observed".into());
                }
                ObservedResult::Stopped {
                    pid: observation.pid,
                    start_ticks: observation.start_time_ticks,
                }
            }
            core::Command::CleanupVm {
                sandbox,
                generation,
                vm,
            } => {
                if !self.fence(*sandbox)?.terminal
                    || self.vm_for_workspace(*sandbox, *generation)? != *vm
                {
                    return Err("cleanup requires terminal fence and exact VM".into());
                }
                let runtime = self.runtime(*sandbox, *vm)?;
                if runtime.observe(&self.vm_name(*vm))?.state != VmState::Stopped {
                    return Err("cannot clean live VM".into());
                }
                runtime.cleanup(&self.vm_name(*vm))?;
                require_absent(Path::new(&format!("/run/netns/{}", self.vm_name(*vm))))?;
                ObservedResult::VmCleaned
            }
            core::Command::CleanupSlot {
                sandbox,
                generation,
                slot,
            } => self.observe_closed_slot(*sandbox, *generation, *slot)?,
            _ => return Err("command is outside the V1 host executor".into()),
        };
        Ok(HostObservation {
            resource: action.identity.resource.clone(),
            command: command.clone(),
            result,
        })
    }

    fn observe_network(
        &self,
        sandbox: core::SandboxId,
        epoch: core::Epoch,
        generation: core::Generation,
        vm: core::VmId,
        slot: core::NetSlotId,
        policy: &core::NetworkPolicy,
    ) -> NodeResult<ObservedResult> {
        self.require_fence(sandbox, epoch, false)?;
        if *policy != core::NetworkPolicy::DenyAll {
            return Err("only private no-egress network policy is implemented".into());
        }
        self.require_slot(sandbox, generation, vm, slot)?;
        self.running(sandbox, vm)?;
        let name = self.vm_name(vm);
        let namespace = PathBuf::from(format!("/run/netns/{name}"));
        check_trusted_path(&namespace)?;
        let metadata = fs::symlink_metadata(&namespace)?;
        if !metadata.is_file() || metadata.uid() != 0 || metadata.ino() == 0 {
            return Err("private network namespace identity mismatch".into());
        }
        let output = Process::new("/usr/bin/ip")
            .args([
                "netns",
                "exec",
                &name,
                "/usr/bin/ip",
                "-4",
                "route",
                "show",
                "default",
            ])
            .output()?;
        if !output.status.success() || !output.stdout.is_empty() {
            return Err("private namespace has external route or cannot be checked".into());
        }
        Ok(ObservedResult::Network {
            namespace_inode: metadata.ino(),
        })
    }

    fn observe_file_policy(
        &self,
        sandbox: core::SandboxId,
        epoch: core::Epoch,
        generation: core::Generation,
        policy: &core::FilePolicyRef,
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
        let uid = self
            .config
            .uid_base
            .checked_add(u32::try_from(vm.0)?)
            .ok_or("VM uid overflow")?;
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

    fn observe_closed_slot(
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

    fn resource(&self, command: &core::Command) -> NodeResult<ResourceRef> {
        let (kind, prefix, number) = match command {
            core::Command::InstallFence { sandbox, .. } => (ResourceKind::Sandbox, "sb", sandbox.0),
            core::Command::PrepareVm { vm, .. }
            | core::Command::ActivateVm { vm, .. }
            | core::Command::StopVm { vm, .. }
            | core::Command::CleanupVm { vm, .. } => (ResourceKind::Vm, "vm", vm.0),
            core::Command::ConfigureNetwork { slot, .. }
            | core::Command::CleanupSlot { slot, .. } => {
                (ResourceKind::NetworkSlot, "slot", slot.0)
            }
            core::Command::ApplyFilePolicy { workspace, .. } => {
                (ResourceKind::Workspace, "fs", workspace.0)
            }
            _ => return Err("unsupported host resource command".into()),
        };
        Ok(ResourceRef {
            kind,
            id: ResourceId::try_from(format!("{prefix}-{}-{number}", self.config.namespace))?,
        })
    }

    fn require_slot(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        vm: core::VmId,
        slot: core::NetSlotId,
    ) -> NodeResult<()> {
        if self.slot(slot)?
            != (SlotBinding {
                sandbox,
                generation,
                vm,
                slot,
            })
        {
            return Err("slot binding mismatch".into());
        }
        Ok(())
    }

    fn running(
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

    fn initialize_guest(&self, sandbox: core::SandboxId, vm: core::VmId) -> NodeResult<()> {
        let name = self.vm_name(vm);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if Self::guest_status(&name, "GET", "/health", None)? == "204" {
                break;
            }
            if Instant::now() >= deadline {
                return Err("envd health deadline exceeded".into());
            }
            thread::sleep(Duration::from_millis(100));
        }
        let token = self.secret(sandbox)?.token;
        let init = serde_json::json!({"accessToken": token, "defaultUser": "root", "defaultWorkdir": "/root"});
        let body = serde_json::to_vec(&init)?;
        if Self::guest_status(&name, "POST", "/init", Some(&body))? != "204" {
            return Err("envd rejected authenticated initialization".into());
        }
        if Self::guest_file_status(&name, &token)? != "200" {
            return Err("envd authenticated file access not ready".into());
        }
        Ok(())
    }

    fn guest_status(
        name: &str,
        method: &str,
        path: &str,
        input: Option<&[u8]>,
    ) -> NodeResult<String> {
        let mut command = Process::new("/usr/bin/ip");
        command.args([
            "netns",
            "exec",
            name,
            "/usr/bin/curl",
            "-q",
            "--noproxy",
            "*",
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "5",
            "-X",
            method,
        ]);
        if input.is_some() {
            command.args([
                "-H",
                "Content-Type: application/json",
                "--data-binary",
                "@-",
            ]);
        }
        command.arg(format!("{ENVD_URL}{path}"));
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn()?;
        if let Some(input) = input {
            child
                .stdin
                .take()
                .ok_or("curl stdin unavailable")?
                .write_all(input)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Ok(String::new());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    fn guest_file_status(name: &str, token: &str) -> NodeResult<String> {
        let mut child = Process::new("/usr/bin/ip")
            .args([
                "netns",
                "exec",
                name,
                "/usr/bin/curl",
                "-q",
                "--noproxy",
                "*",
                "-s",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "--max-time",
                "5",
                "-K",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let config = format!(
            "url = \"{ENVD_URL}/files?path=%2Fetc%2Fpasswd\"\nheader = \"X-Access-Token: {token}\"\n"
        );
        child
            .stdin
            .take()
            .ok_or("curl stdin unavailable")?
            .write_all(config.as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err("envd file observation failed".into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
}

fn require_absent(path: &Path) -> NodeResult<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err("host resource still exists after cleanup".into()),
        Err(error) => Err(error.into()),
    }
}

fn hash_token(token: &str) -> NodeResult<String> {
    let mut child = Process::new("/usr/bin/sha512sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("sha512 stdin unavailable")?
        .write_all(token.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err("sha512 computation failed".into());
    }
    let output = String::from_utf8(output.stdout)?;
    let digest = output
        .split_whitespace()
        .next()
        .ok_or("empty sha512 digest")?;
    if digest.len() != 128 || !digest.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid sha512 digest".into());
    }
    Ok(digest.to_owned())
}

fn write_new_json<T: Serialize>(path: &Path, value: &T) -> NodeResult<()> {
    let parent = path.parent().ok_or("missing state parent")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

fn replace_json<T: Serialize>(path: &Path, value: &T) -> NodeResult<()> {
    let parent = path.parent().ok_or("missing state parent")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{require_absent, write_new_json};

    #[test]
    fn dangling_symlink_is_not_mistaken_for_cleaned_resource() {
        let directory = tempfile::TempDir::new().unwrap();
        let link = directory.path().join("dangling");
        std::os::unix::fs::symlink(directory.path().join("missing"), &link).unwrap();
        assert!(require_absent(&link).is_err());
        assert!(require_absent(&directory.path().join("missing")).is_ok());
    }

    #[test]
    fn new_host_record_is_published_atomically_without_overwrite() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("secret.json");
        let first = serde_json::json!({"token": "first"});
        write_new_json(&path, &first).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
            first
        );
        assert!(write_new_json(&path, &serde_json::json!({"token": "second"})).is_err());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
            first
        );
    }
}
