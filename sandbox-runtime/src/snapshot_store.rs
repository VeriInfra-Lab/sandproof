//! Original-FD copy and durable publication for full snapshot materials.
//!
//! This module owns files, not restore eligibility or VM lifetime. In particular,
//! dropping a stage (including on error) never removes its partial files and never
//! stops a VM. The controller must retain the durable capture work on every error;
//! even an error from the final directory sync may follow a completed rename.
//!
//! Callers serialize the capture against resume/host writers and bind these file
//! measurements to their complete operation/profile manifest before publication.
//! Protected root-owned parents exclude unrelated privileged writers/mount changes.

use crate::{
    RuntimeError,
    snapshot_policy::{SnapshotFileFacts, accepts},
};
use rustix::fs::{
    AtFlags, Mode, OFlags, RenameFlags, mkdirat, open, openat, renameat_with, statat,
};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{File, Metadata, Permissions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

#[path = "snapshot_bundle.rs"]
mod bundle;
pub use bundle::{BundleReadLimits, SnapshotBundle};

const MAX_MANIFEST_BYTES: usize = 1_048_576;
const COPY_BUFFER_BYTES: usize = 65_536;

/// A physical artifact role, not an authorization or logical manifest digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotArtifact {
    State,
    Memory,
    /// Index in the fixed compatibility profile's writable-drive list.
    Disk(u32),
}

impl SnapshotArtifact {
    #[must_use]
    pub fn relative_path(self) -> String {
        match self {
            Self::State => "state.bin".to_owned(),
            Self::Memory => "memory.bin".to_owned(),
            Self::Disk(index) => format!("disk-{index}.img"),
        }
    }
}

/// SHA-256 of actual copied bytes. This is not `core::ManifestDigest(u64)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotFileMeasurement {
    pub artifact: SnapshotArtifact,
    pub length: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    device: u64,
    inode: u64,
    owner: u32,
    group: u32,
    mode: u32,
    links: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl FileStamp {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
            group: metadata.gid(),
            mode: metadata.mode(),
            links: metadata.nlink(),
            length: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

struct CopiedFile {
    file: File,
    stamp: FileStamp,
    measurement: SnapshotFileMeasurement,
}

/// An already existing private store, separate from every reusable VM jail.
/// No pathname supplied by a guest/request is used as a destination filename.
pub struct SnapshotStore {
    path: PathBuf,
    directory: File,
    owner: u32,
}

/// A unique original directory; interrupted stages are deliberately retained.
pub struct SnapshotStage {
    parent: File,
    directory: File,
    directory_key: [u8; 32],
    stage_name: String,
    owner: u32,
    copied: Vec<CopiedFile>,
}

/// Physical file and sync facts only. This does not permit stopping the source
/// VM; the controller must durably accept the bound bundle first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedSnapshotFiles {
    directory_key: [u8; 32],
    files: Vec<SnapshotFileMeasurement>,
    manifest_sha256: [u8; 32],
}

impl PublishedSnapshotFiles {
    #[must_use]
    pub fn directory_key(&self) -> &[u8; 32] {
        &self.directory_key
    }
    #[must_use]
    pub fn files(&self) -> &[SnapshotFileMeasurement] {
        &self.files
    }
    #[must_use]
    pub fn manifest_sha256(&self) -> &[u8; 32] {
        &self.manifest_sha256
    }
}

fn invalid(message: &'static str) -> RuntimeError {
    RuntimeError::IdentityMismatch(message)
}

fn os_error(error: rustix::io::Errno) -> RuntimeError {
    RuntimeError::Io(io::Error::from(error))
}

fn file_facts(metadata: &Metadata) -> SnapshotFileFacts {
    SnapshotFileFacts {
        owner: metadata.uid(),
        mode: metadata.mode(),
        regular: metadata.is_file(),
        links: metadata.nlink(),
        length: metadata.len(),
    }
}

fn trusted_directory(metadata: &Metadata, owner: u32, private: bool) -> bool {
    let group_other_permissions = metadata.mode() & 0o077;
    let trusted_owner = metadata.uid() == 0 || metadata.uid() == owner;
    let writes =
        metadata.mode() & 0o022 == 0 || (metadata.uid() == 0 && metadata.mode() & 0o1000 != 0);
    metadata.is_dir()
        && trusted_owner
        && writes
        && (!private || (metadata.uid() == owner && group_other_permissions == 0))
}

