use rustix::process::geteuid;
use sandbox_controller_core as core;
use sandbox_runtime::{
    Observation, Runtime, RuntimeConfig, RuntimeError, VmState, check_trusted_path,
};
use sandproof_controller_service::{
    BoundedHostConfig, CreateOptions, HostSlotContext,
    template_registry::CandidateVerification,
    templates::{TemplateArtifact, TemplateBinding},
};
use sandproof_controller_service::{HostEvidence, WorkerTransportError};
use sandproof_protocol::{
    FailureCode, ResourceEvidence, ResourceId, ResourceKind, ResourceRef, TenantId, UnknownReason,
    WorkAction, WorkOutcome, WorkReceipt,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::{self, DirBuilder, File};
use std::io::{Read, Seek, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

type NodeResult<T> = Result<T, Box<dyn Error>>;
#[path = "child_process.rs"]
mod child_process;
#[path = "host_lease.rs"]
mod host_lease;
#[path = "host_slots.rs"]
mod host_slots;
#[path = "node_staging.rs"]
mod node_staging;
#[path = "storage_envelope.rs"]
mod storage_envelope;
pub use storage_envelope::HostMetadataEnvelope;

const NODE_STATE_DIRECTORIES: &[&str] = &["secrets", "fences", "slots", "startup"];

const ENVD_URL: &str = "http://172.16.0.2:49983";

/// Deployment inputs are fixed, root-owned host paths, not API request data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    pub namespace: String,
    pub state_dir: PathBuf,
    pub jailer: PathBuf,
    pub firecracker: PathBuf,
    pub chroot_base: PathBuf,
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
    /// Required physical mapping; permanent VM identities remain monotonic.
    pub bounded_host: BoundedHostConfig,
    pub vcpu_count: u8,
    pub memory_mib: u32,
    pub host_resources: Option<sandbox_runtime::HostResources>,
    /// V1 has one deployment-controlled managed-disk policy, not arbitrary
    /// user-supplied host paths or a directory ACL implementation.
    pub file_policy_digest: core::PolicyDigest,
}

impl NodeConfig {
    fn validate_deployment(&self) -> NodeResult<()> {
        host_slots::validate_limits(&self.bounded_host)?;
        if self.bounded_host.physical_storage.is_none()
            || self.bounded_host.console_policy
                != Some(sandproof_controller_service::ConsolePolicy::Discard)
        {
            return Err(
                "node requires complete physical storage and discard console policy".into(),
            );
        }
        Ok(())
    }
}

/// Read-only host inventory. A claim without a process record is explicitly
/// inconclusive; it must not be treated as a stopped VM or deleted by name.
#[derive(Clone, Debug)]
pub struct HostInventory {
    pub slots: Vec<ObservedSlot>,
    pub unbound_claims: Vec<String>,
    pub unbound_networks: Vec<String>,
    pub unbound_cgroups: Vec<String>,
}

#[derive(Clone, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "inventory carries independent physical evidence and one explicit unknown flag, not a second lifecycle state machine"
)]
pub struct ObservedSlot {
    pub owner: TenantId,
    pub sandbox: core::SandboxId,
    pub generation: core::Generation,
    pub vm: core::VmId,
    pub slot: core::NetSlotId,
    pub claim_present: bool,
    /// Exact registered identity is retained, but a surviving helper prevents
    /// a stable physical observation. This is never stopped/cleanup evidence.
    pub helper_pending: bool,
    pub aborted_unlaunched: bool,
    pub failed_prepare_reclaimed: bool,
    pub process: Option<Observation>,
    pub network_inode: Option<u64>,
    pub disk_inode: Option<u64>,
    pub cgroup_inode: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestSecret {
    token: String,
    sha512: String,
    #[serde(default)]
    env_vars: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    template: Option<TemplateBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotBinding {
    owner: TenantId,
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

fn fence_can_advance(prior: Fence, next: Fence) -> bool {
    prior.epoch <= next.epoch && (next.terminal || !prior.terminal)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ObservedResult {
    Fence(Fence),
    Prepared { pid: u32, start_ticks: u64 },
    Network { namespace_inode: u64 },
    Activated { pid: u32, start_ticks: u64 },
    FilePolicy { disk_inode: u64 },
    Stopped { pid: u32, start_ticks: u64 },
    PrepareAborted,
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
            | (core::Command::CleanupSlot { .. }, ObservedResult::SlotClosed)
            | (core::Command::AbortPrepare { .. }, ObservedResult::PrepareAborted) => true,
            _ => false,
        }
    }
}

/// The durable relay census reached a configured global or per-slot budget.
#[derive(Debug)]
pub struct RouteCapacityExceeded;
impl std::fmt::Display for RouteCapacityExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("durable data-route capacity exhausted")
    }
}
impl Error for RouteCapacityExceeded {}

pub struct LinuxWorker {
    config: NodeConfig,
    host_lease: host_lease::HostLease,
    last_error: Option<String>,
    validated_artifacts: Mutex<BTreeMap<PathBuf, ArtifactStamp>>,
}

