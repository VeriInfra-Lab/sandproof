//! One-time, read-only import of explicitly granted host folders.
//!
//! The caller supplies an already-open, trusted grant root. Every lookup below
//! it uses a directory file descriptor and refuses symlinks. Linux additionally
//! uses `openat2` to refuse all mount crossings, including bind mounts.
//! The grant root must not be modified concurrently while an import runs; a
//! durable immutable source or a caller-held filesystem snapshot is preferred.

#[cfg(not(target_os = "linux"))]
use rustix::fs::openat;
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, Stat, fstat, statat};
#[cfg(target_os = "linux")]
use rustix::fs::{ResolveFlags, openat2};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use tempfile::{Builder, TempDir};

/// A folder below a trusted root and its name inside the guest input image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderGrant {
    source: PathBuf,
    guest_name: String,
}

impl FolderGrant {
    /// Paths must be relative and contain only ordinary, nonempty components.
    ///
    /// # Errors
    ///
    /// Returns `InvalidGrant` for an absolute, escaping, or invalid path/name.
    pub fn new(
        source: impl Into<PathBuf>,
        guest_name: impl Into<String>,
    ) -> Result<Self, ImportError> {
        let source = source.into();
        let guest_name = guest_name.into();
        if source.as_os_str().is_empty()
            || source
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || !valid_guest_name(&guest_name)
        {
            return Err(ImportError::InvalidGrant);
        }
        Ok(Self { source, guest_name })
    }

    #[must_use]
    pub fn source(&self) -> &Path {
        &self.source
    }

    #[must_use]
    pub fn guest_name(&self) -> &str {
        &self.guest_name
    }
}

fn valid_guest_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Resource limits are enforced while copying, not just from source metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportLimits {
    pub max_grants: usize,
    pub max_entries: usize,
    pub max_bytes: u64,
    pub max_depth: usize,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            max_grants: 8,
            max_entries: 20_000,
            max_bytes: 512 * 1024 * 1024,
            max_depth: 32,
        }
    }
}

#[derive(Debug)]
pub enum ImportError {
    InvalidGrant,
    DuplicateOrOverlappingGrant,
    UnsupportedEntry(PathBuf),
    CrossesMount(PathBuf),
    HardLink(PathBuf),
    LimitExceeded,
    Io(io::Error),
}

impl From<io::Error> for ImportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGrant => write!(formatter, "invalid folder grant"),
            Self::DuplicateOrOverlappingGrant => {
                write!(formatter, "duplicate or overlapping folder grant")
            }
            Self::UnsupportedEntry(path) => {
                write!(formatter, "unsupported source entry: {}", path.display())
            }
            Self::CrossesMount(path) => write!(
                formatter,
                "source crosses a mount point: {}",
                path.display()
            ),
            Self::HardLink(path) => {
                write!(formatter, "hard-linked source file: {}", path.display())
            }
            Self::LimitExceeded => write!(formatter, "folder import limit exceeded"),
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// The private temporary tree is deleted on drop unless consumed by an image builder.
#[derive(Debug)]
pub struct StagedInputs {
    tree: TempDir,
    entries: usize,
    bytes: u64,
}

impl StagedInputs {
    #[must_use]
    pub fn path(&self) -> &Path {
        self.tree.path()
    }

    #[must_use]
    pub fn entries(&self) -> usize {
        self.entries
    }

    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

struct Budget {
    entries: usize,
    bytes: u64,
    limits: ImportLimits,
}

impl Budget {
    fn entry(&mut self) -> Result<(), ImportError> {
        self.entries = self
            .entries
            .checked_add(1)
            .ok_or(ImportError::LimitExceeded)?;
        if self.entries > self.limits.max_entries {
            return Err(ImportError::LimitExceeded);
        }
        Ok(())
    }

    fn bytes(&mut self, amount: usize) -> Result<(), ImportError> {
        self.bytes = self
            .bytes
            .checked_add(amount as u64)
            .ok_or(ImportError::LimitExceeded)?;
        if self.bytes > self.limits.max_bytes {
            return Err(ImportError::LimitExceeded);
        }
        Ok(())
    }
}

/// Copy only selected directories into a new, private staging tree.
///
/// `scratch` must be an operator-owned directory inaccessible to tenants.
/// The resulting tree is never a live mount of the original host directories.
///
/// # Errors
///
/// Rejects invalid grants, links, special files, mount crossings, hard links,
/// resource-limit violations, or an underlying I/O failure.
pub fn stage_inputs(
    grant_root: &File,
    grants: &[FolderGrant],
    scratch: &Path,
    limits: ImportLimits,
) -> Result<StagedInputs, ImportError> {
    validate_grants(grants, limits.max_grants)?;
    require_private_directory(scratch)?;
    let root_stat = fstat(grant_root).map_err(io::Error::from)?;
    if FileType::from_raw_mode(root_stat.st_mode) != FileType::Directory {
        return Err(ImportError::InvalidGrant);
    }
    let tree = Builder::new()
        .prefix("sandbox-input-")
        .tempdir_in(scratch)?;
    fs::set_permissions(tree.path(), fs::Permissions::from_mode(0o700))?;
    let mut budget = Budget {
        entries: 0,
        bytes: 0,
        limits,
    };
    for grant in grants {
        let source = open_granted_directory(grant_root, grant.source(), &root_stat)?;
        let target = tree.path().join(grant.guest_name());
        fs::DirBuilder::new().mode(0o755).create(&target)?;
        budget.entry()?;
        copy_directory(&source, &target, grant.source(), &root_stat, 1, &mut budget)?;
    }
    Ok(StagedInputs {
        tree,
        entries: budget.entries,
        bytes: budget.bytes,
    })
}

pub(crate) fn require_private_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime directory must be a private directory, not a link",
        ));
    }
    Ok(())
}