pub(crate) fn open_store(path: &Path, owner: u32) -> Result<File, RuntimeError> {
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err(invalid("snapshot store path"));
    }
    let names: Vec<_> = path
        .components()
        .skip(1)
        .map(|part| match part {
            Component::Normal(name) => Ok(name),
            _ => Err(invalid("snapshot store component")),
        })
        .collect::<Result<_, _>>()?;
    if names.is_empty() || names.len() > 128 {
        return Err(invalid("snapshot store depth"));
    }
    let flags = OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut current = File::from(open("/", flags, Mode::empty()).map_err(os_error)?);
    if !trusted_directory(&current.metadata()?, owner, false) {
        return Err(invalid("snapshot store root"));
    }
    for (index, name) in names.iter().enumerate() {
        let last = index + 1 == names.len();
        let access = if last {
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
        } else {
            flags
        };
        current = File::from(openat(&current, *name, access, Mode::empty()).map_err(os_error)?);
        if !trusted_directory(&current.metadata()?, owner, last) {
            return Err(invalid("snapshot store directory"));
        }
    }
    Ok(current)
}

pub(crate) fn matches_entry(
    directory: &File,
    name: &OsStr,
    file: &File,
) -> Result<(), RuntimeError> {
    let entry = statat(directory, name, AtFlags::SYMLINK_NOFOLLOW).map_err(os_error)?;
    let opened = file.metadata()?;
    if entry.st_dev != opened.dev() || entry.st_ino != opened.ino() {
        return Err(invalid("snapshot file or directory replaced"));
    }
    Ok(())
}

fn key_name(prefix: &str, key: &[u8; 32]) -> String {
    use std::fmt::Write as _;
    let mut name = prefix.to_owned();
    for byte in key {
        write!(&mut name, "{byte:02x}").expect("writing to a String cannot fail");
    }
    name
}

impl SnapshotStore {
    pub(crate) fn from_layout(path: PathBuf, directory: File, owner: u32) -> Self {
        Self {
            path,
            directory,
            owner,
        }
    }
    /// Open a root-owned protected store through checked ancestor descriptors.
    /// # Errors
    /// Rejects non-root execution, symlinks, replaceable ancestors and loose modes.
    pub fn open(path: &Path) -> Result<Self, RuntimeError> {
        if rustix::process::geteuid().as_raw() != 0 {
            return Err(invalid("snapshot store requires root controller"));
        }
        Self::open_for_owner(path, 0)
    }

    fn open_for_owner(path: &Path, owner: u32) -> Result<Self, RuntimeError> {
        Ok(Self {
            path: path.to_owned(),
            directory: open_store(path, owner)?,
            owner,
        })
    }

    pub(crate) fn require_separate_from(&self, runtime_root: &Path) -> Result<(), RuntimeError> {
        if !runtime_root.is_absolute()
            || runtime_root
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
            || self.path.starts_with(runtime_root)
            || runtime_root.starts_with(&self.path)
        {
            return Err(invalid("snapshot store overlaps reusable runtime tree"));
        }
        Ok(())
    }

    /// Create one new stage. The key is derived from the complete bound capture
    /// identity by the caller; it selects storage and never grants permission.
    /// # Errors
    /// Existing stages/bundles are never adopted or overwritten.
    pub fn begin(&self, directory_key: [u8; 32]) -> Result<SnapshotStage, RuntimeError> {
        let stage_name = key_name("staging-", &directory_key);
        let final_name = key_name("capture-", &directory_key);
        match statat(
            &self.directory,
            final_name.as_str(),
            AtFlags::SYMLINK_NOFOLLOW,
        ) {
            Ok(_) => return Err(RuntimeError::AlreadyClaimed),
            Err(rustix::io::Errno::NOENT) => {}
            Err(error) => return Err(os_error(error)),
        }
        mkdirat(&self.directory, stage_name.as_str(), Mode::RWXU).map_err(os_error)?;
        let directory = File::from(
            openat(
                &self.directory,
                stage_name.as_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(os_error)?,
        );
        if !trusted_directory(&directory.metadata()?, self.owner, true) {
            return Err(invalid("snapshot stage owner"));
        }
        matches_entry(&self.directory, OsStr::new(&stage_name), &directory)?;
        directory.sync_all()?;
        self.directory.sync_all()?;
        Ok(SnapshotStage {
            parent: self.directory.try_clone()?,
            directory,
            directory_key,
            stage_name,
            owner: self.owner,
            copied: Vec::new(),
        })
    }
}

impl SnapshotStage {
    #[must_use]
    pub fn directory_key(&self) -> &[u8; 32] {
        &self.directory_key
    }
    #[must_use]
    pub fn measurements(&self) -> Vec<SnapshotFileMeasurement> {
        self.copied
            .iter()
            .map(|entry| entry.measurement.clone())
            .collect()
    }

