use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::chown;
use rustix::process::{Gid, Pid, PidfdFlags, Signal, Uid, pidfd_open, pidfd_send_signal};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::error::Error;
use std::fmt::{self, Display};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const API_SOCKET: &str = "api.socket";
const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum RuntimeError {
    Io(io::Error),
    KvmUnavailable(io::Error),
    Json(serde_json::Error),
    InvalidConfiguration(&'static str),
    InvalidId,
    AlreadyClaimed,
    MissingRecord,
    IdentityMismatch(&'static str),
    TimedOut(&'static str),
    HostCommand {
        operation: &'static str,
        stderr: String,
    },
    Api {
        status: u16,
        body: String,
    },
    Protocol(&'static str),
}

impl Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "host I/O: {error}"),
            Self::KvmUnavailable(error) => write!(f, "Linux/KVM is unavailable: {error}"),
            Self::Json(error) => write!(f, "record/response JSON: {error}"),
            Self::InvalidConfiguration(reason) => {
                write!(f, "invalid runtime configuration: {reason}")
            }
            Self::InvalidId => f.write_str("invalid jailer resource ID"),
            Self::AlreadyClaimed => {
                f.write_str("resource ID is already claimed; observe before retrying")
            }
            Self::MissingRecord => {
                f.write_str("resource claim has no process record; reconcile manually")
            }
            Self::IdentityMismatch(reason) => write!(
                f,
                "observed process differs from recorded resource ({reason}); no signal sent"
            ),
            Self::TimedOut(action) => write!(f, "timed out waiting for {action}"),
            Self::HostCommand { operation, stderr } => {
                write!(f, "host network {operation} failed: {stderr}")
            }
            Self::Api { status, body } => write!(f, "Firecracker API returned {status}: {body}"),
            Self::Protocol(reason) => write!(f, "invalid Firecracker/proc response: {reason}"),
        }
    }
}

impl Error for RuntimeError {}

