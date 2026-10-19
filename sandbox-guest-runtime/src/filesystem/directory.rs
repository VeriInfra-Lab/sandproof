use super::{Budget, FileError, FileFailure, Ownership, metadata, path};
use crate::file_protocol::{Entry, FaultStage};
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
    walk_root: &Path,
) -> Result<Vec<Child>, FileFailure> {
    let mut children = Vec::new();
    for child in fs::read_dir(root)
        .map_err(|error| FileFailure::walk(FaultStage::WalkOpen, root, walk_root, &error))?
    {
        let child = child
            .map_err(|error| FileFailure::walk(FaultStage::WalkRead, root, walk_root, &error))?;
        budget.visit()?;
        let path = child.path();
        budget.path(&path)?;
        let relative = relative.join(child.file_name());
        let directory = match child.file_type() {
            Ok(kind) => kind.is_dir(),
            // Go ReadDir ignores disappearing entries during unknown-type
            // fstatat, whose PathError names the opened parent directory.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(FileFailure::walk(
                    FaultStage::WalkType,
                    root,
                    walk_root,
                    &error,
                ));
            }
        };
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
) -> Result<Vec<Entry>, FileFailure> {
    list_with(requested, depth, budget, |_| {})
}

fn list_with(
    requested: &Path,
    depth: u32,
    budget: &mut Budget,
    after_resolution: impl FnOnce(&Path),
) -> Result<Vec<Entry>, FileFailure> {
    let resolved = path::resolve_detailed(requested, budget.limits.path_bytes)?;
    after_resolution(&resolved);
    if !fs::metadata(&resolved)
        .map_err(|error| FileFailure::io(FaultStage::DirectoryStat, &resolved, None, &error))?
        .is_dir()
    {
        return Err(FileFailure::observed(
            FileError::InvalidArgument,
            FaultStage::DirectoryNotDirectory,
            &resolved,
            None,
            None,
        ));
    }
    walk(requested, &resolved, depth, budget)
}

fn walk(
    requested: &Path,
    resolved: &Path,
    depth: u32,
    budget: &mut Budget,
) -> Result<Vec<Entry>, FileFailure> {
    walk_with(requested, resolved, depth, budget, |_| {})
}

fn walk_with(
    requested: &Path,
    resolved: &Path,
    depth: u32,
    budget: &mut Budget,
    mut before_entry: impl FnMut(&Path),
) -> Result<Vec<Entry>, FileFailure> {
    budget.visit()?;
    // filepath.WalkDir independently lstats its root after checkIfDirectory.
    // A root replaced by a link/file is visited but never traversed.
    if !fs::symlink_metadata(resolved)
        .map_err(|error| FileFailure::walk(FaultStage::WalkStat, resolved, resolved, &error))?
        .is_dir()
    {
        return Ok(Vec::new());
    }
    let mut pending = children(resolved, Path::new(""), 1, budget, resolved)?;
    let mut result = Vec::new();
    while let Some(child) = pending.pop() {
        before_entry(&child.path);
        match metadata::entry_detailed(&child.path, budget.limits) {
            Ok(mut entry) => {
                let public_path = path::clean(&requested.join(&child.relative));
                budget.path(&public_path)?;
                entry.path = metadata::text(public_path.as_os_str())?;
                budget.retain(&entry)?;
                result.push(entry);
            }
            Err(failure) if failure.error == FileError::NotFound => {}
            Err(failure) if failure.error == FileError::ResourceExhausted => return Err(failure),
            Err(failure) => return Err(failure.in_walk(resolved)),
        }
        if child.directory {
            // WalkDir reads even a boundary directory before its first
            // too-deep callback. Preserve those errors without keeping rows.
            let descendants = children(
                &child.path,
                &child.relative,
                child.depth.saturating_add(1),
                budget,
                resolved,
            )?;
            if child.depth < depth {
                pending.extend(descendants);
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