    /// Copy and hash one original open regular file while its writers are drained.
    /// The descriptor, metadata and length are checked again after the copy.
    /// # Errors
    /// Short reads/writes, budget overflow, links, duplicate roles and mutation
    /// retain the partial destination under the same capture responsibility.
    pub fn copy_artifact(
        &mut self,
        source: &mut File,
        source_owner: u32,
        artifact: SnapshotArtifact,
        max_length: u64,
    ) -> Result<SnapshotFileMeasurement, RuntimeError> {
        if self
            .copied
            .iter()
            .any(|entry| entry.measurement.artifact == artifact)
        {
            return Err(invalid("duplicate snapshot artifact"));
        }
        matches_entry(&self.parent, OsStr::new(&self.stage_name), &self.directory)?;
        let original = source.metadata()?;
        if !accepts(file_facts(&original), source_owner, max_length) {
            return Err(invalid("snapshot source ownership, type or length"));
        }
        let before = FileStamp::from_metadata(&original);
        let name = artifact.relative_path();
        let mut destination = File::from(
            openat(
                &self.directory,
                name.as_str(),
                OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(os_error)?,
        );
        source.seek(SeekFrom::Start(0))?;
        let mut digest = Sha256::new();
        let mut buffer = Vec::new();
        buffer.try_reserve_exact(COPY_BUFFER_BYTES).map_err(|_| {
            RuntimeError::Io(io::Error::other("snapshot copy buffer allocation failed"))
        })?;
        buffer.resize(COPY_BUFFER_BYTES, 0_u8);
        let mut remaining = original.len();
        while remaining > 0 {
            let count = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| invalid("snapshot copy block length"))?;
            source.read_exact(&mut buffer[..count])?;
            destination.write_all(&buffer[..count])?;
            digest.update(&buffer[..count]);
            remaining -= count as u64;
        }
        let mut extra = [0_u8; 1];
        if source.read(&mut extra)? != 0 || before != FileStamp::from_metadata(&source.metadata()?)
        {
            return Err(invalid("snapshot source changed during copy"));
        }
        destination.set_permissions(Permissions::from_mode(0o400))?;
        destination.sync_all()?;
        let metadata = destination.metadata()?;
        if !accepts(file_facts(&metadata), self.owner, original.len())
            || metadata.len() != original.len()
        {
            return Err(invalid("snapshot destination identity or length"));
        }
        matches_entry(&self.directory, OsStr::new(&name), &destination)?;
        let measurement = SnapshotFileMeasurement {
            artifact,
            length: original.len(),
            sha256: digest.finalize().into(),
        };
        self.copied.push(CopiedFile {
            file: destination,
            stamp: FileStamp::from_metadata(&metadata),
            measurement: measurement.clone(),
        });
        Ok(measurement)
    }

    /// Publish exact measured files with the caller's completely bound manifest.
    /// The caller must check the manifest's identity/profile/file projection before
    /// this call; this primitive neither parses nor certifies business fields.
    /// # Errors
    /// No error removes materials. Rename/sync errors can have visible effects.
    pub fn publish(
        self,
        manifest: &[u8],
        expected: &[SnapshotFileMeasurement],
    ) -> Result<PublishedSnapshotFiles, RuntimeError> {
        if manifest.is_empty()
            || manifest.len() > MAX_MANIFEST_BYTES
            || expected != self.measurements()
        {
            return Err(invalid("snapshot manifest or file measurements"));
        }
        if !expected
            .iter()
            .any(|entry| entry.artifact == SnapshotArtifact::State)
            || !expected
                .iter()
                .any(|entry| entry.artifact == SnapshotArtifact::Memory)
            || !expected
                .iter()
                .any(|entry| matches!(entry.artifact, SnapshotArtifact::Disk(_)))
        {
            return Err(invalid("incomplete full snapshot files"));
        }
        matches_entry(&self.parent, OsStr::new(&self.stage_name), &self.directory)?;
        for entry in &self.copied {
            if entry.stamp != FileStamp::from_metadata(&entry.file.metadata()?) {
                return Err(invalid("snapshot copy changed before publication"));
            }
            matches_entry(
                &self.directory,
                OsStr::new(&entry.measurement.artifact.relative_path()),
                &entry.file,
            )?;
        }
        let mut file = File::from(
            openat(
                &self.directory,
                "manifest.json",
                OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(os_error)?,
        );
        file.write_all(manifest)?;
        file.set_permissions(Permissions::from_mode(0o400))?;
        file.sync_all()?;
        matches_entry(&self.directory, OsStr::new("manifest.json"), &file)?;
        self.directory
            .set_permissions(Permissions::from_mode(0o500))?;
        self.directory.sync_all()?;
        matches_entry(&self.parent, OsStr::new(&self.stage_name), &self.directory)?;
        let final_name = key_name("capture-", &self.directory_key);
        renameat_with(
            &self.parent,
            self.stage_name.as_str(),
            &self.parent,
            final_name.as_str(),
            RenameFlags::NOREPLACE,
        )
        .map_err(os_error)?;
        self.parent.sync_all()?;
        matches_entry(&self.parent, OsStr::new(&final_name), &self.directory)?;
        Ok(PublishedSnapshotFiles {
            directory_key: self.directory_key,
            files: expected.to_vec(),
            manifest_sha256: Sha256::digest(manifest).into(),
        })
    }
}

#[cfg(test)]
#[path = "snapshot_store_tests.rs"]
mod tests;
