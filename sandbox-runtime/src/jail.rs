//! A jail is reserved and durably identified before the launch gate opens.
//! Its immutable witness survives physical deletion until claim retirement.

use super::{Runtime, RuntimeError, path_present, staging::MAX_CLAIM_RECORD_BYTES, validate_id};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Read},
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JailObservation {
    Absent,
    Registered {
        device: u64,
        inode: u64,
    },
    /// Presence is not deletion authority, including a crash before publication.
    Unregistered,
}

impl JailObservation {
    #[must_use]
    pub fn is_present(self) -> bool {
        self != Self::Absent
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DirectoryIdentity {
    device: u64,
    inode: u64,
    birth_seconds: i64,
    birth_nanoseconds: u32,
}

impl DirectoryIdentity {
    pub(super) fn capture(file: &File) -> Result<Self, RuntimeError> {
        let metadata = file.metadata()?;
        let extended = rustix::fs::statx(
            file,
            "",
            rustix::fs::AtFlags::EMPTY_PATH,
            rustix::fs::StatxFlags::BTIME,
        )
        .map_err(io::Error::from)?;
        if extended.stx_mask & rustix::fs::StatxFlags::BTIME.bits() == 0 {
            return Err(RuntimeError::IdentityMismatch(
                "filesystem lacks directory birth identity",
            ));
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            birth_seconds: extended.stx_btime.tv_sec,
            birth_nanoseconds: extended.stx_btime.tv_nsec,
        })
    }

    pub(super) fn matches(&self, file: &File) -> Result<bool, RuntimeError> {
        Ok(self == &Self::capture(file)?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JailRecord {
    version: u32,
    resource_id: String,
    vm_uid: u32,
    vm_gid: u32,
    host_slot: Option<[u64; 3]>,
    parent: DirectoryIdentity,
    jail: DirectoryIdentity,
    root: DirectoryIdentity,
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn owned_directory(path: &Path, owner: u32, private: bool) -> Result<File, RuntimeError> {
    let file = File::from(
        rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::CLOEXEC
                | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    let value = file.metadata()?;
    if !value.is_dir()
        || value.file_type().is_symlink()
        || value.uid() != owner
        || value.mode() & if private { 0o077 } else { 0o022 } != 0
    {
        return Err(RuntimeError::IdentityMismatch(
            "jail directory ownership or type",
        ));
    }
    Ok(file)
}

fn directory_at(parent: &File, name: &std::ffi::CStr) -> Result<Option<File>, RuntimeError> {
    match rustix::fs::openat(
        parent,
        name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    ) {
        Ok(fd) => Ok(Some(File::from(fd))),
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(io::Error::from(error).into()),
    }
}

fn unlink_directory(
    parent: &File,
    name: &std::ffi::CStr,
    expected: &DirectoryIdentity,
) -> Result<(), RuntimeError> {
    let opened =
        directory_at(parent, name)?.ok_or(RuntimeError::IdentityMismatch("directory vanished"))?;
    if !expected.matches(&opened)? {
        return Err(RuntimeError::IdentityMismatch(
            "directory replaced before unlink",
        ));
    }
    rustix::fs::unlinkat(parent, name, rustix::fs::AtFlags::REMOVEDIR).map_err(io::Error::from)?;
    parent.sync_all()?;
    Ok(())
}

/// Anchored traversal cannot follow a substituted pathname or a symlink out
/// of the stopped VM's opened root. Private-parent mutations are serialized;
/// unrelated privileged writers/mounts are not part of the host contract.
fn empty_directory(directory: &File, device: u64, depth: u16) -> Result<(), RuntimeError> {
    if depth >= 64 || directory.metadata()?.dev() != device {
        return Err(RuntimeError::IdentityMismatch(
            "unexpected jail mount or nesting",
        ));
    }
    let mut entries = rustix::fs::Dir::read_from(directory).map_err(io::Error::from)?;
    for entry in &mut entries {
        let entry = entry.map_err(io::Error::from)?;
        let name = entry.file_name();
        if name.to_bytes() == b"." || name.to_bytes() == b".." {
            continue;
        }
        let current = rustix::fs::statat(directory, name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
            .map_err(io::Error::from)?;
        if rustix::fs::FileType::from_raw_mode(current.st_mode) == rustix::fs::FileType::Directory {
            let child = directory_at(directory, name)?
                .ok_or(RuntimeError::IdentityMismatch("jail child vanished"))?;
            let identity = DirectoryIdentity::capture(&child)?;
            empty_directory(&child, device, depth + 1)?;
            unlink_directory(directory, name, &identity)?;
        } else {
            rustix::fs::unlinkat(directory, name, rustix::fs::AtFlags::empty())
                .map_err(io::Error::from)?;
        }
    }
    directory.sync_all()?;
    Ok(())
}

impl Runtime {
    pub(super) fn jail_path(&self, id: &str) -> PathBuf {
        self.config
            .chroot_base
            .join(self.firecracker_name())
            .join(id)
    }

    fn jail_record_path(&self, id: &str) -> PathBuf {
        self.config
            .chroot_base
            .join(".claims")
            .join(id)
            .join("jail.json")
    }

    fn jail_slot(&self) -> Option<[u64; 3]> {
        self.config
            .host_slot
            .as_ref()
            .map(|slot| [slot.position, slot.vm, slot.uplink_index])
    }

    fn read_jail_record(&self, id: &str, owner: u32) -> Result<Option<JailRecord>, RuntimeError> {
        let path = self.jail_record_path(id);
        let Some(current) = metadata(&path)? else {
            return Ok(None);
        };
        owned_directory(&self.config.chroot_base.join(".claims"), owner, true)?;
        let parent = path.parent().ok_or(RuntimeError::InvalidId)?;
        owned_directory(parent, owner, true)?;
        let descriptor = rustix::fs::open(
            &path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
            rustix::fs::Mode::empty(),
        )
        .map_err(io::Error::from)?;
        let file = File::from(descriptor);
        let opened = file.metadata()?;
        if !current.is_file()
            || current.file_type().is_symlink()
            || current.uid() != owner
            || current.mode() & 0o077 != 0
            || current.nlink() != 1
            || current.dev() != opened.dev()
            || current.ino() != opened.ino()
            || current.len() > MAX_CLAIM_RECORD_BYTES
        {
            return Err(RuntimeError::IdentityMismatch("jail witness identity"));
        }
        let mut bytes = Vec::new();
        file.take(MAX_CLAIM_RECORD_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_CLAIM_RECORD_BYTES {
            return Err(RuntimeError::IdentityMismatch("jail witness exceeds bound"));
        }
        let record: JailRecord = serde_json::from_slice(&bytes)?;
        if record.version != 1
            || record.resource_id != id
            || record.vm_uid != self.config.vm_uid
            || record.vm_gid != self.config.vm_gid
            || record.host_slot != self.jail_slot()
            || [&record.parent, &record.jail, &record.root]
                .iter()
                .any(|value| value.inode == 0 || value.birth_nanoseconds >= 1_000_000_000)
        {
            return Err(RuntimeError::IdentityMismatch("jail witness context"));
        }
        File::open(parent)?.sync_all()?;
        Ok(Some(record))
    }

    /// Root production callers pass zero; tests use their isolated fixture owner.
    pub(super) fn reserve_jail(&self, id: &str, owner: u32) -> Result<(), RuntimeError> {
        self.reserve_jail_with_writer(id, owner, |path, value| self.write_new_record(path, value))
    }

    // Controlled current-account fixture only. The real producer below still
    // captures the original open directory/birth identities and full slot;
    // root-owned bounded staging is independently exercised by the existing
    // privileged registration test, never disabled in production.
    #[cfg(test)]
    pub(super) fn reserve_jail_for_fixture(
        &self,
        id: &str,
        owner: u32,
    ) -> Result<(), RuntimeError> {
        self.reserve_jail_with_writer(id, owner, super::write_new_record)
    }

    fn reserve_jail_with_writer(
        &self,
        id: &str,
        owner: u32,
        publish: impl FnOnce(&Path, &JailRecord) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        validate_id(id)?;
        let record_path = self.jail_record_path(id);
        if path_present(&record_path)?
            || path_present(&record_path.with_file_name(".staging-jail.json"))?
        {
            return Err(RuntimeError::AlreadyClaimed);
        }
        owned_directory(&self.config.chroot_base, owner, false)?;
        owned_directory(&self.config.chroot_base.join(".claims"), owner, true)?;
        owned_directory(
            record_path.parent().ok_or(RuntimeError::InvalidId)?,
            owner,
            true,
        )?;
        let jail = self.jail_path(id);
        let parent = jail.parent().ok_or(RuntimeError::InvalidId)?;
        fs::DirBuilder::new()
            .mode(0o700)
            .create(parent)
            .or_else(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    Ok(())
                } else {
                    Err(error)
                }
            })?;
        let parent_identity = owned_directory(parent, owner, false)?;
        File::open(&self.config.chroot_base)?.sync_all()?;
        // Never adopt a pre-existing resource, even an empty root-owned one.
        fs::DirBuilder::new().mode(0o700).create(&jail)?;
        let root = jail.join("root");
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        let record = JailRecord {
            version: 1,
            resource_id: id.to_owned(),
            vm_uid: self.config.vm_uid,
            vm_gid: self.config.vm_gid,
            host_slot: self.jail_slot(),
            parent: DirectoryIdentity::capture(&parent_identity)?,
            jail: DirectoryIdentity::capture(&owned_directory(&jail, owner, true)?)?,
            root: DirectoryIdentity::capture(&owned_directory(&root, owner, true)?)?,
        };
        File::open(&root)?.sync_all()?;
        File::open(&jail)?.sync_all()?;
        File::open(parent)?.sync_all()?;
        // If publication fails or the controller dies before it, the directory
        // remains unregistered/unknown. No child can have passed the launch gate.
        publish(&record_path, &record)?;
        self.require_launch_jail(id, owner)
    }

    /// Read-only identity evidence; never adopts an unregistered directory.
    /// # Errors
    /// Rejects corrupt witnesses, replacement directories and changed context.
    pub fn observe_jail(&self, id: &str) -> Result<JailObservation, RuntimeError> {
        self.observe_jail_for_owner(id, 0)
    }

    fn observe_jail_for_owner(
        &self,
        id: &str,
        owner: u32,
    ) -> Result<JailObservation, RuntimeError> {
        validate_id(id)?;
        let record = self.read_jail_record(id, owner)?;
        let jail = self.jail_path(id);
        if let Some(record) = &record {
            let parent =
                owned_directory(jail.parent().ok_or(RuntimeError::InvalidId)?, owner, false)?;
            if !record.parent.matches(&parent)? {
                return Err(RuntimeError::IdentityMismatch(
                    "jail parent identity changed",
                ));
            }
        }
        let Some(current) = metadata(&jail)? else {
            return Ok(JailObservation::Absent);
        };
        let Some(record) = record else {
            return Ok(JailObservation::Unregistered);
        };
        let parent = owned_directory(jail.parent().ok_or(RuntimeError::InvalidId)?, owner, false)?;
        let directory = owned_directory(&jail, owner, true)?;
        if !record.parent.matches(&parent)?
            || !record.jail.matches(&directory)?
            || current.dev() != directory.metadata()?.dev()
            || current.ino() != directory.metadata()?.ino()
        {
            return Err(RuntimeError::IdentityMismatch(
                "jail directory identity changed",
            ));
        }
        if let Some(root) = directory_at(&directory, c"root")? {
            let value = root.metadata()?;
            if (value.uid() != owner && value.uid() != self.config.vm_uid)
                || value.mode() & 0o077 != 0
                || !record.root.matches(&root)?
            {
                return Err(RuntimeError::IdentityMismatch("jail root identity changed"));
            }
        }
        Ok(JailObservation::Registered {
            device: directory.metadata()?.dev(),
            inode: directory.metadata()?.ino(),
        })
    }

    pub(super) fn require_launch_jail(&self, id: &str, owner: u32) -> Result<(), RuntimeError> {
        if !matches!(
            self.observe_jail_for_owner(id, owner)?,
            JailObservation::Registered { .. }
        ) || metadata(&self.jail_path(id).join("root"))?.is_none()
        {
            return Err(RuntimeError::IdentityMismatch(
                "launch jail witness missing",
            ));
        }
        Ok(())
    }

    pub(super) fn cleanup_jail(
        &self,
        id: &str,
        owner: u32,
        unlaunched: bool,
    ) -> Result<(), RuntimeError> {
        self.cleanup_jail_with_probe(id, owner, unlaunched, || {})
    }

    pub(super) fn observe_cleanup_jail(
        &self,
        id: &str,
        owner: u32,
    ) -> Result<JailObservation, RuntimeError> {
        match self.observe_jail_for_owner(id, owner)? {
            JailObservation::Unregistered => Err(RuntimeError::IdentityMismatch(
                "unregistered jail directory",
            )),
            known => Ok(known),
        }
    }

    fn cleanup_jail_with_probe(
        &self,
        id: &str,
        owner: u32,
        unlaunched: bool,
        before_delete: impl FnOnce(),
    ) -> Result<(), RuntimeError> {
        let claim = owned_directory(
            self.jail_record_path(id)
                .parent()
                .ok_or(RuntimeError::InvalidId)?,
            owner,
            true,
        )?;
        rustix::fs::flock(&claim, rustix::fs::FlockOperation::LockExclusive)
            .map_err(io::Error::from)?;
        if self.observe_cleanup_jail(id, owner)? == JailObservation::Absent {
            return Ok(());
        }
        let jail = self.jail_path(id);
        let record = self
            .read_jail_record(id, owner)?
            .ok_or(RuntimeError::MissingRecord)?;
        let parent = owned_directory(jail.parent().ok_or(RuntimeError::InvalidId)?, owner, false)?;
        let name = std::ffi::CString::new(id).map_err(|_| RuntimeError::InvalidId)?;
        let directory =
            directory_at(&parent, &name)?.ok_or(RuntimeError::IdentityMismatch("jail vanished"))?;
        if !record.parent.matches(&parent)? || !record.jail.matches(&directory)? {
            return Err(RuntimeError::IdentityMismatch(
                "jail changed before cleanup",
            ));
        }
        // Only jailer owns the contents of root. Unknown siblings of root are
        // not part of that resource and must not be recursively taken over.
        let mut entries = rustix::fs::Dir::read_from(&directory).map_err(io::Error::from)?;
        for entry in &mut entries {
            let entry = entry.map_err(io::Error::from)?;
            if ![b".".as_slice(), b"..", b"root"].contains(&entry.file_name().to_bytes()) {
                return Err(RuntimeError::IdentityMismatch("unknown jail entry"));
            }
        }
        let root = directory_at(&directory, c"root")?;
        if let Some(root) = &root
            && !record.root.matches(root)?
        {
            return Err(RuntimeError::IdentityMismatch(
                "jail root changed before cleanup",
            ));
        }
        if unlaunched {
            if path_present(&self.jail_record_path(id).with_file_name("record.json"))? {
                return Err(RuntimeError::IdentityMismatch(
                    "unlaunched jail has a process record",
                ));
            }
            if let Some(root) = &root {
                let mut entries = rustix::fs::Dir::read_from(root).map_err(io::Error::from)?;
                if entries.any(|entry| {
                    entry.map_or(true, |entry| {
                        ![b".".as_slice(), b".."].contains(&entry.file_name().to_bytes())
                    })
                }) {
                    return Err(RuntimeError::IdentityMismatch(
                        "unlaunched jail root is not empty",
                    ));
                }
            }
        }
        before_delete();
        if let Some(root) = &root {
            if !unlaunched {
                empty_directory(root, record.jail.device, 0)?;
            }
            unlink_directory(&directory, c"root", &record.root)?;
        }
        unlink_directory(&parent, &name, &record.jail)?;
        // Keep jail.json unchanged: a cleanup retry must reject a replacement.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::RuntimeConfig;
    use super::*;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    const ID: &str = "review-jail";

    fn fixture() -> (tempfile::TempDir, Runtime, u32) {
        fixture_in(None, ID)
    }

    fn fixture_in(parent: Option<&Path>, id: &str) -> (tempfile::TempDir, Runtime, u32) {
        // CI and developer hosts can have a group-writable umask. The fixture
        // must satisfy the same private-parent contract as production.
        let mut builder = tempfile::Builder::new();
        builder.permissions(fs::Permissions::from_mode(0o700));
        let temp = match parent {
            Some(parent) => builder.tempdir_in(parent),
            None => builder.tempdir(),
        }
        .unwrap();
        let claim = temp.path().join(".claims").join(id);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(claim)
            .unwrap();
        let runtime = Runtime {
            config: RuntimeConfig {
                console_policy: None,
                host_slot: None,
                jailer: PathBuf::from("/usr/bin/true"),
                firecracker: PathBuf::from("/usr/bin/firecracker"),
                chroot_base: temp.path().to_owned(),
                kernel: PathBuf::new(),
                rootfs: PathBuf::new(),
                vm_uid: 10001,
                vm_gid: 10001,
                vcpu_count: 1,
                memory_mib: 128,
                host_resources: None,
                private_netns: None,
                access_token_hash: None,
            },
        };
        (temp, runtime, rustix::process::geteuid().as_raw())
    }

    #[test]
    fn reservation_precedes_launch_and_empty_unlaunched_abort_retains_its_witness() {
        let (_temp, runtime, owner) = fixture();
        assert!(runtime.require_launch_jail(ID, owner).is_err());
        runtime.reserve_jail(ID, owner).unwrap();
        runtime.require_launch_jail(ID, owner).unwrap();
        runtime.cleanup_jail(ID, owner, true).unwrap();
        assert_eq!(
            runtime.observe_jail_for_owner(ID, owner).unwrap(),
            JailObservation::Absent
        );
        assert!(runtime.jail_record_path(ID).exists());
        runtime.cleanup_jail(ID, owner, true).unwrap();
        assert!(runtime.reserve_jail(ID, owner).is_err());
        assert!(!runtime.jail_path(ID).exists());
    }

    #[test]
    fn first_cleanup_preserves_replaced_jail_and_root_directories() {
        for replace_root in [false, true] {
            let (temp, runtime, owner) = fixture();
            runtime.reserve_jail(ID, owner).unwrap();
            let target = if replace_root {
                runtime.jail_path(ID).join("root")
            } else {
                runtime.jail_path(ID)
            };
            fs::rename(&target, temp.path().join("original")).unwrap();
            fs::DirBuilder::new().mode(0o700).create(&target).unwrap();
            let canary = target.join("foreign-canary");
            fs::write(&canary, b"must remain").unwrap();
            assert!(runtime.cleanup_jail(ID, owner, false).is_err());
            assert!(runtime.require_launch_jail(ID, owner).is_err());
            assert_eq!(fs::read(canary).unwrap(), b"must remain");
        }
    }

    #[test]
    fn repeated_cleanup_preserves_a_replacement_directory_and_the_original_witness() {
        let (_temp, runtime, owner) = fixture();
        runtime.reserve_jail(ID, owner).unwrap();
        let jail = runtime.jail_path(ID);
        let original = File::open(&jail).unwrap();
        let original_identity = original.metadata().unwrap();
        fs::write(jail.join("root/private-disk"), b"original").unwrap();
        let witness = fs::read(runtime.jail_record_path(ID)).unwrap();
        runtime.cleanup_jail(ID, owner, false).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&jail).unwrap();
        let replacement = fs::symlink_metadata(&jail).unwrap();
        assert_ne!(
            (original_identity.dev(), original_identity.ino()),
            (replacement.dev(), replacement.ino())
        );
        let canary = jail.join("foreign-canary");
        fs::write(&canary, b"must remain").unwrap();
        assert!(runtime.cleanup_jail(ID, owner, false).is_err());
        assert_eq!(fs::read(canary).unwrap(), b"must remain");
        assert_eq!(fs::read(runtime.jail_record_path(ID)).unwrap(), witness);
    }

    #[test]
    fn missing_witness_at_each_directory_creation_boundary_remains_unknown() {
        for (root_created, staged) in [(false, false), (true, false), (true, true)] {
            let (_temp, runtime, owner) = fixture();
            let jail = runtime.jail_path(ID);
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&jail)
                .unwrap();
            if root_created {
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(jail.join("root"))
                    .unwrap();
            }
            if staged {
                fs::write(
                    runtime
                        .jail_record_path(ID)
                        .with_file_name(".staging-jail.json"),
                    b"{\"version\":",
                )
                .unwrap();
            }
            assert_eq!(
                runtime.observe_jail_for_owner(ID, owner).unwrap(),
                JailObservation::Unregistered
            );
            assert!(runtime.require_launch_jail(ID, owner).is_err());
            assert!(runtime.cleanup_jail(ID, owner, true).is_err());
            assert!(jail.exists());
        }
    }

    #[test]
    fn markerless_legacy_absence_can_finish_but_presence_is_never_adopted() {
        let (_temp, runtime, owner) = fixture();
        runtime.cleanup_jail(ID, owner, true).unwrap();
        assert!(!runtime.jail_record_path(ID).exists());
        let jail = runtime.jail_path(ID);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&jail)
            .unwrap();
        fs::write(jail.join("foreign"), b"keep").unwrap();
        assert!(runtime.cleanup_jail(ID, owner, true).is_err());
        assert!(runtime.cleanup_jail(ID, owner, false).is_err());
        assert!(runtime.reserve_jail(ID, owner).is_err());
        assert_eq!(fs::read(jail.join("foreign")).unwrap(), b"keep");
    }

    #[test]
    fn unlaunched_abort_refuses_unknown_contents_and_a_committed_process_record() {
        for has_process in [false, true] {
            let (_temp, runtime, owner) = fixture();
            runtime.reserve_jail(ID, owner).unwrap();
            let foreign = if has_process {
                runtime.jail_record_path(ID).with_file_name("record.json")
            } else {
                runtime.jail_path(ID).join("root/foreign")
            };
            fs::write(&foreign, b"keep").unwrap();
            assert!(runtime.cleanup_jail(ID, owner, true).is_err());
            assert!(runtime.jail_path(ID).join("root").exists());
            assert_eq!(fs::read(foreign).unwrap(), b"keep");
        }
    }

    #[test]
    fn changed_parent_and_configuration_cannot_turn_registered_resources_into_absence() {
        let (temp, mut runtime, owner) = fixture();
        runtime.reserve_jail(ID, owner).unwrap();
        runtime.config.vm_uid += 1;
        assert!(runtime.cleanup_jail(ID, owner, false).is_err());
        runtime.config.vm_uid -= 1;
        let parent = runtime.jail_path(ID).parent().unwrap().to_owned();
        fs::rename(&parent, temp.path().join("original-parent")).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&parent).unwrap();
        assert!(runtime.observe_jail_for_owner(ID, owner).is_err());
        assert!(temp.path().join("original-parent").join(ID).exists());
    }

    #[test]
    fn partial_physical_deletion_keeps_the_witness_and_cannot_release_a_missing_root_gate() {
        let (_temp, runtime, owner) = fixture();
        runtime.reserve_jail(ID, owner).unwrap();
        fs::remove_dir(runtime.jail_path(ID).join("root")).unwrap();
        assert!(runtime.require_launch_jail(ID, owner).is_err());
        runtime.cleanup_jail(ID, owner, false).unwrap();
        assert!(runtime.jail_record_path(ID).exists());
    }

    #[test]
    fn production_cleanup_retry_preserves_replacement_after_lost_receipt() {
        let (_temp, runtime, owner) = fixture();
        runtime.reserve_jail(ID, owner).unwrap();
        super::super::write_new_record(
            &runtime.jail_record_path(ID).with_file_name("record.json"),
            &super::super::ProcessRecord {
                resource_id: ID.to_owned(),
                pid: i32::MAX as u32,
                start_time_ticks: 1,
                boot_id: "01234567-89ab-cdef-0123-456789abcdef".into(),
                launch_cpu_domain: vec![0],
            },
        )
        .unwrap();
        let jail = runtime.jail_path(ID);
        let original = File::open(&jail).unwrap();
        runtime.cleanup_for_owner(ID, owner).unwrap();
        let witness = fs::read(runtime.jail_record_path(ID)).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&jail).unwrap();
        fs::write(jail.join("foreign-canary"), b"keep").unwrap();
        assert!(runtime.cleanup_for_owner(ID, owner).is_err());
        assert_eq!(fs::read(jail.join("foreign-canary")).unwrap(), b"keep");
        assert_eq!(fs::read(runtime.jail_record_path(ID)).unwrap(), witness);
        drop(original);
    }

    #[test]
    fn refused_cgroup_cleanup_preserves_the_registered_private_disk() {
        let (temp, mut runtime, owner) = fixture();
        runtime.reserve_jail(ID, owner).unwrap();
        let disk = runtime.jail_path(ID).join("root/private-disk");
        fs::write(&disk, b"keep until every host task is ruled out").unwrap();
        super::super::write_new_record(
            &runtime.jail_record_path(ID).with_file_name("record.json"),
            &super::super::ProcessRecord {
                resource_id: ID.into(),
                pid: i32::MAX as u32,
                start_time_ticks: 1,
                boot_id: "01234567-89ab-cdef-0123-456789abcdef".into(),
                launch_cpu_domain: vec![0],
            },
        )
        .unwrap();
        let parent = temp.path().join("groups");
        fs::create_dir_all(parent.join(ID)).unwrap();
        runtime.config.host_resources = Some(super::super::HostResources {
            cgroup_parent: parent.clone(),
            memory_max_bytes: 256 * 1024 * 1024,
        });
        // The unregistered group cannot be assumed empty just because the
        // recorded VMM stopped. Preserve the disk until that check succeeds.
        assert!(runtime.cleanup_for_owner(ID, owner).is_err());
        assert!(parent.join(ID).exists());
        assert_eq!(
            fs::read(&disk).unwrap(),
            b"keep until every host task is ruled out"
        );
        runtime.config.host_resources = None;
        runtime.cleanup_for_owner(ID, owner).unwrap();
        assert!(!runtime.jail_path(ID).exists());
    }

    #[test]
    fn opened_directory_cleanup_never_walks_a_substituted_jail_path() {
        let (temp, runtime, owner) = fixture();
        runtime.reserve_jail(ID, owner).unwrap();
        let jail = runtime.jail_path(ID);
        fs::write(jail.join("root/private-disk"), b"old disk").unwrap();
        let saved = temp.path().join("saved-original");
        assert!(
            runtime
                .cleanup_jail_with_probe(ID, owner, false, || {
                    fs::rename(&jail, &saved).unwrap();
                    fs::DirBuilder::new()
                        .recursive(true)
                        .mode(0o700)
                        .create(jail.join("root"))
                        .unwrap();
                    fs::write(jail.join("root/foreign-canary"), b"keep").unwrap();
                })
                .is_err()
        );
        assert_eq!(fs::read(jail.join("root/foreign-canary")).unwrap(), b"keep");
        assert!(!saved.join("root/private-disk").exists());
    }

    #[test]
    fn changed_birth_identity_is_rejected_even_when_device_and_inode_match() {
        let (_temp, runtime, owner) = fixture();
        runtime.reserve_jail(ID, owner).unwrap();
        let path = runtime.jail_record_path(ID);
        let mut record: JailRecord = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record.jail.birth_seconds = record.jail.birth_seconds.wrapping_sub(1);
        fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
        assert!(runtime.cleanup_jail(ID, owner, false).is_err());
        assert!(runtime.jail_path(ID).exists());
    }

    #[test]
    #[ignore = "requires root and an isolated Linux host for read-only conntrack/retirement observations"]
    fn bounded_registration_uses_staging_and_survives_until_exact_retirement() {
        const BOUNDED_ID: &str = "vm-0123456789abcdef-9000000";
        assert_eq!(rustix::process::geteuid().as_raw(), 0);
        let (temp, mut runtime, owner) = fixture_in(Some(Path::new("/var/lib")), BOUNDED_ID);
        let gate_path = temp.path().join("helper.lock");
        let gate_file = File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&gate_path)
            .unwrap();
        let gate = gate_file.metadata().unwrap();
        runtime.config.host_slot = Some(super::super::RuntimeHostSlot {
            position: 0,
            vm: 9_000_000,
            uplink_index: 900_001,
            helper_gate: super::super::RuntimeHelperGate {
                path: gate_path,
                device: gate.dev(),
                inode: gate.ino(),
                max_data_routes_per_slot: 2,
            },
        });
        runtime.reserve_jail(BOUNDED_ID, owner).unwrap();
        let witness = runtime.jail_record_path(BOUNDED_ID);
        assert!(!witness.with_file_name(".staging-jail.json").exists());
        let record: JailRecord = serde_json::from_slice(&fs::read(&witness).unwrap()).unwrap();
        assert_eq!(record.host_slot, Some([0, 9_000_000, 900_001]));
        runtime
            .write_new_record(
                &witness.with_file_name("record.json"),
                &super::super::ProcessRecord {
                    resource_id: BOUNDED_ID.into(),
                    pid: i32::MAX as u32,
                    start_time_ticks: 1,
                    boot_id: "01234567-89ab-cdef-0123-456789abcdef".into(),
                    launch_cpu_domain: vec![0],
                },
            )
            .unwrap();
        runtime.cleanup(BOUNDED_ID).unwrap();
        runtime.cleanup(BOUNDED_ID).unwrap();
        assert!(witness.exists());
        let manifest = runtime.retirement_manifest(BOUNDED_ID).unwrap();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert!(
            String::from_utf8(bytes.clone())
                .unwrap()
                .contains("jail.json")
        );
        let restored = serde_json::from_slice(&bytes).unwrap();
        runtime.retire(BOUNDED_ID, &restored).unwrap();
        assert!(!witness.parent().unwrap().exists());
        runtime.retire(BOUNDED_ID, &restored).unwrap();
        runtime.observe_retired(BOUNDED_ID, &restored).unwrap();
    }
}
