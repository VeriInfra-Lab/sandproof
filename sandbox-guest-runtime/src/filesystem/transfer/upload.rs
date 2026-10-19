//! Upload operation stages and retained kernel observations. Existing methods
//! keep their coarse errors; detailed methods preserve the original failure.
use super::super::{Budget, Ownership, directory, metadata, path};
use super::{
    TransferIoError, TransferLimits, UploadResult, linux_errno, map_file_error, validate_metadata,
};
use crate::file_download::LinuxErrno;
use crate::file_upload::{Fault, Stage};
use sandproof_guest_core::{StreamFailure, Transfer, TransferOutcome};
use std::os::unix::{
    ffi::{OsStrExt, OsStringExt},
    fs::OpenOptionsExt,
};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadError {
    Rejected(TransferIoError),
    Fault(Fault),
}
impl From<TransferIoError> for UploadError {
    fn from(error: TransferIoError) -> Self {
        Self::Rejected(error)
    }
}
impl UploadError {
    /// Original coarse component classification, without discarding the
    /// detailed observation until the caller explicitly chooses this adapter.
    #[must_use]
    pub fn into_legacy(self) -> TransferIoError {
        match self {
            Self::Rejected(error) => error,
            Self::Fault(fault) => match fault.status() {
                400 => TransferIoError::InvalidArgument,
                507 => TransferIoError::InsufficientStorage,
                _ => TransferIoError::Internal,
            },
        }
    }
}
fn fault(
    stage: Stage,
    path: &str,
    errno: Option<LinuxErrno>,
    attribute: Option<Vec<u8>>,
) -> UploadError {
    UploadError::Fault(Fault {
        stage,
        path: path.as_bytes().to_vec(),
        errno,
        attribute,
    })
}
fn directory_error(error: directory::EnsureError) -> UploadError {
    match error {
        directory::EnsureError::Budget(error) => map_file_error(error).into(),
        directory::EnsureError::NotDirectory(path) => UploadError::Fault(Fault {
            stage: Stage::ParentNotDirectory,
            path: path.into_os_string().into_vec(),
            errno: None,
            attribute: None,
        }),
        directory::EnsureError::Io {
            operation,
            path,
            errno,
        } => UploadError::Fault(Fault {
            stage: match operation {
                directory::EnsureOperation::Stat => Stage::ParentStat,
                directory::EnsureOperation::Create => Stage::ParentCreate,
                directory::EnsureOperation::Chown => Stage::ParentChown,
            },
            path: path.into_os_string().into_vec(),
            errno,
            attribute: None,
        }),
    }
}

pub struct UploadFile {
    file: File,
    path: String,
    metadata: BTreeMap<String, String>,
    limits: TransferLimits,
    accounting: Transfer,
}

impl UploadFile {
    /// Preserves the pinned endpoint's pre-chown, truncate, fallback fchown
    /// order. Errors can leave parent directories, ownership or partial data.
    /// Following guest symlinks is intentional; this is not host confinement.
    /// # Errors
    /// Invalid inputs/budgets, directory targets or filesystem failures.
    pub fn open(
        value: String,
        owner: Ownership,
        metadata: BTreeMap<String, String>,
        expected: Option<u64>,
        limits: TransferLimits,
    ) -> Result<Self, TransferIoError> {
        Self::open_detailed(value, owner, metadata, expected, limits)
            .map_err(UploadError::into_legacy)
    }

