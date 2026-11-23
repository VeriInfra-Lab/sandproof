//! One dirty capture relative to the original private File-backend RAM image.
//! The lifecycle worker holds its existing exclusive helper/slot gate. These
//! records establish memory lineage only, never capture or resume authority.

use super::{OriginalVmSource, error, open_directory_at, request_end::CaptureKind};
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
    #[serde(default)]
    pub(super) kind: Option<CaptureKind>,
}

struct DiffParent {
    directory: File,
    backing: File,
    stamp: FileStamp,
}

pub(super) struct DiffTarget {
    parent: DiffParent,
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

        let parent = if intent.config == self.config {
            Self::tracked_restore_parent(&claim, &intent, &original.identity)?
                .map(|stamp| self.open_diff_parent(original, &stamp))
                .transpose()?
                .flatten()
        } else {
            None
        };
        let kind = if parent.is_some() {
            CaptureKind::Diff
        } else {
            CaptureKind::Full
        };
        // The read-only parent decision is fixed before committing the mode.
        // Either request may consume or clear dirty logs. Never clone or send
        // for this attempt until its one-shot choice is durable.
        self.write_new_record(
            &claim.join(USED_RECORD),
            &DiffAttempt {
                source: original.identity.clone(),
                directory_key,
                kind: Some(kind),
            },
        )?;
        let Some(parent) = parent else {
            return Ok(None);
        };
        require_same_xfs_mount(&parent.backing, directory)?;
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
        clone_whole_file(&parent.backing, &output)?;
        fs::fchown(
            &output,
            Some(Uid::from_raw(self.config.vm_uid)),
            Some(Gid::from_raw(self.config.vm_gid)),
        )
        .map_err(io::Error::from)?;
        output.sync_all()?;
        let target = DiffTarget { parent, output };
        target.require_unchanged_lineage(&original.root, directory)?;
        Ok(Some(target))
    }

    fn open_diff_parent(
        &self,
        original: &OriginalVmSource,
        expected: &FileStamp,
    ) -> Result<Option<DiffParent>, RuntimeError> {
        let parent = open_directory_at(&original.root, "restore").and_then(|directory| {
            let (backing, stamp) = self.restored_memory_backing(&directory)?;
            Ok(DiffParent {
                directory,
                backing,
                stamp,
            })
        });
        match parent {
            Ok(parent) if parent.stamp == *expected => Ok(Some(parent)),
            // Full reads live memory without depending on the old backing's
            // current directory entry. Nothing has been sent at this point.
            Ok(_) => Ok(None),
            Err(RuntimeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

impl DiffTarget {
    pub(super) fn require_unchanged_lineage(
        &self,
        original_root: &File,
        capture: &File,
    ) -> Result<(), RuntimeError> {
        matches_entry(original_root, OsStr::new("restore"), &self.parent.directory)?;
        matches_entry(
            &self.parent.directory,
            OsStr::new(BACKING_MEMORY),
            &self.parent.backing,
        )?;
        if FileStamp::from_metadata(&self.parent.backing.metadata()?) != self.parent.stamp {
            return Err(error("dirty capture parent changed"));
        }
        matches_entry(capture, OsStr::new(CAPTURE_MEMORY), &self.output)
    }
}

fn check_request_kind(
    attempt: Option<&DiffAttempt>,
    source: &RuntimeProcessIdentity,
    directory_key: [u8; 32],
    returned: Option<CaptureKind>,
) -> Result<(), RuntimeError> {
    if let Some(attempt) = attempt.filter(|attempt| attempt.directory_key == directory_key) {
        return if attempt.source == *source && attempt.kind.is_some() && returned == attempt.kind {
            Ok(())
        } else {
            Err(error(
                "capture request end differs from its original memory mode",
            ))
        };
    }
    if returned == Some(CaptureKind::Diff) {
        return Err(error("Diff request end lacks its original dirty attempt"));
    }
    // A cold Full, a later Full after the dirty epoch was consumed, and an
    // original worker that declined release have no marker for their key.
    Ok(())
}

pub(super) fn require_capture_request_kind(
    claim: &Path,
    source: &RuntimeProcessIdentity,
    directory_key: [u8; 32],
    returned: Option<CaptureKind>,
) -> Result<(), RuntimeError> {
    let attempt = match read_trusted_record::<DiffAttempt>(&claim.join(USED_RECORD)) {
        Ok(attempt) => Some(attempt),
        Err(RuntimeError::Io(error)) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    check_request_kind(attempt.as_ref(), source, directory_key, returned)
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
    fn original_dirty_attempt_cannot_be_reinterpreted_as_a_full_or_unsent_request() {
        let original = RuntimeProcessIdentity {
            pid: 17,
            start_time_ticks: 43,
            boot_id: "00000000-0000-0000-0000-000000000001".into(),
        };
        let diff_attempt = DiffAttempt {
            source: original.clone(),
            directory_key: [8; 32],
            kind: Some(CaptureKind::Diff),
        };
        let full_attempt = DiffAttempt {
            kind: Some(CaptureKind::Full),
            source: original.clone(),
            directory_key: [8; 32],
        };
        for attempt in [&diff_attempt, &full_attempt] {
            assert!(check_request_kind(Some(attempt), &original, [8; 32], attempt.kind).is_ok());
            let other = if attempt.kind == Some(CaptureKind::Diff) {
                CaptureKind::Full
            } else {
                CaptureKind::Diff
            };
            assert!(check_request_kind(Some(attempt), &original, [8; 32], Some(other)).is_err());
            assert!(check_request_kind(Some(attempt), &original, [8; 32], None).is_err());
        }
        let mut replacement = original.clone();
        replacement.start_time_ticks += 1;
        assert!(
            check_request_kind(
                Some(&diff_attempt),
                &replacement,
                [8; 32],
                Some(CaptureKind::Diff)
            )
            .is_err()
        );
        assert!(
            check_request_kind(
                Some(&diff_attempt),
                &original,
                [9; 32],
                Some(CaptureKind::Full)
            )
            .is_ok()
        );
        assert!(check_request_kind(Some(&diff_attempt), &original, [9; 32], None).is_ok());
        assert!(check_request_kind(None, &original, [8; 32], Some(CaptureKind::Full)).is_ok());
        assert!(check_request_kind(None, &original, [8; 32], Some(CaptureKind::Diff)).is_err());
    }

    #[test]
    fn legacy_consumption_marker_cannot_authorize_a_reply_for_its_old_attempt() {
        let original = RuntimeProcessIdentity {
            pid: 17,
            start_time_ticks: 43,
            boot_id: "00000000-0000-0000-0000-000000000001".into(),
        };
        let key = [8; 32];
        let marker: DiffAttempt = serde_json::from_value(serde_json::json!({
            "source": original, "directory_key": key,
        }))
        .unwrap();
        assert!(
            check_request_kind(Some(&marker), &original, key, Some(CaptureKind::Full)).is_err()
        );
        assert!(
            check_request_kind(Some(&marker), &original, [8; 32], Some(CaptureKind::Diff)).is_err()
        );
        assert!(
            check_request_kind(Some(&marker), &original, [9; 32], Some(CaptureKind::Full)).is_ok()
        );
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
