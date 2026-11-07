//! Single-file `CoW` on already-open, private descriptors; no publication or GC.
//!
//! Callers bind expected owners, lengths/budgets and protected directory entries,
//! exclude writers, and verify the cloned bytes before syncing/publishing them.
//! This operation never falls back to byte copying and never unlinks on error.

use super::{FileStamp, invalid, os_error};
use crate::RuntimeError;
use rustix::fs::{self, AtFlags, StatxFlags, ioctl_ficlone};
use std::{fs::File, os::unix::fs::MetadataExt};

// Linux statfs magic for XFS. rustix exposes FsWord with the target ABI type.
const XFS_MAGIC: fs::FsWord = 0x5846_5342;

/// Check two currently held file/directory FDs belong to the same XFS mount.
/// Mount IDs are only compared here, not serialized as durable volume identity.
/// # Errors
/// Missing statx mount identity, another FS/device/mount or an invalid FD fails
/// closed. Same `st_dev` alone is insufficient for private bind mounts.
pub(crate) fn require_same_xfs_mount(left: &File, right: &File) -> Result<(), RuntimeError> {
    let first = left.metadata()?;
    let second = right.metadata()?;
    if !(first.is_file() || first.is_dir())
        || !(second.is_file() || second.is_dir())
        || first.dev() != second.dev()
        || fs::fstatfs(left).map_err(os_error)?.f_type != XFS_MAGIC
        || fs::fstatfs(right).map_err(os_error)?.f_type != XFS_MAGIC
    {
        return Err(invalid(
            "snapshot materials require the same XFS filesystem",
        ));
    }
    let first = fs::statx(left, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID).map_err(os_error)?;
    let second = fs::statx(right, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID).map_err(os_error)?;
    if first.stx_mask & StatxFlags::MNT_ID.bits() == 0
        || second.stx_mask & StatxFlags::MNT_ID.bits() == 0
        || first.stx_mnt_id == 0
        || first.stx_mnt_id != second.stx_mnt_id
    {
        return Err(invalid(
            "snapshot materials require matching current FD mount identities",
        ));
    }
    Ok(())
}

/// Clone a whole non-empty file into a fresh, empty private file on the same FS.
///
/// Success establishes only this file's content, length and independent inode.
/// It does not sync, hash, register a bundle, stop a VM or release any budget.
/// Failure may leave destination effects; the same work retains both FDs/files.
/// # Errors
/// Unsupported/cross-FS cloning, invalid FDs, space/quota errors or changed
/// identities are returned without copying bytes or deleting partial materials.
pub(crate) fn clone_whole_file(source: &File, destination: &File) -> Result<(), RuntimeError> {
    let before = source.metadata()?;
    let target = destination.metadata()?;
    if !before.is_file() || before.nlink() != 1 || before.len() == 0 || before.mode() & 0o077 != 0 {
        return Err(invalid("snapshot clone source type, links or permissions"));
    }
    if !target.is_file()
        || target.nlink() != 1
        || target.len() != 0
        || target.mode() & 0o777 != 0o600
    {
        return Err(invalid(
            "snapshot clone requires an empty private destination",
        ));
    }
    if before.dev() == target.dev() && before.ino() == target.ino() {
        return Err(invalid(
            "snapshot clone requires same-device independent inodes",
        ));
    }
    require_same_xfs_mount(source, destination)?;
    // FICLONE is deliberately the sole content-creation algorithm. In
    // particular, EXDEV/EOPNOTSUPP/ENOSPC/EDQUOT are returned to the caller.
    ioctl_ficlone(destination, source).map_err(os_error)?;
    if FileStamp::from_metadata(&before) != FileStamp::from_metadata(&source.metadata()?) {
        return Err(invalid("snapshot clone source changed during FICLONE"));
    }
    let after = destination.metadata()?;
    if !after.is_file()
        || after.dev() != target.dev()
        || after.ino() != target.ino()
        || after.uid() != target.uid()
        || after.gid() != target.gid()
        || after.mode() != target.mode()
        || after.nlink() != 1
        || after.len() != before.len()
    {
        return Err(invalid(
            "snapshot clone destination identity or length changed",
        ));
    }
    Ok(())
}
