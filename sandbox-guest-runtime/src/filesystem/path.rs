// Component resolution follows Go 1.26.8 filepath.walkSymlinks.
// Copyright 2012 The Go Authors. BSD license: ../multipart/GO-LICENSE.
use super::{FileError, FileFailure};
use crate::file_protocol::FaultStage;
use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

// Lexical cleaning is only used where the fixed endpoint uses filepath.Join/
// Dir. It must not pre-clean a path before actual symlink resolution.
pub(super) fn clean(path: &Path) -> PathBuf {
    let bytes = path.as_os_str().as_bytes();
    let mut parts: Vec<&[u8]> = Vec::new();
    for part in bytes.split(|b| *b == b'/') {
        match part {
            b"" | b"." => {}
            b".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let mut result = vec![b'/'];
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            result.push(b'/');
        }
        result.extend_from_slice(part);
    }
    PathBuf::from(OsString::from_vec(result))
}

pub(super) fn parent_clean(path: &Path) -> PathBuf {
    let bytes = path.as_os_str().as_bytes();
    let at = bytes.iter().rposition(|b| *b == b'/').unwrap_or(0);
    clean(Path::new(OsStr::from_bytes(&bytes[..at])))
}

/// Resolve components without kernel canonicalize's shorter link-chain limit.
/// The fixed Go walk allows 255 dereferences. Reads are bounded before keeping
/// targets, and relative links are interpreted at their actual parent.
pub(super) fn resolve(path: &Path, limit: usize) -> Result<PathBuf, FileError> {
    resolve_detailed(path, limit).map_err(|failure| failure.error)
}

pub(super) fn resolve_detailed(path: &Path, limit: usize) -> Result<PathBuf, FileFailure> {
    resolve_with(path, limit, |_| {})
}

fn resolve_with(
    path: &Path,
    limit: usize,
    mut before_readlink: impl FnMut(&Path),
) -> Result<PathBuf, FileFailure> {
    let mut pending: VecDeque<Vec<u8>> = path
        .as_os_str()
        .as_bytes()
        .split(|b| *b == b'/')
        .map(<[u8]>::to_vec)
        .collect();
    let mut resolved = vec![b'/'];
    let mut followed = 0;
    while let Some(part) = pending.pop_front() {
        match part.as_slice() {
            b"" | b"." => continue,
            b".." => {
                parent_resolution(&mut resolved, limit)?;
                continue;
            }
            _ => {}
        }
        append(&mut resolved, &part, limit)?;
        let actual = Path::new(OsStr::from_bytes(&resolved));
        let info = fs::symlink_metadata(actual)
            .map_err(|error| resolve_error(path, FaultStage::ResolveStat, actual, &error))?;
        if !info.file_type().is_symlink() {
            if !info.is_dir() && !pending.is_empty() {
                return Err(FileFailure::observed(
                    FileError::Internal,
                    FaultStage::ResolveNotDirectory,
                    actual,
                    None,
                    None,
                ));
            }
            continue;
        }
        followed += 1;
        if followed > 255 {
            return Err(FileFailure::observed(
                FileError::FailedPrecondition,
                FaultStage::ResolveCycle,
                path,
                None,
                None,
            ));
        }
        before_readlink(actual);
        let mut link = vec![0; limit + 1];
        let length = rustix::fs::readlinkat_raw(rustix::fs::CWD, actual, link.as_mut_slice())
            .map_err(|error| {
                resolve_error(path, FaultStage::ResolveReadlink, actual, &error.into())
            })?;
        if length > limit {
            return Err(FileError::ResourceExhausted.into());
        }
        link.truncate(length);
        if link.starts_with(b"/") {
            resolved.truncate(1);
        } else {
            let slash = resolved.iter().rposition(|b| *b == b'/').unwrap_or(0);
            resolved.truncate(slash.max(1));
        }
        let pending_bytes = pending.iter().map(|p| p.len() + 1).sum::<usize>();
        if link
            .len()
            .checked_add(pending_bytes)
            .is_none_or(|n| n > limit)
        {
            return Err(FileError::ResourceExhausted.into());
        }
        for part in link.split(|b| *b == b'/').rev() {
            pending.push_front(part.to_vec());
        }
    }
    Ok(clean(Path::new(OsStr::from_bytes(&resolved))))
}

fn append(resolved: &mut Vec<u8>, part: &[u8], limit: usize) -> Result<(), FileError> {
    let separator = !resolved.ends_with(b"/");
    if resolved
        .len()
        .checked_add(part.len())
        .and_then(|n| n.checked_add(usize::from(separator)))
        .is_none_or(|n| n > limit)
    {
        return Err(FileError::ResourceExhausted);
    }
    if separator {
        resolved.push(b'/');
    }
    resolved.extend_from_slice(part);
    Ok(())
}

// Fixed Go walkSymlinks retains a leading '..' segment until final Clean.
// PathBuf::pop would change intermediate syscall paths and their diagnostics.
fn parent_resolution(resolved: &mut Vec<u8>, limit: usize) -> Result<(), FileError> {
    match resolved[1..]
        .iter()
        .rposition(|b| *b == b'/')
        .map(|n| n + 1)
    {
        Some(slash) if &resolved[slash + 1..] != b".." => resolved.truncate(slash),
        _ => append(resolved, b"..", limit)?,
    }
    Ok(())
}

fn resolve_error(
    requested: &Path,
    stage: FaultStage,
    actual: &Path,
    error: &std::io::Error,
) -> FileFailure {
    // Original followSymlink checks os.IsNotExist first, then searches the
    // full PathError string for this phrase, including its unquoted path.
    let errno = super::transfer::linux_errno(error);
    if error.kind() != std::io::ErrorKind::NotFound
        && (errno == Some(crate::file_download::LinuxErrno(31))
            || actual
                .as_os_str()
                .as_bytes()
                .windows(b"too many links".len())
                .any(|p| p == b"too many links"))
    {
        FileFailure::observed(
            FileError::FailedPrecondition,
            FaultStage::ResolveCycle,
            requested,
            None,
            errno,
        )
    } else {
        FileFailure::io(stage, actual, None, error)
    }
}

#[cfg(test)]
mod tests;
