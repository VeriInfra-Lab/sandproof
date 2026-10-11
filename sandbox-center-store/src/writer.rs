use crate::StoreError;
use sandproof_protocol::DeploymentId;
use std::{
    fs::{self, File},
    os::unix::fs::MetadataExt,
    path::Path,
};

/// Process-scoped, not a distributed lease. All writers for a deployment must
/// use the same protected directory on the approved center machine. Different
/// machines/directories cannot be detected by a local file lock.
pub struct WriterLease {
    pub(crate) deployment: DeploymentId,
    _file: File,
}

impl WriterLease {
    /// The deployment directory must already exist and be private. No import,
    /// migration, repairing an unsafe directory, PID-only lock or waiting queue.
    /// # Errors
    /// Rejects unsafe/replaced/linkable directories and a second writer.
    pub fn acquire(root: &Path, deployment: DeploymentId) -> Result<Self, StoreError> {
        let directory = fs::symlink_metadata(root).map_err(|_| StoreError::Configuration)?;
        if !root.is_absolute() || !directory.is_dir() || directory.mode() & 0o077 != 0 {
            return Err(StoreError::Configuration);
        }
        let name = format!("center-{}.lock", deployment.as_str());
        let path = root.join(&name);
        if fs::symlink_metadata(&path).is_ok_and(|metadata| !metadata.is_file()) {
            return Err(StoreError::Configuration);
        }
        let directory_fd = File::from(
            rustix::fs::open(
                root,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|_| StoreError::Configuration)?,
        );
        let opened = directory_fd
            .metadata()
            .map_err(|_| StoreError::Configuration)?;
        if opened.dev() != directory.dev() || opened.ino() != directory.ino() {
            return Err(StoreError::Configuration);
        }
        let file = File::from(
            rustix::fs::openat(
                &directory_fd,
                name.as_str(),
                rustix::fs::OFlags::RDWR
                    | rustix::fs::OFlags::CREATE
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::NONBLOCK
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            )
            .map_err(|_| StoreError::Configuration)?,
        );
        let metadata = file.metadata().map_err(|_| StoreError::Configuration)?;
        let named = fs::symlink_metadata(&path).map_err(|_| StoreError::Configuration)?;
        let after = fs::symlink_metadata(root).map_err(|_| StoreError::Configuration)?;
        if !metadata.is_file()
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
            || metadata.uid() != directory.uid()
            || named.dev() != metadata.dev()
            || named.ino() != metadata.ino()
            || !named.is_file()
            || after.dev() != directory.dev()
            || after.ino() != directory.ino()
            || after.mode() & 0o077 != 0
        {
            return Err(StoreError::Configuration);
        }
        file.try_lock().map_err(|_| StoreError::WriterUnavailable)?;
        Ok(Self {
            deployment,
            _file: file,
        })
    }
}