impl From<io::Error> for RuntimeError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for RuntimeError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    pub jailer: PathBuf,
    pub firecracker: PathBuf,
    pub chroot_base: PathBuf,
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
    /// A dedicated, non-root uid/gid. The deployment owner must allocate
    /// distinct IDs for mutually untrusted VMs.
    pub vm_uid: u32,
    pub vm_gid: u32,
    pub vcpu_count: u8,
    pub memory_mib: u32,
    /// A short-lived SSH command should use false and stop before it exits.
    /// Long-lived services may use false too; daemon mode suppresses serial logs.
    pub daemonize: bool,
    /// Per-VM namespace handle at `/run/netns/<resource-id>`. The runtime
    /// creates its private TAP and link when launching and removes it after
    /// the exact VM process has stopped.
    pub private_netns: Option<PathBuf>,
    /// SHA-512 hex digest of the envd access token. Only used with a private
    /// namespace; the raw token must never enter Firecracker metadata.
    pub access_token_hash: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VmState {
    Starting,
    /// Firecracker reports Running; guest-service readiness is separate.
    VmmRunning,
    /// The process exists, but its management API cannot be observed.
    ApiUnavailable,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub resource_id: String,
    pub pid: u32,
    pub start_time_ticks: u64,
    pub state: VmState,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessRecord {
    resource_id: String,
    pid: u32,
    start_time_ticks: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NetworkRecord {
    resource_id: String,
    namespace_inode: u64,
}

pub struct Runtime {
    config: RuntimeConfig,
}

impl Runtime {
    /// Construct an executor using operator-selected, root-owned inputs.
    ///
    /// # Errors
    /// Rejects missing or untrusted filesystem paths and invalid limits.
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        if rustix::process::geteuid().as_raw() != 0 {
            return Err(RuntimeError::InvalidConfiguration(
                "jailer controller must run as root",
            ));
        }
        check_kvm(Path::new("/dev/kvm"))?;
        if config.vm_uid == 0
            || config.vm_gid == 0
            || config.vcpu_count == 0
            || config.memory_mib < 128
        {
            return Err(RuntimeError::InvalidConfiguration(
                "invalid guest uid/gid or machine size",
            ));
        }
        for path in [
            &config.jailer,
            &config.firecracker,
            &config.chroot_base,
            &config.kernel,
            &config.rootfs,
        ] {
            check_trusted_path(path)?;
        }
        if let Some(namespace) = &config.private_netns {
            if namespace.parent() != Some(Path::new("/run/netns")) {
                return Err(RuntimeError::InvalidConfiguration(
                    "private network namespace must live under /run/netns",
                ));
            }
            check_trusted_path(Path::new("/run"))?;
            check_trusted_path(Path::new("/usr/bin/ip"))?;
            let hash =
                config
                    .access_token_hash
                    .as_deref()
                    .ok_or(RuntimeError::InvalidConfiguration(
                        "MMDS access token hash is required",
                    ))?;
            if hash.len() != 128 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(RuntimeError::InvalidConfiguration(
                    "MMDS access token hash must be SHA-512 hex",
                ));
            }
        } else if config.access_token_hash.is_some() {
            return Err(RuntimeError::InvalidConfiguration(
                "MMDS token hash requires a private network",
            ));
        }
        if !config.jailer.is_file()
            || !config.firecracker.is_file()
            || !config.kernel.is_file()
            || !config.rootfs.is_file()
            || !config.chroot_base.is_dir()
        {
            return Err(RuntimeError::InvalidConfiguration(
                "expected binaries/images and chroot directory",
            ));
        }
        if config
            .firecracker
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_none()
        {
            return Err(RuntimeError::InvalidConfiguration(
                "Firecracker executable name must be UTF-8",
            ));
        }
        Ok(Self { config })
    }

    /// Claim a fresh resource ID, launch jailer, stage a private disk and boot.
    /// A failed/unknown launch leaves its claim for explicit reconciliation.
    ///
    /// # Errors
    /// Preserves partial resources when a launch phase fails.
    pub fn launch(&self, id: &str) -> Result<Observation, RuntimeError> {
        validate_id(id)?;
        self.validate_network_id(id)?;
        let claim = self.claim_resource(id)?;
        if self.config.private_netns.is_some() {
            self.prepare_private_network(id, &claim)?;
        }
        let record = self.spawn_jailer(id, &claim)?;
        self.stage_and_boot(id, record.pid)?;
        let observation = self.observe(id)?;
        if observation.state != VmState::VmmRunning {
            return Err(RuntimeError::Protocol("VM did not enter Running"));
        }
        Ok(observation)
    }

    fn claim_resource(&self, id: &str) -> Result<PathBuf, RuntimeError> {
        let claims = self.config.chroot_base.join(".claims");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&claims)?;
        if fs::symlink_metadata(&claims)?.uid() != 0 {
            return Err(RuntimeError::InvalidConfiguration(
                "claim directory must be root-owned",
            ));
        }
        fs::set_permissions(&claims, fs::Permissions::from_mode(0o700))?;
        let claim = claims.join(id);
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&claim)
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    RuntimeError::AlreadyClaimed
                } else {
                    RuntimeError::Io(error)
                }
            })?;
        Ok(claim)
    }

    fn spawn_jailer(&self, id: &str, claim: &Path) -> Result<ProcessRecord, RuntimeError> {
        let output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(claim.join("console.log"))?;
        let stderr = output.try_clone()?;
        let mut command = Command::new(&self.config.jailer);
        command
            .arg("--id")
            .arg(id)
            .arg("--exec-file")
            .arg(&self.config.firecracker)
            .arg("--uid")
            .arg(self.config.vm_uid.to_string())
            .arg("--gid")
            .arg(self.config.vm_gid.to_string())
            .arg("--chroot-base-dir")
            .arg(&self.config.chroot_base)
            .arg("--cgroup-version")
            .arg("2");
        if let Some(namespace) = &self.config.private_netns {
            command.arg("--netns").arg(namespace);
        }
        if self.config.daemonize {
            command.arg("--daemonize");
        }
        let mut child = command
            .arg("--")
            .arg("--api-sock")
            .arg(format!("/{API_SOCKET}"))
            .stdin(Stdio::null())
            .stdout(Stdio::from(output))
            .stderr(Stdio::from(stderr))
            .spawn()?;
        let pid = if self.config.daemonize {
            let pid_file = self
                .root_path(id)
                .join(format!("{}.pid", self.firecracker_name()));
            let deadline = Instant::now() + START_TIMEOUT;
            let pid = loop {
                if let Ok(contents) = fs::read_to_string(&pid_file) {
                    break contents
                        .trim()
                        .parse()
                        .map_err(|_| RuntimeError::Protocol("invalid jailer PID file"))?;
                }
                if Instant::now() >= deadline {
                    return Err(RuntimeError::TimedOut("jailer PID file"));
                }
                thread::sleep(Duration::from_millis(20));
            };
            let _ = child.wait()?;
            pid
        } else {
            child.id()
        };
        let start_time_ticks = read_proc(pid)?
            .ok_or(RuntimeError::Protocol("new Firecracker PID vanished"))?
            .start_time_ticks;
        let record = ProcessRecord {
            resource_id: id.to_owned(),
            pid,
            start_time_ticks,
        };
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(claim.join("record.json"))?;
        serde_json::to_writer(&mut file, &record)?;
        file.flush()?;
        file.sync_all()?;
        File::open(claim)?.sync_all()?;
        Ok(record)
    }

    fn stage_and_boot(&self, id: &str, pid: u32) -> Result<(), RuntimeError> {
        let socket = self.socket_path(id);
        let deadline = Instant::now() + START_TIMEOUT;
        while !socket.exists() {
            if read_proc(pid)?.is_none() {
                return Err(RuntimeError::Protocol(
                    "Firecracker exited before API socket",
                ));
            }
            if Instant::now() >= deadline {
                return Err(RuntimeError::TimedOut("Firecracker API socket"));
            }
            thread::sleep(Duration::from_millis(20));
        }
        let root = self.root_path(id);
        fs::copy(&self.config.kernel, root.join("vmlinux"))?;
        copy_private_disk(&self.config.rootfs, &root.join("rootfs.ext4"))?;
        chown(
            root.join("rootfs.ext4"),
            Some(Uid::from_raw(self.config.vm_uid)),
            Some(Gid::from_raw(self.config.vm_gid)),
        )
        .map_err(errno_to_io)?;
        self.put(id, "/machine-config", &json!({ "vcpu_count": self.config.vcpu_count, "mem_size_mib": self.config.memory_mib }))?;
        self.put(id, "/boot-source", &json!({ "kernel_image_path": "/vmlinux", "boot_args": "console=ttyS0 reboot=k panic=1 pci=off" }))?;
        self.put(id, "/drives/rootfs", &json!({ "drive_id": "rootfs", "path_on_host": "/rootfs.ext4", "is_root_device": true, "is_read_only": false }))?;
        if let Some(hash) = &self.config.access_token_hash {
            self.put(id, "/network-interfaces/eth0", &json!({ "iface_id": "eth0", "host_dev_name": "tap0", "guest_mac": "06:00:ac:10:00:02" }))?;
            self.put(
                id,
                "/mmds/config",
                &json!({ "version": "V2", "network_interfaces": ["eth0"] }),
            )?;
            self.put(
                id,
                "/mmds",
                &json!({ "instanceID": id, "envID": "sandproof-base", "accessTokenHash": hash }),
            )?;
        }
        self.put(id, "/actions", &json!({ "action_type": "InstanceStart" }))?;
        Ok(())
    }

    /// Observe only the process bound to this resource claim.
    ///
    /// # Errors
    /// Rejects a reused PID or corrupted record instead of guessing.
    pub fn observe(&self, id: &str) -> Result<Observation, RuntimeError> {
        validate_id(id)?;
        let record = self.read_record(id)?;
        let state = match verified_process(&record)? {
            None => VmState::Stopped,
            Some(_) => match self.api(id, "GET", "/", None) {
                Ok(body) => {
                    let status: serde_json::Value = serde_json::from_slice(&body)?;
                    if status.get("state").and_then(serde_json::Value::as_str) == Some("Running") {
                        VmState::VmmRunning
                    } else {
                        VmState::Starting
                    }
                }
                Err(_) => VmState::ApiUnavailable,
            },
        };
        Ok(Observation {
            resource_id: id.to_owned(),
            pid: record.pid,
            start_time_ticks: record.start_time_ticks,
            state,
        })
    }

    /// Stop precisely the recorded process using a pidfd; never signal a PID
    /// after its recorded start time or jailer ID has changed.
    ///
    /// # Errors
    /// Returns an identity error or timeout without claiming destruction.
    pub fn stop(&self, id: &str) -> Result<Observation, RuntimeError> {
        validate_id(id)?;
        let record = self.read_record(id)?;
        if let Some(pidfd) = verified_process(&record)? {
            pidfd_send_signal(&pidfd, Signal::TERM).map_err(errno_to_io)?;
            if !wait_pidfd_exit(&pidfd, STOP_TIMEOUT)? {
                pidfd_send_signal(&pidfd, Signal::KILL).map_err(errno_to_io)?;
                if !wait_pidfd_exit(&pidfd, STOP_TIMEOUT)? {
                    return Err(RuntimeError::TimedOut("exact VM process exit"));
                }
            }
        }
        Ok(Observation {
            resource_id: id.to_owned(),
            pid: record.pid,
            start_time_ticks: record.start_time_ticks,
            state: VmState::Stopped,
        })
    }

    /// Remove only this stopped VM's jail and private disk. Keep its claim and
    /// process record as a tombstone, so the resource ID cannot be reused.
    ///
    /// # Errors
    /// Refuses to remove a live or unverified process's backing resources.
    pub fn cleanup(&self, id: &str) -> Result<(), RuntimeError> {
        validate_id(id)?;
        self.validate_network_id(id)?;
        let record = self.read_record(id)?;
        if verified_process(&record)?.is_some() {
            return Err(RuntimeError::InvalidConfiguration("cannot clean a live VM"));
        }
        let jail = self
            .config
            .chroot_base
            .join(self.firecracker_name())
            .join(id);
        match fs::symlink_metadata(&jail) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
                fs::remove_dir_all(&jail)?;
            }
            Ok(_) => return Err(RuntimeError::IdentityMismatch("jail directory type")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if self.config.private_netns.is_some() {
            self.cleanup_private_network(id)?;
        }
        Ok(())
    }

    fn prepare_private_network(&self, id: &str, claim: &Path) -> Result<(), RuntimeError> {
        let namespace =
            self.config
                .private_netns
                .as_ref()
                .ok_or(RuntimeError::InvalidConfiguration(
                    "private network not configured",
                ))?;
        match fs::symlink_metadata(namespace) {
            Ok(_) => return Err(RuntimeError::AlreadyClaimed),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        run_ip("create namespace", &["netns", "add", id])?;
        check_trusted_path(namespace)?;
        let metadata = fs::symlink_metadata(namespace)?;
        if !metadata.is_file() {
            return Err(RuntimeError::IdentityMismatch(
                "network namespace handle type",
            ));
        }
        let network = NetworkRecord {
            resource_id: id.to_owned(),
            namespace_inode: metadata.ino(),
        };
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(claim.join("network.json"))?;
        serde_json::to_writer(&mut file, &network)?;
        file.flush()?;
        file.sync_all()?;
        File::open(claim)?.sync_all()?;

        // The same private IPv4 range is safe because each VM has its own
        // network namespace. No default route or external bridge is added.
        run_ip(
            "create TAP",
            &[
                "netns",
                "exec",
                id,
                "/usr/bin/ip",
                "tuntap",
                "add",
                "dev",
                "tap0",
                "mode",
                "tap",
            ],
        )?;
        run_ip(
            "assign host address",
            &[
                "netns",
                "exec",
                id,
                "/usr/bin/ip",
                "addr",
                "add",
                "172.16.0.1/30",
                "dev",
                "tap0",
            ],
        )?;
        run_ip(
            "raise TAP",
            &[
                "netns",
                "exec",
                id,
                "/usr/bin/ip",
                "link",
                "set",
                "tap0",
                "up",
            ],
        )?;
        Ok(())
    }

    fn cleanup_private_network(&self, id: &str) -> Result<(), RuntimeError> {
        let namespace =
            self.config
                .private_netns
                .as_ref()
                .ok_or(RuntimeError::InvalidConfiguration(
                    "private network not configured",
                ))?;
        let path = self
            .config
            .chroot_base
            .join(".claims")
            .join(id)
            .join("network.json");
        let network: NetworkRecord = serde_json::from_reader(File::open(path)?)?;
        if network.resource_id != id || network.namespace_inode == 0 {
            return Err(RuntimeError::IdentityMismatch("network record fields"));
        }
        let metadata = match fs::symlink_metadata(namespace) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if !metadata.is_file() || metadata.uid() != 0 || metadata.ino() != network.namespace_inode {
            return Err(RuntimeError::IdentityMismatch("network namespace handle"));
        }
        run_ip("delete namespace", &["netns", "delete", id])
    }

    fn validate_network_id(&self, id: &str) -> Result<(), RuntimeError> {
        if let Some(namespace) = &self.config.private_netns {
            validate_namespace_id(namespace, id)?;
        }
        Ok(())
    }

    /// Discover claimed IDs without treating directory names as proof that a
    /// VM is running. Each result must still be checked with `observe`.
    ///
    /// # Errors
    /// Rejects invalid claim names and filesystem errors.
    pub fn discover_ids(&self) -> Result<Vec<String>, RuntimeError> {
        let mut ids = Vec::new();
        let claims = self.config.chroot_base.join(".claims");
        for entry in fs::read_dir(claims)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                return Err(RuntimeError::InvalidConfiguration("unexpected claim entry"));
            }
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| RuntimeError::InvalidId)?;
            validate_id(&id)?;
            ids.push(id);
        }
        ids.sort();
        Ok(ids)
    }

    fn read_record(&self, id: &str) -> Result<ProcessRecord, RuntimeError> {
        let path = self
            .config
            .chroot_base
            .join(".claims")
            .join(id)
            .join("record.json");
        let file = File::open(path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                RuntimeError::MissingRecord
            } else {
                RuntimeError::Io(error)
            }
        })?;
        let record: ProcessRecord = serde_json::from_reader(file)?;
        if record.resource_id != id || record.pid == 0 || record.start_time_ticks == 0 {
            return Err(RuntimeError::IdentityMismatch("record fields"));
        }
        Ok(record)
    }

    fn root_path(&self, id: &str) -> PathBuf {
        self.config
            .chroot_base
            .join(self.firecracker_name())
            .join(id)
            .join("root")
    }

    fn firecracker_name(&self) -> &str {
        self.config
            .firecracker
            .file_name()
            .expect("validated binary path")
            .to_str()
            .expect("operator-provided executable name must be UTF-8")
    }

    fn socket_path(&self, id: &str) -> PathBuf {
        self.root_path(id).join(API_SOCKET)
    }

    fn put(&self, id: &str, path: &str, body: &serde_json::Value) -> Result<(), RuntimeError> {
        let data = serde_json::to_vec(body)?;
        self.api(id, "PUT", path, Some(&data))?;
        Ok(())
    }

    fn api(
        &self,
        id: &str,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<Vec<u8>, RuntimeError> {
        let mut stream = UnixStream::connect(self.socket_path(id))?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let data = body.unwrap_or_default();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            data.len()
        )?;
        stream.write_all(data)?;
        let mut response = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            if let Some(position) = response.windows(4).position(|part| part == b"\r\n\r\n") {
                break position;
            }
            if response.len() > 65_536 {
                return Err(RuntimeError::Protocol("API headers exceeded limit"));
            }
            let count = stream.read(&mut chunk)?;
            if count == 0 {
                return Err(RuntimeError::Protocol("API closed before headers"));
            }
            response.extend_from_slice(&chunk[..count]);
        };
        let header = std::str::from_utf8(&response[..header_end])
            .map_err(|_| RuntimeError::Protocol("non-UTF-8 HTTP headers"))?;
        let status: u16 = header
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .ok_or(RuntimeError::Protocol("missing HTTP status"))?;
        let declared_length: Option<usize> = header.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())
                .flatten()
        });
        let content_length = if status == 204 || status == 304 || status < 200 {
            declared_length.unwrap_or(0)
        } else {
            declared_length.ok_or(RuntimeError::Protocol("missing Content-Length"))?
        };
        if content_length > 1_048_576 {
            return Err(RuntimeError::Protocol("API response exceeded limit"));
        }
        let body_start = header_end + 4;
        while response.len() - body_start < content_length {
            let count = stream.read(&mut chunk)?;
            if count == 0 {
                return Err(RuntimeError::Protocol("API closed before response body"));
            }
            response.extend_from_slice(&chunk[..count]);
        }
        let body = response[body_start..body_start + content_length].to_vec();
        if !(200..300).contains(&status) {
            return Err(RuntimeError::Api {
                status,
                body: String::from_utf8_lossy(&body).into_owned(),
            });
        }
        Ok(body)
    }
}

