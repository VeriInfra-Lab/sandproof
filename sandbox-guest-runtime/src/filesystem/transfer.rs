//! Descriptor-owned, blocking file-body operations. Call only in admitted
//! work. No HTTP parsing, authentication, retries, atomic replacement or fsync.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use sandproof_guest_core::{StreamFailure, Transfer, TransferOutcome};

use super::{Budget, FileError, FileLimits, Ownership, directory, metadata, path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransferIoError {
    InvalidArgument,
    NotFound,
    ResourceExhausted,
    InsufficientStorage,
    Incomplete,
    Internal,
}

/// Explicit component parameters, not production defaults. At most one body
/// chunk is owned here; callers must separately bound their queues and input.
#[derive(Clone, Copy, Debug)]
pub struct TransferLimits {
    pub file: FileLimits,
    pub chunk_bytes: usize,
    pub total_bytes: u64,
}

impl TransferLimits {
    /// # Errors
    /// Rejects invalid/unrepresentable limits before any file operation.
    pub fn validate(self) -> Result<Self, TransferIoError> {
        self.file.validate().map_err(map_file_error)?;
        if self.chunk_bytes == 0
            || self.chunk_bytes > isize::MAX as usize / 16
            || self.total_bytes == 0
        {
            return Err(TransferIoError::InvalidArgument);
        }
        Ok(self)
    }

    fn check_path(self, value: &str) -> Result<(), TransferIoError> {
        if !Path::new(value).is_absolute() {
            return Err(TransferIoError::InvalidArgument);
        }
        if value.len() > self.file.path_bytes {
            return Err(TransferIoError::ResourceExhausted);
        }
        Ok(())
    }
}

/// Matches the fixed upload endpoint's printable-ASCII xattr contract. Call
/// before creating directories, changing ownership or truncating the file.
/// # Errors
/// Empty/oversized keys, non-printable bytes or more than 4096 stored bytes.
pub fn validate_metadata(values: &BTreeMap<String, String>) -> Result<(), TransferIoError> {
    let mut total = 0usize;
    for (key, value) in values {
        if key.is_empty()
            || key.len() > 246
            || !key
                .bytes()
                .chain(value.bytes())
                .all(|b| (32..=126).contains(&b))
        {
            return Err(TransferIoError::InvalidArgument);
        }
        total = total
            .checked_add(9 + key.len())
            .and_then(|n| n.checked_add(value.len()))
            .filter(|n| *n <= 4096)
            .ok_or(TransferIoError::InvalidArgument)?;
    }
    Ok(())
}

/// Completed body/metadata work, not a durability or whole-batch transaction.
#[derive(Debug, PartialEq, Eq)]
pub struct UploadResult {
    pub path: String,
    pub written: u64,
    pub metadata: BTreeMap<String, String>,
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
        let limits = limits.validate()?;
        limits.check_path(&value)?;
        validate_metadata(&metadata)?;
        if owner.uid == u32::MAX || owner.gid == u32::MAX {
            return Err(TransferIoError::InvalidArgument);
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
        directory::ensure_dirs(&path::parent_clean(target), owner, &mut budget)
            .map_err(map_file_error)?;
        let existing = match fs::metadata(target) {
            Ok(info) if info.is_dir() => return Err(TransferIoError::InvalidArgument),
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(TransferIoError::Internal),
        };
        let uid = Some(rustix::process::Uid::from_raw(owner.uid));
        let gid = Some(rustix::process::Gid::from_raw(owner.gid));
        let chowned = if existing {
            match rustix::fs::chown(target, uid, gid) {
                Ok(()) => true,
                Err(rustix::io::Errno::NOENT) => false,
                Err(_) => return Err(TransferIoError::Internal),
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
            .map_err(|e| map_io(&e))?;
        if !chowned {
            rustix::fs::fchown(&file, uid, gid).map_err(|_| TransferIoError::Internal)?;
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
        if let Err(error) = self.accounting.receive(bytes.len() as u64) {
            let _ = self.accounting.fail(StreamFailure::Write);
            return Err(match error {
                sandproof_guest_core::TransferError::LimitExceeded
                | sandproof_guest_core::TransferError::BufferFull => {
                    TransferIoError::ResourceExhausted
                }
                _ => TransferIoError::Incomplete,
            });
        }
        let mut rest = bytes;
        while !rest.is_empty() {
            match self.file.write(rest) {
                Ok(0) => {
                    let _ = self.accounting.fail(StreamFailure::Write);
                    return Err(TransferIoError::Internal);
                }
                Ok(n) => {
                    self.accounting
                        .record_write(n as u64)
                        .map_err(|_| TransferIoError::Internal)?;
                    rest = &rest[n..];
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => {
                    let _ = self.accounting.fail(StreamFailure::Write);
                    return Err(map_io(&e));
                }
            }
        }
        Ok(())
    }

    /// Invoke only for a complete, valid body end (not disconnect or timeout).
    /// Metadata replaces the previous set through the original open descriptor.
    /// # Errors
    /// Truncated/failed body or metadata failure. Already written bytes remain.
    pub fn finish(mut self) -> Result<UploadResult, TransferIoError> {
        self.accounting
            .end_input()
            .map_err(|_| TransferIoError::Incomplete)?;
        if self.accounting.outcome() != TransferOutcome::Pending {
            return Err(TransferIoError::Incomplete);
        }
        replace_metadata(&self.file, &self.metadata, self.limits.file.xattr_bytes)?;
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

/// Open descriptor and advertised length. Like identity `ServeContent`, the
/// transfer is the original length: later growth is not appended, shrinkage
/// is an error. This is not a consistent snapshot of concurrent modifications.
pub struct DownloadFile {
    file: File,
    length: u64,
    remaining: u64,
    chunk_bytes: usize,
    failed: bool,
}

impl DownloadFile {
    /// # Errors
    /// Missing/directory paths, unseekable files, length budget or I/O failure.
    pub fn open(value: &str, limits: TransferLimits) -> Result<Self, TransferIoError> {
        let limits = limits.validate()?;
        limits.check_path(value)?;
        let info = fs::metadata(value).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                TransferIoError::NotFound
            } else {
                TransferIoError::Internal
            }
        })?;
        if info.is_dir() {
            return Err(TransferIoError::InvalidArgument);
        }
        let mut file = File::open(value).map_err(|_| TransferIoError::Internal)?;
        let length = file
            .seek(SeekFrom::End(0))
            .map_err(|_| TransferIoError::Internal)?;
        if length > limits.total_bytes {
            return Err(TransferIoError::ResourceExhausted);
        }
        file.seek(SeekFrom::Start(0))
            .map_err(|_| TransferIoError::Internal)?;
        Ok(Self {
            file,
            length,
            remaining: length,
            chunk_bytes: limits.chunk_bytes,
            failed: false,
        })
    }

    #[must_use]
    pub const fn length(&self) -> u64 {
        self.length
    }

    /// # Errors
    /// Early EOF, read failure or reuse after a failed transfer.
    pub fn read_chunk(&mut self) -> Result<Option<Vec<u8>>, TransferIoError> {
        if self.failed {
            return Err(TransferIoError::Incomplete);
        }
        if self.remaining == 0 {
            return Ok(None);
        }
        let count = usize::try_from(self.remaining.min(self.chunk_bytes as u64))
            .map_err(|_| TransferIoError::Internal)?;
        let mut bytes = vec![0; count];
        loop {
            match self.file.read(&mut bytes) {
                Ok(0) => {
                    self.failed = true;
                    return Err(TransferIoError::Incomplete);
                }
                Ok(n) => {
                    self.remaining -= n as u64;
                    bytes.truncate(n);
                    return Ok(Some(bytes));
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    self.failed = true;
                    return Err(TransferIoError::Internal);
                }
            }
        }
    }
}

fn replace_metadata(
    file: &File,
    values: &BTreeMap<String, String>,
    limit: usize,
) -> Result<(), TransferIoError> {
    let mut names = vec![0; limit];
    let n = match rustix::fs::flistxattr(file, &mut names) {
        Ok(n) => n,
        Err(rustix::io::Errno::NOTSUP) => return Ok(()),
        Err(rustix::io::Errno::RANGE) => return Err(TransferIoError::ResourceExhausted),
        Err(e) => return Err(map_errno(e)),
    };
    for name in names[..n]
        .split(|b| *b == 0)
        .filter(|name| name.starts_with(b"user.e2b."))
    {
        let keep = std::str::from_utf8(&name[9..])
            .ok()
            .is_some_and(|key| values.contains_key(key));
        if !keep {
            use std::os::unix::ffi::OsStrExt;
            match rustix::fs::fremovexattr(file, std::ffi::OsStr::from_bytes(name)) {
                Ok(()) | Err(rustix::io::Errno::NODATA) => {}
                Err(rustix::io::Errno::NOTSUP) => return Ok(()),
                Err(e) => return Err(map_errno(e)),
            }
        }
    }
    for (key, value) in values {
        match rustix::fs::fsetxattr(
            file,
            format!("user.e2b.{key}"),
            value.as_bytes(),
            rustix::fs::XattrFlags::empty(),
        ) {
            Ok(()) => {}
            Err(rustix::io::Errno::NOTSUP) => return Ok(()),
            Err(e) => return Err(map_errno(e)),
        }
    }
    Ok(())
}

fn map_errno(error: rustix::io::Errno) -> TransferIoError {
    if matches!(error, rustix::io::Errno::NOSPC | rustix::io::Errno::DQUOT) {
        TransferIoError::InsufficientStorage
    } else {
        TransferIoError::Internal
    }
}

fn map_io(error: &std::io::Error) -> TransferIoError {
    if error.raw_os_error() == Some(rustix::io::Errno::NOSPC.raw_os_error()) {
        TransferIoError::InsufficientStorage
    } else {
        TransferIoError::Internal
    }
}

fn map_file_error(error: FileError) -> TransferIoError {
    match error {
        FileError::InvalidArgument => TransferIoError::InvalidArgument,
        FileError::ResourceExhausted => TransferIoError::ResourceExhausted,
        FileError::NotFound => TransferIoError::NotFound,
        _ => TransferIoError::Internal,
    }
}