    /// Open in the original mutation order, preserving the failing operation's
    /// bounded path and structured errno. No arbitrary diagnostic is retained.
    /// # Errors
    /// Input/budget rejection, confirmed directory or actual preparation/open failure.
    pub fn open_detailed(
        value: String,
        owner: Ownership,
        metadata: BTreeMap<String, String>,
        expected: Option<u64>,
        limits: TransferLimits,
    ) -> Result<Self, UploadError> {
        let limits = limits.validate()?;
        limits.check_path(&value)?;
        validate_metadata(&metadata)?;
        if owner.uid == u32::MAX || owner.gid == u32::MAX {
            return Err(TransferIoError::InvalidArgument.into());
        }
        let accounting = Transfer::new(limits.total_bytes, limits.chunk_bytes as u64, expected)
            .map_err(|_| TransferIoError::ResourceExhausted)?;
        let target = Path::new(&value);
        let mut budget = Budget {
            limits: limits.file,
            visited: 0,
            entries: 0,
            bytes: 0,
        };
        directory::ensure_dirs_detailed(&path::parent_clean(target), owner, &mut budget)
            .map_err(directory_error)?;
        let existing = match fs::metadata(target) {
            Ok(info) if info.is_dir() => return Err(fault(Stage::Directory, &value, None, None)),
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(fault(Stage::Stat, &value, linux_errno(&error), None)),
        };
        let uid = Some(rustix::process::Uid::from_raw(owner.uid));
        let gid = Some(rustix::process::Gid::from_raw(owner.gid));
        let chowned = if existing {
            match rustix::fs::chown(target, uid, gid) {
                Ok(()) => true,
                Err(rustix::io::Errno::NOENT) => false,
                Err(error) => {
                    return Err(fault(
                        Stage::Chown,
                        &value,
                        Some(LinuxErrno(error.raw_os_error())),
                        None,
                    ));
                }
            }
        } else {
            false
        };
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o666)
            .open(target)
            .map_err(|e| fault(Stage::Open, &value, linux_errno(&e), None))?;
        if !chowned {
            rustix::fs::fchown(&file, uid, gid).map_err(|error| {
                fault(
                    Stage::FileChown,
                    &value,
                    Some(LinuxErrno(error.raw_os_error())),
                    None,
                )
            })?;
        }
        Ok(Self {
            file,
            path: value,
            metadata,
            limits,
            accounting,
        })
    }

    /// The accepted chunk is never replayed after a partial write/error.
    /// # Errors
    /// Chunk/body limits or an actual write failure; the old body is not restored.
    pub fn write_chunk(&mut self, bytes: &[u8]) -> Result<(), TransferIoError> {
        self.write_chunk_detailed(bytes)
            .map_err(UploadError::into_legacy)
    }
    /// Accept each input byte once; preserve partial-write errno without replay.
    /// # Errors
    /// Original body/chunk budget, failed transfer, zero write or real I/O error.
    pub fn write_chunk_detailed(&mut self, bytes: &[u8]) -> Result<(), UploadError> {
        write_chunk_to(&mut self.file, &mut self.accounting, &self.path, bytes)
    }

    /// Invoke only for a complete, valid body end (not disconnect or timeout).
    /// Metadata replaces the previous set through the original open descriptor.
    /// # Errors
    /// Truncated/failed body or metadata failure. Already written bytes remain.
    pub fn finish(self) -> Result<UploadResult, TransferIoError> {
        self.finish_detailed().map_err(UploadError::into_legacy)
    }
    /// Finish only after a valid complete input. Metadata faults preserve the
    /// original descriptor operation/name, including already applied changes.
    /// # Errors
    /// Incomplete input, metadata I/O, scratch or retained result budget.
    pub fn finish_detailed(mut self) -> Result<UploadResult, UploadError> {
        self.accounting
            .end_input()
            .map_err(|_| TransferIoError::Incomplete)?;
        if self.accounting.outcome() != TransferOutcome::Pending {
            return Err(TransferIoError::Incomplete.into());
        }
        replace_metadata(
            &self.file,
            &self.path,
            &self.metadata,
            self.limits.file.xattr_bytes,
        )?;
        // Fixed envd reads response metadata back through the original path;
        // concurrent replacement is not a snapshot or identity guarantee.
        let metadata = metadata::xattrs(Path::new(&self.path), self.limits.file.xattr_bytes)
            .map_err(map_file_error)?;
        metadata
            .iter()
            .flat_map(|(key, value)| [key.len(), value.len()])
            .try_fold(self.path.len(), usize::checked_add)
            .filter(|bytes| *bytes <= self.limits.file.retained_bytes)
            .ok_or(TransferIoError::ResourceExhausted)?;
        self.accounting
            .finish()
            .map_err(|_| TransferIoError::Incomplete)?;
        Ok(UploadResult {
            path: self.path,
            written: self.accounting.written_bytes(),
            metadata,
        })
    }
}

fn replace_metadata(
    file: &File,
    path: &str,
    values: &BTreeMap<String, String>,
    limit: usize,
) -> Result<(), UploadError> {
    let mut names = vec![0; limit];
    let n = match rustix::fs::flistxattr(file, &mut names) {
        Ok(n) => n,
        Err(rustix::io::Errno::NOTSUP) => return Ok(()),
        Err(rustix::io::Errno::RANGE) => return Err(TransferIoError::ResourceExhausted.into()),
        Err(e) => {
            return Err(fault(
                Stage::MetadataList,
                path,
                Some(LinuxErrno(e.raw_os_error())),
                None,
            ));
        }
    };
    for name in names[..n]
        .split(|b| *b == 0)
        .filter(|name| name.starts_with(b"user.e2b."))
    {
        let keep = std::str::from_utf8(&name[9..])
            .ok()
            .is_some_and(|key| values.contains_key(key));
        if !keep {
            match rustix::fs::fremovexattr(file, std::ffi::OsStr::from_bytes(name)) {
                Ok(()) | Err(rustix::io::Errno::NODATA) => {}
                Err(rustix::io::Errno::NOTSUP) => return Ok(()),
                Err(e) => {
                    return Err(fault(
                        Stage::MetadataRemove,
                        path,
                        Some(LinuxErrno(e.raw_os_error())),
                        Some(name.to_vec()),
                    ));
                }
            }
        }
    }
    for (key, value) in values {
        let name = format!("user.e2b.{key}");
        match rustix::fs::fsetxattr(
            file,
            &name,
            value.as_bytes(),
            rustix::fs::XattrFlags::empty(),
        ) {
            Ok(()) => {}
            Err(rustix::io::Errno::NOTSUP) => return Ok(()),
            Err(e) => {
                return Err(fault(
                    Stage::MetadataSet,
                    path,
                    Some(LinuxErrno(e.raw_os_error())),
                    Some(name.into_bytes()),
                ));
            }
        }
    }
    Ok(())
}

fn write_chunk_to(
    writer: &mut impl Write,
    accounting: &mut Transfer,
    path: &str,
    bytes: &[u8],
) -> Result<(), UploadError> {
    if let Err(error) = accounting.receive(bytes.len() as u64) {
        let _ = accounting.fail(StreamFailure::Write);
        return Err((match error {
            sandproof_guest_core::TransferError::LimitExceeded
            | sandproof_guest_core::TransferError::BufferFull => TransferIoError::ResourceExhausted,
            _ => TransferIoError::Incomplete,
        })
        .into());
    }
    let mut rest = bytes;
    while !rest.is_empty() {
        match writer.write(rest) {
            Ok(0) => {
                let _ = accounting.fail(StreamFailure::Write);
                return Err(fault(Stage::WriteZero, path, None, None));
            }
            Ok(n) => {
                accounting
                    .record_write(n as u64)
                    .map_err(|_| TransferIoError::Internal)?;
                rest = &rest[n..];
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => {
                let _ = accounting.fail(StreamFailure::Write);
                return Err(fault(Stage::Write, path, linux_errno(&e), None));
            }
        }
    }
    Ok(())
}