fn validate_id(id: &str) -> Result<(), RuntimeError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || id.starts_with('-')
    {
        return Err(RuntimeError::InvalidId);
    }
    Ok(())
}

fn validate_namespace_id(namespace: &Path, id: &str) -> Result<(), RuntimeError> {
    if namespace.file_name().and_then(std::ffi::OsStr::to_str) != Some(id) {
        return Err(RuntimeError::InvalidConfiguration(
            "network namespace must match the exact VM resource ID",
        ));
    }
    Ok(())
}

/// Check a root-owned, non-symlink path and all of its components.
/// # Errors
/// Rejects a component which a less trusted process can replace.
pub fn check_trusted_path(path: &Path) -> Result<(), RuntimeError> {
    if !path.is_absolute() {
        return Err(RuntimeError::InvalidConfiguration("paths must be absolute"));
    }
    let mut current = PathBuf::from("/");
    for component in path.components().skip(1) {
        current.push(component);
        let metadata = fs::symlink_metadata(&current)?;
        if metadata.file_type().is_symlink() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0
        {
            return Err(RuntimeError::InvalidConfiguration(
                "runtime path has a symlink or writable/non-root-owned component",
            ));
        }
    }
    Ok(())
}

fn check_kvm(path: &Path) -> Result<(), RuntimeError> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map(|_| ())
        .map_err(RuntimeError::KvmUnavailable)
}

