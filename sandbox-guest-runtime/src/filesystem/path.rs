use super::{FileError, io_error};
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
    let mut pending: VecDeque<Vec<u8>> = path
        .as_os_str()
        .as_bytes()
        .split(|b| *b == b'/')
        .map(<[u8]>::to_vec)
        .collect();
    let mut resolved = PathBuf::from("/");
    let mut followed = 0;
    while let Some(part) = pending.pop_front() {
        match part.as_slice() {
            b"" | b"." => continue,
            b".." => {
                resolved.pop();
                continue;
            }
            _ => {}
        }
        resolved.push(OsStr::from_bytes(&part));
        if resolved.as_os_str().len() > limit {
            return Err(FileError::ResourceExhausted);
        }
        let info = fs::symlink_metadata(&resolved).map_err(io_error)?;
        if !info.file_type().is_symlink() {
            if !info.is_dir() && !pending.is_empty() {
                return Err(FileError::Internal);
            }
            continue;
        }
        followed += 1;
        if followed > 255 {
            return Err(FileError::FailedPrecondition);
        }
        let mut link = vec![0; limit + 1];
        let length = rustix::fs::readlinkat_raw(rustix::fs::CWD, &resolved, link.as_mut_slice())
            .map_err(|e| io_error(e.into()))?;
        if length > limit {
            return Err(FileError::ResourceExhausted);
        }
        link.truncate(length);
        if link.starts_with(b"/") {
            resolved = PathBuf::from("/");
        } else {
            resolved.pop();
        }
        let pending_bytes = pending.iter().map(|p| p.len() + 1).sum::<usize>();
        if link
            .len()
            .checked_add(pending_bytes)
            .is_none_or(|n| n > limit)
        {
            return Err(FileError::ResourceExhausted);
        }
        for part in link.split(|b| *b == b'/').rev() {
            pending.push_front(part.to_vec());
        }
    }
    Ok(clean(&resolved))
}
