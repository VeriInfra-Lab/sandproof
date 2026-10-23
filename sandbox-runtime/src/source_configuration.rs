//! Independent, nonmutating facts for the original accepted source work.
//! The node still checks the registered epoch/template and constructs the
//! compatibility profile; these bytes are neither admission nor Stop rights.
use super::{OriginalVmSource, Runtime, RuntimeError, RuntimeProcessIdentity, error};
use crate::snapshot_store::{FileStamp, matches_entry};
use rustix::fs::{self, Mode, OFlags};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{File, Metadata},
    io::{self, Read},
    os::unix::fs::MetadataExt,
};

#[derive(Clone, Copy, Debug)]
pub struct SourceConfigurationLimits {
    pub executable_bytes: u64,
    pub kernel_bytes: u64,
    pub disk_bytes: u64,
    pub configuration_bytes: usize,
}

/// Actual read-time facts only. Private construction does not prove a future
/// live source, CPU compatibility or a successful persistent registration.
pub struct OriginalConfigurationObservation {
    resource_id: String,
    source: RuntimeProcessIdentity,
    configuration: Vec<u8>,
    executable_sha256: [u8; 32],
    kernel_sha256: [u8; 32],
    kernel_bytes: u64,
    disk_bytes: u64,
}

impl OriginalConfigurationObservation {
    #[must_use]
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    #[must_use]
    pub fn source(&self) -> &RuntimeProcessIdentity {
        &self.source
    }
    #[must_use]
    pub fn configuration(&self) -> &[u8] {
        &self.configuration
    }
    #[must_use]
    pub fn executable_sha256(&self) -> &[u8; 32] {
        &self.executable_sha256
    }
    #[must_use]
    pub fn kernel_sha256(&self) -> &[u8; 32] {
        &self.kernel_sha256
    }
    #[must_use]
    pub fn kernel_bytes(&self) -> u64 {
        self.kernel_bytes
    }
    #[must_use]
    pub fn disk_bytes(&self) -> u64 {
        self.disk_bytes
    }
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct InstanceInfo {
    app_name: String,
    id: String,
    state: String,
    vmm_version: String,
}

fn limits_valid(limits: SourceConfigurationLimits) -> bool {
    (1..=64 * 1024 * 1024).contains(&limits.executable_bytes)
        && (1..=256 * 1024 * 1024).contains(&limits.kernel_bytes)
        && (1..=i64::MAX as u64).contains(&limits.disk_bytes)
        && (1..=1024 * 1024).contains(&limits.configuration_bytes)
}

fn instance(bytes: &[u8], id: &str) -> Result<InstanceInfo, RuntimeError> {
    let info: InstanceInfo = serde_json::from_slice(bytes)?;
    if info.id != id
        || info.app_name != "Firecracker"
        || info.vmm_version != "1.16.1"
        || !matches!(info.state.as_str(), "Running" | "Paused")
    {
        return Err(error("original source instance/version/state differs"));
    }
    Ok(info)
}

fn executable(pid: u32) -> Result<File, RuntimeError> {
    // This one kernel-owned magic link intentionally selects the actual
    // executing inode, not the current configured executable pathname.
    Ok(File::from(
        fs::open(
            format!("/proc/{pid}/exe").as_str(),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(io::Error::from)?,
    ))
}

fn original_file(original: &OriginalVmSource, name: &str) -> Result<File, RuntimeError> {
    let file = File::from(
        fs::openat(
            &original.root,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    matches_entry(&original.root, OsStr::new(name), &file)?;
    Ok(file)
}

fn measure(file: &mut File, maximum: u64) -> Result<([u8; 32], FileStamp), RuntimeError> {
    measure_with(file, maximum, || Ok(()))
}

fn measure_with(
    file: &mut File,
    maximum: u64,
    after_read: impl FnOnce() -> Result<(), RuntimeError>,
) -> Result<([u8; 32], FileStamp), RuntimeError> {
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.len() == 0
        || metadata.len() > maximum
    {
        return Err(error("source profile file type, links or budget"));
    }
    let stamp = FileStamp::from_metadata(&metadata);
    let mut count = 0_u64;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let capacity = usize::try_from((metadata.len() - count + 1).min(buffer.len() as u64))
            .map_err(|_| error("source profile read size"))?;
        let read = file.read(&mut buffer[..capacity])?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > metadata.len() {
            return Err(error("source profile file grew"));
        }
        hasher.update(&buffer[..read]);
    }
    after_read()?;
    if count != metadata.len() || FileStamp::from_metadata(&file.metadata()?) != stamp {
        return Err(error("source profile file changed"));
    }
    Ok((hasher.finalize().into(), stamp))
}

fn private_metadata(
    metadata: &Metadata,
    owner: u32,
    group: u32,
    mode: u32,
    maximum: u64,
) -> Result<(), RuntimeError> {
    if !metadata.is_file()
        || metadata.uid() != owner
        || metadata.gid() != group
        || metadata.mode() & 0o7777 != mode
        || metadata.nlink() != 1
        || metadata.len() == 0
        || metadata.len() > maximum
    {
        return Err(error("original profile file ownership, mode or budget"));
    }
    Ok(())
}

impl Runtime {
    /// Fingerprint the same local plan used by actual cold startup and visible
    /// configuration checks. This does not observe or certify a live VM.
    /// # Errors
    /// An encoding failure cannot invent a device/profile identity.
    pub fn boot_plan_digest(&self) -> Result<[u8; 32], RuntimeError> {
        super::super::boot_plan::BootPlan::for_runtime(&self.config).digest()
    }

    /// Inspect the same original source under its already accepted read work.
    /// Uses only GET and readonly file descriptors, without `Pause`/`Full`/`Stop`,
    /// chmod, new files or replacement-source adoption. The caller holds its
    /// original work/gate, then rechecks its version and commits these facts
    /// against the protected epoch/template before any capture effect. Visible
    /// device JSON must match the same `BootPlan` used at launch. Hidden custom
    /// CPU/logger/metrics state, actual image provenance and CPU compatibility
    /// still need the original protected deployment and writer checks.
    /// # Errors
    /// Unknown/missing/changed sources, files or replies remain unresolved. No
    /// error grants cleanup or implies that the unique original state is gone.
    pub fn observe_original_configuration(
        &self,
        id: &str,
        expected: &RuntimeProcessIdentity,
        limits: SourceConfigurationLimits,
    ) -> Result<OriginalConfigurationObservation, RuntimeError> {
        if !limits_valid(limits) {
            return Err(error("source profile observation limits"));
        }
        super::super::uid_retirement::require_host_namespaces(std::path::Path::new("/proc"))?;
        let original = self.open_original_source(id, expected)?;
        let before = instance(&self.api(id, "GET", "/", None)?, id)?;
        let configuration = self.api(id, "GET", "/vm/config", None)?;
        if configuration.is_empty() || configuration.len() > limits.configuration_bytes {
            return Err(error("source configuration response budget"));
        }
        super::super::boot_plan::BootPlan::for_runtime(&self.config)
            .check_full_configuration(&configuration)?;
        let mut actual_executable = executable(expected.pid)?;
        let (executable_sha256, executable_stamp) =
            measure(&mut actual_executable, limits.executable_bytes)?;
        let mut kernel = original_file(&original, "vmlinux")?;
        private_metadata(
            &kernel.metadata()?,
            self.config.vm_uid,
            self.config.vm_gid,
            0o400,
            limits.kernel_bytes,
        )?;
        let (kernel_sha256, kernel_stamp) = measure(&mut kernel, limits.kernel_bytes)?;
        let disk = original_file(&original, "rootfs.ext4")?;
        private_metadata(
            &disk.metadata()?,
            self.config.vm_uid,
            self.config.vm_gid,
            0o600,
            limits.disk_bytes,
        )?;
        let disk_bytes = disk.metadata()?.len();
        if self.api(id, "GET", "/vm/config", None)? != configuration
            || instance(&self.api(id, "GET", "/", None)?, id)? != before
        {
            return Err(error(
                "original source configuration changed during observation",
            ));
        }
        self.verify_original_source(&original, false)?;
        matches_entry(&original.root, OsStr::new("vmlinux"), &kernel)?;
        matches_entry(&original.root, OsStr::new("rootfs.ext4"), &disk)?;
        let kernel_metadata = kernel.metadata()?;
        private_metadata(
            &kernel_metadata,
            self.config.vm_uid,
            self.config.vm_gid,
            0o400,
            limits.kernel_bytes,
        )?;
        if FileStamp::from_metadata(&kernel_metadata) != kernel_stamp
            || FileStamp::from_metadata(&executable(expected.pid)?.metadata()?) != executable_stamp
        {
            return Err(error("original source executable/kernel changed"));
        }
        private_metadata(
            &disk.metadata()?,
            self.config.vm_uid,
            self.config.vm_gid,
            0o600,
            limits.disk_bytes,
        )?;
        if disk.metadata()?.len() != disk_bytes {
            return Err(error("original disk size changed"));
        }
        // Writable disk timestamps/content may change while Running. Its full
        // bytes belong to the later continuously paused Full-capture interval.
        Ok(OriginalConfigurationObservation {
            resource_id: id.to_owned(),
            source: expected.clone(),
            configuration,
            executable_sha256,
            kernel_sha256,
            kernel_bytes: kernel_metadata.len(),
            disk_bytes,
        })
    }
}

#[cfg(test)]
#[path = "source_configuration_tests.rs"]
mod tests;
