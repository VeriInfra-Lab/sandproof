//! Read-only reopening of an original published bundle, not restore authority.
use super::{
    FileStamp, SnapshotArtifact, SnapshotFileMeasurement, SnapshotStore, accepts, file_facts,
    invalid, key_name, matches_entry, os_error,
};
use crate::RuntimeError;
use rustix::fs::{self, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
};

const NAMES: [&str; 4] = ["manifest.json", "state.bin", "memory.bin", "disk-0.img"];

/// Upper bounds come from the original work and fixed physical profile.
#[derive(Clone, Copy, Debug)]
pub struct BundleReadLimits {
    pub state_bytes: u64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
}

pub(super) struct OriginalFile {
    pub(super) file: File,
    pub(super) stamp: FileStamp,
    pub(super) measurement: SnapshotFileMeasurement,
}

/// Existing bytes checked through retained original descriptors. This value
/// does not prove capture provenance, database registration, pin or stop rights.
pub struct SnapshotBundle {
    pub(super) parent: File,
    pub(super) directory: File,
    pub(super) directory_key: [u8; 32],
    pub(super) owner: u32,
    pub(super) manifest_file: File,
    pub(super) manifest_stamp: FileStamp,
    manifest: Vec<u8>,
    pub(super) files: Vec<OriginalFile>,
}

fn names_only(directory: &File) -> Result<(), RuntimeError> {
    let mut found = Vec::new();
    for item in fs::Dir::read_from(directory).map_err(os_error)? {
        let item = item.map_err(os_error)?;
        let name = item.file_name().to_bytes();
        if name != b"." && name != b".." {
            let name = std::str::from_utf8(name).map_err(|_| invalid("bundle file name"))?;
            if !NAMES.contains(&name) || found.iter().any(|value| value == name) {
                return Err(invalid("unknown or duplicate bundle file"));
            }
            found.push(name.to_owned());
        }
    }
    if found.len() != NAMES.len() {
        return Err(invalid("missing full bundle file"));
    }
    Ok(())
}