fn copy_private_disk(source: &Path, destination: &Path) -> Result<(), RuntimeError> {
    fs::copy(source, destination)?;
    // A root-owned read-only template must still produce a writable private
    // guest disk. fs::copy preserves the source mode, including 0444.
    fs::set_permissions(destination, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn run_ip(operation: &'static str, arguments: &[&str]) -> Result<(), RuntimeError> {
    let output = Command::new("/usr/bin/ip")
        .args(arguments)
        .env_clear()
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(RuntimeError::HostCommand {
            operation,
            stderr: String::from_utf8_lossy(&output.stderr[..output.stderr.len().min(4096)])
                .into_owned(),
        });
    }
    Ok(())
}

#[derive(Debug)]
struct ProcStatus {
    state: char,
    start_time_ticks: u64,
}

fn read_proc(pid: u32) -> Result<Option<ProcStatus>, RuntimeError> {
    let path = format!("/proc/{pid}/stat");
    let stat = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(parse_proc_stat(&stat)?))
}

fn parse_proc_stat(stat: &str) -> Result<ProcStatus, RuntimeError> {
    let close = stat
        .rfind(')')
        .ok_or(RuntimeError::Protocol("malformed /proc stat"))?;
    let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    if fields.len() < 20 {
        return Err(RuntimeError::Protocol("short /proc stat"));
    }
    let state = fields[0]
        .chars()
        .next()
        .ok_or(RuntimeError::Protocol("missing process state"))?;
    let start_time_ticks = fields[19]
        .parse()
        .map_err(|_| RuntimeError::Protocol("invalid process start time"))?;
    Ok(ProcStatus {
        state,
        start_time_ticks,
    })
}

