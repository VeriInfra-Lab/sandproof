//! Exact, restartable removal of one registered bundle. A deletion plan is a
//! physical identity witness, never eligibility revocation or absence of users.
use super::{FileStamp, SnapshotBundle, SnapshotStore, invalid, key_name, matches_entry, os_error};
use crate::RuntimeError;
use rustix::fs::{self, AtFlags, Mode, OFlags};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::{File, Permissions},
    os::unix::fs::{MetadataExt, PermissionsExt},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryIdentity {
    device: u64,
    inode: u64,
    owner: u32,
    birth_seconds: i64,
    birth_nanoseconds: u32,
}
impl DirectoryIdentity {
    fn capture(directory: &File) -> Result<Self, RuntimeError> {
        let metadata = directory.metadata()?;
        let extended = fs::statx(directory, "", AtFlags::EMPTY_PATH, fs::StatxFlags::BTIME)
            .map_err(os_error)?;
        if !metadata.is_dir() || extended.stx_mask & fs::StatxFlags::BTIME.bits() == 0 {
            return Err(invalid("bundle retirement directory lacks birth identity"));
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            owner: metadata.uid(),
            birth_seconds: extended.stx_btime.tv_sec,
            birth_nanoseconds: extended.stx_btime.tv_nsec,
        })
    }
}

/// Persist in the original cleanup work BEFORE unlinking any entry. The
/// controller serializes retirement with acquisition of all material users and
/// establishes every loader/VMM/backing writer has ended. Deserializing this
/// witness is not an API-level authority to delete a user-selected directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleRetirement {
    version: u32,
    directory_key: [u8; 32],
    manifest_sha256: [u8; 32],
    parent: DirectoryIdentity,
    directory: DirectoryIdentity,
    files: Vec<(String, FileStamp)>,
    completion: Vec<(String, Option<FileStamp>)>,
}

impl SnapshotBundle {
    /// Capture exact original descriptors before any deletion. This does not
    /// close admission, revoke eligibility, or decide that backing is unused.
    /// # Errors
    /// Rejects a changed bundle or unsupported directory identity.
    pub fn retirement_plan(&self) -> Result<BundleRetirement, RuntimeError> {
        self.verify_unchanged()?;
        let mut files = vec![("manifest.json".into(), self.manifest_stamp.clone())];
        for entry in &self.files {
            files.push((
                entry.measurement.artifact.relative_path(),
                entry.stamp.clone(),
            ));
        }
        let mut completion = Vec::new();
        for prefix in [
            "completed-",
            "completion-stage-",
            "api-returned-",
            "api-return-stage-",
        ] {
            let name = key_name(prefix, &self.directory_key);
            completion.push((
                name.clone(),
                completion_stamp(&self.parent, &name, self.owner)?,
            ));
        }
        Ok(BundleRetirement {
            completion,
            version: 1,
            directory_key: self.directory_key,
            manifest_sha256: self.manifest_sha256(),
            parent: DirectoryIdentity::capture(&self.parent)?,
            directory: DirectoryIdentity::capture(&self.directory)?,
            files,
        })
    }
}
impl BundleRetirement {
    #[must_use]
    pub fn directory_key(&self) -> &[u8; 32] {
        &self.directory_key
    }
    #[must_use]
    pub fn manifest_sha256(&self) -> &[u8; 32] {
        &self.manifest_sha256
    }
}

