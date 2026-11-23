//! One dirty capture relative to the original private File-backend RAM image.
//! The lifecycle worker holds its existing exclusive helper/slot gate. These
//! records establish memory lineage only, never capture or resume authority.

use super::{OriginalVmSource, error, open_directory_at};
use crate::{
    RuntimeError,
    linux::{Runtime, RuntimeProcessIdentity, path_present, read_trusted_record},
    snapshot_store::{FileStamp, clone_whole_file, matches_entry, require_same_xfs_mount},
};
use rustix::{
    fs::{self, Mode, OFlags},
    process::{Gid, Uid},
};
use serde::{Deserialize, Serialize};
use std::{ffi::OsStr, fs::File, io, os::unix::fs::MetadataExt, path::Path};

const BACKING_MEMORY: &str = "restore-memory.bin";
const CAPTURE_MEMORY: &str = "memory.bin";
pub(crate) const USED_RECORD: &str = "restore-diff-used.json";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DiffAttempt {
    pub(super) source: RuntimeProcessIdentity,
    pub(super) directory_key: [u8; 32],
}

pub(super) struct DiffTarget {
    backing_directory: File,
    backing: File,
    parent_stamp: FileStamp,
    output: File,
}

fn backing_memory(directory: &File, uid: u32, gid: u32, size: u64) -> Result<File, RuntimeError> {
    let file = File::from(
        fs::openat(
            directory,
            BACKING_MEMORY,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.len() != size
        || metadata.nlink() != 1
        || metadata.uid() != uid
        || metadata.gid() != gid
        || metadata.mode() & 0o7777 != 0o400
    {
        return Err(error("invalid private restore memory backing"));
    }
    matches_entry(directory, OsStr::new(BACKING_MEMORY), &file)?;
    Ok(file)
}

fn spent(claim: &Path) -> Result<bool, RuntimeError> {
    // A partial/staged marker is conservatively spent too. The original
    // attempt never falls through to a new request if writing its marker fails.
    Ok(path_present(&claim.join(USED_RECORD))?
        || path_present(&claim.join(format!(".staging-{USED_RECORD}")))?)
}

impl Runtime {
    pub(in crate::linux) fn restored_memory_backing(
        &self,
        directory: &File,
    ) -> Result<(File, FileStamp), RuntimeError> {
        let memory = backing_memory(
            directory,
            self.config.vm_uid,
            self.config.vm_gid,
            u64::from(self.config.memory_mib) * 1_048_576,
        )?;
        let stamp = FileStamp::from_metadata(&memory.metadata()?);
        Ok((memory, stamp))
    }

    pub(super) fn prepare_capture_memory(
        &self,
        original: &OriginalVmSource,
        directory: &File,
        directory_key: [u8; 32],
    ) -> Result<Option<DiffTarget>, RuntimeError> {
        let Some((claim, intent)) = self.tracked_restore(&original.resource_id)? else {
            return Ok(None);
        };
        if spent(&claim)? {
            return Ok(None);
        }

        // Full also clears the dirty logs. A later attempt on this same
        // incarnation must use Full even if the original outcome was unknown.
        self.write_new_record(
            &claim.join(USED_RECORD),
            &DiffAttempt {
                source: original.identity.clone(),
                directory_key,
            },
        )?;
        if intent.config != self.config {
            return Ok(None);
        }
        let Some(parent_stamp) = Self::tracked_restore_parent(&claim, &intent, &original.identity)?
        else {
            return Ok(None);
        };
        let parent = open_directory_at(&original.root, "restore").and_then(|directory| {
            let (backing, stamp) = self.restored_memory_backing(&directory)?;
            Ok((directory, backing, stamp))
        });
        let (backing_directory, backing, actual_stamp) = match parent {
            Ok(parent) => parent,
            Err(RuntimeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        if actual_stamp != parent_stamp {
            // Nothing has been sent; Full reads live guest memory without
            // depending on the current directory entry for the old backing.
            return Ok(None);
        }
        require_same_xfs_mount(&backing, directory)?;
        let output = File::from(
            fs::openat(
                directory,
                CAPTURE_MEMORY,
                OFlags::CREATE | OFlags::EXCL | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(io::Error::from)?,
        );
        // Firecracker 1.16.1 overlays its dirty pages directly into this
        // pre-sized file. The restore backing mapped by the VM stays private
        // and unchanged, including after errors or missing responses.
        clone_whole_file(&backing, &output)?;
        fs::fchown(
            &output,
            Some(Uid::from_raw(self.config.vm_uid)),
            Some(Gid::from_raw(self.config.vm_gid)),
        )
        .map_err(io::Error::from)?;
        output.sync_all()?;
        let target = DiffTarget {
            backing_directory,
            backing,
            parent_stamp,
            output,
        };
        target.require_unchanged_lineage(&original.root, directory)?;
        Ok(Some(target))
    }
}

impl DiffTarget {
    pub(super) fn require_unchanged_lineage(
        &self,
        original_root: &File,
        capture: &File,
    ) -> Result<(), RuntimeError> {
        matches_entry(
            original_root,
            OsStr::new("restore"),
            &self.backing_directory,
        )?;
        matches_entry(
            &self.backing_directory,
            OsStr::new(BACKING_MEMORY),
            &self.backing,
        )?;
        if FileStamp::from_metadata(&self.backing.metadata()?) != self.parent_stamp {
            return Err(error("dirty capture parent changed"));
        }
        matches_entry(capture, OsStr::new(CAPTURE_MEMORY), &self.output)
    }
}

pub(super) fn require_diff_attempt(
    claim: &Path,
    source: &RuntimeProcessIdentity,
    directory_key: [u8; 32],
) -> Result<(), RuntimeError> {
    let attempt: DiffAttempt = read_trusted_record(&claim.join(USED_RECORD))?;
    if attempt.source != *source || attempt.directory_key != directory_key {
        return Err(error(
            "Diff request end differs from the original dirty attempt",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn partial_and_durable_attempts_disable_the_dirty_path_after_reopen() {
        let claim = tempfile::tempdir().unwrap();
        assert!(!spent(claim.path()).unwrap());
        let staged = claim.path().join(format!(".staging-{USED_RECORD}"));
        std::fs::write(&staged, b"{").unwrap();
        assert!(spent(claim.path()).unwrap());
        std::fs::rename(staged, claim.path().join(USED_RECORD)).unwrap();
        assert!(spent(claim.path()).unwrap());
    }

    #[test]
    fn backing_is_fixed_length_private_and_never_follows_a_replacement_link() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(BACKING_MEMORY);
        std::fs::write(&path, [4; 64]).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
        let dir = File::open(root.path()).unwrap();
        let metadata = std::fs::metadata(&path).unwrap();
        assert!(backing_memory(&dir, metadata.uid(), metadata.gid(), 64).is_ok());
        assert!(backing_memory(&dir, metadata.uid(), metadata.gid(), 65).is_err());
        let displaced = root.path().join("displaced");
        std::fs::rename(&path, &displaced).unwrap();
        std::os::unix::fs::symlink(displaced, &path).unwrap();
        assert!(backing_memory(&dir, metadata.uid(), metadata.gid(), 64).is_err());
    }
}
