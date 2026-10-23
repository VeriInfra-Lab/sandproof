//! Full capture of an exact original VMM; no lifetime/eligibility decisions.
//!
//! V5 holds the per-VM work/fence guard across pause, capture, publication and
//! handoff. V8 has closed new access and drained all writers. These physical
//! operations neither grant that authority nor stop/resume a VM on failure.

use super::{
    Runtime, RuntimeError, RuntimeProcessIdentity, SnapshotLayout, validate_id, verified_process,
};
use crate::snapshot_store::{
    PublishedSnapshotFiles, SnapshotArtifact, SnapshotFileMeasurement, SnapshotStage,
};
use rustix::{
    fs::{self, Mode, OFlags},
    process::{Gid, Uid},
};
use serde_json::json;
use std::{
    fmt::Write as _,
    fs::File,
    io,
    os::{
        fd::OwnedFd,
        unix::fs::{MetadataExt, PermissionsExt},
    },
    time::Duration,
};

/// Budgets were reserved by the controller before starting physical work.
#[derive(Clone, Copy, Debug)]
pub struct FullCaptureLimits {
    pub state_bytes: u64,
    pub disk_bytes: u64,
    pub api_timeout: Duration,
}

/// Observation of the exact original process/jail, not a persistent permission.
/// Dropping this handle does not stop, thaw or clean up the original VMM.
pub struct PausedOriginalVm {
    original: OriginalVmSource,
}

impl PausedOriginalVm {
    #[must_use]
    pub fn resource_id(&self) -> &str {
        &self.original.resource_id
    }
    #[must_use]
    pub fn source(&self) -> &RuntimeProcessIdentity {
        &self.original.identity
    }
}

// A live original identity without implying that it is paused.
struct OriginalVmSource {
    resource_id: String,
    identity: RuntimeProcessIdentity,
    root: File,
    _pidfd: OwnedFd,
}

/// Produced only by the real Full-capture path, not by arbitrary file copying.
pub struct CapturedFullSnapshot {
    resource_id: String,
    stage: SnapshotStage,
    source: RuntimeProcessIdentity,
}

/// Physical Full-capture publication; no database handoff/stop authority.
pub struct PublishedFullCapture {
    resource_id: String,
    files: PublishedSnapshotFiles,
    source: RuntimeProcessIdentity,
}

impl CapturedFullSnapshot {
    #[must_use]
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    #[must_use]
    pub fn directory_key(&self) -> &[u8; 32] {
        self.stage.directory_key()
    }
    #[must_use]
    pub fn measurements(&self) -> Vec<SnapshotFileMeasurement> {
        self.stage.measurements()
    }
    #[must_use]
    pub fn source(&self) -> &RuntimeProcessIdentity {
        &self.source
    }

    /// Publish only after the node has bound the measured files to the original
    /// capture operation/profile. Every error preserves partial materials.
    /// # Errors
    /// Propagates file validation, write, rename and sync errors as effectful.
    pub fn publish(
        self,
        manifest: &[u8],
        expected: &[SnapshotFileMeasurement],
    ) -> Result<PublishedFullCapture, RuntimeError> {
        Ok(PublishedFullCapture {
            resource_id: self.resource_id,
            files: self.stage.publish(manifest, expected)?,
            source: self.source,
        })
    }
}

impl PublishedFullCapture {
    #[must_use]
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    #[must_use]
    pub fn files(&self) -> &PublishedSnapshotFiles {
        &self.files
    }
    #[must_use]
    pub fn source(&self) -> &RuntimeProcessIdentity {
        &self.source
    }
}

fn error(message: &'static str) -> RuntimeError {
    RuntimeError::IdentityMismatch(message)
}

fn errno(error: rustix::io::Errno) -> RuntimeError {
    RuntimeError::Io(io::Error::from(error))
}

fn open_directory_at(parent: &File, name: &str) -> Result<File, RuntimeError> {
    Ok(File::from(
        fs::openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno)?,
    ))
}

