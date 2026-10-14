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
    ResolveStat,
    ResolveReadlink,
    ResolveNotDirectory,
    ResolveCycle,
    DirectoryStat,
    DirectoryNotDirectory,
    WalkStat,
    WalkOpen,
    WalkRead,
    WalkType,
    WalkEntryStat,
}

/// Exactly one additional path, with its role fixed by the observed stage.
#[derive(Clone, PartialEq, Eq)]
pub enum FaultContext {
    RenameDestination(Vec<u8>),
    WalkRoot(Vec<u8>),
}

/// Construct only from an actual operation observation or a controlled test.
/// Missing errno is not permission to invent a public diagnostic.
#[derive(Clone, PartialEq, Eq)]
pub struct FileFault {
    pub stage: FaultStage,
    pub path: Vec<u8>,
    pub context: Option<FaultContext>,
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
