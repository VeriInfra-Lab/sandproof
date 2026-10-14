use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::path::Path;

use super::{FileError, FileFailure, FileLimits, path};
use crate::accounts::{AccountError, lookup_local_ownership};
use crate::file_protocol::{Entry, FaultStage, FileType, ModifiedTime};

pub(super) fn text(value: &OsStr) -> Result<String, FileError> {
    value.to_str().map(str::to_owned).ok_or(FileError::Internal)
}

pub(super) fn entry(path: &Path, limits: FileLimits) -> Result<Entry, FileError> {
    entry_detailed(path, limits).map_err(|failure| failure.error)
}

pub(super) fn entry_detailed(path: &Path, limits: FileLimits) -> Result<Entry, FileFailure> {
    let info = fs::symlink_metadata(path)
        .map_err(|error| FileFailure::io(FaultStage::EntryStat, path, None, &error))?;
    let (symlink_target, target_info) = if info.file_type().is_symlink() {
        let target = match path::resolve(path, limits.path_bytes) {
            Ok(target) => target,
            Err(FileError::ResourceExhausted) => return Err(FileError::ResourceExhausted.into()),
            Err(_) => path.to_owned(),
        };
        let target_info = fs::metadata(&target).ok();
        (Some(text(target.as_os_str())?), target_info)
    } else {
        (None, None)
    };
    let attributes = if symlink_target.is_some() {
        target_info.as_ref()
    } else {
        Some(&info)
    };
    let (kind, mode) = attributes.map_or((FileType::Unspecified, 0), |m| {
        let kind = if m.is_file() {
            FileType::File
        } else if m.is_dir() {
            FileType::Directory
        } else {
            FileType::Unspecified
        };
        (kind, m.mode() & 0o777)
    });
    let (owner, group) = lookup_local_ownership(info.uid(), info.gid(), limits.account_bytes)
        .map_err(|e| {
            if e == AccountError::ResourceExhausted {
                FileError::ResourceExhausted
            } else {
                FileError::Internal
            }
        })?;
    let modified_time = if info.mtime() == 0 && info.mtime_nsec() == 0 {
        None
    } else {
        Some(
            ModifiedTime::new(
                info.mtime(),
                u32::try_from(info.mtime_nsec()).map_err(|_| FileError::Internal)?,
            )
            .map_err(|_| FileError::Internal)?,
        )
    };
    // Go FileInfo.Name strips trailing separators, but retains a final dot.
    let name = basename(path);
    Ok(Entry {
        name: text(name)?,
        kind,
        path: text(path.as_os_str())?,
        size: i64::try_from(info.size()).map_err(|_| FileError::Internal)?,
        mode,
        permissions: permissions(info.mode()),
        owner,
        group,
        modified_time,
        symlink_target,
        metadata: xattrs(path, limits.xattr_bytes)?,
    })
}

fn basename(path: &Path) -> &OsStr {
    let raw = path.as_os_str().as_bytes();
    let end = raw.iter().rposition(|b| *b != b'/').map_or(1, |i| i + 1);
    let raw = &raw[..end];
    let start = raw.iter().rposition(|b| *b == b'/').map_or(0, |i| i + 1);
    if start == end {
        OsStr::new("/")
    } else {
        OsStr::from_bytes(&raw[start..])
    }
}

fn permissions(mode: u32) -> String {
    let mut value = match mode & 0o170_000 {
        0o040_000 => "d",
        0o120_000 => "L",
        0o060_000 => "D",
        0o020_000 => "Dc",
        0o010_000 => "p",
        0o140_000 => "S",
        _ => "",
    }
    .to_owned();
    // Go's mode string has separate u/g/t flags, not ls's substituted s/t.
    // Character-device 'c' follows setuid/setgid in Go's flag order.
    let character = value.ends_with('c');
    if character {
        value.pop();
    }
    if mode & 0o4000 != 0 {
        value.push('u');
    }
    if mode & 0o2000 != 0 {
        value.push('g');
    }
    if character {
        value.push('c');
    }
    if mode & 0o1000 != 0 {
        value.push('t');
    }
    if value.is_empty() {
        value.push('-');
    }
    for (index, bit) in (0..9).rev().enumerate() {
        value.push(if mode & (1 << bit) != 0 {
            ['r', 'w', 'x'][index % 3]
        } else {
            '-'
        });
    }
    value
}

// Ordinary xattr I/O failures discard metadata (fixed Go best-effort policy).
// Explicit capacity failures must remain observable, never truncated success.
pub(super) fn xattrs(path: &Path, limit: usize) -> Result<BTreeMap<String, String>, FileError> {
    let mut names = vec![0; limit];
    let len = match rustix::fs::listxattr(path, names.as_mut_slice()) {
        Ok(len) => len,
        Err(rustix::io::Errno::RANGE) => return Err(FileError::ResourceExhausted),
        Err(_) => return Ok(BTreeMap::new()),
    };
    names.truncate(len);
    let mut value = vec![0; limit];
    let mut result = BTreeMap::new();
    let mut bytes = 0;
    for name in names
        .split(|b| *b == 0)
        .filter(|name| name.starts_with(b"user.e2b."))
    {
        let len = match rustix::fs::getxattr(path, OsStr::from_bytes(name), value.as_mut_slice()) {
            Ok(len) => len,
            Err(rustix::io::Errno::NODATA) => continue,
            Err(rustix::io::Errno::RANGE) => return Err(FileError::ResourceExhausted),
            Err(_) => return Ok(BTreeMap::new()),
        };
        let (Ok(key), Ok(value)) = (
            std::str::from_utf8(&name[9..]),
            std::str::from_utf8(&value[..len]),
        ) else {
            continue;
        };
        let size = key.len() + value.len();
        if size > limit - bytes {
            return Err(FileError::ResourceExhausted);
        }
        bytes += size;
        result.insert(key.to_owned(), value.to_owned());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::permissions;

    #[test]
    fn permission_strings_match_fixed_go_mode_vectors_without_privileged_inodes() {
        // Expected strings checked against Go 1.26.8 io/fs.FileMode.String.
        // Creating SUID/SGID/device inodes is forbidden in ordinary CI; these
        // vectors validate formatting, not actual kernel permission changes.
        for (mode, expected) in [
            (0o100_000, "----------"),
            (0o106_750, "ugrwxr-x---"),
            (0o041_777, "dtrwxrwxrwx"),
            (0o026_666, "Dugcrw-rw-rw-"),
            (0o060_640, "Drw-r-----"),
            (0o010_600, "prw-------"),
            (0o140_600, "Srw-------"),
            (0o120_777, "Lrwxrwxrwx"),
        ] {
            assert_eq!(permissions(mode), expected, "mode {mode:o}");
        }
    }
}
