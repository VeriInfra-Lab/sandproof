//! Descriptor-owned, blocking file-body operations. Call only in admitted
//! work. No HTTP parsing, authentication, retries, atomic replacement or fsync.
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::file_download::LinuxErrno;

use super::{FileError, FileLimits};

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
    crate::file_upload::validate_metadata(values).map_err(|_| TransferIoError::InvalidArgument)
}

pub use crate::file_upload::UploadResult;

mod upload;
pub use upload::{UploadError, UploadFile};

/// Original descriptor read to observed EOF, as in the fixed gzip endpoint's
/// `io.Copy`. No seek or stat-length cap: growth before EOF may be included,
/// and shrinkage is ordinary EOF. This is not a filesystem snapshot.
/// Callers must bound reads/total bytes with the codec and retain admission.
pub struct DownloadSource(File);

impl DownloadSource {
    /// # Errors
    /// Path/budget checks, missing/directory paths or an open failure.
    pub fn open(value: &str, limits: TransferLimits) -> Result<Self, TransferIoError> {
        Self::open_with_stat(value, limits).map(|(source, _)| source)
    }

    /// Retain fixed endpoint pre-open stat metadata. This is not metadata of a
    /// replacement descriptor or a file snapshot; no extra stat follows open.
    /// # Errors
    /// Same original path/stat/directory/open checks as `open`.
    pub fn open_with_stat(
        value: &str,
        limits: TransferLimits,
    ) -> Result<(Self, fs::Metadata), TransferIoError> {
        let limits = limits.validate()?;
        limits.check_path(value)?;
        DownloadPath::inspect(value.to_owned(), limits)?.open()
    }
}
/// Validated original path and pre-open metadata, kept across encoding choice.
/// This does not pin an inode or claim a consistent filesystem snapshot.
pub struct DownloadPath {
    path: String,
    metadata: fs::Metadata,
}
impl DownloadPath {
    /// # Errors
    /// Path/budget, stat or directory rejection. Does not open the file body.
    pub fn inspect(path: String, limits: TransferLimits) -> Result<Self, TransferIoError> {
        Self::inspect_detailed(path, limits).map_err(|error| match error {
            DownloadPathError::Rejected(error) => error,
            DownloadPathError::Directory => TransferIoError::InvalidArgument,
            DownloadPathError::Io(Some(LinuxErrno(2))) => TransferIoError::NotFound,
            DownloadPathError::Io(_) => TransferIoError::Internal,
        })
    }
    /// Preserve the operation's Linux errno without copying arbitrary I/O text.
    /// # Errors
    /// Budget/path rejection, directory, or stat errno (unknown stays unknown).
    pub fn inspect_detailed(
        path: String,
        limits: TransferLimits,
    ) -> Result<Self, DownloadPathError> {
        let limits = limits.validate().map_err(DownloadPathError::Rejected)?;
        limits
            .check_path(&path)
            .map_err(DownloadPathError::Rejected)?;
        let metadata = fs::metadata(&path).map_err(|e| DownloadPathError::Io(linux_errno(&e)))?;
        if metadata.is_dir() {
            return Err(DownloadPathError::Directory);
        }
        Ok(Self { path, metadata })
    }
    #[must_use]
    pub const fn metadata(&self) -> &fs::Metadata {
        &self.metadata
    }
    /// # Errors
    /// Original-path open failure. Does not restat even after replacement.
    pub fn open(self) -> Result<(DownloadSource, fs::Metadata), TransferIoError> {
        self.open_detailed().map_err(|_| TransferIoError::Internal)
    }
    /// # Errors
    /// Original-path open errno, without restat or arbitrary diagnostic text.
    pub fn open_detailed(self) -> Result<(DownloadSource, fs::Metadata), DownloadPathError> {
        File::open(&self.path)
            .map(|file| (DownloadSource(file), self.metadata))
            .map_err(|e| DownloadPathError::Io(linux_errno(&e)))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownloadPathError {
    Rejected(TransferIoError),
    Directory,
    Io(Option<LinuxErrno>),
}
pub(super) fn linux_errno(error: &std::io::Error) -> Option<LinuxErrno> {
    // std rejects a NUL path before the syscall; Go's BytePtrFromString returns EINVAL.
    error
        .raw_os_error()
        .map(LinuxErrno)
        .or_else(|| (error.kind() == std::io::ErrorKind::InvalidInput).then_some(LinuxErrno(22)))
}

impl Read for DownloadSource {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(output)
    }
}
impl Seek for DownloadSource {
    fn seek(&mut self, offset: SeekFrom) -> std::io::Result<u64> {
        self.0.seek(offset).map_err(|error| {
            if let Some(errno) = error.raw_os_error() {
                std::io::Error::new(error.kind(), LinuxErrno(errno))
            } else {
                error
            }
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
        let DownloadSource(mut file) = DownloadSource::open(value, limits)?;
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

fn map_file_error(error: FileError) -> TransferIoError {
    match error {
        FileError::InvalidArgument => TransferIoError::InvalidArgument,
        FileError::ResourceExhausted => TransferIoError::ResourceExhausted,
        FileError::NotFound => TransferIoError::NotFound,
        _ => TransferIoError::Internal,
    }
}
