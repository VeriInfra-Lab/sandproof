//! Original birth identity shared by the one finite material retirement plan.
use super::{invalid, os_error};
use crate::RuntimeError;
use rustix::fs::{self, AtFlags};
use serde::{Deserialize, Serialize};
use std::{fs::File, os::unix::fs::MetadataExt};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DirectoryIdentity {
    device: u64,
    inode: u64,
    owner: u32,
    birth_seconds: i64,
    birth_nanoseconds: u32,
}
impl DirectoryIdentity {
    pub(super) fn capture(directory: &File) -> Result<Self, RuntimeError> {
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
