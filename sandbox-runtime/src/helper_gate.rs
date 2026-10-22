//! Per-position mutation admission with a durable exact-process witness.
//! The child cannot exec until its PID/start/boot/context marker is synced.
//! A surviving helper blocks every later lease even if it closes every FD.

use super::child_process::{HelperLimits, OwnedChild};
use super::{Runtime, RuntimeError, RuntimeHostSlot, check_trusted_path, read_proc};
use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
};

const EXEC_GATE: &str =
    "IFS= read -r release || exit 1; [ \"$release\" = release ] || exit 1; exec \"$@\"";
pub const MAX_HELPER_MARKER_BYTES: u64 = 16_384;

/// Fixed gate inode created by the trusted node's mapping initialization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeHelperGate {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
    pub max_data_routes_per_slot: u64,
}

/// Read-only state of one bounded data-route cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeRouteState {
    Idle,
    Live,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRouteSnapshot {
    pub index: u64,
    pub control: Option<bool>,
    pub state: RuntimeRouteState,
}

/// Kernel process identity, valid across executable changes and PID reuse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeProcessIdentity {
    pub pid: u32,
    pub start_time_ticks: u64,
    pub boot_id: String,
}

impl RuntimeProcessIdentity {
    /// Capture a just-spawned, still-owned child's exact identity.
    /// # Errors
    /// Rejects a vanished process or unavailable boot/process observations.
    pub fn capture(pid: u32) -> Result<Self, RuntimeError> {
        let boot_id = boot_id()?;
        let process =
            read_proc(pid)?.ok_or(RuntimeError::Protocol("helper launch gate vanished"))?;
        Ok(Self {
            pid,
            start_time_ticks: process.start_time_ticks,
            boot_id,
        })
    }

    /// Check this exact task, including unreaped zombies; only disappearance
    /// or a different PID identity proves its bounded process cell is free.
    /// # Errors
    /// Invalid records and unavailable observations remain unknown.
    pub fn is_alive(&self) -> Result<bool, RuntimeError> {
        self.validate_fields()?;
        if boot_id()? != self.boot_id {
            return Ok(false);
        }
        // Unreaped tasks, including zombies, retain their capacity cell.
        Ok(read_proc(self.pid)?
            .is_some_and(|process| process.start_time_ticks == self.start_time_ticks))
    }

    /// Validate representation only; this does not observe or authorize a task.
    /// # Errors
    /// Rejects invalid PID/start fields and noncanonical host boot identities.
    pub fn validate_fields(&self) -> Result<(), RuntimeError> {
        if self.pid == 0
            || self.pid > i32::MAX as u32
            || self.start_time_ticks == 0
            || !valid_boot_id(&self.boot_id)
        {
            return Err(RuntimeError::IdentityMismatch(
                "helper process identity fields",
            ));
        }
        Ok(())
    }
}