fn verified_process(record: &ProcessRecord) -> Result<Option<std::os::fd::OwnedFd>, RuntimeError> {
    let pid = Pid::from_raw(
        i32::try_from(record.pid).map_err(|_| RuntimeError::IdentityMismatch("pid range"))?,
    )
    .ok_or(RuntimeError::IdentityMismatch("zero pid"))?;
    let pidfd = match pidfd_open(pid, PidfdFlags::empty()) {
        Ok(fd) => fd,
        Err(error) if error == rustix::io::Errno::SRCH => return Ok(None),
        Err(error) => return Err(errno_to_io(error).into()),
    };
    let Some(status) = read_proc(record.pid)? else {
        return Ok(None);
    };
    // A reused PID proves the recorded process exited. Keep the pidfd bound
    // to the original task and never signal the replacement.
    if status.start_time_ticks != record.start_time_ticks {
        return Ok(None);
    }
    if status.state == 'Z' || status.state == 'X' {
        return Ok(None);
    }
    let cmdline = fs::read(format!("/proc/{}/cmdline", record.pid))?;
    let args: Vec<&[u8]> = cmdline
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .collect();
    let exact_id = format!("--id={}", record.resource_id);
    let id_pair = args
        .windows(2)
        .any(|part| part[0] == b"--id" && part[1] == record.resource_id.as_bytes());
    if !args.contains(&exact_id.as_bytes()) && !id_pair {
        return Err(RuntimeError::IdentityMismatch("process command line"));
    }
    Ok(Some(pidfd))
}