fn validate_grants(grants: &[FolderGrant], max_grants: usize) -> Result<(), ImportError> {
    if grants.len() > max_grants {
        return Err(ImportError::LimitExceeded);
    }
    let mut names = HashSet::new();
    for grant in grants {
        if !names.insert(grant.guest_name.to_ascii_lowercase()) {
            return Err(ImportError::DuplicateOrOverlappingGrant);
        }
    }
    for (index, first) in grants.iter().enumerate() {
        for second in &grants[index + 1..] {
            if first.source.starts_with(&second.source) || second.source.starts_with(&first.source)
            {
                return Err(ImportError::DuplicateOrOverlappingGrant);
            }
        }
    }
    Ok(())
}

fn open_granted_directory(
    root: &File,
    relative: &Path,
    root_stat: &Stat,
) -> Result<File, ImportError> {
    let mut current = root.try_clone()?;
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(ImportError::InvalidGrant);
        };
        let next = open_child(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            relative,
        )?;
        let stat = fstat(&next).map_err(io::Error::from)?;
        if stat.st_dev != root_stat.st_dev {
            return Err(ImportError::CrossesMount(relative.to_path_buf()));
        }
        current = next;
    }
    Ok(current)
}

fn open_child(
    parent: &File,
    name: &OsStr,
    flags: OFlags,
    source_name: &Path,
) -> Result<File, ImportError> {
    #[cfg(target_os = "linux")]
    let result = openat2(
        parent,
        name,
        flags,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
    );
    #[cfg(not(target_os = "linux"))]
    let result = openat(parent, name, flags, Mode::empty());
    match result {
        Ok(fd) => Ok(File::from(fd)),
        Err(rustix::io::Errno::XDEV) => Err(ImportError::CrossesMount(source_name.to_path_buf())),
        Err(error) => Err(ImportError::Io(io::Error::from(error))),
    }
}