fn valid_boot_id(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

pub(super) fn boot_id() -> Result<String, RuntimeError> {
    let value = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let value = value.trim();
    if !valid_boot_id(value) {
        return Err(RuntimeError::Protocol("host boot identity"));
    }
    Ok(value.to_owned())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InflightRecord {
    version: u32,
    context: RuntimeHostSlot,
    process: RuntimeProcessIdentity,
    command_digest: [u8; 32],
    route_index: Option<u64>,
    control: Option<bool>,
}

#[derive(Clone)]
struct OwnedMarker {
    record: InflightRecord,
    device: u64,
    inode: u64,
}

/// A held admission lock. Its drop only closes this descriptor, never calls
/// `LOCK_UN`. Durable in-flight evidence survives every guard/process death.
pub struct RuntimeHelperGuard {
    file: File,
    gate: RuntimeHelperGate,
    shell: PathBuf,
    owner: u32,
    active: RefCell<Option<OwnedMarker>>,
    expected: Option<RuntimeHostSlot>,
}

impl RuntimeHostSlot {
    /// Pure preflight for the largest future singleton or route marker. The
    /// context must carry maximum-width incarnation and inode fields when
    /// used to validate a deployment before its first helper-gate write.
    /// # Errors
    /// Rejects contexts whose fixed records would exceed the enforced cap.
    pub fn validate_marker_capacity(&self) -> Result<(), RuntimeError> {
        let record = InflightRecord {
            version: u32::MAX,
            context: self.clone(),
            process: RuntimeProcessIdentity {
                pid: u32::MAX,
                start_time_ticks: u64::MAX,
                boot_id: "ffffffff-ffff-ffff-ffff-ffffffffffff".to_owned(),
            },
            command_digest: [u8::MAX; 32],
            route_index: Some(u64::MAX),
            control: Some(false),
        };
        if serde_json::to_vec(&record)?.len() as u64 > MAX_HELPER_MARKER_BYTES {
            return Err(RuntimeError::InvalidConfiguration(
                "future helper marker exceeds byte limit",
            ));
        }
        Ok(())
    }
}

impl RuntimeHelperGate {
    /// Fixed canonical and staging files beside the immutable gate inode.
    /// # Errors
    /// Rejects a gate path without a UTF-8 filename.
    pub fn inflight_paths(&self) -> Result<(PathBuf, PathBuf), RuntimeError> {
        let name = self
            .path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or(RuntimeError::InvalidId)?;
        let marker = format!("{name}.inflight.json");
        Ok((
            self.path.with_file_name(&marker),
            self.path.with_file_name(format!(".staging-{marker}")),
        ))
    }

    /// Fixed canonical and staging files for one bounded data-route cell.
    /// # Errors
    /// Rejects indices outside the explicitly configured route budget.
    pub fn route_paths(&self, index: u64) -> Result<(PathBuf, PathBuf), RuntimeError> {
        if index >= self.max_data_routes_per_slot {
            return Err(RuntimeError::InvalidConfiguration(
                "data route cell exceeds capacity",
            ));
        }
        let name = self
            .path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .ok_or(RuntimeError::InvalidId)?;
        let marker = format!("{name}.route-{index}.json");
        Ok((
            self.path.with_file_name(&marker),
            self.path.with_file_name(format!(".staging-{marker}")),
        ))
    }

    /// Observe all fixed route cells without taking a lease or deleting state.
    /// Charge every Live/Unknown entry against admission, including incomplete
    /// stages and temporarily unavailable process observations.
    /// # Errors
    /// Rejects untrusted gates or malformed/foreign canonical authority.
    pub fn route_snapshot(
        &self,
        expected: Option<&RuntimeHostSlot>,
    ) -> Result<Vec<RuntimeRouteSnapshot>, RuntimeError> {
        check_trusted_path(&self.path)?;
        let metadata = fs::metadata(&self.path)?;
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
            || metadata.dev() != self.device
            || metadata.ino() != self.inode
            || self.max_data_routes_per_slot == 0
        {
            return Err(RuntimeError::IdentityMismatch("route census gate identity"));
        }
        self.snapshot_for_owner(expected, 0)
    }

    fn snapshot_for_owner(
        &self,
        expected: Option<&RuntimeHostSlot>,
        owner: u32,
    ) -> Result<Vec<RuntimeRouteSnapshot>, RuntimeError> {
        if expected.is_some_and(|context| context.helper_gate != *self) {
            return Err(RuntimeError::IdentityMismatch("route census expected gate"));
        }
        let mut result = Vec::new();
        for index in 0..self.max_data_routes_per_slot {
            let (path, stage) = self.route_paths(index)?;
            let marker = read_marker(&path, self, owner, Some(index))?;
            if marker
                .as_ref()
                .is_some_and(|marker| expected != Some(&marker.record.context))
            {
                return Err(RuntimeError::IdentityMismatch(
                    "route census full incarnation",
                ));
            }
            let staged = private_metadata(&stage, owner)?;
            if staged
                .as_ref()
                .is_some_and(|metadata| metadata.len() > MAX_HELPER_MARKER_BYTES)
            {
                return Err(RuntimeError::IdentityMismatch(
                    "route census staging byte bound",
                ));
            }
            let (control, observed) = if staged.is_some() {
                (None, RuntimeRouteState::Unknown)
            } else if let Some(marker) = marker {
                let observed = match marker.record.process.is_alive() {
                    Ok(true) => RuntimeRouteState::Live,
                    Ok(false) => RuntimeRouteState::Idle,
                    Err(_) => RuntimeRouteState::Unknown,
                };
                (marker.record.control, observed)
            } else {
                (None, RuntimeRouteState::Idle)
            };
            result.push(RuntimeRouteSnapshot {
                index,
                control,
                state: observed,
            });
        }
        Ok(result)
    }

    /// Acquire the exact gate and establish that no prior tracked helper lives.
    /// # Errors
    /// Missing/replaced/untrusted gates, live helpers and unknown evidence fail.
    pub fn try_acquire(
        &self,
        expected: Option<&RuntimeHostSlot>,
    ) -> Result<RuntimeHelperGuard, RuntimeError> {
        self.acquire_trusted(false, expected)
    }

    /// Acquire a shared read/relay lease after the same durable helper check.
    /// # Errors
    /// Refuses live helpers, an exclusive transition, or unknown/replaced state.
    pub fn try_acquire_shared(
        &self,
        expected: Option<&RuntimeHostSlot>,
    ) -> Result<RuntimeHelperGuard, RuntimeError> {
        self.acquire_trusted(true, expected)
    }

    fn acquire_trusted(
        &self,
        shared: bool,
        expected: Option<&RuntimeHostSlot>,
    ) -> Result<RuntimeHelperGuard, RuntimeError> {
        check_trusted_path(&self.path)?;
        let guard = self.acquire_file(File::open(&self.path)?, shared, 0, expected)?;
        check_trusted_path(&guard.shell)?;
        let current = fs::symlink_metadata(&self.path)?;
        if !current.is_file() || current.dev() != self.device || current.ino() != self.inode {
            return Err(RuntimeError::IdentityMismatch(
                "host helper gate replaced during acquire",
            ));
        }
        Ok(guard)
    }

    fn acquire_file(
        &self,
        file: File,
        shared: bool,
        owner: u32,
        expected: Option<&RuntimeHostSlot>,
    ) -> Result<RuntimeHelperGuard, RuntimeError> {
        if expected.is_some_and(|context| context.helper_gate != *self) {
            return Err(RuntimeError::IdentityMismatch(
                "expected helper gate binding",
            ));
        }
        if self.max_data_routes_per_slot == 0
            || usize::try_from(self.max_data_routes_per_slot).is_err()
        {
            return Err(RuntimeError::InvalidConfiguration(
                "invalid helper route capacity",
            ));
        }
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != owner
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
            || metadata.dev() != self.device
            || metadata.ino() != self.inode
            || self.inode == 0
        {
            return Err(RuntimeError::IdentityMismatch(
                "host helper gate identity or permissions",
            ));
        }
        lock(&file, shared)?;
        let guard = RuntimeHelperGuard {
            file,
            gate: self.clone(),
            shell: fs::canonicalize("/bin/sh")?,
            owner,
            active: RefCell::new(None),
            expected: expected.cloned(),
        };
        guard.recover_previous(shared)?;
        Ok(guard)
    }
}

fn lock(file: &File, shared: bool) -> Result<(), RuntimeError> {
    flock(
        file,
        if shared {
            FlockOperation::NonBlockingLockShared
        } else {
            FlockOperation::NonBlockingLockExclusive
        },
    )
    .map_err(|error| {
        if error == rustix::io::Errno::WOULDBLOCK {
            RuntimeError::InvalidConfiguration("previous physical-slot lease is still active")
        } else {
            RuntimeError::Io(io::Error::from(error))
        }
    })
}

impl RuntimeHelperGuard {
    /// Transfer the held OFD to another trusted lifetime owner. The caller
    /// must supply durable bounded process witnesses for its own child launches.
    #[must_use]
    pub fn into_file(self) -> File {
        self.file
    }

    fn read_cell(
        &self,
        path: &Path,
        index: Option<u64>,
    ) -> Result<Option<OwnedMarker>, RuntimeError> {
        let record = read_marker(path, &self.gate, self.owner, index)?;
        if record
            .as_ref()
            .is_some_and(|record| self.expected.as_ref() != Some(&record.record.context))
        {
            return Err(RuntimeError::IdentityMismatch(
                "helper marker differs from expected full incarnation",
            ));
        }
        Ok(record)
    }

    fn gated_command(&self, program: &str, args: &[&str]) -> Command {
        let mut command = Command::new(&self.shell);
        command
            .arg("-c")
            .arg(EXEC_GATE)
            .arg("sandproof-tracked-helper")
            .arg(program)
            .args(args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    /// Spawn a trusted synchronous helper under this slot's durable singleton
    /// process witness. Shell and helper retain the SAME PID through exec.
    /// The first stdin line only releases the gate; returned stdin is available
    /// for the actual script/body. Do not wrap a forking or daemonizing program.
    /// Call `finish_tracked` after wait; dropping this guard never clears proof.
    /// # Errors
    /// Refuses concurrent/stale helpers and leaves uncertain evidence intact.
    pub fn spawn_tracked(
        &self,
        context: &RuntimeHostSlot,
        program: &str,
        args: &[&str],
    ) -> Result<OwnedChild, RuntimeError> {
        self.validate_context(context)?;
        let (path, stage) = self.gate.inflight_paths()?;
        if super::path_present(&path)? {
            return Err(RuntimeError::IdentityMismatch(
                "previous tracked helper has not been discharged",
            ));
        }
        let staged = File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&stage)?;
        self.spawn_reserved(context, None, None, &path, &stage, staged, program, args)
    }

    /// Whether a configured route cell is absent or its exact process exited.
    /// An uncommitted reservation remains unavailable until exclusive recovery.
    /// # Errors
    /// Rejects invalid indices, foreign records and unknown process observations.
    pub fn route_idle(&self, index: u64) -> Result<bool, RuntimeError> {
        let (path, stage) = self.gate.route_paths(index)?;
        if super::path_present(&stage)? {
            return Ok(false);
        }
        match self.read_cell(&path, Some(index))? {
            Some(record) => Ok(matches!(record.record.process.is_alive(), Ok(false))),
            None => Ok(true),
        }
    }

    /// Reserve one fixed route cell and return the same-PID released child.
    /// Its marker deliberately remains after wait; only a later cell reservation
    /// or exclusive recovery may discharge it after proving exact process exit.
    /// # Errors
    /// Refuses exhausted capacity, another incarnation or unknown old helpers.
    pub fn spawn_route(
        &self,
        context: &RuntimeHostSlot,
        control: bool,
        program: &str,
        args: &[&str],
    ) -> Result<Child, RuntimeError> {
        self.validate_context(context)?;
        let (singleton, singleton_stage) = self.gate.inflight_paths()?;
        if self.read_cell(&singleton, None)?.is_some() || super::path_present(&singleton_stage)? {
            return Err(RuntimeError::InvalidConfiguration(
                "singleton helper publication is pending",
            ));
        }
        for index in 0..self.gate.max_data_routes_per_slot {
            if !self.route_idle(index)? {
                continue;
            }
            let (path, stage) = self.gate.route_paths(index)?;
            let staged = match File::options()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&stage)
            {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            };
            // create_new serializes this cell even when other shared leases
            // exist. No asynchronous old-route finisher can remove its marker.
            if let Some(old) = self.read_cell(&path, Some(index))? {
                if old.record.process.is_alive().unwrap_or(true) {
                    fs::remove_file(&stage)?;
                    sync_parent(&stage)?;
                    continue;
                }
                fs::remove_file(&path)?;
            }
            return self
                .spawn_reserved(
                    context,
                    Some(index),
                    Some(control),
                    &path,
                    &stage,
                    staged,
                    program,
                    args,
                )
                .map(OwnedChild::into_child);
        }
        Err(RuntimeError::InvalidConfiguration(
            "all bounded data route cells are occupied",
        ))
    }

    fn validate_context(&self, context: &RuntimeHostSlot) -> Result<(), RuntimeError> {
        if self.expected.as_ref() != Some(context)
            || context.helper_gate != self.gate
            || self.active.borrow().is_some()
        {
            return Err(RuntimeError::IdentityMismatch(
                "tracked helper gate context",
            ));
        }
        for index in 0..self.gate.max_data_routes_per_slot {
            let (path, _) = self.gate.route_paths(index)?;
            self.read_cell(&path, Some(index))?;
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one gated publication covers both fixed singleton and fixed route cells"
    )]
    fn spawn_reserved(
        &self,
        context: &RuntimeHostSlot,
        route_index: Option<u64>,
        control: Option<bool>,
        path: &Path,
        stage: &Path,
        mut staged: File,
        program: &str,
        args: &[&str],
    ) -> Result<OwnedChild, RuntimeError> {
        sync_parent(stage)?;
        let mut child = match OwnedChild::spawn(&mut self.gated_command(program, args)) {
            Ok(child) => child,
            Err(error) => {
                // No child obtained permission or published a process witness.
                // Remove only this still-empty reservation; otherwise a clean
                // shared data pin would prevent its later exclusive recovery.
                self.abandon_unspawned_stage(stage, &staged)?;
                return Err(error.into());
            }
        };
        self.publish_and_release(
            context,
            route_index,
            control,
            path,
            stage,
            &mut staged,
            program,
            args,
            &mut child,
        )?;
        Ok(child)
    }

    fn abandon_unspawned_stage(&self, stage: &Path, staged: &File) -> Result<(), RuntimeError> {
        let original = staged.metadata()?;
        let current = private_metadata(stage, self.owner)?.ok_or(
            RuntimeError::IdentityMismatch("unspawned helper reservation disappeared"),
        )?;
        if current.dev() != original.dev()
            || current.ino() != original.ino()
            || current.len() != 0
            || original.len() != 0
        {
            return Err(RuntimeError::IdentityMismatch(
                "unspawned helper reservation changed",
            ));
        }
        fs::remove_file(stage)?;
        sync_parent(stage)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "publication binds one pre-spawn reservation and its exact owned process"
    )]
    fn publish_and_release(
        &self,
        context: &RuntimeHostSlot,
        route_index: Option<u64>,
        control: Option<bool>,
        path: &Path,
        stage: &Path,
        staged: &mut File,
        program: &str,
        args: &[&str],
        child: &mut Child,
    ) -> Result<(), RuntimeError> {
        let record = InflightRecord {
            version: 1,
            context: context.clone(),
            process: RuntimeProcessIdentity::capture(child.id())?,
            command_digest: Sha256::digest(serde_json::to_vec(&(program, args))?).into(),
            route_index,
            control,
        };
        let bytes = serde_json::to_vec(&record)?;
        if bytes.len() as u64 > MAX_HELPER_MARKER_BYTES {
            return Err(RuntimeError::InvalidConfiguration(
                "helper marker exceeds byte limit",
            ));
        }
        staged.write_all(&bytes)?;
        staged.sync_all()?;
        let metadata = staged.metadata()?;
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            stage,
            rustix::fs::CWD,
            path,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(io::Error::from)?;
        sync_parent(path)?;
        if route_index.is_none() {
            *self.active.borrow_mut() = Some(OwnedMarker {
                record,
                device: metadata.dev(),
                inode: metadata.ino(),
            });
        }
        child
            .stdin
            .as_mut()
            .ok_or(RuntimeError::Protocol("tracked helper gate pipe"))?
            .write_all(b"release\n")?;
        Ok(())
    }

    /// Discharge only this guard's exact completed helper, never a successor.
    /// # Errors
    /// Refuses live/unknown processes or a replaced canonical marker.
    pub fn finish_tracked(&self) -> Result<(), RuntimeError> {
        let Some(owned) = self.active.borrow().clone() else {
            return Err(RuntimeError::IdentityMismatch(
                "no tracked helper owned by this lease",
            ));
        };
        if owned.record.process.is_alive()? {
            return Err(RuntimeError::InvalidConfiguration(
                "tracked helper is still alive",
            ));
        }
        let (path, _) = self.gate.inflight_paths()?;
        if let Some(current) = self.read_cell(&path, None)? {
            if current.record != owned.record
                || current.device != owned.device
                || current.inode != owned.inode
            {
                return Err(RuntimeError::IdentityMismatch(
                    "tracked helper marker was replaced",
                ));
            }
            fs::remove_file(&path)?;
        }
        sync_parent(&path)?;
        *self.active.borrow_mut() = None;
        Ok(())
    }

    fn recover_previous(&self, shared: bool) -> Result<(), RuntimeError> {
        let (path, stage) = self.gate.inflight_paths()?;
        let previous = self.read_cell(&path, None)?;
        if let Some(previous) = &previous
            && previous.record.process.is_alive()?
        {
            return Err(RuntimeError::InvalidConfiguration(
                "previous tracked helper is still alive",
            ));
        }
        let mut route_stage_dirty = false;
        for index in 0..self.gate.max_data_routes_per_slot {
            let (route, staged) = self.gate.route_paths(index)?;
            if let Some(record) = self.read_cell(&route, Some(index))?
                && !shared
                && record.record.process.is_alive()?
            {
                return Err(RuntimeError::InvalidConfiguration(
                    "previous tracked data route is still alive",
                ));
            }
            // Even shared admission rejects malformed/foreign staged objects.
            if let Some(metadata) = private_metadata(&staged, self.owner)? {
                route_stage_dirty = true;
                if metadata.len() > MAX_HELPER_MARKER_BYTES {
                    return Err(RuntimeError::IdentityMismatch(
                        "data route staging exceeds byte bound",
                    ));
                }
            }
        }
        let singleton_dirty = previous.is_some() || super::path_present(&stage)?;
        if shared && !singleton_dirty && !route_stage_dirty {
            return Ok(());
        }
        // Upgrade before recovering a dead singleton. Existing shared leases
        // can make recovery pending but cannot cause premature completion.
        if shared {
            lock(&self.file, false)?;
        }
        self.clear_stopped_cell(&path, &stage, None)?;
        if shared {
            // Unpublished route reservations never received permission. An
            // exclusive table window proves no live publisher can still use
            // the stage, even if an older canonical route remains alive.
            for index in 0..self.gate.max_data_routes_per_slot {
                let (_, stage) = self.gate.route_paths(index)?;
                if private_metadata(&stage, self.owner)?.is_some() {
                    fs::remove_file(&stage)?;
                    sync_parent(&stage)?;
                }
            }
        } else {
            for index in 0..self.gate.max_data_routes_per_slot {
                let (path, stage) = self.gate.route_paths(index)?;
                self.clear_stopped_cell(&path, &stage, Some(index))?;
            }
        }
        if shared {
            flock(&self.file, FlockOperation::LockShared).map_err(io::Error::from)?;
        }
        Ok(())
    }

    fn clear_stopped_cell(
        &self,
        path: &Path,
        stage: &Path,
        index: Option<u64>,
    ) -> Result<(), RuntimeError> {
        if let Some(current) = self.read_cell(path, index)? {
            if current.record.process.is_alive()? {
                return Err(RuntimeError::InvalidConfiguration(
                    "tracked process remains alive",
                ));
            }
            fs::remove_file(path)?;
        }
        if let Some(metadata) = private_metadata(stage, self.owner)? {
            if metadata.len() > MAX_HELPER_MARKER_BYTES {
                return Err(RuntimeError::IdentityMismatch(
                    "helper staging exceeds byte bound",
                ));
            }
            // Permission is sent only after canonical publication and fsync.
            // Thus unpublished staging cannot authorize an untracked effect.
            fs::remove_file(stage)?;
        }
        sync_parent(path)
    }
}

fn private_metadata(path: &Path, owner: u32) -> Result<Option<fs::Metadata>, RuntimeError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file()
        || metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(RuntimeError::IdentityMismatch(
            "helper marker type or permissions",
        ));
    }
    Ok(Some(metadata))
}

