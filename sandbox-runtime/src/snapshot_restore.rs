//! Load one protected Full into one never-reused candidate; never cold boot it.
//! Durable claim/pins/MayHaveExecuted are controller obligations, not inferred
//! from a path, this physical record, or a paused response.
use super::{
    OriginalPauseObservation, Runtime, RuntimeConfig, RuntimeError, RuntimeProcessIdentity,
    SnapshotLayout, copy_private_kernel, read_trusted_record, validate_id, validate_socket_path,
};
use crate::snapshot_store::{SnapshotArtifact, SnapshotBundle};
use rustix::fs::{self, Mode, OFlags};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{fs::File, os::unix::fs::PermissionsExt, time::Duration};

/// Physical state of the same candidate. Exited does not permit replaying its
/// source: the controller's irreversible execution marker decides state loss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadedSnapshotState {
    Paused,
    Running,
    Exited,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedSnapshotObservation {
    pub process: RuntimeProcessIdentity,
    pub state: LoadedSnapshotState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreIntent {
    version: u32,
    resource_id: String,
    config: RuntimeConfig,
    directory_key: [u8; 32],
    manifest_sha256: [u8; 32],
    work_sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoadedRecord {
    intent_sha256: [u8; 32],
    process: RuntimeProcessIdentity,
}

fn invalid(message: &'static str) -> RuntimeError {
    RuntimeError::IdentityMismatch(message)
}

fn load_request() -> serde_json::Value {
    // Fixed Firecracker 1.16.1 interface. No request-controlled file paths,
    // implicit auto-resume, UFFD handler or network remapping.
    json!({
        "snapshot_path": "/restore/restore-state.bin",
        "mem_backend": {"backend_type":"File", "backend_path":"/restore/restore-memory.bin"},
        "track_dirty_pages": false,
        "resume_vm": false,
        "network_overrides": [],
        "clock_realtime": false,
    })
}

// Keep the actual state-dependent resume decision shared with the interruption
// regression. The caller still holds the original helper gate and intent.
fn activate_same_loaded_candidate(
    before: &LoadedSnapshotObservation,
    resume: impl FnOnce() -> Result<(), RuntimeError>,
    observe: impl FnOnce() -> Result<LoadedSnapshotObservation, RuntimeError>,
) -> Result<LoadedSnapshotObservation, RuntimeError> {
    match before.state {
        LoadedSnapshotState::Paused => resume()?,
        LoadedSnapshotState::Running => {}
        LoadedSnapshotState::Exited => return Err(invalid("restore candidate exited")),
    }
    let after = observe()?;
    if after.process != before.process || after.state != LoadedSnapshotState::Running {
        return Err(invalid(
            "restore activation not observed on original candidate",
        ));
    }
    Ok(after)
}

impl RestoreIntent {
    fn digest(&self) -> Result<[u8; 32], RuntimeError> {
        Ok(Sha256::digest(serde_json::to_vec(self)?).into())
    }
    fn require(
        &self,
        runtime: &Runtime,
        id: &str,
        manifest: [u8; 32],
        work: [u8; 32],
    ) -> Result<(), RuntimeError> {
        if self.version != 1
            || self.resource_id != id
            || self.config != runtime.config
            || self.manifest_sha256 != manifest
            || self.work_sha256 != work
        {
            return Err(invalid("restore candidate intent differs"));
        }
        Ok(())
    }
}

impl Runtime {
    /// Issue the only load for a fresh candidate, with data access closed.
    /// The caller has durably acquired unique authority, source pin and space
    /// for private RAM/disk/state before entry. Existing claims are rejected;
    /// after any error use `observe_loaded_snapshot` on this same candidate.
    /// The private RAM File backend lives until exact VMM exit and jail cleanup.
    /// Neither a successful return nor dropping `bundle` ends that obligation.
    /// # Errors
    /// Every error preserves the claim, source and partial private materials.
    pub fn load_paused_snapshot(
        &self,
        id: &str,
        bundle: &SnapshotBundle,
        layout: &SnapshotLayout,
        manifest_sha256: [u8; 32],
        work_sha256: [u8; 32],
        timeout: Duration,
    ) -> Result<LoadedSnapshotObservation, RuntimeError> {
        if timeout < Duration::from_secs(1) || timeout > Duration::from_mins(5) {
            return Err(invalid("restore API timeout"));
        }
        validate_id(id)?;
        self.validate_network_id(id)?;
        validate_socket_path(&self.socket_path(id))?;
        layout.store_for_runtime(&self.config.chroot_base)?;
        bundle.verify_unchanged()?;
        if bundle.manifest_sha256() != manifest_sha256 {
            return Err(invalid("restore original full manifest digest"));
        }
        // Validate the fixed template kernel, but never copy its pristine disk
        // over the saved writable drive.
        super::check_trusted_path(&self.config.kernel)?;
        let intent = RestoreIntent {
            version: 1,
            resource_id: id.into(),
            config: self.config.clone(),
            directory_key: *bundle.directory_key(),
            manifest_sha256,
            work_sha256,
        };
        let claim = self.claim_resource(id)?;
        self.write_new_record(&claim.join("restore-intent.json"), &intent)?;
        self.reserve_jail(id, 0)?;
        self.prepare_cgroup(id)?;
        if self.config.private_netns.is_some() {
            self.prepare_private_network(id, &claim)?;
        }
        // Network helpers have completed; hold the original exclusive gate
        // across process launch, private clones and the load API exchange.
        let _gate = self.acquire_helper_gate()?;
        let record = self.spawn_jailer(id, &claim)?;
        self.wait_api_socket(id, record.pid)?;
        self.require_launch_jail(id, 0)?;
        let root = File::from(
            fs::open(
                self.root_path(id).as_path(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(std::io::Error::from)?,
        );
        copy_private_kernel(
            &self.config.kernel,
            &self.root_path(id).join("vmlinux"),
            self.config.vm_uid,
            self.config.vm_gid,
        )?;
        // Private File backing is charged to the same enforced material
        // project BEFORE RAM/state files acquire shared extents. The writable disk
        // remains in the separately reserved runtime position.
        fs::mkdirat(&root, "restore", Mode::RWXU).map_err(std::io::Error::from)?;
        let backing = File::from(
            fs::openat(
                &root,
                "restore",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(std::io::Error::from)?,
        );
        layout.assign_new_capture_project(&backing)?;
        for role in [SnapshotArtifact::State, SnapshotArtifact::Memory] {
            bundle.clone_for_restore(&backing, role, self.config.vm_uid, self.config.vm_gid)?;
        }
        fs::fchown(
            &backing,
            Some(rustix::process::Uid::from_raw(self.config.vm_uid)),
            Some(rustix::process::Gid::from_raw(self.config.vm_gid)),
        )
        .map_err(std::io::Error::from)?;
        backing.set_permissions(std::fs::Permissions::from_mode(0o500))?;
        backing.sync_all()?;
        root.sync_all()?;
        bundle.clone_for_restore(
            &root,
            SnapshotArtifact::Disk(0),
            self.config.vm_uid,
            self.config.vm_gid,
        )?;
        self.require_launch_jail(id, 0)?;
        bundle.verify_unchanged()?;
        self.api_with_timeout(
            id,
            "PUT",
            "/snapshot/load",
            Some(&serde_json::to_vec(&load_request())?),
            timeout,
        )?;
        let observed = self.observe_loaded_snapshot(id, manifest_sha256, work_sha256)?;
        if observed.state != LoadedSnapshotState::Paused {
            return Err(invalid("load did not leave exact candidate paused"));
        }
        let loaded = LoadedRecord {
            intent_sha256: intent.digest()?,
            process: observed.process.clone(),
        };
        self.write_new_record(&claim.join("restore-loaded.json"), &loaded)?;
        Ok(observed)
    }

    /// Read only the existing intent/process. No load, fallback, new VM, file
    /// copying or fresh identity adoption occurs after uncertain completion.
    /// # Errors
    /// Missing/incomplete claims and unobservable API state remain unknown.
    pub fn observe_loaded_snapshot(
        &self,
        id: &str,
        manifest_sha256: [u8; 32],
        work_sha256: [u8; 32],
    ) -> Result<LoadedSnapshotObservation, RuntimeError> {
        validate_id(id)?;
        let claim = self.config.chroot_base.join(".claims").join(id);
        let intent: RestoreIntent = read_trusted_record(&claim.join("restore-intent.json"))?;
        intent.require(self, id, manifest_sha256, work_sha256)?;
        let record = self.read_record(id)?;
        let process = record.source_identity()?;
        match read_trusted_record::<LoadedRecord>(&claim.join("restore-loaded.json")) {
            Ok(loaded) if loaded.intent_sha256 == intent.digest()? && loaded.process == process => {
            }
            Ok(_) => return Err(invalid("restore loaded process differs")),
            Err(RuntimeError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let state = match self.observe_original_pause_state(id, &process)? {
            OriginalPauseObservation::Paused(_) => LoadedSnapshotState::Paused,
            OriginalPauseObservation::Running => LoadedSnapshotState::Running,
            OriginalPauseObservation::Exited => LoadedSnapshotState::Exited,
        };
        Ok(LoadedSnapshotObservation { process, state })
    }

    /// Continue the same loaded candidate only after the controller has
    /// durably recorded `MayHaveExecuted`. Repeated calls do not reload bytes.
    /// Current credential metadata is replaced while still paused, before any
    /// resumed guest can accept traffic. Guest readiness is a separate step.
    /// # Errors
    /// Unknown activation keeps the same candidate and all backing obligations.
    pub fn activate_loaded_snapshot(
        &self,
        id: &str,
        manifest_sha256: [u8; 32],
        work_sha256: [u8; 32],
        expected: &RuntimeProcessIdentity,
    ) -> Result<LoadedSnapshotObservation, RuntimeError> {
        let _gate = self.acquire_helper_gate()?;
        let before = self.observe_loaded_snapshot(id, manifest_sha256, work_sha256)?;
        if before.process != *expected || before.state == LoadedSnapshotState::Exited {
            return Err(invalid(
                "restore activation original candidate differs or exited",
            ));
        }
        activate_same_loaded_candidate(
            &before,
            || {
                self.configure_access_metadata(id)?;
                self.api(
                    id,
                    "PATCH",
                    "/vm",
                    Some(&serde_json::to_vec(&json!({"state":"Resumed"}))?),
                )?;
                Ok(())
            },
            || self.observe_loaded_snapshot(id, manifest_sha256, work_sha256),
        )
    }

    /// Physical continuation after an ordinary capture failure. The caller
    /// first durably revokes capture eligibility and resolves every old worker
    /// and commit; this method supplies no such business authority.
    /// # Errors
    /// Rejects a replaced, exited or unobservable source. No source is reloaded.
    pub fn continue_original_vm(
        &self,
        id: &str,
        expected: &RuntimeProcessIdentity,
    ) -> Result<(), RuntimeError> {
        match self.observe_original_pause_state(id, expected)? {
            OriginalPauseObservation::Running => return Ok(()),
            OriginalPauseObservation::Paused(_) => {}
            OriginalPauseObservation::Exited => return Err(invalid("continue original exited")),
        }
        self.api(
            id,
            "PATCH",
            "/vm",
            Some(&serde_json::to_vec(&json!({"state":"Resumed"}))?),
        )?;
        match self.observe_original_pause_state(id, expected)? {
            OriginalPauseObservation::Running => Ok(()),
            _ => Err(invalid("original continuation not observed")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn candidate(state: LoadedSnapshotState) -> LoadedSnapshotObservation {
        LoadedSnapshotObservation {
            process: RuntimeProcessIdentity {
                pid: 7,
                start_time_ticks: 11,
                boot_id: "00000000-0000-0000-0000-000000000001".into(),
            },
            state,
        }
    }

    #[test]
    fn effectful_lost_resume_recovers_running_identity_without_second_patch() {
        let state = Cell::new(LoadedSnapshotState::Paused);
        let patches = Cell::new(0);
        let failed = activate_same_loaded_candidate(
            &candidate(state.get()),
            || {
                patches.set(patches.get() + 1);
                state.set(LoadedSnapshotState::Running);
                Err(invalid("reply lost after actual resume"))
            },
            || panic!("lost reply must remain unknown"),
        );
        assert!(failed.is_err());
        assert_eq!(state.get(), LoadedSnapshotState::Running);
        let recovered = activate_same_loaded_candidate(
            &candidate(state.get()),
            || panic!("an already running candidate must not be resumed again"),
            || Ok(candidate(state.get())),
        )
        .unwrap();
        assert_eq!(recovered, candidate(LoadedSnapshotState::Running));
        assert_eq!(patches.get(), 1);
    }

    #[test]
    fn activation_refuses_exit_and_replacement_instead_of_reloading() {
        assert!(
            activate_same_loaded_candidate(
                &candidate(LoadedSnapshotState::Exited),
                || panic!("cannot resume a dead candidate"),
                || panic!("cannot adopt a new candidate")
            )
            .is_err()
        );
        assert!(
            activate_same_loaded_candidate(
                &candidate(LoadedSnapshotState::Running),
                || panic!("running must not send PATCH"),
                || {
                    let mut foreign = candidate(LoadedSnapshotState::Running);
                    foreign.process.start_time_ticks += 1;
                    Ok(foreign)
                }
            )
            .is_err()
        );
    }
    #[test]
    fn fixed_load_request_cannot_auto_run_or_select_other_devices() {
        let request = load_request();
        assert_eq!(request["resume_vm"], false);
        assert_eq!(request["mem_backend"]["backend_type"], "File");
        assert_eq!(
            request["mem_backend"]["backend_path"],
            "/restore/restore-memory.bin"
        );
        assert_eq!(request["snapshot_path"], "/restore/restore-state.bin");
        assert_eq!(request["network_overrides"], json!([]));
        assert_eq!(request["track_dirty_pages"], false);
        assert_eq!(request["clock_realtime"], false);
    }
}