fn copy_directory(
    source: &File,
    target: &Path,
    source_name: &Path,
    root_stat: &Stat,
    depth: usize,
    budget: &mut Budget,
) -> Result<(), ImportError> {
    if depth > budget.limits.max_depth {
        return Err(ImportError::LimitExceeded);
    }
    let mut directory = Dir::read_from(source).map_err(io::Error::from)?;
    while let Some(entry) = directory.read() {
        let entry = entry.map_err(io::Error::from)?;
        let raw_name = entry.file_name().to_bytes();
        if raw_name == b"." || raw_name == b".." {
            continue;
        }
        let name: OsString = OsStr::from_bytes(raw_name).to_os_string();
        let child_source_name = source_name.join(&name);
        let child_target = target.join(&name);
        let before = statat(source, &name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from)?;
        if before.st_dev != root_stat.st_dev {
            return Err(ImportError::CrossesMount(child_source_name));
        }
        match FileType::from_raw_mode(before.st_mode) {
            FileType::Directory => {
                let child = open_child(
                    source,
                    &name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    &child_source_name,
                )?;
                let after = fstat(&child).map_err(io::Error::from)?;
                if after.st_dev != root_stat.st_dev {
                    return Err(ImportError::CrossesMount(child_source_name));
                }
                fs::DirBuilder::new().mode(0o755).create(&child_target)?;
                budget.entry()?;
                copy_directory(
                    &child,
                    &child_target,
                    &child_source_name,
                    root_stat,
                    depth + 1,
                    budget,
                )?;
            }
            FileType::RegularFile => {
                if before.st_nlink > 1 {
                    return Err(ImportError::HardLink(child_source_name));
                }
                let child = open_child(
                    source,
                    &name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    &child_source_name,
                )?;
                let after = fstat(&child).map_err(io::Error::from)?;
                if after.st_dev != root_stat.st_dev {
                    return Err(ImportError::CrossesMount(child_source_name));
                }
                if FileType::from_raw_mode(after.st_mode) != FileType::RegularFile
                    || after.st_nlink > 1
                {
                    return Err(ImportError::UnsupportedEntry(child_source_name));
                }
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o644)
                    .open(child_target)?;
                budget.entry()?;
                let mut input = child;
                let mut buffer = [0_u8; 8192];
                loop {
                    let count = input.read(&mut buffer)?;
                    if count == 0 {
                        break;
                    }
                    budget.bytes(count)?;
                    output.write_all(&buffer[..count])?;
                }
                output.sync_all()?;
            }
            _ => return Err(ImportError::UnsupportedEntry(child_source_name)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn root_and_scratch() -> (TempDir, TempDir) {
        let source = tempfile::tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        fs::set_permissions(scratch.path(), fs::Permissions::from_mode(0o700)).unwrap();
        (source, scratch)
    }

    #[test]
    fn only_explicit_folders_are_copied() {
        let (source, scratch) = root_and_scratch();
        fs::create_dir_all(source.path().join("project/src")).unwrap();
        fs::create_dir_all(source.path().join("project/secret")).unwrap();
        fs::write(source.path().join("project/src/main.rs"), b"allowed").unwrap();
        fs::write(source.path().join("project/secret/key"), b"hidden").unwrap();
        let root = File::open(source.path()).unwrap();
        let grants = [FolderGrant::new("project/src", "src").unwrap()];
        let staged = stage_inputs(&root, &grants, scratch.path(), ImportLimits::default()).unwrap();
        assert_eq!(
            fs::read(staged.path().join("src/main.rs")).unwrap(),
            b"allowed"
        );
        assert!(!staged.path().join("secret").exists());
        assert_eq!(staged.entries(), 2);
        assert_eq!(staged.bytes(), 7);
    }

    #[test]
    fn rejects_escape_and_duplicate_grants() {
        assert!(FolderGrant::new("../secret", "secret").is_err());
        assert!(FolderGrant::new("/etc", "etc").is_err());
        assert!(FolderGrant::new("src", "../out").is_err());
        let (source, scratch) = root_and_scratch();
        fs::create_dir(source.path().join("src")).unwrap();
        let root = File::open(source.path()).unwrap();
        let grants = [
            FolderGrant::new("src", "A").unwrap(),
            FolderGrant::new("src", "B").unwrap(),
        ];
        assert!(matches!(
            stage_inputs(&root, &grants, scratch.path(), ImportLimits::default()),
            Err(ImportError::DuplicateOrOverlappingGrant)
        ));
    }

    #[test]
    fn rejects_symlinks_and_special_files() {
        let (source, scratch) = root_and_scratch();
        fs::create_dir(source.path().join("src")).unwrap();
        symlink("/etc", source.path().join("src/link")).unwrap();
        let root = File::open(source.path()).unwrap();
        let grants = [FolderGrant::new("src", "src").unwrap()];
        assert!(matches!(
            stage_inputs(&root, &grants, scratch.path(), ImportLimits::default()),
            Err(ImportError::UnsupportedEntry(_))
        ));
    }

    #[test]
    fn rejects_symlink_in_grant_path() {
        let (source, scratch) = root_and_scratch();
        fs::create_dir(source.path().join("real")).unwrap();
        symlink("real", source.path().join("alias")).unwrap();
        let root = File::open(source.path()).unwrap();
        let grants = [FolderGrant::new("alias", "alias").unwrap()];
        assert!(stage_inputs(&root, &grants, scratch.path(), ImportLimits::default()).is_err());
    }

    #[test]
    fn rejects_hardlinks_and_symlinked_scratch() {
        let (source, scratch) = root_and_scratch();
        fs::create_dir(source.path().join("src")).unwrap();
        fs::write(source.path().join("elsewhere"), b"secret").unwrap();
        fs::hard_link(
            source.path().join("elsewhere"),
            source.path().join("src/alias"),
        )
        .unwrap();
        let root = File::open(source.path()).unwrap();
        let grants = [FolderGrant::new("src", "src").unwrap()];
        assert!(matches!(
            stage_inputs(&root, &grants, scratch.path(), ImportLimits::default()),
            Err(ImportError::HardLink(_))
        ));
        let link = source.path().join("scratch-link");
        symlink(scratch.path(), &link).unwrap();
        assert!(matches!(
            stage_inputs(&root, &[], &link, ImportLimits::default()),
            Err(ImportError::Io(_))
        ));
    }

    #[test]
    fn enforces_actual_copy_quota() {
        let (source, scratch) = root_and_scratch();
        fs::create_dir(source.path().join("src")).unwrap();
        fs::write(source.path().join("src/file"), b"12345").unwrap();
        let root = File::open(source.path()).unwrap();
        let grants = [FolderGrant::new("src", "src").unwrap()];
        let limits = ImportLimits {
            max_bytes: 4,
            ..ImportLimits::default()
        };
        assert!(matches!(
            stage_inputs(&root, &grants, scratch.path(), limits),
            Err(ImportError::LimitExceeded)
        ));
        assert_eq!(fs::read_dir(scratch.path()).unwrap().count(), 0);
    }
}