#[derive(PartialEq, Eq)]
struct ArtifactStamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    sha256: String,
}

impl ArtifactStamp {
    fn read(artifact: &TemplateArtifact) -> NodeResult<Self> {
        check_trusted_path(&artifact.path)?;
        let metadata = fs::symlink_metadata(&artifact.path)?;
        if !metadata.is_file() || metadata.len() != artifact.size_bytes {
            return Err("template artifact type or size mismatch".into());
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
            sha256: artifact.sha256.clone(),
        })
    }
}

fn artifact_digest(mut input: impl Read) -> std::io::Result<String> {
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => digest.update(&buffer[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn artifact_file_digest(input: File) -> std::io::Result<String> {
    artifact_file_digest_with(input, |file, from| rustix::fs::seek(file, from))
}

fn artifact_file_digest_with(
    mut input: File,
    seek_extent: impl FnMut(&File, rustix::fs::SeekFrom) -> rustix::io::Result<u64>,
) -> std::io::Result<String> {
    let length = input.metadata()?.len();
    let result = if let Some(digest) = artifact_extent_digest(&mut input, length, seek_extent)? {
        format!("{:x}", digest.finalize())
    } else {
        // A filesystem may reject extent queries after partial progress.
        // Restart both the input and hash rather than accepting a prefix.
        input.rewind()?;
        artifact_digest(&mut input)?
    };
    if input.metadata()?.len() != length {
        return Err(std::io::Error::other(
            "artifact size changed during hashing",
        ));
    }
    Ok(result)
}

fn artifact_extent_digest(
    input: &mut File,
    length: u64,
    mut seek_extent: impl FnMut(&File, rustix::fs::SeekFrom) -> rustix::io::Result<u64>,
) -> std::io::Result<Option<Sha256>> {
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let zeros = vec![0_u8; 64 * 1024];
    let mut position = 0;
    while position < length {
        let data = match seek_extent(input, rustix::fs::SeekFrom::Data(position)) {
            Ok(data) if position <= data && data < length => data,
            Ok(_) => return Err(std::io::Error::other("invalid artifact data extent")),
            Err(rustix::io::Errno::NXIO) => length,
            Err(rustix::io::Errno::INVAL) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        // Holes contribute exactly the same zero bytes to SHA-256. Avoid
        // reading/materializing them in the filesystem page cache; do not
        // omit logical bytes or substitute an allocated-extents-only digest.
        while position < data {
            let count = usize::try_from((data - position).min(zeros.len() as u64))
                .expect("count is bounded by the buffer size");
            digest.update(&zeros[..count]);
            position += count as u64;
        }
        if position == length {
            break;
        }
        let end = match seek_extent(input, rustix::fs::SeekFrom::Hole(data)) {
            Ok(end) => end.min(length),
            Err(rustix::io::Errno::INVAL) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if end <= data {
            return Err(std::io::Error::other("invalid artifact hole extent"));
        }
        input.seek(std::io::SeekFrom::Start(data))?;
        while position < end {
            let count = usize::try_from((end - position).min(buffer.len() as u64))
                .expect("count is bounded by the buffer size");
            input.read_exact(&mut buffer[..count])?;
            digest.update(&buffer[..count]);
            position += count as u64;
        }
    }
    Ok(Some(digest))
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
            let aborted_unlaunched = claim_present
                && !helper_pending
                && self
                    .runtime(binding.sandbox, binding.vm)?
                    .was_reclaimed_unlaunched(&vm_name)?;
            let failed_prepare_reclaimed = claim_present
                && !helper_pending
                && self
                    .runtime(binding.sandbox, binding.vm)?
                    .was_reclaimed_stopped_prepare(&vm_name)?;
            let process = if claim_present && !helper_pending {
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
            // An extant namespace is not evidence that its firewall still
            // enforces the policy. A live (or unobservable) VM must retain
            // the exact pre-launch guard across controller/worker restarts.
            // Incomplete, definitely stopped launches are handled by the
            // separate exact-resource reclamation path instead.
            if process
                .as_ref()
                .is_some_and(|observed| observed.state != VmState::Stopped)
            {
                let observed_inode = self
                    .runtime(binding.sandbox, binding.vm)?
                    .observe_recorded_network(&vm_name)?;
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
                let observation = self
                    .runtime(binding.sandbox, binding.vm)?
                    .observe_jail(&vm_name)?;
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
                cgroup_inode: self
                    .runtime(binding.sandbox, binding.vm)?
                    .cgroup_inode(&vm_name)?,
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

    fn unbound_cgroups(&self, bound: &BTreeSet<String>) -> NodeResult<Vec<String>> {
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

    fn ensure_prepare_slot(
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

    fn observe_aborted_prepare(
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
        config.validate_deployment()?;
        if config.vcpu_count == 0 || config.memory_mib < 128 {
            return Err("invalid node resource limits".into());
        }
        if let Some(settings) = &config.host_resources {
            settings.validate(config.memory_mib)?;
        }
        let longest_socket = config
            .chroot_base
            .join(
                config
                    .firecracker
                    .file_name()
                    .ok_or("invalid firecracker name")?,
            )
            .join(format!("vm-{}-{}", config.namespace, u64::MAX))
            .join("root/api.socket");
        if longest_socket.as_os_str().as_bytes().len() >= 108 {
            return Err("Firecracker API socket path exceeds Linux limit".into());
        }
        let expected = host_slots::stored_lease_identity(&config)?;
        let host_lease = host_lease::HostLease::acquire(&config, expected.as_ref())?;
        for subdir in NODE_STATE_DIRECTORIES {
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
        let worker = Self {
            config,
            host_lease,
            last_error: None,
            validated_artifacts: Mutex::new(BTreeMap::new()),
        };
        worker.initialize_host_slots()?;
        Ok(worker)
    }

    /// Create a one-time root-only credential before accepting a sandbox.
    /// The caller must use a CSPRNG and not log or pass the token in argv.
    /// # Errors
    /// Rejects reuse, malformed tokens, or a hash computation failure.
    pub fn put_secret(&self, sandbox: core::SandboxId, token: &str) -> NodeResult<()> {
        self.put_launch_secret(sandbox, token, &BTreeMap::new())
    }

    /// Store the exact creation environment in the same root-only record as
    /// the token. This record exists before the VM can be reserved or booted.
    /// # Errors
    /// Rejects reuse, invalid variables or an unsafe credential location.
    pub fn put_launch_secret(
        &self,
        sandbox: core::SandboxId,
        token: &str,
        env_vars: &BTreeMap<String, String>,
    ) -> NodeResult<()> {
        self.put_bound_launch_secret(
            sandbox,
            token,
            &CreateOptions {
                env_vars: env_vars.clone(),
                ..CreateOptions::default()
            },
        )
    }

    /// Persist the exact admitted prebuilt template with the launch credential.
    /// # Errors
    /// Rejects invalid artifacts, credentials, configuration or record reuse.
    pub fn put_bound_launch_secret(
        &self,
        sandbox: core::SandboxId,
        token: &str,
        options: &CreateOptions,
    ) -> NodeResult<()> {
        self.require_occupied_sandbox(sandbox)?;
        options.validate()?;
        if let Some(template) = &options.template {
            self.validate_template(template)?;
        }
        let env_vars = &options.env_vars;
        if token.len() < 32
            || token.len() > 128
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err("invalid guest access token shape".into());
        }
        if env_vars.iter().any(|(key, value)| {
            key.is_empty() || key.contains('=') || key.contains('\0') || value.contains('\0')
        }) {
            return Err("invalid guest initialization variable".into());
        }
        let sha512 = hash_token(token)?;
        let value = GuestSecret {
            token: token.to_owned(),
            sha512,
            env_vars: env_vars.clone(),
            template: options.template.clone(),
        };
        let path = self.secret_path(sandbox);
        self.write_node_json(&path, &value)
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
                self.sync_node_record(&path)?;
                self.secret(sandbox)?;
                Ok(true)
            }
        }
    }

    /// A present but mismatched initialization record cannot be reused after
    /// a controller restart; that would start the VM with the wrong inputs.
    /// # Errors
    /// Rejects unreadable, malformed or mismatched stored data.
    pub fn has_launch_secret(
        &self,
        sandbox: core::SandboxId,
        env_vars: &BTreeMap<String, String>,
    ) -> NodeResult<bool> {
        if !self.has_secret(sandbox)? {
            return Ok(false);
        }
        if self.secret(sandbox)?.env_vars != *env_vars {
            return Err("stored guest initialization differs from admitted create".into());
        }
        Ok(true)
    }

    /// # Errors
    /// A mismatched descriptor is not permission to replace or relaunch a VM.
    pub fn has_bound_launch_secret(
        &self,
        sandbox: core::SandboxId,
        options: &CreateOptions,
    ) -> NodeResult<bool> {
        if !self.has_secret(sandbox)? {
            return Ok(false);
        }
        let stored = self.secret(sandbox)?;
        if stored.env_vars != options.env_vars || stored.template != options.template {
            return Err("stored template or initialization differs from admitted create".into());
        }
        Ok(true)
    }

    /// Check protected inputs and cgroup limits before controller admission.
    /// # Errors
    /// Rejects missing, modified or untrusted artifacts and unavailable host limits.
    pub fn validate_template(&self, template: &TemplateBinding) -> NodeResult<()> {
        if !template.is_valid() {
            return Err("invalid template descriptor".into());
        }
        self.validate_artifact(&template.kernel)?;
        self.validate_artifact(&template.rootfs)?;
        let mut resources = self
            .config
            .host_resources
            .clone()
            .ok_or("templates require explicit host resources")?;
        resources.memory_max_bytes = template.resources.host_memory_bytes;
        resources.validate(template.resources.memory_mib)?;
        Ok(())
    }

    /// Reuse completed registry hashes only for unchanged, trusted sealed files.
    /// Missing/stale evidence leaves the normal node cache/hash checks in place.
    pub fn cache_candidate_verification(
        &self,
        template: &TemplateBinding,
        verification: &CandidateVerification,
    ) {
        let Ok(mut cache) = self.validated_artifacts.lock() else {
            return;
        };
        for artifact in [&template.kernel, &template.rootfs] {
            let Ok(stamp) = ArtifactStamp::read(artifact) else {
                continue;
            };
            if verification.matches(artifact)
                && ArtifactStamp::read(artifact).is_ok_and(|current| current == stamp)
            {
                cache.insert(artifact.path.clone(), stamp);
            }
        }
    }

    fn validate_artifact(&self, artifact: &TemplateArtifact) -> NodeResult<()> {
        let stamp = ArtifactStamp::read(artifact)?;
        let mut cache = self
            .validated_artifacts
            .lock()
            .map_err(|_| "artifact cache lock failed")?;
        if cache.get(&artifact.path) == Some(&stamp) {
            return Ok(());
        }
        if artifact_file_digest(File::open(&artifact.path)?)? != artifact.sha256
            || ArtifactStamp::read(artifact)? != stamp
        {
            return Err("template artifact digest mismatch or changed during verification".into());
        }
        cache.insert(artifact.path.clone(), stamp);
        Ok(())
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

    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    fn sync_node_record(&self, path: &Path) -> NodeResult<()> {
        node_staging::cleanup(&self.config, path)?;
        File::open(path.parent().ok_or("node record parent")?)?.sync_all()?;
        Ok(())
    }

    fn write_node_json<T: Serialize>(&self, path: &Path, value: &T) -> NodeResult<()> {
        node_staging::write(&self.config, path, &serde_json::to_vec(value)?, false)
    }

    fn replace_node_json<T: Serialize>(&self, path: &Path, value: &T) -> NodeResult<()> {
        node_staging::write(&self.config, path, &serde_json::to_vec(value)?, true)
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
        let value: GuestSecret = serde_json::from_reader(File::open(&path)?)?;
        File::open(path.parent().ok_or("secret parent")?)?.sync_all()?;
        if value.token.len() < 32
            || value.sha512.len() != 128
            || hash_token(&value.token)? != value.sha512
            || value
                .template
                .as_ref()
                .is_some_and(|binding| !binding.is_valid())
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
        Ok(Runtime::for_existing(self.runtime_config(sandbox, vm)?)?)
    }

    fn runtime_config(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
    ) -> NodeResult<RuntimeConfig> {
        let secret = self.secret(sandbox)?;
        let id = self.vm_uid(vm)?;
        let vm_name = self.vm_name(vm);
        let mut kernel = self.config.kernel.clone();
        let mut rootfs = self.config.rootfs.clone();
        let mut vcpu_count = self.config.vcpu_count;
        let mut memory_mib = self.config.memory_mib;
        let mut host_resources = self.config.host_resources.clone();
        if let Some(template) = &secret.template {
            kernel.clone_from(&template.kernel.path);
            rootfs.clone_from(&template.rootfs.path);
            vcpu_count = template.resources.cpu_count;
            memory_mib = template.resources.memory_mib;
            host_resources
                .as_mut()
                .ok_or("templates require host limits")?
                .memory_max_bytes = template.resources.host_memory_bytes;
        }
        Ok(RuntimeConfig {
            console_policy: self
                .config
                .bounded_host
                .console_policy
                .map(|policy| match policy {
                    sandproof_controller_service::ConsolePolicy::Discard => {
                        sandbox_runtime::ConsolePolicy::Discard
                    }
                }),
            jailer: self.config.jailer.clone(),
            firecracker: self.config.firecracker.clone(),
            chroot_base: self.config.chroot_base.clone(),
            kernel,
            rootfs,
            vm_uid: id,
            vm_gid: id,
            vcpu_count,
            memory_mib,
            host_resources,
            private_netns: Some(PathBuf::from(format!("/run/netns/{vm_name}"))),
            access_token_hash: Some(secret.sha512),
            host_slot: Some(self.runtime_host_slot(self.physical_slot_for_vm(vm)?, vm)?),
        })
    }

    fn vm_uid(&self, vm: core::VmId) -> NodeResult<u32> {
        host_slots::position_uid(&self.config.bounded_host, self.physical_slot_for_vm(vm)?)
    }

    fn slot(&self, slot: core::NetSlotId) -> NodeResult<SlotBinding> {
        let path = self.slot_path(slot);
        check_trusted_path(&path)?;
        let value: SlotBinding = serde_json::from_reader(File::open(&path)?)?;
        File::open(path.parent().ok_or("slot parent")?)?.sync_all()?;
        if value.slot != slot {
            return Err("slot mapping identity mismatch".into());
        }
        Ok(value)
    }

    fn binding_for_workspace(
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

    fn vm_for_workspace(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> NodeResult<core::VmId> {
        Ok(self.binding_for_workspace(sandbox, generation)?.vm)
    }

    fn fence(&self, sandbox: core::SandboxId) -> NodeResult<Fence> {
        let path = self.fence_path(sandbox);
        check_trusted_path(&path)?;
        let value = serde_json::from_reader(File::open(&path)?)?;
        File::open(path.parent().ok_or("fence parent")?)?.sync_all()?;
        Ok(value)
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
        self.require_occupied_action(action)?;
        let command = action.command.command();
        let generation = match command {
            core::Command::PrepareVm { generation, .. }
            | core::Command::AbortPrepare { generation, .. }
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
                    if !fence_can_advance(prior, next) {
                        return Err("stale or reopened host fence".into());
                    }
                    if prior == next {
                        self.sync_node_record(&path)?;
                    } else {
                        self.replace_node_json(&path, &next)?;
                    }
                } else {
                    self.write_node_json(&path, &next)?;
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
                if let Some(template) = &self.secret(*sandbox)?.template {
                    self.validate_template(template)?;
                }
                self.vm_uid(*vm)?;
                self.require_fence(*sandbox, *epoch, false)?;
                let binding = SlotBinding {
                    owner: action.identity.owner.clone(),
                    sandbox: *sandbox,
                    generation: *generation,
                    vm: *vm,
                    slot: *slot,
                };
                self.write_node_json(&self.slot_path(*slot), &binding)?;
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
            core::Command::AbortPrepare {
                sandbox,
                epoch,
                generation,
                vm,
                slot,
            } => {
                self.require_fence(*sandbox, *epoch, true)?;
                self.ensure_prepare_slot(
                    &action.identity.owner,
                    *sandbox,
                    *generation,
                    *vm,
                    *slot,
                )?;
                let name = self.vm_name(*vm);
                let runtime = self.runtime(*sandbox, *vm)?;
                match runtime.observe(&name) {
                    Err(RuntimeError::MissingRecord) => {
                        self.reclaim_aborted_prepare(*sandbox, *generation, *vm, *slot)?;
                    }
                    Ok(Observation {
                        state: VmState::Stopped,
                        ..
                    }) => {
                        self.reclaim_stopped_prepare(*sandbox, *generation, *vm, *slot)?;
                    }
                    Ok(_) => {
                        // Runtime::stop uses a pidfd and the durable process
                        // start identity; no name-only or PID-only signal.
                        if runtime.stop(&name)?.state != VmState::Stopped {
                            return Err("exact VM exit not observed".into());
                        }
                        self.reclaim_stopped_prepare(*sandbox, *generation, *vm, *slot)?;
                    }
                    Err(error) => return Err(error.into()),
                }
                self.observe_aborted_prepare(*sandbox, *epoch, *generation, *vm, *slot)?
            }
            core::Command::ConfigureNetwork {
                sandbox,
                epoch,
                generation,
                vm,
                slot,
                policy,
            } => {
                self.require_fence(*sandbox, *epoch, false)?;
                self.require_slot(*sandbox, *generation, *vm, *slot)?;
                self.running(*sandbox, *vm)?;
                if !matches!(policy, core::NetworkPolicy::DenyAll) {
                    self.runtime(*sandbox, *vm)?
                        .configure_ipv4_uplink(&self.vm_name(*vm), policy)?;
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
            core::Command::ActivateVm {
                sandbox,
                epoch,
                generation,
                vm,
            } => {
                let after = self.complete_guest_activation(*sandbox, *epoch, *generation, *vm)?;
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
            core::Command::CleanupVm { .. } => {
                self.reclaim_incomplete_cleanup(action)?;
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

    fn observe_file_policy(
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

    fn resource(&self, command: &core::Command) -> NodeResult<ResourceRef> {
        let (kind, prefix, number) = match command {
            core::Command::InstallFence { sandbox, .. } => (ResourceKind::Sandbox, "sb", sandbox.0),
            core::Command::PrepareVm { vm, .. }
            | core::Command::AbortPrepare { vm, .. }
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
        let binding = self.slot(slot)?;
        self.require_occupied_binding(&binding)?;
        if binding.sandbox != sandbox
            || binding.generation != generation
            || binding.vm != vm
            || binding.slot != slot
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

    fn complete_guest_activation(
        &self,
        sandbox: core::SandboxId,
        epoch: core::Epoch,
        generation: core::Generation,
        vm: core::VmId,
    ) -> NodeResult<Observation> {
        self.require_fence(sandbox, epoch, false)?;
        if self.vm_for_workspace(sandbox, generation)? != vm {
            return Err("activation VM binding mismatch".into());
        }
        let before = self.running(sandbox, vm)?;
        self.initialize_guest(sandbox, vm)?;
        self.start_template_once(sandbox, vm, &before)?;
        let after = self.running(sandbox, vm)?;
        if after.pid != before.pid || after.start_time_ticks != before.start_time_ticks {
            return Err("VM changed while initializing guest service".into());
        }
        Ok(after)
    }

    fn initialize_guest(&self, sandbox: core::SandboxId, vm: core::VmId) -> NodeResult<()> {
        let name = self.vm_name(vm);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if self.guest_status(&name, "GET", "/health", None)? == "204" {
                break;
            }
            if Instant::now() >= deadline {
                return Err("envd health deadline exceeded".into());
            }
            thread::sleep(Duration::from_millis(100));
        }
        let secret = self.secret(sandbox)?;
        let startup = secret
            .template
            .as_ref()
            .and_then(|value| value.startup.as_ref());
        let init = crate::startup::init_payload(&secret.token, &secret.env_vars, startup);
        let body = serde_json::to_vec(&init)?;
        if self.guest_status(&name, "POST", "/init", Some(&body))? != "204" {
            return Err("envd rejected authenticated initialization".into());
        }
        // A debug-only crash window after envd accepted the request but
        // before the host can record authenticated readiness.
        #[cfg(debug_assertions)]
        if std::env::var("SANDPROOF_TEST_EXIT_AFTER_GUEST_INIT")
            .ok()
            .as_deref()
            == Some("activate_vm")
        {
            std::process::exit(88);
        }
        if self.guest_file_status(&name, &secret.token)? != "200" {
            return Err("envd authenticated file access not ready".into());
        }
        Ok(())
    }

    /// Intent is durable before dispatch. Recovery reuses confirmed dispatch,
    /// never repeats an unknown one. Start confirmation is not app readiness.
    fn start_template_once(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        observation: &Observation,
    ) -> NodeResult<()> {
        let secret = self.secret(sandbox)?;
        let Some(template) = secret.template else {
            return Ok(());
        };
        let Some(startup) = &template.startup else {
            return Ok(());
        };
        if startup.argv.is_empty() {
            return Ok(());
        }
        let path = self
            .config
            .state_dir
            .join("startup")
            .join(format!("{}.json", self.vm_name(vm)));
        let mut expected = crate::startup::StartupRecord {
            sandbox: sandbox.0,
            vm: vm.0,
            vmm_pid: observation.pid,
            start_ticks: observation.start_time_ticks,
            template: template.clone(),
            env_vars: secret.env_vars,
            guest_pid: None,
        };
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                check_trusted_path(&path)?;
                self.sync_node_record(&path)?;
                let stored: crate::startup::StartupRecord =
                    serde_json::from_reader(File::open(&path)?)?;
                return stored.confirmed_for(&expected);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.write_node_json(&path, &expected)?;
        let pid = self.dispatch_startup(&self.vm_name(vm), &secret.token, startup)?;
        #[cfg(debug_assertions)]
        if std::env::var("SANDPROOF_TEST_EXIT_AFTER_TEMPLATE_START")
            .ok()
            .as_deref()
            == Some("activate_vm")
        {
            std::process::exit(88);
        }
        expected.guest_pid = Some(pid);
        self.replace_node_json(&path, &expected)
    }

    fn dispatch_startup(
        &self,
        name: &str,
        token: &str,
        startup: &sandproof_controller_service::templates::TemplateStartup,
    ) -> NodeResult<u32> {
        // Neither the credential nor tenant argv enters host process argv.
        // Bodies are retired by the exact per-incarnation manifest.
        let payload = crate::startup::start_request(startup)?;
        let durable_path = self
            .config
            .state_dir
            .join("startup")
            .join(format!("{name}.request"));
        node_staging::write(&self.config, &durable_path, &payload, false)?;
        let (gate, mut command) = self.namespace_command(name)?;
        command
            .args([
                "netns",
                "exec",
                name,
                "/usr/bin/curl",
                "-q",
                "--noproxy",
                "*",
                "-fsN",
                "--max-time",
                "5",
                "-K",
                "-",
                "-H",
                "Content-Type: application/connect+json",
                "-H",
                "Connect-Protocol-Version: 1",
                "--data-binary",
            ])
            .arg(format!("@{}", durable_path.display()))
            .arg(format!("{ENVD_URL}/process.Process/Start"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        let mut child = self.spawn_namespace_command(name, &gate, &mut command)?;
        let result = (|| {
            child
                .stdin
                .take()
                .ok_or("curl stdin unavailable")?
                .write_all(format!("header = \"X-Access-Token: {token}\"\n").as_bytes())?;
            crate::startup::read_started(child.stdout.take().ok_or("curl stdout unavailable")?)
        })();
        // envd deliberately detaches child lifetime from the HTTP stream.
        // Close the observation stream; never send a guest termination here.
        // Tracked launch execs curl in the recorded SAME PID; stopping this
        // exact child cannot orphan a supervisor's separate descendant.
        let _ = child.kill();
        child.wait()?;
        gate.finish_tracked()?;
        result
    }

    fn namespace_command(
        &self,
        name: &str,
    ) -> NodeResult<(sandbox_runtime::RuntimeHelperGuard, Process)> {
        let number = name
            .strip_prefix(&format!("vm-{}-", self.config.namespace))
            .ok_or("namespace helper identity")?
            .parse::<u64>()?;
        let vm = core::VmId(number);
        if self.vm_name(vm) != name {
            return Err("namespace helper identity not canonical".into());
        }
        let context = self.runtime_host_slot(self.physical_slot_for_vm(vm)?, vm)?;
        let guard = context.helper_gate.try_acquire_shared(Some(&context))?;
        let mut command = Process::new("/usr/bin/ip");
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        Ok((guard, command))
    }

    fn spawn_namespace_command(
        &self,
        name: &str,
        guard: &sandbox_runtime::RuntimeHelperGuard,
        command: &mut Process,
    ) -> NodeResult<child_process::OwnedChild> {
        let vm = core::VmId(
            name.strip_prefix(&format!("vm-{}-", self.config.namespace))
                .ok_or("namespace helper identity")?
                .parse::<u64>()?,
        );
        let context = self.runtime_host_slot(self.physical_slot_for_vm(vm)?, vm)?;
        let args = command
            .get_args()
            .map(|arg| arg.to_str().ok_or("non-UTF8 namespace helper argument"))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(child_process::OwnedChild(guard.spawn_tracked(
            &context,
            "/usr/bin/ip",
            &args,
        )?))
    }

    fn guest_status(
        &self,
        name: &str,
        method: &str,
        path: &str,
        input: Option<&[u8]>,
    ) -> NodeResult<String> {
        let (gate, mut command) = self.namespace_command(name)?;
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
        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut child = self.spawn_namespace_command(name, &gate, &mut command)?;
        if let Some(input) = input {
            child
                .stdin
                .take()
                .ok_or("curl stdin unavailable")?
                .write_all(input)?;
        }
        let output = child.wait_output()?;
        gate.finish_tracked()?;
        if !output.status.success() {
            return Ok(String::new());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    fn guest_file_status(&self, name: &str, token: &str) -> NodeResult<String> {
        let (gate, mut command) = self.namespace_command(name)?;
        command
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
            .stdout(Stdio::piped());
        let mut child = self.spawn_namespace_command(name, &gate, &mut command)?;
        let config = format!(
            "url = \"{ENVD_URL}/files?path=%2Fetc%2Fpasswd\"\nheader = \"X-Access-Token: {token}\"\n"
        );
        child
            .stdin
            .take()
            .ok_or("curl stdin unavailable")?
            .write_all(config.as_bytes())?;
        let output = child.wait_output()?;
        gate.finish_tracked()?;
        if !output.status.success() {
            return Err("envd file observation failed".into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
}

fn helper_observation_pending(result: Result<(), RuntimeError>) -> NodeResult<bool> {
    match result {
        Ok(()) => Ok(false),
        Err(RuntimeError::InvalidConfiguration(_)) => Ok(true),
        Err(error) => Err(error.into()),
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
    let child = Process::new("/usr/bin/sha512sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut child = child_process::OwnedChild(child);
    child
        .stdin
        .take()
        .ok_or("sha512 stdin unavailable")?
        .write_all(token.as_bytes())?;
    let output = child.wait_output()?;
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

#[cfg(test)]
mod tests {
    use super::{
        Fence, NodeResult, RuntimeError, artifact_digest, artifact_file_digest,
        artifact_file_digest_with, fence_can_advance, helper_observation_pending, require_absent,
    };
    use std::io::{Cursor, Error, ErrorKind, Read, Seek, SeekFrom, Write};

    #[test]
    fn artifact_digest_includes_sparse_holes_cross_buffer_data_and_trailing_bytes() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"prefix").unwrap();
        file.seek(SeekFrom::Start(128 * 1024 - 1)).unwrap();
        file.write_all(b"edge").unwrap();
        file.set_len(192 * 1024 + 7).unwrap();
        file.rewind().unwrap();
        let expected = "f7c9f35110eefc8d8c8ded4d76ac5ffc1fbcf5849e0dc96549302f774f0a7b56";
        assert_eq!(artifact_digest(&mut file).unwrap(), expected);
        assert_eq!(
            artifact_file_digest(file.try_clone().unwrap()).unwrap(),
            expected
        );
        let mut dense = vec![0_u8; 192 * 1024 + 7];
        dense[..6].copy_from_slice(b"prefix");
        dense[128 * 1024 - 1..128 * 1024 + 3].copy_from_slice(b"edge");
        assert_eq!(artifact_digest(Cursor::new(dense)).unwrap(), expected);
        file.seek(SeekFrom::End(-1)).unwrap();
        file.write_all(b"x").unwrap();
        file.rewind().unwrap();
        assert_ne!(artifact_digest(file).unwrap(), expected);
    }

    #[test]
    fn file_digest_handles_empty_all_hole_and_dense_zero_files() {
        for (length, dense) in [(0, false), (1, false), (131_079, false), (131_079, true)] {
            let mut file = tempfile::tempfile().unwrap();
            let bytes = vec![0; length];
            if dense {
                file.write_all(&bytes).unwrap();
            } else {
                file.set_len(length as u64).unwrap();
            }
            assert_eq!(
                artifact_file_digest(file).unwrap(),
                artifact_digest(Cursor::new(bytes)).unwrap()
            );
        }
    }

    #[test]
    fn unsupported_extent_queries_restart_hash_even_after_data_was_read() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"first").unwrap();
        file.seek(SeekFrom::Start(128 * 1024 + 3)).unwrap();
        file.write_all(b"last").unwrap();
        file.set_len(256 * 1024 + 7).unwrap();
        file.rewind().unwrap();
        let expected = artifact_digest(&mut file).unwrap();
        for fail_on in [1, 2, 3] {
            let mut queries = 0;
            let result = artifact_file_digest_with(file.try_clone().unwrap(), |input, from| {
                queries += 1;
                if queries == fail_on {
                    Err(rustix::io::Errno::INVAL)
                } else {
                    rustix::fs::seek(input, from)
                }
            })
            .unwrap();
            assert_eq!(queries, fail_on);
            assert_eq!(result, expected);
        }
    }

    #[test]
    fn invalid_extents_and_real_seek_errors_do_not_produce_a_digest() {
        use rustix::io::Errno;
        for replies in [
            vec![Ok(8)],
            vec![Ok(0), Ok(0)],
            vec![Ok(0), Ok(1), Ok(0)],
            vec![Err(Errno::IO)],
            vec![Ok(0), Err(Errno::IO)],
        ] {
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(b"12345678").unwrap();
            let mut replies = replies.into_iter();
            assert!(artifact_file_digest_with(file, |_, _| replies.next().unwrap()).is_err());
            assert!(replies.next().is_none());
        }
    }

    #[test]
    fn file_digest_rejects_truncation_and_growth_during_extent_queries() {
        for truncate in [false, true] {
            let mut file = tempfile::tempfile().unwrap();
            file.write_all(b"12345678").unwrap();
            let result = artifact_file_digest_with(file, |input, from| {
                let value = rustix::fs::seek(input, from)?;
                if matches!(from, rustix::fs::SeekFrom::Hole(_)) {
                    input.set_len(if truncate { 2 } else { 9 }).unwrap();
                }
                Ok(value)
            });
            let error = result.unwrap_err();
            if truncate {
                assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
            } else {
                assert!(error.to_string().contains("size changed"));
            }
        }
    }

    struct InterruptedReader {
        bytes: Cursor<Vec<u8>>,
        next_interrupt: bool,
        fail_at_end: bool,
    }

    impl Read for InterruptedReader {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            self.next_interrupt = !self.next_interrupt;
            if self.next_interrupt {
                return Err(ErrorKind::Interrupted.into());
            }
            if self.bytes.position() == self.bytes.get_ref().len() as u64 && self.fail_at_end {
                return Err(Error::from(ErrorKind::PermissionDenied));
            }
            let limit = output.len().min(1);
            self.bytes.read(&mut output[..limit])
        }
    }

    #[test]
    fn artifact_digest_retries_interrupted_short_reads_without_losing_bytes() {
        let input = InterruptedReader {
            bytes: Cursor::new(b"abc".to_vec()),
            next_interrupt: false,
            fail_at_end: false,
        };
        assert_eq!(
            artifact_digest(input).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn artifact_digest_never_accepts_a_prefix_when_a_read_fails() {
        let input = InterruptedReader {
            bytes: Cursor::new(b"abc".to_vec()),
            next_interrupt: false,
            fail_at_end: true,
        };
        assert_eq!(
            artifact_digest(input).unwrap_err().kind(),
            ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn fence_may_close_in_same_epoch_but_never_reopen_or_go_back() {
        let open = Fence {
            epoch: 1,
            terminal: false,
        };
        let closed = Fence {
            epoch: 1,
            terminal: true,
        };
        assert!(fence_can_advance(open, closed));
        assert!(fence_can_advance(closed, closed));
        assert!(!fence_can_advance(closed, open));
        assert!(!fence_can_advance(
            closed,
            Fence {
                epoch: 0,
                terminal: true
            }
        ));
    }

    #[test]
    fn dangling_symlink_is_not_mistaken_for_cleaned_resource() {
        let directory = tempfile::TempDir::new().unwrap();
        let link = directory.path().join("dangling");
        std::os::unix::fs::symlink(directory.path().join("missing"), &link).unwrap();
        assert!(require_absent(&link).is_err());
        assert!(require_absent(&directory.path().join("missing")).is_ok());
    }

    #[test]
    fn known_live_helper_parks_only_its_slot_but_corruption_rejects_inventory() {
        let observations = [
            Ok(()),
            Err(RuntimeError::InvalidConfiguration(
                "previous tracked helper is still alive",
            )),
            Ok(()),
        ];
        let pending = observations
            .into_iter()
            .map(helper_observation_pending)
            .collect::<NodeResult<Vec<_>>>()
            .unwrap();
        assert_eq!(pending, vec![false, true, false]);
        let corrupt = [
            Ok(()),
            Err(RuntimeError::IdentityMismatch("tracked helper context")),
            Ok(()),
        ];
        assert!(
            corrupt
                .into_iter()
                .map(helper_observation_pending)
                .collect::<NodeResult<Vec<_>>>()
                .is_err()
        );
    }
}

#[cfg(test)]
mod publication_tests;