fn errno_to_io(error: rustix::io::Errno) -> io::Error {
    io::Error::from_raw_os_error(error.raw_os_error())
}

fn wait_pidfd_exit(pidfd: &std::os::fd::OwnedFd, timeout: Duration) -> Result<bool, RuntimeError> {
    let mut fds = [PollFd::new(pidfd, PollFlags::IN)];
    let timeout = Timespec {
        tv_sec: i64::try_from(timeout.as_secs())
            .map_err(|_| RuntimeError::Protocol("invalid stop timeout"))?,
        tv_nsec: i64::from(timeout.subsec_nanos()),
    };
    let ready = poll(&mut fds, Some(&timeout)).map_err(errno_to_io)?;
    Ok(ready > 0 && fds[0].revents().intersects(PollFlags::IN | PollFlags::HUP))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_cannot_escape_or_alias_jailer_paths() {
        for id in ["", "-bad", "../other", "a/b", "UPPER", "a_b", "x.y"] {
            assert!(validate_id(id).is_err());
        }
        assert!(validate_id("sandproof-001").is_ok());
        assert!(validate_id(&"a".repeat(65)).is_err());
    }

    #[test]
    fn network_cleanup_must_target_same_resource_id() {
        assert!(validate_namespace_id(Path::new("/run/netns/sandbox-a"), "sandbox-a").is_ok());
        assert!(validate_namespace_id(Path::new("/run/netns/sandbox-b"), "sandbox-a").is_err());
    }

    #[test]
    fn proc_stat_start_time_uses_field_22() {
        let stat = "123 (name with spaces) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 12345 20";
        assert_eq!(parse_proc_stat(stat).unwrap().start_time_ticks, 12345);
        assert!(parse_proc_stat("123 (bad stat)").is_err());
    }

    #[test]
    fn missing_kvm_is_reported_explicitly() {
        assert!(matches!(
            check_kvm(Path::new("/nonexistent-sandproof-kvm")),
            Err(RuntimeError::KvmUnavailable(_))
        ));
    }

    #[test]
    fn readonly_template_produces_independent_writable_disk() {
        let directory = std::env::temp_dir().join(format!("sandproof-copy-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("base");
        let destination = directory.join("private");
        fs::write(&source, b"base contents").unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o444)).unwrap();
        copy_private_disk(&source, &destination).unwrap();
        assert_eq!(fs::metadata(&destination).unwrap().mode() & 0o777, 0o600);
        fs::write(&destination, b"guest changes").unwrap();
        assert_eq!(fs::read(&source).unwrap(), b"base contents");
        assert_eq!(fs::metadata(&source).unwrap().mode() & 0o777, 0o444);
        fs::remove_file(destination).unwrap();
        fs::remove_file(source).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
