//! Observed unary filesystem failures. Paths must already satisfy the admitted
//! operation's budget; at most two path payloads survive a failed operation.
use crate::file_download::LinuxErrno;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultStage {
    EntryStat,
    MakeDirStat,
    MakeDirExists,
    MakeDirNotDirectory,
    Rename,
    ParentStat,
    ParentNotDirectory,
    ParentCreate,
    ParentChown,
}

/// Construct only from an actual operation observation or a controlled test.
/// Missing errno is not permission to invent a public diagnostic.
#[derive(Clone, PartialEq, Eq)]
pub struct FileFault {
    pub stage: FaultStage,
    pub path: Vec<u8>,
    pub destination: Option<Vec<u8>>,
    pub errno: Option<LinuxErrno>,
}
impl std::fmt::Debug for FileFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileFault")
            .field("stage", &self.stage)
            .field("errno", &self.errno)
            .finish_non_exhaustive()
    }
}
