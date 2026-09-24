//! Write guest-produced artifacts only into a preallocated private result area.

use crate::import::require_private_directory;
use crate::protocol::MAX_PAYLOAD_BYTES;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use tempfile::Builder;

#[derive(Debug)]
pub enum ResultStoreError {
    InvalidName,
    TooLarge,
    AlreadyExists,
    Io(io::Error),
}

impl From<io::Error> for ResultStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for ResultStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidName => write!(formatter, "invalid result name"),
            Self::TooLarge => write!(formatter, "result exceeds size limit"),
            Self::AlreadyExists => write!(formatter, "result already exists"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ResultStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// The caller allocates one private root per sandbox and controls its lifetime.
#[derive(Clone, Debug)]
pub struct ResultStore {
    root: PathBuf,
}

impl ResultStore {
    /// Bind to an operator-owned private result directory.
    ///
    /// # Errors
    ///
    /// Rejects a missing, public, or symlinked directory.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, ResultStoreError> {
        let root = root.into();
        require_private_directory(&root)?;
        Ok(Self { root })
    }

    /// Atomically publish one bounded artifact without replacing an old one.
    ///
    /// # Errors
    ///
    /// Rejects an invalid name, oversized payload, existing result, or I/O
    /// failure. This operation never interprets an artifact name as a path.
    pub fn publish(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, ResultStoreError> {
        if !valid_result_name(name) {
            return Err(ResultStoreError::InvalidName);
        }
        if bytes.len() > MAX_PAYLOAD_BYTES {
            return Err(ResultStoreError::TooLarge);
        }
        let destination = self.root.join(name);
        let mut temporary = Builder::new()
            .prefix(".pending-result-")
            .tempfile_in(&self.root)?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(&destination)
            .map_err(|failure| {
                if failure.error.kind() == io::ErrorKind::AlreadyExists {
                    ResultStoreError::AlreadyExists
                } else {
                    ResultStoreError::Io(failure.error)
                }
            })?;
        fs::File::open(&self.root)?.sync_all()?;
        Ok(destination)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn valid_result_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn store() -> (tempfile::TempDir, ResultStore) {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let store = ResultStore::new(directory.path()).unwrap();
        (directory, store)
    }

    #[test]
    fn publishes_only_inside_private_result_area() {
        let (directory, store) = store();
        let path = store.publish("build.log", b"hello\0world").unwrap();
        assert_eq!(path, directory.path().join("build.log"));
        assert_eq!(fs::read(path).unwrap(), b"hello\0world");
        assert!(matches!(
            store.publish("../escape", b"bad"),
            Err(ResultStoreError::InvalidName)
        ));
        assert!(!directory.path().parent().unwrap().join("escape").exists());
    }

    #[test]
    fn never_overwrites_a_published_result() {
        let (_directory, store) = store();
        store.publish("result", b"first").unwrap();
        assert!(matches!(
            store.publish("result", b"second"),
            Err(ResultStoreError::AlreadyExists)
        ));
        assert_eq!(fs::read(store.root().join("result")).unwrap(), b"first");
    }
}