pub(super) fn original_file(
    directory: &File,
    name: &str,
    owner: u32,
    limit: u64,
) -> Result<File, RuntimeError> {
    let file = File::from(
        fs::openat(
            directory,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(os_error)?,
    );
    let metadata = file.metadata()?;
    if !accepts(file_facts(&metadata), owner, limit) || metadata.mode() & 0o777 != 0o400 {
        return Err(invalid("sealed source type, owner, links, length or mode"));
    }
    matches_entry(directory, OsStr::new(name), &file)?;
    Ok(file)
}

fn measure(
    file: &mut File,
    artifact: SnapshotArtifact,
) -> Result<(SnapshotFileMeasurement, FileStamp), RuntimeError> {
    let before = FileStamp::from_metadata(&file.metadata()?);
    file.seek(SeekFrom::Start(0))?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(super::COPY_BUFFER_BYTES)
        .map_err(|_| invalid("bundle read buffer allocation"))?;
    buffer.resize(super::COPY_BUFFER_BYTES, 0_u8);
    let mut remaining = before.length;
    let mut hash = Sha256::new();
    while remaining > 0 {
        let count = usize::try_from(remaining.min(buffer.len() as u64))
            .map_err(|_| invalid("bundle block size"))?;
        file.read_exact(&mut buffer[..count])?;
        hash.update(&buffer[..count]);
        remaining -= count as u64;
    }
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 || before != FileStamp::from_metadata(&file.metadata()?) {
        return Err(invalid("sealed source changed during read"));
    }
    Ok((
        SnapshotFileMeasurement {
            artifact,
            length: before.length,
            sha256: hash.finalize().into(),
        },
        before,
    ))
}

impl SnapshotStore {
    /// Reopen one existing original key; never creates, republishes or adopts a
    /// stage. The caller still establishes the original accepted work/pin.
    /// # Errors
    /// Rejects links, extra/missing files, mode changes and changed identities.
    pub fn open_bundle(
        &self,
        directory_key: [u8; 32],
        limits: BundleReadLimits,
    ) -> Result<SnapshotBundle, RuntimeError> {
        let name = key_name("capture-", &directory_key);
        let directory = File::from(
            fs::openat(
                &self.directory,
                name.as_str(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(os_error)?,
        );
        let metadata = directory.metadata()?;
        if metadata.uid() != self.owner || metadata.mode() & 0o777 != 0o500 {
            return Err(invalid("sealed bundle directory owner or mode"));
        }
        matches_entry(&self.directory, OsStr::new(&name), &directory)?;
        names_only(&directory)?;
        let mut manifest_file = original_file(
            &directory,
            "manifest.json",
            self.owner,
            super::MAX_MANIFEST_BYTES as u64,
        )?;
        let manifest_stamp = FileStamp::from_metadata(&manifest_file.metadata()?);
        let mut manifest = Vec::new();
        manifest
            .try_reserve_exact(
                usize::try_from(manifest_stamp.length).map_err(|_| invalid("manifest length"))?,
            )
            .map_err(|_| invalid("manifest allocation"))?;
        Read::by_ref(&mut manifest_file)
            .take(super::MAX_MANIFEST_BYTES as u64 + 1)
            .read_to_end(&mut manifest)?;
        if manifest.len() as u64 != manifest_stamp.length {
            return Err(invalid("manifest changed during read"));
        }
        let mut files = Vec::new();
        for (artifact, limit) in [
            (SnapshotArtifact::State, limits.state_bytes),
            (SnapshotArtifact::Memory, limits.memory_bytes),
            (SnapshotArtifact::Disk(0), limits.disk_bytes),
        ] {
            let mut file = original_file(&directory, &artifact.relative_path(), self.owner, limit)?;
            let (measurement, stamp) = measure(&mut file, artifact)?;
            files.push(OriginalFile {
                file,
                stamp,
                measurement,
            });
        }
        let opened = SnapshotBundle {
            parent: self.directory.try_clone()?,
            directory,
            directory_key,
            owner: self.owner,
            manifest_file,
            manifest_stamp,
            manifest,
            files,
        };
        opened.verify_unchanged()?;
        Ok(opened)
    }
}

impl SnapshotBundle {
    #[must_use]
    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest
    }
    #[must_use]
    pub fn manifest_sha256(&self) -> [u8; 32] {
        Sha256::digest(&self.manifest).into()
    }
    #[must_use]
    pub fn directory_key(&self) -> &[u8; 32] {
        &self.directory_key
    }
    #[must_use]
    pub fn measurements(&self) -> Vec<SnapshotFileMeasurement> {
        self.files
            .iter()
            .map(|entry| entry.measurement.clone())
            .collect()
    }

    /// Copy an original sealed artifact into one fresh private jail file.
    /// Source ownership/mode never changes. The caller has already reserved
    /// both copies and keeps its durable loader/backing responsibility until
    /// exact process exit and explicit cleanup, including on any error here.
    /// # Errors
    /// Existing destinations, changed sources, short I/O or hash mismatches
    /// preserve the partial destination; they never release source usage.
    pub(crate) fn copy_for_restore(
        &self,
        destination_root: &File,
        artifact: SnapshotArtifact,
        uid: u32,
        gid: u32,
    ) -> Result<(), RuntimeError> {
        self.verify_unchanged()?;
        let original = self
            .files
            .iter()
            .find(|file| file.measurement.artifact == artifact)
            .ok_or_else(|| invalid("missing restore artifact"))?;
        let name = match artifact {
            SnapshotArtifact::State => "restore-state.bin",
            SnapshotArtifact::Memory => "restore-memory.bin",
            SnapshotArtifact::Disk(0) => "rootfs.ext4",
            SnapshotArtifact::Disk(_) => return Err(invalid("unsupported restore disk")),
        };
        let mut destination = File::from(
            fs::openat(
                destination_root,
                name,
                OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(os_error)?,
        );
        let mut source = original.file.try_clone()?;
        source.seek(SeekFrom::Start(0))?;
        let mut remaining = original.measurement.length;
        let mut hash = Sha256::new();
        let mut buffer = vec![0_u8; super::COPY_BUFFER_BYTES];
        while remaining > 0 {
            let count = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| invalid("restore copy length"))?;
            source.read_exact(&mut buffer[..count])?;
            destination.write_all(&buffer[..count])?;
            hash.update(&buffer[..count]);
            remaining -= count as u64;
        }
        let mut extra = [0_u8; 1];
        let digest: [u8; 32] = hash.finalize().into();
        if source.read(&mut extra)? != 0 || digest != original.measurement.sha256 {
            return Err(invalid("restore source bytes changed"));
        }
        self.verify_unchanged()?;
        fs::fchown(
            &destination,
            Some(rustix::process::Uid::from_raw(uid)),
            Some(rustix::process::Gid::from_raw(gid)),
        )
        .map_err(os_error)?;
        let mode = if artifact == SnapshotArtifact::Disk(0) {
            0o600
        } else {
            0o400
        };
        destination.set_permissions(std::fs::Permissions::from_mode(mode))?;
        destination.sync_all()?;
        matches_entry(destination_root, OsStr::new(name), &destination)?;
        destination_root.sync_all()?;
        Ok(())
    }

    /// Check the same original entries after node binding validation. Immutable
    /// source/no unrelated privileged writer remains a deployment condition.
    /// # Errors
    /// A metadata/entry/name change leaves the original responsibility unknown.
    pub fn verify_unchanged(&self) -> Result<(), RuntimeError> {
        matches_entry(
            &self.parent,
            OsStr::new(&key_name("capture-", &self.directory_key)),
            &self.directory,
        )?;
        let directory = self.directory.metadata()?;
        if directory.uid() != self.owner || directory.mode() & 0o777 != 0o500 {
            return Err(invalid("bundle directory changed"));
        }
        names_only(&self.directory)?;
        if FileStamp::from_metadata(&self.manifest_file.metadata()?) != self.manifest_stamp {
            return Err(invalid("original manifest changed"));
        }
        matches_entry(
            &self.directory,
            OsStr::new("manifest.json"),
            &self.manifest_file,
        )?;
        for entry in &self.files {
            if FileStamp::from_metadata(&entry.file.metadata()?) != entry.stamp {
                return Err(invalid("original bundle file changed"));
            }
            matches_entry(
                &self.directory,
                OsStr::new(&entry.measurement.artifact.relative_path()),
                &entry.file,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "snapshot_bundle_tests.rs"]
mod tests;
