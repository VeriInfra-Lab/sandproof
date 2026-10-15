//! Explicit trusted source loading. No environment search or UTC fallback.
use super::{Error, Limits, Snapshot};
use rustix::fs::{Mode, OFlags};
use std::{
    fs::File,
    io::{self, Read},
    os::unix::ffi::OsStrExt,
    path::Path,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug)]
pub struct LoadLimits {
    pub path_bytes: usize,
    pub snapshot: Limits,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    InvalidConfiguration,
    InvalidPath,
    NotRegular,
    Io(io::ErrorKind),
    Allocation,
    Snapshot(Error),
}

impl Snapshot {
    /// Read an explicitly selected trusted Unix timezone file and retain an
    /// immutable parsed snapshot. Call only in bounded blocking startup work.
    /// Symlinks are followed, including the usual /etc/localtime link; the
    /// caller must establish the path's trusted source and template identity.
    /// The opened descriptor supplies both metadata and bytes. This does not
    /// promise a coherent snapshot of a file being concurrently overwritten,
    /// interrupt a filesystem call or prove a total process memory bound.
    /// # Errors
    /// Invalid budgets/path, non-regular source, I/O, allocation, excess input
    /// or invalid `TZif`. Errors neither search another file nor return UTC.
    pub fn load_file(path: &Path, limits: LoadLimits) -> Result<Self, LoadError> {
        let snapshot = limits
            .snapshot
            .validate()
            .map_err(|_| LoadError::InvalidConfiguration)?;
        if limits.path_bytes == 0 || limits.path_bytes > isize::MAX as usize / 16 {
            return Err(LoadError::InvalidConfiguration);
        }
        let bytes = path.as_os_str().as_bytes();
        if !path.is_absolute() || bytes.len() > limits.path_bytes || bytes.contains(&0) {
            return Err(LoadError::InvalidPath);
        }
        // Nonblocking open prevents an unopened FIFO from waiting for a
        // writer. Regular files retain normal semantics; reject other kinds
        // before reading. A symlink still resolves to its selected target.
        let fd = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|error| LoadError::Io(io::Error::from(error).kind()))?;
        from_file(File::from(fd), snapshot)
    }
}

fn from_file(mut file: File, limits: Limits) -> Result<Snapshot, LoadError> {
    let metadata = file
        .metadata()
        .map_err(|error| LoadError::Io(error.kind()))?;
    if !metadata.is_file() {
        return Err(LoadError::NotRegular);
    }
    if metadata.len() > limits.input_bytes as u64 {
        return Err(LoadError::Snapshot(Error::InputLimit));
    }
    read_snapshot(&mut file, limits)
}

fn read_snapshot(source: &mut impl Read, limits: Limits) -> Result<Snapshot, LoadError> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let remaining = limits.input_bytes - bytes.len();
        // Even an exactly full source must produce a real EOF. Keep its one
        // byte probe in fixed scratch, never extend the retained input budget.
        let wanted = buffer.len().min(remaining + 1);
        let count = match source.read(&mut buffer[..wanted]) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(LoadError::Io(error.kind())),
        };
        if count == 0 {
            break;
        }
        if count > remaining {
            return Err(LoadError::Snapshot(Error::InputLimit));
        }
        let required = bytes.len() + count;
        if required > bytes.capacity() {
            let capacity = required
                .max(bytes.capacity().saturating_mul(2))
                .min(limits.input_bytes);
            bytes
                .try_reserve_exact(capacity - bytes.len())
                .map_err(|_| LoadError::Allocation)?;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Snapshot::from_tzif(&bytes, limits).map_err(LoadError::Snapshot)
}