fn read_marker(
    path: &Path,
    gate: &RuntimeHelperGate,
    owner: u32,
    route_index: Option<u64>,
) -> Result<Option<OwnedMarker>, RuntimeError> {
    let Some(metadata) = private_metadata(path, owner)? else {
        return Ok(None);
    };
    if metadata.len() > MAX_HELPER_MARKER_BYTES {
        return Err(RuntimeError::IdentityMismatch(
            "helper marker exceeds byte bound",
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_HELPER_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_HELPER_MARKER_BYTES {
        return Err(RuntimeError::IdentityMismatch("helper marker grew"));
    }
    let record: InflightRecord = serde_json::from_slice(&bytes)?;
    if record.version != 1
        || record.context.helper_gate != *gate
        || record.route_index != route_index
        || record.control.is_some() != route_index.is_some()
    {
        return Err(RuntimeError::IdentityMismatch("helper marker gate binding"));
    }
    record.process.validate_fields()?;
    sync_parent(path)?;
    Ok(Some(OwnedMarker {
        record,
        device: metadata.dev(),
        inode: metadata.ino(),
    }))
}

fn sync_parent(path: &Path) -> Result<(), RuntimeError> {
    File::open(path.parent().ok_or(RuntimeError::InvalidId)?)?.sync_all()?;
    Ok(())
}

pub(crate) fn helper_output(
    gate: Option<&RuntimeHelperGuard>,
    context: Option<&RuntimeHostSlot>,
    operation: &'static str,
    program: &str,
    args: &[&str],
    input: Option<&str>,
) -> Result<Vec<u8>, RuntimeError> {
    let input = input.unwrap_or_default().as_bytes();
    if input.len() > HelperLimits::CONTROL.input_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "host helper input exceeds budget",
        )
        .into());
    }
    let mut child = match (gate, context) {
        (Some(gate), Some(context)) => gate.spawn_tracked(context, program, args)?,
        (None, None) => OwnedChild::spawn(
            Command::new(program)
                .args(args)
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )?,
        _ => {
            return Err(RuntimeError::IdentityMismatch(
                "host helper gate/context presence",
            ));
        }
    };
    let result = child.exchange(input, HelperLimits::CONTROL)?;
    if let Some(gate) = gate {
        gate.finish_tracked()?;
    }
    checked_output(operation, result)
}

