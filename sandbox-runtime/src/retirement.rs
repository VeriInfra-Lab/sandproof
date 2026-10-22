//! Bounded tombstone retirement. The node persists this immutable manifest
//! before deleting anything, and serializes all access to the physical slot.

use super::{
    NetworkRecord, ProcessRecord, ResourceMarker, Runtime, RuntimeConfig, RuntimeError,
    check_trusted_path, path_present, read_trusted_record, verified_process,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, Metadata},
    io::{self, Read},
    os::unix::fs::MetadataExt,
    path::Path,
};

use super::staging::{MAX_CLAIM_RECORD_BYTES, claim_file_names, known_claim_file};

/// A bounded exact-delete capability, durable outside the claim it retires.
/// Fields are deliberately private; consumers may persist/round-trip it but
/// must capture it with `Runtime::retirement_manifest` before deletion starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetirementManifest {
    version: u32,
    resource_id: String,
    config: RuntimeConfig,
    claim: Option<FileIdentity>,
    files: BTreeMap<String, RetiredFile>,
    process: Option<ProcessRecord>,
    uplink_intent: bool,
    namespace: Option<NamespaceIdentity>,
    unlaunched_network_absence: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NamespaceIdentity {
    device: u64,
    inode: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    inode: u64,
    owner: u32,
    mode: u32,
}

impl FileIdentity {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
            mode: metadata.mode(),
        }
    }
    fn matches(&self, metadata: &Metadata) -> bool {
        self == &Self::from_metadata(metadata)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetiredFile {
    identity: FileIdentity,
    size: u64,
    change_seconds: i64,
    change_nanoseconds: i64,
    // Console can be large, but has stopped changing once the process exits.
    // Exact inode/size/ctime is sufficient; record files also bind their bytes.
    digest: Option<[u8; 32]>,
}

impl RetiredFile {
    fn capture(path: &Path, console: bool) -> Result<Self, RuntimeError> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_file() || metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
            return Err(RuntimeError::IdentityMismatch(
                "retirement file type or permissions",
            ));
        }
        let digest = if console {
            None
        } else {
            Some(record_digest(path)?)
        };
        Ok(Self {
            identity: FileIdentity::from_metadata(&metadata),
            size: metadata.len(),
            change_seconds: metadata.ctime(),
            change_nanoseconds: metadata.ctime_nsec(),
            digest,
        })
    }
    fn verify(&self, path: &Path) -> Result<(), RuntimeError> {
        let current = Self::capture(path, self.digest.is_none())?;
        if &current != self {
            return Err(RuntimeError::IdentityMismatch(
                "retirement file identity or content changed",
            ));
        }
        Ok(())
    }
}

