//! Structured observations from the original upload operation, never arbitrary
//! diagnostic strings. Actual I/O and path/attribute budgets belong to the caller.
use crate::file_download::LinuxErrno;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    ParentStat,
    ParentNotDirectory,
    ParentCreate,
    ParentChown,
    Stat,
    Directory,
    Chown,
    Open,
    FileChown,
    Write,
    WriteZero,
    MetadataList,
    MetadataRemove,
    MetadataSet,
}

/// Paths are the exact argument of the failed operation (a parent when parent
/// preparation fails). Attribute bytes preserve the actual name, even if not
/// UTF-8. None errno means unclassified I/O, not a made-up kernel error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub stage: Stage,
    pub path: Vec<u8>,
    pub errno: Option<LinuxErrno>,
    pub attribute: Option<Vec<u8>>,
}
impl Fault {
    /// Fixed processFile's status by failure phase. This alone is not an HTTP
    /// response or a complete fixed-error message; caller still owns delivery.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match (self.stage, self.errno) {
            (Stage::Directory, _) => 400,
            (Stage::Open | Stage::Write, Some(LinuxErrno(28)))
            | (
                Stage::MetadataList | Stage::MetadataRemove | Stage::MetadataSet,
                Some(LinuxErrno(28 | 122)),
            ) => 507,
            _ => 500,
        }
    }
}