impl SnapshotStore {
    /// Remove only entries recorded by the same durable plan. Missing entries
    /// allow restart after partial unlink; replacement entries are never adopted.
    /// Success includes parent fsync. The caller retains charge on any error.
    /// # Errors
    /// Refuses foreign stores, directories, extra files and changed identities.
    pub fn retire_bundle(&self, plan: &BundleRetirement) -> Result<(), RuntimeError> {
        if plan.version != 1
            || plan.files.len() != 4
            || !plan
                .files
                .iter()
                .zip(["manifest.json", "state.bin", "memory.bin", "disk-0.img"])
                .all(|((actual, _), expected)| actual == expected)
            || plan.completion.len() != 4
            || !plan
                .completion
                .iter()
                .zip([
                    "completed-",
                    "completion-stage-",
                    "api-returned-",
                    "api-return-stage-",
                ])
                .all(|((name, _), prefix)| *name == key_name(prefix, &plan.directory_key))
            || DirectoryIdentity::capture(&self.directory)? != plan.parent
        {
            return Err(invalid("bundle retirement original store differs"));
        }
        for (name, expected) in &plan.completion {
            if completion_stamp(&self.directory, name, self.owner)? != *expected {
                // An absent original may become absent after this same plan
                // unlinked it; no later file may be adopted for deletion.
                if completion_stamp(&self.directory, name, self.owner)?.is_some() {
                    return Err(invalid("retiring original completion changed"));
                }
            }
        }
        self.retire_bundle_directory(plan)?;
        for (name, expected) in &plan.completion {
            if let Some(expected) = expected {
                if verify_remaining(&self.directory, name, expected)? {
                    fs::unlinkat(&self.directory, name.as_str(), AtFlags::empty())
                        .map_err(os_error)?;
                    self.directory.sync_all()?;
                }
            } else if completion_stamp(&self.directory, name, self.owner)?.is_some() {
                return Err(invalid("unplanned completion appeared during retirement"));
            }
        }
        self.directory.sync_all()?;
        Ok(())
    }

    fn retire_bundle_directory(&self, plan: &BundleRetirement) -> Result<(), RuntimeError> {
        let name = key_name("capture-", &plan.directory_key);
        let directory = match fs::openat(
            &self.directory,
            name.as_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => {
                self.directory.sync_all()?;
                return Ok(());
            }
            Err(error) => return Err(os_error(error)),
        };
        if DirectoryIdentity::capture(&directory)? != plan.directory
            || ![0o500, 0o700].contains(&(directory.metadata()?.mode() & 0o777))
        {
            return Err(invalid("bundle retirement original directory differs"));
        }
        matches_entry(&self.directory, OsStr::new(&name), &directory)?;
        // Validate the entire remaining set before the first unlink. This also
        // rejects a foreign addition made between interrupted deletion attempts.
        for entry in fs::Dir::read_from(&directory).map_err(os_error)? {
            let entry = entry.map_err(os_error)?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            if !plan
                .files
                .iter()
                .any(|(expected, _)| expected.as_bytes() == name)
            {
                return Err(invalid("unplanned file in retiring bundle"));
            }
        }
        for (name, expected) in &plan.files {
            verify_remaining(&directory, name, expected)?;
        }
        directory.set_permissions(Permissions::from_mode(0o700))?;
        directory.sync_all()?;
        for (name, expected) in &plan.files {
            if verify_remaining(&directory, name, expected)? {
                fs::unlinkat(&directory, name.as_str(), AtFlags::empty()).map_err(os_error)?;
                directory.sync_all()?;
            }
        }
        matches_entry(&self.directory, OsStr::new(&name), &directory)?;
        fs::unlinkat(&self.directory, name.as_str(), AtFlags::REMOVEDIR).map_err(os_error)?;
        self.directory.sync_all()?;
        Ok(())
    }
}
fn verify_remaining(
    directory: &File,
    name: &str,
    expected: &FileStamp,
) -> Result<bool, RuntimeError> {
    let file = match fs::openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(false),
        Err(error) => return Err(os_error(error)),
    };
    if FileStamp::from_metadata(&file.metadata()?) != *expected {
        return Err(invalid("retiring original bundle file changed"));
    }
    matches_entry(directory, OsStr::new(name), &file)?;
    Ok(true)
}

fn completion_stamp(
    parent: &File,
    name: &str,
    owner: u32,
) -> Result<Option<FileStamp>, RuntimeError> {
    let file = match fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(os_error(error)),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != owner
        || metadata.nlink() != 1
        || ![0o400, 0o600].contains(&(metadata.mode() & 0o777))
        || metadata.len() > super::completion::MAX_COMPLETION_BYTES
    {
        return Err(invalid(
            "retiring completion original type, owner, mode or bound",
        ));
    }
    matches_entry(parent, OsStr::new(name), &file)?;
    Ok(Some(FileStamp::from_metadata(&metadata)))
}
