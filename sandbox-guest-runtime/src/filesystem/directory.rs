use super::{Budget, FileError, Ownership, io_error, metadata, path};
use crate::file_protocol::Entry;
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

pub(super) enum EnsureOperation {
    Stat,
    Create,
    Chown,
}
pub(super) enum EnsureError {
    Budget(FileError),
    NotDirectory(PathBuf),
    Io {
        operation: EnsureOperation,
        path: PathBuf,
        errno: Option<crate::file_download::LinuxErrno>,
    },
}
pub(super) fn ensure_dirs_detailed(
    path: &Path,
    owner: Ownership,
    budget: &mut Budget,
) -> Result<(), EnsureError> {
    let mut pending = Vec::new();
    let mut current = path.to_owned();
    loop {
        budget.visit().map_err(EnsureError::Budget)?;
        pending.push(current.clone());
        current = path::parent_clean(&current);
        if current == Path::new("/") {
            break;
        }
    }
    for path in pending.into_iter().rev() {
        match fs::metadata(&path) {
            Ok(info) if info.is_dir() => continue,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => return Err(EnsureError::NotDirectory(path)),
            Err(error) => {
                return Err(EnsureError::Io {
                    operation: EnsureOperation::Stat,
                    path,
                    errno: super::transfer::linux_errno(&error),
                });
            }
        }
        match fs::DirBuilder::new().mode(0o755).create(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(EnsureError::Io {
                    operation: EnsureOperation::Create,
                    path,
                    errno: super::transfer::linux_errno(&error),
                });
            }
        }
        rustix::fs::chown(
            &path,
            Some(rustix::process::Uid::from_raw(owner.uid)),
            Some(rustix::process::Gid::from_raw(owner.gid)),
        )
        .map_err(|error| EnsureError::Io {
            operation: EnsureOperation::Chown,
            path,
            errno: Some(crate::file_download::LinuxErrno(error.raw_os_error())),
        })?;
    }
    Ok(())
}

struct Child {
    path: PathBuf,
    relative: PathBuf,
    depth: u32,
    directory: bool,
}

fn children(
    root: &Path,
    relative: &Path,
    depth: u32,
    budget: &mut Budget,
) -> Result<Vec<Child>, FileError> {
    let mut children = Vec::new();
    for child in fs::read_dir(root).map_err(|_| FileError::Internal)? {
        let child = child.map_err(|_| FileError::Internal)?;
        budget.visit()?;
        let path = child.path();
        budget.path(&path)?;
        let relative = relative.join(child.file_name());
        let directory = child.file_type().map_err(|_| FileError::Internal)?.is_dir();
        children.push(Child {
            path,
            relative,
            depth,
            directory,
        });
    }
    // Reverse lexical order makes pop() a depth-first Go WalkDir traversal.
    children.sort_by(|a, b| b.path.cmp(&a.path));
    Ok(children)
}

pub(super) fn list(
    requested: &Path,
    depth: u32,
    budget: &mut Budget,
) -> Result<Vec<Entry>, FileError> {
    let resolved = path::resolve(requested, budget.limits.path_bytes)?;
    if !fs::metadata(&resolved).map_err(io_error)?.is_dir() {
        return Err(FileError::InvalidArgument);
    }
    budget.visit()?;
    let mut pending = children(&resolved, Path::new(""), 1, budget)?;
    let mut result = Vec::new();
    while let Some(child) = pending.pop() {
        match metadata::entry(&child.path, budget.limits) {
            Ok(mut entry) => {
                let public_path = path::clean(&requested.join(&child.relative));
                budget.path(&public_path)?;
                entry.path = metadata::text(public_path.as_os_str())?;
                budget.retain(&entry)?;
                result.push(entry);
            }
            Err(FileError::NotFound) => {}
            Err(FileError::ResourceExhausted) => return Err(FileError::ResourceExhausted),
            Err(_) => return Err(FileError::Internal),
        }
        if child.directory {
            // WalkDir reads even a boundary directory before its first
            // too-deep callback. Preserve those errors without keeping rows.
            let descendants = children(
                &child.path,
                &child.relative,
                child.depth.saturating_add(1),
                budget,
            )?;
            if child.depth < depth {
                pending.extend(descendants);
            }
        }
    }
    Ok(result)
}