fn record_digest(path: &Path) -> Result<[u8; 32], RuntimeError> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_CLAIM_RECORD_BYTES {
        return Err(RuntimeError::IdentityMismatch(
            "retirement record exceeds byte bound",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CLAIM_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_CLAIM_RECORD_BYTES {
        return Err(RuntimeError::IdentityMismatch(
            "retirement record grew beyond byte bound",
        ));
    }
    Ok(Sha256::digest(bytes).into())
}

impl RetirementManifest {
    /// Pure serialized upper bound for one future retirement manifest. The
    /// supplied runtime configuration and resource ID must already bound the
    /// caller's configured paths/identity. This returns only a byte count,
    /// never an exact-delete capability. All canonical and staged claim names
    /// are charged, including the legacy console entry's metadata (not its
    /// contents). Numeric fields use their longest JSON representation.
    /// # Errors
    /// Rejects configurations that cannot be serialized or sized as u64.
    pub fn maximum_serialized_bytes(
        config: &RuntimeConfig,
        resource_id: &str,
    ) -> Result<u64, RuntimeError> {
        let identity = FileIdentity {
            device: u64::MAX,
            inode: u64::MAX,
            owner: u32::MAX,
            mode: u32::MAX,
        };
        let file = RetiredFile {
            identity: identity.clone(),
            size: u64::MAX,
            change_seconds: i64::MIN,
            change_nanoseconds: i64::MIN,
            digest: Some([u8::MAX; 32]),
        };
        let files = claim_file_names()
            .map(|name| (name, file.clone()))
            .collect();
        let value = Self {
            version: u32::MAX,
            resource_id: resource_id.to_owned(),
            config: config.clone(),
            claim: Some(identity),
            files,
            process: Some(ProcessRecord {
                resource_id: resource_id.to_owned(),
                pid: u32::MAX,
                start_time_ticks: u64::MAX,
                boot_id: Some("ffffffff-ffff-ffff-ffff-ffffffffffff".into()),
            }),
            uplink_intent: false,
            namespace: Some(NamespaceIdentity {
                device: u64::MAX,
                inode: u64::MAX,
            }),
            unlaunched_network_absence: false,
        };
        u64::try_from(serde_json::to_vec(&value)?.len())
            .map_err(|_| RuntimeError::InvalidConfiguration("retirement size overflow"))
    }
}

impl Runtime {
    /// Capture the bounded claim deletion plan after physical cleanup.
    /// The caller must durably store it before calling `retire` and retain the
    /// same runtime configuration throughout retries and free-slot observation.
    /// # Errors
    /// Rejects a live/unknown resource, untrusted claim or unrecognized entry.
    pub fn retirement_manifest(&self, id: &str) -> Result<RetirementManifest, RuntimeError> {
        self.validate_network_id(id)?;
        if self.config.host_slot.is_none() {
            return Err(RuntimeError::InvalidConfiguration(
                "retirement requires an explicit physical host slot",
            ));
        }
        let _gate = self.acquire_helper_gate()?;
        let claim_path = self.config.chroot_base.join(".claims").join(id);
        let mut manifest = RetirementManifest {
            version: 1,
            resource_id: id.to_owned(),
            config: self.config.clone(),
            claim: None,
            files: BTreeMap::new(),
            process: None,
            uplink_intent: false,
            namespace: None,
            unlaunched_network_absence: false,
        };
        if path_present(&claim_path)? {
            check_trusted_path(&claim_path)?;
            let metadata = fs::symlink_metadata(&claim_path)?;
            if !metadata.is_dir() || metadata.mode() & 0o077 != 0 {
                return Err(RuntimeError::IdentityMismatch(
                    "retirement claim type or permissions",
                ));
            }
            manifest.claim = Some(FileIdentity::from_metadata(&metadata));
            for entry in fs::read_dir(&claim_path)? {
                let entry = entry?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| RuntimeError::InvalidId)?;
                if !known_claim_file(name.as_str()) {
                    return Err(RuntimeError::IdentityMismatch(
                        "unknown retirement claim entry",
                    ));
                }
                let file = RetiredFile::capture(&entry.path(), name == "console.log")?;
                if file.identity.owner != 0 {
                    return Err(RuntimeError::IdentityMismatch("retirement file owner"));
                }
                manifest.files.insert(name, file);
            }
            if manifest.files.contains_key("record.json") {
                manifest.process = Some(self.read_record(id)?);
            } else if !self.was_reclaimed_unlaunched(id)? {
                return Err(RuntimeError::MissingRecord);
            }
            if manifest.files.contains_key("network.json") {
                let record: NetworkRecord = read_trusted_record(&claim_path.join("network.json"))?;
                if record.resource_id != id || record.namespace_inode == 0 {
                    return Err(RuntimeError::IdentityMismatch(
                        "retirement namespace record",
                    ));
                }
                manifest.namespace = Some(NamespaceIdentity {
                    inode: record.namespace_inode,
                    device: record
                        .namespace_device
                        .ok_or(RuntimeError::IdentityMismatch(
                            "retirement namespace device missing",
                        ))?,
                });
            } else if manifest.files.contains_key("network-intent.json") {
                let intent: ResourceMarker =
                    read_trusted_record(&claim_path.join("network-intent.json"))?;
                if intent.resource_id != id || !permits_unlaunched_namespace_absence(&manifest) {
                    return Err(RuntimeError::IdentityMismatch(
                        "retirement namespace identity was not preserved",
                    ));
                }
                // The exact helper/route gate is already drained. Canonical
                // network.json precedes every VMM and guest/relay entrant; the
                // validated abort marker plus absent physical resources below
                // proves the never-entered branch without inventing an inode.
                manifest.unlaunched_network_absence = true;
            }
            manifest.uplink_intent = self.validate_retirement_uplink_intent(id)?;
        }
        self.require_cgroup_absent(id)?;
        if self.config.private_netns.is_none()
            && manifest
                .files
                .keys()
                .any(|name| name.starts_with("network") || name.starts_with("uplink"))
        {
            return Err(RuntimeError::IdentityMismatch(
                "recorded private network cannot be disabled for retirement",
            ));
        }
        if !manifest.uplink_intent && manifest.files.contains_key("uplink-prepared.json") {
            return Err(RuntimeError::IdentityMismatch(
                "prepared uplink lost its intent",
            ));
        }
        self.verify_retirement_physical(id, &manifest)?;
        // A claim with no uplink intent cannot authorize deleting unrelated
        // flows. Initial slot adoption must have established pool emptiness.
        if !manifest.uplink_intent {
            self.require_retired_conntrack_absent(id)?;
        }
        Ok(manifest)
    }

    /// Delete only the recorded stopped claim's exact files and empty folder.
    /// Missing entries are accepted on retry; replacements or new entries are
    /// rejected before deleting any more files. No recursive deletion is used.
    /// # Errors
    /// Any mismatch or unknown physical/conntrack observation leaves release
    /// incomplete. Persist the manifest and retry under the same slot binding.
    pub fn retire(&self, id: &str, manifest: &RetirementManifest) -> Result<(), RuntimeError> {
        self.validate_retirement_manifest(id, manifest)?;
        let gate = self.acquire_helper_gate()?;
        self.verify_retirement_physical(id, manifest)?;
        let claim = self.config.chroot_base.join(".claims").join(id);
        verify_claim(&claim, manifest)?;
        if manifest.uplink_intent {
            self.purge_retired_conntrack(id, gate.as_ref())?;
        } else {
            self.require_retired_conntrack_absent(id)?;
        }
        retire_claim(&claim, manifest)?;
        self.observe_retired_locked(id, manifest)
    }

    /// Read-only verification of a Free binding, including restart recovery.
    /// # Errors
    /// Rejects residual/recreated claims, physical resources, flows or context.
    pub fn observe_retired(
        &self,
        id: &str,
        manifest: &RetirementManifest,
    ) -> Result<(), RuntimeError> {
        self.validate_retirement_manifest(id, manifest)?;
        let _gate = self.acquire_helper_gate()?;
        self.observe_retired_locked(id, manifest)
    }

    /// Recheck a binding whose complete retirement was durably confirmed Free.
    ///
    /// The node must have persisted Free only after `retire` and
    /// `observe_retired` succeeded under the original incarnation. At that
    /// point no thread or descriptor held the old namespace. Its inode may
    /// subsequently identify an unrelated namespace, so it is no longer a
    /// live identity witness. Named resources, original PID/start, claims and
    /// the exclusive physical address are still checked on every reuse.
    /// # Errors
    /// Rejects changed manifests, helpers, recreated resources or address state.
    pub fn observe_released(
        &self,
        id: &str,
        manifest: &RetirementManifest,
    ) -> Result<(), RuntimeError> {
        self.validate_retirement_manifest(id, manifest)?;
        let _gate = self.acquire_helper_gate()?;
        self.verify_retirement_locations(id, manifest)?;
        if path_present(&self.config.chroot_base.join(".claims").join(id))? {
            return Err(RuntimeError::IdentityMismatch("retired claim still exists"));
        }
        self.require_retired_conntrack_absent(id)
    }

    fn observe_retired_locked(
        &self,
        id: &str,
        manifest: &RetirementManifest,
    ) -> Result<(), RuntimeError> {
        self.verify_retirement_physical(id, manifest)?;
        if path_present(&self.config.chroot_base.join(".claims").join(id))? {
            return Err(RuntimeError::IdentityMismatch("retired claim still exists"));
        }
        self.require_retired_conntrack_absent(id)
    }

    fn validate_retirement_manifest(
        &self,
        id: &str,
        manifest: &RetirementManifest,
    ) -> Result<(), RuntimeError> {
        self.validate_network_id(id)?;
        if manifest.version != 1
            || manifest.resource_id != id
            || manifest.config != self.config
            || self.config.host_slot.is_none()
            || manifest
                .files
                .keys()
                .any(|name| !known_claim_file(name.as_str()))
            || manifest.files.values().any(|file| file.identity.owner != 0)
            || manifest
                .claim
                .as_ref()
                .is_some_and(|claim| claim.owner != 0)
            || (manifest.files.contains_key("record.json") != manifest.process.is_some())
            || (manifest.files.contains_key("network.json") != manifest.namespace.is_some())
            || (manifest.unlaunched_network_absence
                && !permits_unlaunched_namespace_absence(manifest))
            || (manifest.files.contains_key("network-intent.json")
                && manifest.namespace.is_none()
                && !manifest.unlaunched_network_absence)
            || manifest
                .namespace
                .as_ref()
                .is_some_and(|namespace| namespace.inode == 0)
            || (manifest.uplink_intent != manifest.files.contains_key("uplink-intent.json"))
            || (manifest.claim.is_some()
                && manifest.process.is_none()
                && !manifest.files.contains_key("aborted.json"))
            || manifest
                .files
                .iter()
                .any(|(name, file)| (name == "console.log") != file.digest.is_none())
            || (manifest.claim.is_none()
                && (!manifest.files.is_empty()
                    || manifest.process.is_some()
                    || manifest.uplink_intent))
            || manifest.process.as_ref().is_some_and(|process| {
                process.resource_id != id || process.pid == 0 || process.start_time_ticks == 0
            })
        {
            return Err(RuntimeError::IdentityMismatch(
                "retirement manifest context",
            ));
        }
        Ok(())
    }

    fn verify_retirement_physical(
        &self,
        id: &str,
        manifest: &RetirementManifest,
    ) -> Result<(), RuntimeError> {
        self.verify_retirement_locations(id, manifest)?;
        if let Some(namespace) = &manifest.namespace {
            require_namespace_unheld(Path::new("/proc"), namespace)?;
        }
        Ok(())
    }

    fn verify_retirement_locations(
        &self,
        id: &str,
        manifest: &RetirementManifest,
    ) -> Result<(), RuntimeError> {
        if let Some(process) = &manifest.process
            && verified_process(process)?.is_some()
        {
            return Err(RuntimeError::IdentityMismatch("retirement process is live"));
        }
        if self.observe_jail(id)? != super::JailObservation::Absent {
            return Err(RuntimeError::IdentityMismatch(
                "retirement jail is not absent",
            ));
        }
        let namespace = self.config.private_netns.as_deref();
        let cgroup = self
            .config
            .host_resources
            .as_ref()
            .map(|settings| settings.cgroup_parent.join(id));
        for path in [namespace, cgroup.as_deref()].into_iter().flatten() {
            if path_present(path)? {
                return Err(RuntimeError::IdentityMismatch(
                    "retirement physical resource remains",
                ));
            }
        }
        if !self.uplink_resources_absent(id)? {
            return Err(RuntimeError::IdentityMismatch("retirement uplink remains"));
        }
        // The original position remains closed and its helper gate is held.
        // UID absence supplements, rather than replaces, the exact process,
        // namespace-holder, backing and exclusive-incarnation obligations.
        super::uid_retirement::require_host_uid_unheld(self.config.vm_uid)?;
        Ok(())
    }
}

fn permits_unlaunched_namespace_absence(manifest: &RetirementManifest) -> bool {
    manifest.process.is_none()
        && manifest.namespace.is_none()
        && manifest.files.contains_key("aborted.json")
        && manifest.files.contains_key("network-intent.json")
        && !manifest.files.keys().any(|name| {
            let name = name.strip_prefix(".staging-").unwrap_or(name);
            name == "record.json" || name == "failed-prepare.json" || name.starts_with("uplink-")
        })
}

/// Inspect thread namespaces and FD tables, rather than only named handles.
/// A relay may retain an open namespace FD without entering the namespace;
/// non-leader threads may also have their own namespace or descriptor table.
fn require_namespace_unheld(
    proc_root: &Path,
    namespace: &NamespaceIdentity,
) -> Result<(), RuntimeError> {
    for process in fs::read_dir(proc_root)? {
        let process = process?;
        if !numeric_proc_name(&process.file_name()) {
            continue;
        }
        let tasks = match fs::read_dir(process.path().join("task")) {
            Ok(tasks) => tasks,
            Err(error) if process_disappeared(&error) => continue,
            Err(error) => return Err(error.into()),
        };
        for task in tasks {
            let task = task?;
            if !numeric_proc_name(&task.file_name()) {
                continue;
            }
            require_task_namespace_unheld(&task.path(), namespace, || Ok(()))?;
        }
    }
    Ok(())
}

fn require_task_namespace_unheld(
    task: &Path,
    namespace: &NamespaceIdentity,
    after_fd_scan: impl FnOnce() -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    let descriptors = match fs::read_dir(task.join("fd")) {
        Ok(descriptors) => descriptors,
        Err(error) if process_disappeared(&error) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for descriptor in descriptors {
        let descriptor = descriptor?;
        if numeric_proc_name(&descriptor.file_name()) {
            require_not_namespace_holder(&descriptor.path(), namespace)?;
        }
    }
    // The named handle was already absent before this pass. Check descriptors
    // before membership so a same-PID setns-and-close transition cannot fall
    // between a root-namespace observation and a later empty descriptor list.
    after_fd_scan()?;
    require_not_namespace_holder(&task.join("ns/net"), namespace)
}

fn numeric_proc_name(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit()))
}

