//! Blocking guest filesystem operations. Call only inside separately admitted
//! work; these APIs neither authenticate nor change daemon credentials. Paths
//! are guest paths, not a host confinement mechanism. Effects are not atomic.
use std::fs;
use std::path::Path;

use crate::file_protocol::{Entry, Request, Response};
mod directory;
mod metadata;
mod path;
mod remove;
pub mod transfer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileError {
    InvalidArgument,
    NotFound,
    AlreadyExists,
    FailedPrecondition,
    ResourceExhausted,
    Internal,
}
impl FileError {
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidArgument => "invalid_argument",
            Self::NotFound => "not_found",
            Self::AlreadyExists => "already_exists",
            Self::FailedPrecondition => "failed_precondition",
            Self::ResourceExhausted => "resource_exhausted",
            Self::Internal => "internal",
        }
    }
}

/// Explicit component limits, not deployed defaults or a proof of total RAM.
/// `visited` bounds directory work/stack entries, `entries` bounds results,
/// and `retained_bytes` bounds result string/metadata payload. Traversal paths
/// are separately bounded by `visited * path_bytes`; xattr/account scratch by
/// their named limits. Kernel/stdlib allocations remain external obligations.
#[derive(Clone, Copy, Debug)]
pub struct FileLimits {
    pub path_bytes: usize,
    pub entries: usize,
    pub retained_bytes: usize,
    pub visited: usize,
    pub xattr_bytes: usize,
    pub account_bytes: usize,
}
impl FileLimits {
    /// # Errors
    /// Rejects zero/unrepresentable budgets before any filesystem effect.
    pub fn validate(self) -> Result<Self, FileError> {
        if [
            self.path_bytes,
            self.entries,
            self.retained_bytes,
            self.visited,
            self.xattr_bytes,
            self.account_bytes,
        ]
        .into_iter()
        .any(|n| n == 0 || n > isize::MAX as usize / 16)
            || self
                .path_bytes
                .checked_mul(self.visited)
                .is_none_or(|n| n > isize::MAX as usize / 16)
        {
            return Err(FileError::InvalidArgument);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Ownership {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FileResponse {
    Entry(Entry),
    Entries(Vec<Entry>),
    Empty,
}
impl FileResponse {
    #[must_use]
    pub fn as_wire(&self) -> Response<'_> {
        match self {
            Self::Entry(entry) => Response::Entry(entry),
            Self::Entries(entries) => Response::Entries(entries),
            Self::Empty => Response::Empty,
        }
    }
}

struct Budget {
    limits: FileLimits,
    visited: usize,
    entries: usize,
    bytes: usize,
}
impl Budget {
    fn visit(&mut self) -> Result<(), FileError> {
        if self.visited == self.limits.visited {
            return Err(FileError::ResourceExhausted);
        }
        self.visited += 1;
        Ok(())
    }
    fn path(&self, path: &Path) -> Result<(), FileError> {
        if path.as_os_str().len() > self.limits.path_bytes {
            return Err(FileError::ResourceExhausted);
        }
        Ok(())
    }
    fn retain(&mut self, entry: &Entry) -> Result<(), FileError> {
        let bytes = [
            entry.name.len(),
            entry.path.len(),
            entry.permissions.len(),
            entry.owner.len(),
            entry.group.len(),
            entry.symlink_target.as_ref().map_or(0, String::len),
            35,
        ]
        .into_iter()
        .chain(entry.metadata.iter().flat_map(|(k, v)| [k.len(), v.len()]))
        .try_fold(0usize, usize::checked_add)
        .ok_or(FileError::ResourceExhausted)?;
        if self.entries == self.limits.entries || bytes > self.limits.retained_bytes - self.bytes {
            return Err(FileError::ResourceExhausted);
        }
        self.entries += 1;
        self.bytes += bytes;
        Ok(())
    }
}

/// Execute one fully resolved operation. An error can follow directory creation,
/// rename or partial removal; never retry this call to resolve a lost reply.
/// # Errors
/// Fixed filesystem error categories and explicit component budget exhaustion.
pub fn execute(
    request: &Request,
    owner: Ownership,
    limits: FileLimits,
) -> Result<FileResponse, FileError> {
    let limits = limits.validate()?;
    if owner.uid == u32::MAX || owner.gid == u32::MAX {
        return Err(FileError::InvalidArgument);
    }
    let mut budget = Budget {
        limits,
        visited: 0,
        entries: 0,
        bytes: 0,
    };
    let check = |value: &str| {
        let p = Path::new(value);
        if !p.is_absolute() {
            return Err(FileError::InvalidArgument);
        }
        budget.path(p)
    };
    match request {
        Request::Stat(p)
        | Request::MakeDir(p)
        | Request::Remove(p)
        | Request::ListDir { path: p, .. } => check(p)?,
        Request::Move {
            source,
            destination,
        } => {
            check(source)?;
            check(destination)?;
        }
    }
    let entry = match request {
        Request::Stat(p) => metadata::entry(Path::new(p), limits)?,
        Request::MakeDir(p) => {
            match fs::metadata(p) {
                Ok(m) => {
                    return Err(if m.is_dir() {
                        FileError::AlreadyExists
                    } else {
                        FileError::InvalidArgument
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(FileError::Internal),
            }
            directory::ensure_dirs(Path::new(p), owner, &mut budget)?;
            metadata::entry(Path::new(p), limits)?
        }
        Request::Move {
            source,
            destination,
        } => {
            directory::ensure_dirs(
                &path::parent_clean(Path::new(destination)),
                owner,
                &mut budget,
            )?;
            fs::rename(source, destination).map_err(io_error)?;
            metadata::entry(Path::new(destination), limits)?
        }
        Request::ListDir { path, depth } => {
            return directory::list(Path::new(path), (*depth).max(1), &mut budget)
                .map(FileResponse::Entries);
        }
        Request::Remove(p) => {
            remove::remove(Path::new(p), &mut budget)?;
            return Ok(FileResponse::Empty);
        }
    };
    budget.retain(&entry)?;
    Ok(FileResponse::Entry(entry))
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err consumes the original I/O error"
)]
fn io_error(error: std::io::Error) -> FileError {
    if error.kind() == std::io::ErrorKind::NotFound {
        FileError::NotFound
    } else {
        FileError::Internal
    }
}