fn checked_output(operation: &'static str, result: Output) -> Result<Vec<u8>, RuntimeError> {
    if !result.status.success() {
        return Err(RuntimeError::HostCommand {
            operation,
            stderr: String::from_utf8_lossy(&result.stderr[..result.stderr.len().min(4096)])
                .into_owned(),
        });
    }
    Ok(result.stdout)
}

impl Runtime {
    pub(crate) fn acquire_helper_gate(&self) -> Result<Option<RuntimeHelperGuard>, RuntimeError> {
        self.config
            .host_slot
            .as_ref()
            .map(|slot| slot.helper_gate.try_acquire(Some(slot)))
            .transpose()
    }
    pub(crate) fn read_host_helper(
        &self,
        operation: &'static str,
        program: &str,
        args: &[&str],
    ) -> Result<Vec<u8>, RuntimeError> {
        let gate = self
            .config
            .host_slot
            .as_ref()
            .map(|slot| slot.helper_gate.try_acquire_shared(Some(slot)))
            .transpose()?;
        helper_output(
            gate.as_ref(),
            self.config.host_slot.as_ref(),
            operation,
            program,
            args,
            None,
        )
    }
    pub(crate) fn run_ip(
        &self,
        operation: &'static str,
        args: &[&str],
    ) -> Result<(), RuntimeError> {
        let gate = self.acquire_helper_gate()?;
        helper_output(
            gate.as_ref(),
            self.config.host_slot.as_ref(),
            operation,
            "/usr/bin/ip",
            args,
            None,
        )?;
        Ok(())
    }
    pub(crate) fn run_nft_in_namespace(
        &self,
        id: &str,
        operation: &'static str,
        args: &[&str],
        input: Option<&str>,
    ) -> Result<Vec<u8>, RuntimeError> {
        let gate = if input.is_some() {
            self.acquire_helper_gate()?
        } else {
            self.config
                .host_slot
                .as_ref()
                .map(|slot| slot.helper_gate.try_acquire_shared(Some(slot)))
                .transpose()?
        };
        let mut command_args = vec!["netns", "exec", id, "/usr/sbin/nft"];
        command_args.extend_from_slice(args);
        helper_output(
            gate.as_ref(),
            self.config.host_slot.as_ref(),
            operation,
            "/usr/bin/ip",
            &command_args,
            input,
        )
    }
    pub(crate) fn root_nft_script(
        &self,
        operation: &'static str,
        script: &str,
    ) -> Result<(), RuntimeError> {
        let gate = self.acquire_helper_gate()?;
        helper_output(
            gate.as_ref(),
            self.config.host_slot.as_ref(),
            operation,
            "/usr/sbin/nft",
            &["-f", "-"],
            Some(script),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};

    fn fixture(capacity: u64) -> (tempfile::TempDir, RuntimeHostSlot) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("0.lock");
        File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let context = RuntimeHostSlot {
            position: 0,
            uplink_index: 100,
            vm: u64::MAX,
            helper_gate: RuntimeHelperGate {
                path,
                device: metadata.dev(),
                inode: metadata.ino(),
                max_data_routes_per_slot: capacity,
            },
        };
        (temp, context)
    }