fn source_at(parent: &File, name: &str, owner: u32) -> Result<File, RuntimeError> {
    let file = File::from(
        fs::openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(errno)?,
    );
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != owner || metadata.nlink() != 1 {
        return Err(error("capture source owner, type or links"));
    }
    // Newly generated files live in the original private capture directory.
    // Tightening their mode does not reopen or substitute the original inode.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.sync_all()?;
    Ok(file)
}

impl Runtime {
    /// Pause only the recorded original VMM and confirm it through its API.
    /// # Errors
    /// Any error is potentially effectful; retain the source VM/work responsibility.
    pub fn pause_original_vm(
        &self,
        id: &str,
        expected: &RuntimeProcessIdentity,
    ) -> Result<PausedOriginalVm, RuntimeError> {
        let original = self.open_original_source(id, expected)?;
        let request = serde_json::to_vec(&json!({"state": "Paused"}))?;
        self.api(id, "PATCH", "/vm", Some(&request))?;
        self.verify_original_source(&original, true)?;
        Ok(PausedOriginalVm { original })
    }

    fn open_original_source(
        &self,
        id: &str,
        expected: &RuntimeProcessIdentity,
    ) -> Result<OriginalVmSource, RuntimeError> {
        validate_id(id)?;
        let record = self.read_record(id)?;
        if record.pid != expected.pid
            || record.boot_id != expected.boot_id
            || record.start_time_ticks != expected.start_time_ticks
            || !expected.is_alive()?
        {
            return Err(error("pause original process binding"));
        }
        let pidfd = verified_process(&record)?.ok_or(error("original VMM exited"))?;
        self.require_launch_jail(id, 0)?;
        self.verify_cgroup(id, Some(record.pid))?;
        let jail = File::from(
            fs::open(
                self.jail_path(id).as_path(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(errno)?,
        );
        let root = open_directory_at(&jail, "root")?;
        let metadata = root.metadata()?;
        if metadata.uid() != self.config.vm_uid || metadata.mode() & 0o077 != 0 {
            return Err(error("original jail root owner"));
        }
        let original = OriginalVmSource {
            resource_id: id.to_owned(),
            identity: expected.clone(),
            root,
            _pidfd: pidfd,
        };
        // Verify the opened root still matches the durable jail witness before
        // sending an effectful API request through the same protected jail.
        self.verify_original_source(&original, false)?;
        Ok(original)
    }

    /// Re-observe this original VM without issuing a pause/resume action.
    /// # Errors
    /// Rejects missing/replaced processes, jails or any state other than paused.
    pub fn confirm_original_paused(&self, original: &PausedOriginalVm) -> Result<(), RuntimeError> {
        self.verify_original_capture(original, true)
    }

    fn verify_original_capture(
        &self,
        paused: &PausedOriginalVm,
        require_paused: bool,
    ) -> Result<(), RuntimeError> {
        self.verify_original_source(&paused.original, require_paused)
    }

    fn verify_original_source(
        &self,
        source: &OriginalVmSource,
        require_paused: bool,
    ) -> Result<(), RuntimeError> {
        let record = self.read_record(&source.resource_id)?;
        if record.pid != source.identity.pid
            || record.boot_id != source.identity.boot_id
            || record.start_time_ticks != source.identity.start_time_ticks
            || !source.identity.is_alive()?
            || verified_process(&record)?.is_none()
        {
            return Err(error("capture original process binding"));
        }
        self.require_launch_jail(&source.resource_id, 0)?;
        self.verify_cgroup(&source.resource_id, Some(record.pid))?;
        let current = File::from(
            fs::open(
                self.root_path(&source.resource_id).as_path(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(errno)?,
        );
        let metadata = current.metadata()?;
        let original = source.root.metadata()?;
        if metadata.dev() != original.dev() || metadata.ino() != original.ino() {
            return Err(error("capture original root replaced"));
        }
        if require_paused {
            let body = self.api(&source.resource_id, "GET", "/", None)?;
            let status: serde_json::Value = serde_json::from_slice(&body)?;
            if status.get("state").and_then(serde_json::Value::as_str) != Some("Paused") {
                return Err(error("original VMM is not observed paused"));
            }
        }
        Ok(())
    }

    /// Produce Full RAM/state and copy the fixed profile's original private disk
    /// into a separate protected store during the same frozen interval.
    /// # Errors
    /// Preserves the original VMM and every partial source/store directory. The
    /// durable capture key remains responsible even after an API timeout.
    pub fn capture_full_into(
        &self,
        paused: &PausedOriginalVm,
        layout: &SnapshotLayout,
        directory_key: [u8; 32],
        limits: FullCaptureLimits,
    ) -> Result<CapturedFullSnapshot, RuntimeError> {
        if limits.state_bytes == 0
            || limits.disk_bytes == 0
            || limits.api_timeout < Duration::from_secs(1)
            || limits.api_timeout > Duration::from_mins(5)
        {
            return Err(error("full capture budgets or timeout"));
        }
        let store = layout.store_for_runtime(&self.config.chroot_base)?;
        self.verify_original_capture(paused, true)?;
        let mut stage = store.begin(directory_key)?;
        let mut capture_name = "full-".to_owned();
        for byte in directory_key {
            write!(&mut capture_name, "{byte:02x}")
                .map_err(|_| error("capture directory encoding"))?;
        }
        fs::mkdirat(&paused.original.root, capture_name.as_str(), Mode::RWXU).map_err(errno)?;
        let directory = open_directory_at(&paused.original.root, &capture_name)?;
        fs::fchown(
            &directory,
            Some(Uid::from_raw(self.config.vm_uid)),
            Some(Gid::from_raw(self.config.vm_gid)),
        )
        .map_err(errno)?;
        directory.sync_all()?;
        paused.original.root.sync_all()?;
        let request = serde_json::to_vec(&json!({
            "snapshot_type": "Full",
            "snapshot_path": format!("/{capture_name}/state.bin"),
            "mem_file_path": format!("/{capture_name}/memory.bin"),
        }))?;
        self.api_with_timeout(
            &paused.original.resource_id,
            "PUT",
            "/snapshot/create",
            Some(&request),
            limits.api_timeout,
        )?;
        self.verify_original_capture(paused, true)?;
        let mut state_file = source_at(&directory, "state.bin", self.config.vm_uid)?;
        let mut memory = source_at(&directory, "memory.bin", self.config.vm_uid)?;
        let memory_bytes = u64::from(self.config.memory_mib) * 1_048_576;
        if memory.metadata()?.len() != memory_bytes {
            return Err(error("full memory file length"));
        }
        // This profile is exactly the drive configured by stage_and_boot, never
        // a guest-supplied path. Later profiles must enumerate every drive.
        let mut disk = source_at(&paused.original.root, "rootfs.ext4", self.config.vm_uid)?;
        stage.copy_artifact(
            &mut state_file,
            self.config.vm_uid,
            SnapshotArtifact::State,
            limits.state_bytes,
        )?;
        stage.copy_artifact(
            &mut memory,
            self.config.vm_uid,
            SnapshotArtifact::Memory,
            memory_bytes,
        )?;
        stage.copy_artifact(
            &mut disk,
            self.config.vm_uid,
            SnapshotArtifact::Disk(0),
            limits.disk_bytes,
        )?;
        self.verify_original_capture(paused, true)?;
        Ok(CapturedFullSnapshot {
            resource_id: paused.original.resource_id.clone(),
            stage,
            source: paused.original.identity.clone(),
        })
    }
}

#[path = "source_configuration.rs"]
mod source_configuration;
pub use source_configuration::{OriginalConfigurationObservation, SourceConfigurationLimits};

#[path = "paused_observation.rs"]
mod paused_observation;
pub use paused_observation::OriginalPauseObservation;
