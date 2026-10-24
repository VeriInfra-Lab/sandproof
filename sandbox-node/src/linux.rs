use rustix::process::geteuid;
use sandbox_controller_core as core;
use sandbox_runtime::{
    Observation, Runtime, RuntimeConfig, RuntimeError, RuntimeObservationConfig,
    RuntimePhysicalObserver, VmState, check_trusted_path,
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
use sha2::{Digest, Sha256, Sha512};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs::{self, DirBuilder, File};
use std::io::{Read, Seek};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

type NodeResult<T> = Result<T, Box<dyn Error>>;
mod capture;
mod capture_profile;
mod capture_stop;
mod capture_worker;
pub use capture_worker::{
    CAPTURE_WORKER_INPUT_BYTES, CAPTURE_WORKER_OUTPUT_BYTES, PreparedCaptureWorker,
    run_capture_worker,
};
mod full_capture;
pub(crate) use capture_profile::ProtectedCaptureProfile;
pub use capture_profile::ProtectedCaptureSettings;
pub(crate) use full_capture::observe_committed_capture;
mod credentials;
mod decisions;
mod execution;
mod guest_transport;
mod identity;
mod inventory;
mod observation;
mod readonly;
mod recovery;
mod source_observation;
mod source_process;
use decisions::fence_can_advance;
pub use execution::ExecutionReport;
pub use readonly::LinuxObservation;
#[path = "host_lease.rs"]
mod host_lease;
#[path = "host_slots.rs"]
mod host_slots;
#[path = "launch_observation.rs"]
mod launch_observation;
pub use launch_observation::{CredentialFault, LaunchCredentialStatus};
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ObservedResult {
    Fence(Fence),
    Prepared {
        pid: u32,
        start_ticks: u64,
        boot_id: String,
    },
    Network {
        namespace_inode: u64,
    },
    Activated {
        pid: u32,
        start_ticks: u64,
        boot_id: String,
    },
    Frozen {
        pid: u32,
        start_ticks: u64,
        boot_id: String,
    },
    SnapshotSealed {
        manifest_digest: sandproof_protocol::lifecycle::ContentDigest,
        material_bytes: u64,
    },
    SnapshotCommitted {
        manifest_digest: sandproof_protocol::lifecycle::ContentDigest,
        material_bytes: u64,
    },
    FilePolicy {
        disk_inode: u64,
    },
    Stopped {
        pid: u32,
        start_ticks: u64,
    },
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
        decisions::evidence_matches(self, command)
    }

    fn capture_manifest_digest(
        &self,
        command: &core::Command,
    ) -> Option<sandproof_protocol::lifecycle::ContentDigest> {
        if !decisions::evidence_matches(self, command) {
            return None;
        }
        match &self.result {
            ObservedResult::SnapshotSealed {
                manifest_digest, ..
            }
            | ObservedResult::SnapshotCommitted {
                manifest_digest, ..
            } => Some(*manifest_digest),
            _ => None,
        }
    }

    fn capture_material_bytes(&self, command: &core::Command) -> Option<u64> {
        if !decisions::evidence_matches(self, command) {
            return None;
        }
        match &self.result {
            ObservedResult::SnapshotSealed { material_bytes, .. }
            | ObservedResult::SnapshotCommitted { material_bytes, .. } => Some(*material_bytes),
            _ => None,
        }
    }

    fn source_process_identity(
        &self,
        command: &core::Command,
        expected_resource: &ResourceRef,
    ) -> Option<sandproof_controller_service::capture::SourceProcessIdentity> {
        self.recorded_source_identity(command, expected_resource)
            .map(
                |source| sandproof_controller_service::capture::SourceProcessIdentity {
                    pid: source.pid,
                    start_time_ticks: source.start_time_ticks,
                    boot_id: source.boot_id,
                },
            )
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
        validate_worker_configuration(&config)?;
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
        let worker = Self {
            config,
            host_lease,
            validated_artifacts: Mutex::new(BTreeMap::new()),
        };
        worker.initialize_host_slots()?;
        Ok(worker)
    }

    pub(crate) fn host_bootstrap_inputs(&self) -> crate::host_epoch::Inputs<'_> {
        crate::host_epoch::Inputs {
            namespace: &self.config.namespace,
            firecracker: &self.config.firecracker,
            jailer: &self.config.jailer,
        }
    }

    /// Read the existing protected bootstrap and verify its actual host facts.
    /// Only explicit service initialization may persist this registration;
    /// restarting serve must compare the original stored registration instead.
    /// # Errors
    /// Rejects changed host facts, unsafe or replaced files and foreign bindings.
    pub fn host_epoch_registration(
        &self,
        source: &crate::ProtectedHostEpochSource,
        node: &sandproof_protocol::internal::NodeIdentity,
    ) -> Result<sandproof_controller_service::capture::HostEpochRegistration, RuntimeError> {
        source.registration(node, self.host_bootstrap_inputs())
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
}

// One immutable deployment validator for activating and observation-only
// construction. Auditing never bypasses the original physical constraints.
fn validate_worker_configuration(config: &NodeConfig) -> NodeResult<()> {
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
    for binary in ["/usr/bin/ip", "/usr/bin/curl"] {
        check_trusted_path(Path::new(binary))?;
    }
    Ok(())
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

fn hash_token(token: &str) -> String {
    // envd expects the same lowercase, 128-hex SHA-512 digest as sha512sum.
    // No subprocess or extra executable identity is needed for this pure value.
    format!("{:x}", Sha512::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn token_hash_preserves_envd_sha512_format_without_external_programs() {
        for (input, expected) in [
            (
                "",
                "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
            ),
            (
                "abc",
                "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
            ),
        ] {
            assert_eq!(super::hash_token(input), expected);
        }
    }
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
mod test_support;

#[cfg(test)]
mod publication_tests;