    fn acquire(
        context: &RuntimeHostSlot,
        shared: bool,
    ) -> Result<RuntimeHelperGuard, RuntimeError> {
        context.helper_gate.acquire_file(
            File::open(&context.helper_gate.path)?,
            shared,
            rustix::process::geteuid().as_raw(),
            Some(context),
        )
    }

    fn paused(guard: &RuntimeHelperGuard, context: &RuntimeHostSlot, route: bool) -> Child {
        let args = [
            "-c",
            "exec 3<&-; printf 'ready\\n'; IFS= read -r finish; [ \"$finish\" = finish ]",
            "private-command-argument",
        ];
        let mut child = if route {
            guard.spawn_route(context, false, "/bin/sh", &args)
        } else {
            guard
                .spawn_tracked(context, "/bin/sh", &args)
                .map(OwnedChild::into_child)
        }
        .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line, "ready\n");
        child
    }

    fn finish(mut child: Child) {
        child.stdin.take().unwrap().write_all(b"finish\n").unwrap();
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn last_open_file_description_owner_must_release_before_gate_can_be_reacquired() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (_temp, context) = fixture(1);
        let guard = acquire(&context, false).unwrap();
        // Model the same OFD held by an unrelated fork before its exec closes
        // CLOEXEC descriptors. No sleeps, retries or production unlock shortcut.
        let inherited = guard.file.try_clone().unwrap();
        let (ready, started) = std::sync::mpsc::sync_channel(0);
        let (release, waiting) = std::sync::mpsc::sync_channel(0);
        let owner = std::thread::spawn(move || {
            ready.send(()).unwrap();
            let _ = waiting.recv();
            drop(inherited);
        });
        started.recv().unwrap();
        drop(guard);
        let refused = matches!(
            acquire(&context, false),
            Err(RuntimeError::InvalidConfiguration(
                "previous physical-slot lease is still active"
            ))
        );
        release.send(()).unwrap();
        owner.join().unwrap();
        assert!(refused);
        assert!(acquire(&context, false).is_ok());
    }

    #[test]
    fn exact_live_marker_blocks_even_when_original_parent_lock_and_all_child_gate_fds_are_gone() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (_temp, context) = fixture(2);
        let guard = acquire(&context, false).unwrap();
        let child = paused(&guard, &context, false);
        let path = context.helper_gate.inflight_paths().unwrap().0;
        let marker = read_marker(
            &path,
            &context.helper_gate,
            rustix::process::geteuid().as_raw(),
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(marker.record.process.pid, child.id());
        assert!(
            !fs::read_to_string(&path)
                .unwrap()
                .contains("private-command-argument")
        );
        drop(guard);
        // Prove the flock itself is free; durable identity must still reject.
        let untracked_lock = File::open(&context.helper_gate.path).unwrap();
        flock(&untracked_lock, FlockOperation::NonBlockingLockExclusive).unwrap();
        drop(untracked_lock);
        assert!(matches!(
            acquire(&context, false),
            Err(RuntimeError::InvalidConfiguration(_))
        ));
        assert!(matches!(
            acquire(&context, true),
            Err(RuntimeError::InvalidConfiguration(_))
        ));
        finish(child);
        if let Err(error) = acquire(&context, false) {
            panic!("completed child did not drain: {error:?}");
        }
        assert!(!path.exists());
    }

    #[test]
    fn permission_gate_has_no_effect_at_every_unpublished_or_unreleased_crash_cut() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        for cut in 0..=3 {
            let (temp, context) = fixture(1);
            let guard = acquire(&context, false).unwrap();
            let effect = temp.path().join("effect");
            let mut child = guard
                .gated_command(
                    "/bin/sh",
                    &[
                        "-c",
                        "printf unsafe > \"$1\"",
                        "effect",
                        effect.to_str().unwrap(),
                    ],
                )
                .spawn()
                .unwrap();
            let record = InflightRecord {
                version: 1,
                context: context.clone(),
                process: RuntimeProcessIdentity::capture(child.id()).unwrap(),
                command_digest: [0; 32],
                route_index: None,
                control: None,
            };
            let (path, stage) = context.helper_gate.inflight_paths().unwrap();
            let bytes = serde_json::to_vec(&record).unwrap();
            let mut file = File::options()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&stage)
                .unwrap();
            if cut == 1 {
                file.write_all(&bytes[..bytes.len() / 2]).unwrap();
            }
            if cut >= 2 {
                file.write_all(&bytes).unwrap();
                file.sync_all().unwrap();
                fs::rename(&stage, &path).unwrap();
                sync_parent(&path).unwrap();
            }
            if cut == 3 {
                child.kill().unwrap();
            }
            drop(child.stdin.take());
            assert!(!child.wait().unwrap().success());
            drop(guard);
            if let Err(error) = acquire(&context, false) {
                panic!("crash cut {cut}: {error:?}");
            }
            assert!(!effect.exists());
            assert!(!stage.exists());
            assert!(!path.exists());
        }
    }

    #[test]
    fn bounded_routes_coexist_but_block_reuse_and_recycle_only_exact_stopped_cells() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (temp, context) = fixture(2);
        let guard = acquire(&context, true).unwrap();
        let first = paused(&guard, &context, true);
        drop(guard);
        let guard = acquire(&context, true).unwrap();
        let second = paused(&guard, &context, true);
        drop(guard);
        let guard = acquire(&context, true).unwrap();
        assert!(!guard.route_idle(0).unwrap());
        assert!(!guard.route_idle(1).unwrap());
        assert!(
            guard
                .spawn_route(&context, false, "/bin/true", &[])
                .is_err()
        );
        let mut wrong = context.clone();
        wrong.vm -= 1;
        assert!(guard.spawn_route(&wrong, false, "/bin/true", &[]).is_err());
        drop(guard);
        assert!(matches!(
            acquire(&context, false),
            Err(RuntimeError::InvalidConfiguration(_))
        ));
        let first_pid = first.id();
        finish(first);
        let guard = acquire(&context, true).unwrap();
        assert!(guard.route_idle(0).unwrap());
        let third = paused(&guard, &context, true);
        let (path, _) = context.helper_gate.route_paths(0).unwrap();
        let replacement = read_marker(
            &path,
            &context.helper_gate,
            rustix::process::geteuid().as_raw(),
            Some(0),
        )
        .unwrap()
        .unwrap();
        assert_ne!(replacement.record.process.pid, first_pid);
        assert_eq!(replacement.record.process.pid, third.id());
        assert!(fs::read_dir(temp.path()).unwrap().count() <= 5);
        drop(guard);
        finish(second);
        finish(third);
        assert!(acquire(&context, false).is_ok());
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn abandoned_route_stage_recovers_without_consuming_capacity_forever() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (_temp, context) = fixture(1);
        let (_, stage) = context.helper_gate.route_paths(0).unwrap();
        File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&stage)
            .unwrap()
            .write_all(b"{\"partial\"")
            .unwrap();
        let guard = acquire(&context, true).unwrap();
        assert!(guard.route_idle(0).unwrap());
        assert!(!stage.exists());
        let child = paused(&guard, &context, true);
        drop(guard);
        finish(child);
        assert!(acquire(&context, false).is_ok());
    }

    #[test]
    fn failed_gate_spawn_clears_its_stage_while_a_clean_shared_data_pin_remains() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (_temp, context) = fixture(3);
        let pin = acquire(&context, true).unwrap().into_file();
        let mut guard = acquire(&context, true).unwrap();
        guard.shell = context.helper_gate.path.with_file_name("missing-shell");
        assert!(matches!(
            guard.spawn_route(&context, false, "/bin/true", &[]),
            Err(RuntimeError::Io(error)) if error.kind() == io::ErrorKind::NotFound
        ));
        let (path, stage) = context.helper_gate.route_paths(0).unwrap();
        assert!(!path.exists());
        assert!(!stage.exists());
        drop(guard);
        // A new control request can acquire its shared gate without waiting
        // for the unrelated request's clean pin to disappear.
        let control = acquire(&context, true).unwrap();
        assert!(control.route_idle(0).unwrap());
        drop(control);
        drop(pin);
    }

    #[test]
    fn unspawned_stage_cleanup_preserves_replacement_inodes_and_changed_bytes() {
        let (temp, context) = fixture(1);
        let guard = acquire(&context, true).unwrap();
        let (_, stage) = context.helper_gate.route_paths(0).unwrap();
        let mut original = File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&stage)
            .unwrap();
        fs::rename(&stage, temp.path().join("original-reservation")).unwrap();
        let mut replacement = File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&stage)
            .unwrap();
        assert!(guard.abandon_unspawned_stage(&stage, &original).is_err());
        assert!(stage.exists());
        replacement.write_all(b"foreign").unwrap();
        assert!(guard.abandon_unspawned_stage(&stage, &replacement).is_err());
        assert_eq!(fs::read(&stage).unwrap(), b"foreign");
        original.write_all(b"original").unwrap();
        assert_eq!(
            fs::read(temp.path().join("original-reservation")).unwrap(),
            b"original"
        );
    }

    #[test]
    fn synchronous_completion_clears_only_its_owned_marker() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (_temp, context) = fixture(1);
        let guard = acquire(&context, true).unwrap();
        let child = paused(&guard, &context, false);
        assert!(guard.finish_tracked().is_err());
        finish(child);
        guard.finish_tracked().unwrap();
        assert!(!context.helper_gate.inflight_paths().unwrap().0.exists());
        let child = paused(&guard, &context, false);
        finish(child);
        let marker = context.helper_gate.inflight_paths().unwrap().0;
        fs::rename(&marker, marker.with_extension("old")).unwrap();
        File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&marker)
            .unwrap()
            .write_all(b"{}")
            .unwrap();
        assert!(guard.finish_tracked().is_err());
        assert_eq!(fs::read(&marker).unwrap(), b"{}");
    }

    #[test]
    fn stopped_foreign_full_context_and_never_used_markers_are_preserved() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (_temp, context) = fixture(2);
        let guard = acquire(&context, false).unwrap();
        finish(paused(&guard, &context, false));
        drop(guard);
        let path = context.helper_gate.inflight_paths().unwrap().0;
        let original = fs::read(&path).unwrap();
        for field in 0..3 {
            let mut other = context.clone();
            match field {
                0 => other.vm -= 1,
                1 => other.position += 1,
                _ => other.uplink_index += 1,
            }
            assert!(matches!(
                acquire(&other, false),
                Err(RuntimeError::IdentityMismatch(_))
            ));
            assert_eq!(fs::read(&path).unwrap(), original);
        }
        assert!(matches!(
            context.helper_gate.acquire_file(
                File::open(&context.helper_gate.path).unwrap(),
                false,
                rustix::process::geteuid().as_raw(),
                None
            ),
            Err(RuntimeError::IdentityMismatch(_))
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(acquire(&context, false).is_ok());
        assert!(!path.exists());
    }

    #[test]
    fn unreaped_route_zombie_keeps_its_fixed_capacity_cell_reserved() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (_temp, context) = fixture(1);
        let guard = acquire(&context, true).unwrap();
        let mut child = paused(&guard, &context, true);
        let identity = RuntimeProcessIdentity::capture(child.id()).unwrap();
        let pid = rustix::process::Pid::from_raw(i32::try_from(child.id()).unwrap()).unwrap();
        let pidfd = rustix::process::pidfd_open(pid, rustix::process::PidfdFlags::empty()).unwrap();
        drop(guard);
        child.stdin.take().unwrap().write_all(b"finish\n").unwrap();
        assert!(super::super::wait_pidfd_exit(&pidfd, std::time::Duration::from_secs(5)).unwrap());
        assert!(
            identity.is_alive().unwrap(),
            "zombie must not look like released capacity"
        );
        assert!(matches!(
            acquire(&context, false),
            Err(RuntimeError::InvalidConfiguration(_))
        ));
        assert!(child.wait().unwrap().success());
        assert!(!identity.is_alive().unwrap());
        assert!(acquire(&context, false).is_ok());
    }

    #[test]
    fn uncommitted_marker_staging_never_adopts_foreign_types_or_permissions() {
        use std::os::unix::fs::PermissionsExt;
        for variant in 0..3 {
            let (temp, context) = fixture(1);
            let (_, stage) = context.helper_gate.inflight_paths().unwrap();
            let foreign = temp.path().join("foreign");
            fs::write(&foreign, b"keep").unwrap();
            match variant {
                0 => fs::create_dir(&stage).unwrap(),
                1 => std::os::unix::fs::symlink(&foreign, &stage).unwrap(),
                _ => {
                    fs::write(&stage, b"partial").unwrap();
                    fs::set_permissions(&stage, fs::Permissions::from_mode(0o644)).unwrap();
                }
            }
            assert!(matches!(
                acquire(&context, false),
                Err(RuntimeError::IdentityMismatch(_))
            ));
            assert!(fs::symlink_metadata(&stage).is_ok());
            assert_eq!(fs::read(&foreign).unwrap(), b"keep");
        }
    }

    fn assert_reaped(id: u32) {
        let pid = rustix::process::Pid::from_raw(i32::try_from(id).unwrap()).unwrap();
        assert!(matches!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
            Err(rustix::io::Errno::CHILD)
        ));
    }

    #[test]
    fn truncated_marker_publication_error_reaps_before_returning_to_caller() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (_temp, context) = fixture(1);
        let guard = acquire(&context, false).unwrap();
        let (path, stage) = context.helper_gate.inflight_paths().unwrap();
        File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&stage)
            .unwrap()
            .write_all(b"{\"partial\"")
            .unwrap();
        let mut staged = File::open(&stage).unwrap(); // deterministic write failure
        let mut child = OwnedChild::new(guard.gated_command("/bin/true", &[]).spawn().unwrap());
        let id = child.id();
        assert!(
            guard
                .publish_and_release(
                    &context,
                    None,
                    None,
                    &path,
                    &stage,
                    &mut staged,
                    "/bin/true",
                    &[],
                    &mut child
                )
                .is_err()
        );
        drop(child);
        assert_reaped(id);
        assert_eq!(fs::read(&stage).unwrap(), b"{\"partial\"");
        assert!(!path.exists());
        drop(guard);
        assert!(acquire(&context, false).is_ok());
        assert!(!stage.exists());
    }

    #[test]
    fn missing_and_broken_command_stdin_errors_reap_the_exact_child() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        for broken_pipe in [false, true] {
            let (temp, context) = fixture(1);
            let guard = acquire(&context, false).unwrap();
            let fifo = temp.path().join("wait");
            rustix::fs::mkfifoat(
                rustix::fs::CWD,
                &fifo,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            )
            .unwrap();
            let child = if broken_pipe {
                guard
                    .spawn_tracked(
                        &context,
                        "/bin/sh",
                        &[
                            "-c",
                            "exec 0<&-; printf 'ready\n'; IFS= read -r end < \"$1\"",
                            "closed-input",
                            fifo.to_str().unwrap(),
                        ],
                    )
                    .unwrap()
                    .into_child()
            } else {
                paused(&guard, &context, false)
            };
            let mut child = OwnedChild::new(child);
            let id = child.id();
            if broken_pipe {
                let mut line = String::new();
                BufReader::new(child.stdout.take().unwrap())
                    .read_line(&mut line)
                    .unwrap();
                assert_eq!(line, "ready\n");
            } else {
                drop(child.stdin.take());
            }
            assert!(child.exchange(b"body", HelperLimits::CONTROL).is_err());
            drop(child);
            assert_reaped(id);
            guard.finish_tracked().unwrap();
        }
    }

    #[test]
    fn timed_out_exchange_preserves_original_marker_until_exact_observation() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (_temp, context) = fixture(1);
        let guard = acquire(&context, false).unwrap();
        let mut child = guard
            .spawn_tracked(&context, "/bin/sh", &["-c", "while :; do :; done"])
            .unwrap();
        let id = child.id();
        let (path, _) = context.helper_gate.inflight_paths().unwrap();
        let original = fs::read(&path).unwrap();
        let error = child
            .exchange(
                &[],
                HelperLimits {
                    timeout: std::time::Duration::from_millis(100),
                    ..HelperLimits::CONTROL
                },
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert_reaped(id);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(guard.spawn_tracked(&context, "/bin/true", &[]).is_err());
        // This only discharges the exited process witness, not its side effects.
        guard.finish_tracked().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn route_census_preserves_control_class_and_charges_partial_cells_unknown() {
        let _serial = super::super::PROCESS_TEST_LOCK.lock().unwrap();
        let (_temp, context) = fixture(3);
        let guard = acquire(&context, true).unwrap();
        let ordinary = paused(&guard, &context, true);
        let mut control = guard
            .spawn_route(
                &context,
                true,
                "/bin/sh",
                &[
                    "-c",
                    "printf 'ready\n'; IFS= read -r finish; [ \"$finish\" = finish ]",
                ],
            )
            .unwrap();
        let mut line = String::new();
        BufReader::new(control.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line, "ready\n");
        let (_, stage) = context.helper_gate.route_paths(2).unwrap();
        File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&stage)
            .unwrap()
            .write_all(b"{\"control\":")
            .unwrap();
        let census = context
            .helper_gate
            .snapshot_for_owner(Some(&context), rustix::process::geteuid().as_raw())
            .unwrap();
        assert_eq!(
            census
                .iter()
                .map(|cell| (cell.control, cell.state))
                .collect::<Vec<_>>(),
            vec![
                (Some(false), RuntimeRouteState::Live),
                (Some(true), RuntimeRouteState::Live),
                (None, RuntimeRouteState::Unknown)
            ]
        );
        assert_eq!(
            census
                .iter()
                .filter(|cell| cell.state != RuntimeRouteState::Idle)
                .count(),
            3
        );
        let mut other = context.clone();
        other.vm -= 1;
        assert!(
            context
                .helper_gate
                .snapshot_for_owner(Some(&other), rustix::process::geteuid().as_raw())
                .is_err()
        );
        drop(guard);
        finish(ordinary);
        finish(control);
        assert!(acquire(&context, false).is_ok());
    }

    #[test]
    fn malformed_saved_boot_id_is_corruption_not_evidence_of_old_process_exit() {
        let (_temp, context) = fixture(1);
        let valid = RuntimeProcessIdentity::capture(std::process::id()).unwrap();
        assert!(valid_boot_id(&valid.boot_id));
        for boot in [
            "",
            "old",
            "00000000-0000-0000-0000-00000000000",
            "00000000-0000-0000-0000-00000000000G",
            "AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA",
            "000000000000-0000-0000-000000000000",
        ] {
            let invalid = RuntimeProcessIdentity {
                boot_id: boot.into(),
                ..valid.clone()
            };
            assert!(matches!(
                invalid.is_alive(),
                Err(RuntimeError::IdentityMismatch(_))
            ));
        }
        let (path, _) = context.helper_gate.route_paths(0).unwrap();
        let record = InflightRecord {
            version: 1,
            context: context.clone(),
            process: RuntimeProcessIdentity {
                boot_id: "old".into(),
                ..valid
            },
            command_digest: [0; 32],
            route_index: Some(0),
            control: Some(false),
        };
        let bytes = serde_json::to_vec(&record).unwrap();
        File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        assert!(matches!(
            acquire(&context, false),
            Err(RuntimeError::IdentityMismatch(_))
        ));
        assert!(
            context
                .helper_gate
                .snapshot_for_owner(Some(&context), rustix::process::geteuid().as_raw())
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn missing_replaced_and_corrupt_authority_never_become_busy_or_idle() {
        let (temp, context) = fixture(1);
        let marker = context.helper_gate.inflight_paths().unwrap().0;
        File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&marker)
            .unwrap()
            .write_all(b"not JSON")
            .unwrap();
        assert!(matches!(
            acquire(&context, false),
            Err(RuntimeError::Json(_))
        ));
        fs::remove_file(&marker).unwrap();
        fs::rename(&context.helper_gate.path, temp.path().join("old.lock")).unwrap();
        fs::write(&context.helper_gate.path, b"replacement").unwrap();
        assert!(matches!(
            acquire(&context, false),
            Err(RuntimeError::IdentityMismatch(_))
        ));
    }
}
