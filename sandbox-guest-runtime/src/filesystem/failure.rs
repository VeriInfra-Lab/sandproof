use super::{FileError, directory::EnsureError, directory::EnsureOperation, transfer::linux_errno};
use crate::file_protocol::{FaultContext, FaultStage, FileFault};
use std::{os::unix::ffi::OsStrExt, path::Path};

/// Coarse failures remain available to existing component consumers. Detailed
/// observations are captured at the original operation, never by a retry.
#[derive(Debug)]
pub struct FileFailure {
    pub error: FileError,
    pub fault: Option<FileFault>,
}
impl From<FileError> for FileFailure {
    fn from(error: FileError) -> Self {
        Self { error, fault: None }
    }
}
impl FileFailure {
    pub(super) fn observed(
        error: FileError,
        stage: FaultStage,
        path: &Path,
        destination: Option<&Path>,
        errno: Option<crate::file_download::LinuxErrno>,
    ) -> Self {
        Self {
            error,
            fault: Some(FileFault {
                stage,
                path: path.as_os_str().as_bytes().to_vec(),
                context: destination
                    .map(|p| FaultContext::RenameDestination(p.as_os_str().as_bytes().to_vec())),
                errno,
            }),
        }
    }

    pub(super) fn io(
        stage: FaultStage,
        path: &Path,
        destination: Option<&Path>,
        error: &std::io::Error,
    ) -> Self {
        let code = if matches!(
            stage,
            FaultStage::EntryStat
                | FaultStage::Rename
                | FaultStage::ResolveStat
                | FaultStage::ResolveReadlink
                | FaultStage::DirectoryStat
        ) && error.kind() == std::io::ErrorKind::NotFound
        {
            FileError::NotFound
        } else {
            FileError::Internal
        };
        Self::observed(code, stage, path, destination, linux_errno(error))
    }

    pub(super) fn walk(
        stage: FaultStage,
        path: &Path,
        root: &Path,
        error: &std::io::Error,
    ) -> Self {
        Self {
            error: FileError::Internal,
            fault: Some(FileFault {
                stage,
                path: path.as_os_str().as_bytes().to_vec(),
                context: Some(FaultContext::WalkRoot(root.as_os_str().as_bytes().to_vec())),
                errno: linux_errno(error),
            }),
        }
    }

    pub(super) fn in_walk(mut self, root: &Path) -> Self {
        self.error = FileError::Internal;
        if let Some(fault) = &mut self.fault {
            fault.stage = FaultStage::WalkEntryStat;
            fault.context = Some(FaultContext::WalkRoot(root.as_os_str().as_bytes().to_vec()));
        }
        self
    }
}
impl From<EnsureError> for FileFailure {
    fn from(error: EnsureError) -> Self {
        match error {
            EnsureError::Budget(error) => error.into(),
            EnsureError::NotDirectory(path) => Self::observed(
                FileError::Internal,
                FaultStage::ParentNotDirectory,
                &path,
                None,
                None,
            ),
            EnsureError::Io {
                operation,
                path,
                errno,
            } => Self::observed(
                FileError::Internal,
                match operation {
                    EnsureOperation::Stat => FaultStage::ParentStat,
                    EnsureOperation::Create => FaultStage::ParentCreate,
                    EnsureOperation::Chown => FaultStage::ParentChown,
                },
                &path,
                None,
                errno,
            ),
        }
    }
}
