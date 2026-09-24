//! Disposable, per-sandbox ext4 disks: read-only imported input and private work.
//!
//! The caller must keep `DiskImages` alive until its VM has stopped. The
//! Firecracker adapter must configure `input.img` as a read-only drive.

use crate::import::{ImportError, StagedInputs, require_private_directory};
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{Builder, TempDir};

const MIB: u64 = 1024 * 1024;
const MIN_INPUT_BYTES: u64 = 64 * MIB;
const MAX_INPUT_BYTES: u64 = 2 * 1024 * MIB;

#[derive(Debug)]
pub enum ImageError {
    InvalidConfiguration,
    InputTooLarge,
    FormatFailed,
    Import(ImportError),
    Io(io::Error),
}

impl From<io::Error> for ImageError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ImportError> for ImageError {
    fn from(error: ImportError) -> Self {
        Self::Import(error)
    }
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfiguration => write!(formatter, "invalid image configuration"),
            Self::InputTooLarge => write!(formatter, "input image would exceed configured limit"),
            Self::FormatFailed => write!(formatter, "ext4 formatter failed"),
            Self::Import(error) => error.fmt(formatter),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ImageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Import(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ext4Builder {
    /// Absolute, operator-controlled path to `mkfs.ext4`.
    formatter: PathBuf,
    workspace_bytes: u64,
}

impl Ext4Builder {
    /// A workspace image must have a fixed, bounded size, at least 64 MiB.
    ///
    /// # Errors
    ///
    /// Returns `InvalidConfiguration` for an invalid executable path or size.
    pub fn new(formatter: impl Into<PathBuf>, workspace_bytes: u64) -> Result<Self, ImageError> {
        let formatter = formatter.into();
        if !formatter.is_absolute()
            || !(MIN_INPUT_BYTES..=MAX_INPUT_BYTES).contains(&workspace_bytes)
            || !workspace_bytes.is_multiple_of(4096)
        {
            return Err(ImageError::InvalidConfiguration);
        }
        Ok(Self {
            formatter,
            workspace_bytes,
        })
    }

    /// Builds fresh images under an operator-owned private directory.
    ///
    /// # Errors
    ///
    /// Returns an error if space calculation, file creation, or ext4 format
    /// fails. The temporary image directory is removed on failure.
    pub fn build(&self, staged: &StagedInputs, data_root: &Path) -> Result<DiskImages, ImageError> {
        require_private_directory(data_root)?;
        let input_bytes = input_image_size(staged.bytes(), staged.entries())?;
        let tree = Builder::new()
            .prefix("sandbox-disks-")
            .tempdir_in(data_root)?;
        fs::set_permissions(tree.path(), fs::Permissions::from_mode(0o700))?;
        let input = tree.path().join("input.img");
        let workspace = tree.path().join("workspace.img");
        create_sparse_file(&input, input_bytes)?;
        create_sparse_file(&workspace, self.workspace_bytes)?;
        format_ext4(
            &self.formatter,
            &input,
            "sandbox-input",
            Some(staged.path()),
        )?;
        format_ext4(&self.formatter, &workspace, "sandbox-work", None)?;
        fs::set_permissions(&input, fs::Permissions::from_mode(0o400))?;
        Ok(DiskImages {
            tree,
            input,
            workspace,
        })
    }
}

#[derive(Debug)]
pub struct DiskImages {
    tree: TempDir,
    input: PathBuf,
    workspace: PathBuf,
}

impl DiskImages {
    #[must_use]
    pub fn input(&self) -> &Path {
        &self.input
    }

    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        self.tree.path()
    }
}

fn input_image_size(bytes: u64, entries: usize) -> Result<u64, ImageError> {
    let entries = u64::try_from(entries).map_err(|_| ImageError::InputTooLarge)?;
    // Leave room for ext4 metadata, directory blocks, and a conservative
    // per-entry reserve. The image remains sparse until the formatter writes.
    let required = bytes
        .checked_add(
            entries
                .checked_mul(16 * 1024)
                .ok_or(ImageError::InputTooLarge)?,
        )
        .and_then(|sum| sum.checked_add(MIN_INPUT_BYTES))
        .ok_or(ImageError::InputTooLarge)?;
    let rounded = required
        .checked_add(MIB - 1)
        .ok_or(ImageError::InputTooLarge)?
        / MIB
        * MIB;
    let size = rounded.max(MIN_INPUT_BYTES);
    if size > MAX_INPUT_BYTES {
        return Err(ImageError::InputTooLarge);
    }
    Ok(size)
}

fn create_sparse_file(path: &Path, size: u64) -> io::Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.set_len(size)?;
    file.sync_all()?;
    Ok(())
}

fn format_ext4(
    formatter: &Path,
    image: &Path,
    label: &str,
    source: Option<&Path>,
) -> Result<(), ImageError> {
    let mut command = Command::new(formatter);
    command
        .arg("-F")
        .arg("-q")
        .arg("-t")
        .arg("ext4")
        .arg("-L")
        .arg(label);
    if let Some(source) = source {
        command.arg("-d").arg(source);
    }
    // Paths are separate arguments; no shell interprets user-controlled names.
    command.arg(image);
    if !command.status()?.success() {
        return Err(ImageError::FormatFailed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_bounded_and_rounded() {
        assert_eq!(input_image_size(0, 0).unwrap(), 64 * MIB);
        assert_eq!(input_image_size(1, 1).unwrap(), 65 * MIB);
        assert!(input_image_size(u64::MAX, 1).is_err());
        assert!(input_image_size(0, usize::MAX).is_err());
    }

    #[test]
    fn rejects_invalid_configuration() {
        assert!(Ext4Builder::new("mkfs.ext4", 64 * MIB).is_err());
        assert!(Ext4Builder::new("/usr/sbin/mkfs.ext4", 1).is_err());
        assert!(Ext4Builder::new("/usr/sbin/mkfs.ext4", 64 * MIB).is_ok());
    }

    #[test]
    fn rejects_public_scratch() {
        let scratch = tempfile::tempdir().unwrap();
        fs::set_permissions(scratch.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(require_private_directory(scratch.path()).is_err());
    }
}