fn process_disappeared(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error())
}

fn require_not_namespace_holder(
    path: &Path,
    namespace: &NamespaceIdentity,
) -> Result<(), RuntimeError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if process_disappeared(&error) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.dev() == namespace.device && metadata.ino() == namespace.inode {
        return Err(RuntimeError::IdentityMismatch(
            "retired namespace still held by a process or descriptor",
        ));
    }
    Ok(())
}

fn verify_claim(path: &Path, manifest: &RetirementManifest) -> Result<bool, RuntimeError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir()
        || manifest
            .claim
            .as_ref()
            .is_none_or(|identity| !identity.matches(&metadata))
    {
        return Err(RuntimeError::IdentityMismatch(
            "retirement claim identity changed",
        ));
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| RuntimeError::InvalidId)?;
        let file = manifest
            .files
            .get(&name)
            .ok_or(RuntimeError::IdentityMismatch(
                "unrecorded retirement entry",
            ))?;
        file.verify(&entry.path())?;
    }
    Ok(true)
}

fn retire_claim(path: &Path, manifest: &RetirementManifest) -> Result<(), RuntimeError> {
    if !verify_claim(path, manifest)? {
        // An earlier process may have removed the entry then crashed before
        // syncing its parent. Absence alone is not durable completion.
        let parent = path.parent().ok_or(RuntimeError::InvalidId)?;
        if parent.exists() {
            File::open(parent)?.sync_all()?;
        }
        return Ok(());
    }
    for (name, file) in &manifest.files {
        let target = path.join(name);
        if path_present(&target)? {
            file.verify(&target)?;
            fs::remove_file(target)?;
        }
    }
    File::open(path)?.sync_all()?;
    fs::remove_dir(path)?;
    File::open(path.parent().ok_or(RuntimeError::InvalidId)?)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RuntimeHostSlot;
    use std::{
        io::Write,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        path::PathBuf,
    };

    const ID: &str = "vm-0123456789abcdef-9000000";

    fn config(base: &Path) -> RuntimeConfig {
        RuntimeConfig {
            console_policy: None,
            host_slot: Some(RuntimeHostSlot {
                position: 0,
                helper_gate: crate::RuntimeHelperGate {
                    path: base.join("helper.lock"),
                    device: 1,
                    inode: 1,
                    max_data_routes_per_slot: 2,
                },
                uplink_index: 200,
                vm: 9_000_000,
            }),
            jailer: PathBuf::from("/usr/bin/true"),
            firecracker: PathBuf::from("/usr/bin/true"),
            chroot_base: base.to_owned(),
            kernel: PathBuf::from("/kernel"),
            rootfs: PathBuf::from("/rootfs"),
            vm_uid: 10001,
            vm_gid: 10001,
            vcpu_count: 1,
            memory_mib: 128,
            host_resources: None,
            private_netns: None,
            access_token_hash: None,
        }
    }

    fn fixture() -> (tempfile::TempDir, RetirementManifest) {
        let temp = tempfile::tempdir().unwrap();
        let claim = temp.path().join(".claims").join(ID);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&claim)
            .unwrap();
        let mut files = BTreeMap::new();
        for (name, bytes) in [
            ("record.json", "{\"resource_id\":\"old\"}"),
            ("console.log", "stopped"),
            ("network.json", "{}"),
            (".staging-record.json", "{\"resource_id\":"),
        ] {
            fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(claim.join(name))
                .unwrap()
                .write_all(bytes.as_bytes())
                .unwrap();
            files.insert(
                name.into(),
                RetiredFile::capture(&claim.join(name), name == "console.log").unwrap(),
            );
        }
        let manifest = RetirementManifest {
            version: 1,
            resource_id: ID.into(),
            config: config(temp.path()),
            claim: Some(FileIdentity::from_metadata(
                &fs::symlink_metadata(claim).unwrap(),
            )),
            files,
            process: None,
            uplink_intent: false,
            namespace: None,
            unlaunched_network_absence: false,
        };
        (temp, manifest)
    }

    #[test]
    fn manifest_storage_bound_covers_all_claim_names_and_numeric_extremes() {
        let (_temp, mut manifest) = fixture();
        let expected = RetirementManifest::maximum_serialized_bytes(&manifest.config, ID).unwrap();
        let sample = manifest.files["record.json"].clone();
        for name in claim_file_names() {
            assert!(known_claim_file(&name));
            let mut file = sample.clone();
            file.identity.device = u64::MAX;
            file.identity.inode = u64::MAX;
            file.identity.owner = u32::MAX;
            file.identity.mode = u32::MAX;
            file.size = u64::MAX;
            file.change_seconds = i64::MIN;
            file.change_nanoseconds = i64::MIN;
            file.digest = Some([u8::MAX; 32]);
            manifest.files.insert(name, file);
        }
        manifest.process = Some(ProcessRecord {
            resource_id: ID.into(),
            pid: u32::MAX,
            start_time_ticks: u64::MAX,
            boot_id: Some("ffffffff-ffff-ffff-ffff-ffffffffffff".into()),
        });
        manifest.namespace = Some(NamespaceIdentity {
            device: u64::MAX,
            inode: u64::MAX,
        });
        assert!(serde_json::to_vec(&manifest).unwrap().len() as u64 <= expected);
        let prior = expected;
        manifest.config.jailer =
            PathBuf::from(format!("{}suffix", manifest.config.jailer.display()));
        assert_eq!(
            RetirementManifest::maximum_serialized_bytes(&manifest.config, ID).unwrap() - prior,
            6
        );
    }

    #[test]
    fn never_launched_network_failure_requires_abort_and_no_later_launch_evidence() {
        let (_temp, mut manifest) = fixture();
        let entry = manifest.files["network.json"].clone();
        manifest.files.clear();
        manifest
            .files
            .insert("network-intent.json".into(), entry.clone());
        assert!(!permits_unlaunched_namespace_absence(&manifest));
        manifest.files.insert("aborted.json".into(), entry.clone());
        assert!(permits_unlaunched_namespace_absence(&manifest));
        for name in [
            "record.json",
            ".staging-record.json",
            "failed-prepare.json",
            "uplink-intent.json",
            ".staging-uplink-prepared.json",
        ] {
            manifest.files.insert(name.into(), entry.clone());
            assert!(!permits_unlaunched_namespace_absence(&manifest));
            manifest.files.remove(name);
        }
        manifest.namespace = Some(NamespaceIdentity {
            device: 1,
            inode: 1,
        });
        assert!(!permits_unlaunched_namespace_absence(&manifest));
    }

    #[test]
    #[ignore = "requires root on isolated Linux for real read-only namespace/conntrack observations"]
    fn durable_free_allows_recycled_namespace_inode_but_not_recreated_resources() {
        assert_eq!(rustix::process::geteuid().as_raw(), 0);
        let temp = tempfile::tempdir_in("/var/lib").unwrap();
        let mut config = config(temp.path());
        config.private_netns = Some(temp.path().join(ID));
        let slot = config.host_slot.as_mut().unwrap();
        slot.uplink_index = 900_001;
        let gate = File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&slot.helper_gate.path)
            .unwrap()
            .metadata()
            .unwrap();
        slot.helper_gate.device = gate.dev();
        slot.helper_gate.inode = gate.ino();
        let runtime = Runtime { config };
        let claim = temp.path().join(".claims").join(ID);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&claim)
            .unwrap();
        runtime
            .write_new_record(
                &claim.join("record.json"),
                &ProcessRecord {
                    resource_id: ID.into(),
                    pid: i32::MAX as u32,
                    start_time_ticks: 1,
                    boot_id: None,
                },
            )
            .unwrap();
        // Synthetic stopped identity: no VM or namespace is created by this
        // component fixture. Real host inventory must establish absence.
        runtime
            .write_new_record(
                &claim.join("network.json"),
                &NetworkRecord {
                    resource_id: ID.into(),
                    namespace_inode: u64::MAX,
                    namespace_device: Some(u64::MAX),
                },
            )
            .unwrap();
        let mut manifest = runtime.retirement_manifest(ID).unwrap();
        runtime.retire(ID, &manifest).unwrap();
        runtime.observe_retired(ID, &manifest).unwrap();
        let persisted = serde_json::to_vec(&manifest).unwrap();
        manifest = serde_json::from_slice(&persisted).unwrap();
        // Deterministic collision injection after complete retirement: model
        // the old numeric ID now belonging to an unrelated live namespace.
        // No production witness or kernel namespace is modified.
        let current = fs::metadata("/proc/self/ns/net").unwrap();
        manifest.namespace = Some(NamespaceIdentity {
            device: current.dev(),
            inode: current.ino(),
        });
        assert!(runtime.observe_retired(ID, &manifest).is_err());
        runtime.observe_released(ID, &manifest).unwrap();
        fs::create_dir(&claim).unwrap();
        assert!(runtime.observe_released(ID, &manifest).is_err());
        fs::remove_dir(&claim).unwrap();
        let namespace = runtime.config.private_netns.as_ref().unwrap();
        fs::write(namespace, b"foreign named resource").unwrap();
        assert!(runtime.observe_released(ID, &manifest).is_err());
        assert_eq!(fs::read(namespace).unwrap(), b"foreign named resource");
        fs::remove_file(namespace).unwrap();
        runtime.observe_released(ID, &manifest).unwrap();
        assert!(!claim.exists());
    }

    #[test]
    fn fd_to_namespace_transition_at_observation_boundary_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let task = temp.path().join("task");
        fs::create_dir_all(task.join("ns")).unwrap();
        fs::create_dir(task.join("fd")).unwrap();
        let held = temp.path().join("old-inode");
        fs::write(&held, b"namespace fixture").unwrap();
        fs::write(task.join("ns/net"), b"different namespace").unwrap();
        let metadata = fs::metadata(&held).unwrap();
        let namespace = NamespaceIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        assert!(
            require_task_namespace_unheld(&task, &namespace, || {
                fs::remove_file(task.join("ns/net"))?;
                fs::hard_link(&held, task.join("ns/net"))?;
                Ok(())
            })
            .is_err()
        );
    }

    #[test]
    fn namespace_scan_rejects_live_thread_and_open_fd_after_named_handle_disappears() {
        let temp = tempfile::tempdir().unwrap();
        let task = temp.path().join("123/task/124");
        fs::create_dir_all(task.join("ns")).unwrap();
        fs::create_dir(task.join("fd")).unwrap();
        let original = temp.path().join("namespace");
        fs::write(&original, b"namespace inode fixture").unwrap();
        let metadata = fs::metadata(&original).unwrap();
        let namespace = NamespaceIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        fs::hard_link(&original, task.join("ns/net")).unwrap();
        fs::remove_file(&original).unwrap();
        assert!(require_namespace_unheld(temp.path(), &namespace).is_err());
        fs::rename(task.join("ns/net"), task.join("fd/7")).unwrap();
        // The task need not be inside the namespace to keep it alive.
        assert!(require_namespace_unheld(temp.path(), &namespace).is_err());
        fs::remove_file(task.join("fd/7")).unwrap();
        assert!(require_namespace_unheld(temp.path(), &namespace).is_ok());
    }

    #[test]
    fn namespace_scan_does_not_confuse_inode_on_another_device_and_fails_unknown() {
        let temp = tempfile::tempdir().unwrap();
        let task = temp.path().join("1/task/2");
        fs::create_dir_all(task.join("ns")).unwrap();
        fs::create_dir(task.join("fd")).unwrap();
        fs::write(task.join("ns/net"), b"fixture").unwrap();
        let metadata = fs::metadata(task.join("ns/net")).unwrap();
        let namespace = NamespaceIdentity {
            device: metadata.dev() + 1,
            inode: metadata.ino(),
        };
        assert!(require_namespace_unheld(temp.path(), &namespace).is_ok());
        fs::remove_dir(task.join("fd")).unwrap();
        fs::write(task.join("fd"), b"unobservable descriptor table").unwrap();
        assert!(require_namespace_unheld(temp.path(), &namespace).is_err());
    }

    #[test]
    fn serialized_manifest_resumes_every_partial_claim_deletion() {
        for deleted in 0..=4 {
            let (temp, manifest) = fixture();
            let claim = temp.path().join(".claims").join(ID);
            let persisted = serde_json::to_vec(&manifest).unwrap();
            for name in manifest.files.keys().take(deleted) {
                fs::remove_file(claim.join(name)).unwrap();
            }
            let restored: RetirementManifest = serde_json::from_slice(&persisted).unwrap();
            retire_claim(&claim, &restored).unwrap();
            assert!(!claim.exists());
            retire_claim(&claim, &restored).unwrap();
        }
    }

    #[test]
    fn unlaunched_claim_partial_process_stage_is_removed_only_by_saved_manifest() {
        let (temp, mut manifest) = fixture();
        let claim = temp.path().join(".claims").join(ID);
        fs::remove_file(claim.join("record.json")).unwrap();
        manifest.files.remove("record.json");
        let aborted = claim.join("aborted.json");
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&aborted)
            .unwrap()
            .write_all(b"{\"resource_id\":\"vm-test\"}")
            .unwrap();
        manifest.files.insert(
            "aborted.json".into(),
            RetiredFile::capture(&aborted, false).unwrap(),
        );
        assert!(manifest.process.is_none());
        assert!(manifest.files.contains_key(".staging-record.json"));
        let persisted = serde_json::to_vec(&manifest).unwrap();
        let recovered: RetirementManifest = serde_json::from_slice(&persisted).unwrap();
        retire_claim(&claim, &recovered).unwrap();
        assert!(!claim.exists());
    }

    #[test]
    fn replaced_file_is_not_deleted_and_other_files_remain() {
        let (temp, manifest) = fixture();
        let claim = temp.path().join(".claims").join(ID);
        fs::rename(claim.join("network.json"), temp.path().join("old-network")).unwrap();
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(claim.join("network.json"))
            .unwrap()
            .write_all(b"{}")
            .unwrap();
        assert!(retire_claim(&claim, &manifest).is_err());
        assert!(claim.join("record.json").exists());
        assert!(claim.join("console.log").exists());
        assert!(claim.join("network.json").exists());
    }

    #[test]
    fn changed_record_or_unrecognized_entry_blocks_all_deletion() {
        let (temp, manifest) = fixture();
        let claim = temp.path().join(".claims").join(ID);
        fs::write(claim.join("network.json"), b"[]").unwrap();
        assert!(retire_claim(&claim, &manifest).is_err());
        assert!(claim.join("record.json").exists());
        let (temp, manifest) = fixture();
        let claim = temp.path().join(".claims").join(ID);
        fs::write(claim.join("foreign"), b"do not delete").unwrap();
        assert!(retire_claim(&claim, &manifest).is_err());
        assert!(claim.join("record.json").exists());
    }

    #[test]
    fn replaced_directory_or_symlink_is_not_adopted() {
        let (temp, manifest) = fixture();
        let claim = temp.path().join(".claims").join(ID);
        fs::rename(&claim, temp.path().join("old")).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&claim).unwrap();
        assert!(retire_claim(&claim, &manifest).is_err());
        fs::remove_dir(&claim).unwrap();
        std::os::unix::fs::symlink(temp.path().join("old"), &claim).unwrap();
        assert!(retire_claim(&claim, &manifest).is_err());
        assert!(temp.path().join("old").join("record.json").exists());
    }

    #[test]
    fn absent_claim_manifest_cannot_delete_a_later_claim() {
        let (temp, mut manifest) = fixture();
        manifest.claim = None;
        manifest.files.clear();
        let claim = temp.path().join(".claims").join(ID);
        assert!(retire_claim(&claim, &manifest).is_err());
        assert!(claim.join("record.json").exists());
    }

    #[test]
    fn stale_context_and_unknown_manifest_version_fail_before_host_access() {
        let (temp, mut manifest) = fixture();
        // No claim is needed to check all permanent/configuration bindings.
        manifest.claim = None;
        manifest.files.clear();
        let runtime = Runtime {
            config: config(temp.path()),
        };
        assert!(runtime.validate_retirement_manifest(ID, &manifest).is_ok());
        let mut changed = manifest.clone();
        changed.config.host_slot.as_mut().unwrap().vm += 1;
        assert!(runtime.retire(ID, &changed).is_err());
        changed = manifest.clone();
        changed.config.host_slot.as_mut().unwrap().position += 1;
        assert!(runtime.observe_retired(ID, &changed).is_err());
        changed = manifest.clone();
        changed.config.host_slot.as_mut().unwrap().uplink_index += 1;
        assert!(runtime.retire(ID, &changed).is_err());
        changed = manifest.clone();
        changed.config.vm_uid += 1;
        assert!(runtime.retire(ID, &changed).is_err());
        changed = manifest.clone();
        changed.version += 1;
        assert!(runtime.retire(ID, &changed).is_err());
        assert!(
            runtime
                .retire("vm-0123456789abcdef-9000001", &manifest)
                .is_err()
        );
    }

    #[test]
    fn manifest_shape_and_records_are_bounded() {
        let (temp, manifest) = fixture();
        assert!(serde_json::to_vec(&manifest).unwrap().len() < 4096);
        let record = temp.path().join("large-record.json");
        let file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&record)
            .unwrap();
        file.set_len(MAX_CLAIM_RECORD_BYTES + 1).unwrap();
        assert!(RetiredFile::capture(&record, false).is_err());
        assert!(RetiredFile::capture(&record, true).is_ok());
    }
}
