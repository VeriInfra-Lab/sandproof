//! Shared deployment-file primitive; no implicit config or credential lookup.
//! Both entrypoints provide an absolute path, their protected owner and a bound.

use rustix::fs::{Mode, OFlags, open, openat};
use std::{
    fs::{File, Metadata},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Component, Path},
};

const MAX_PROTECTED_FILE_BYTES: usize = 16 * 1024 * 1024;

/// Fixed errors deliberately omit paths, key bytes and operating-system detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtectedFileError {
    Invalid,
    Unavailable,
    Untrusted,
    TooLarge,
}

impl std::fmt::Display for ProtectedFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid protected file configuration",
            Self::Unavailable => "protected file unavailable",
            Self::Untrusted => "protected file ownership or permissions rejected",
            Self::TooLarge => "protected file exceeds its byte budget",
        })
    }
}
impl std::error::Error for ProtectedFileError {}

fn directory(metadata: &Metadata, owner: u32) -> Result<(), ProtectedFileError> {
    let trusted_owner = metadata.uid() == 0 || metadata.uid() == owner;
    // A root-owned sticky temporary ancestor cannot replace another owner's
    // private child. The final directory/leaf must still be owner-only below.
    let trusted_writes =
        metadata.mode() & 0o022 == 0 || (metadata.uid() == 0 && metadata.mode() & 0o1000 != 0);
    if !metadata.is_dir() || !trusted_owner || !trusted_writes {
        return Err(ProtectedFileError::Untrusted);
    }
    Ok(())
}

fn open_checked(path: &Path, owner: u32, leaf_directory: bool) -> Result<File, ProtectedFileError> {
    if !path.is_absolute() || path.as_os_str().len() > 4096 {
        return Err(ProtectedFileError::Invalid);
    }
    let names: Vec<_> = path
        .components()
        .skip(1)
        .map(|part| match part {
            Component::Normal(name) => Ok(name),
            _ => Err(ProtectedFileError::Invalid),
        })
        .collect::<Result<_, _>>()?;
    if names.is_empty() || names.len() > 128 {
        return Err(ProtectedFileError::Invalid);
    }
    // Directory handles are only traversed and inspected, never read. Linux
    // O_PATH preserves search-only root-owned ancestors such as mode 0711;
    // DIRECTORY and NOFOLLOW still reject symlinks before the metadata checks.
    #[cfg(target_os = "linux")]
    let directory_access = OFlags::PATH;
    #[cfg(not(target_os = "linux"))]
    let directory_access = OFlags::RDONLY;
    let dir_flags = directory_access | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut current = File::from(
        open("/", dir_flags, Mode::empty()).map_err(|_| ProtectedFileError::Unavailable)?,
    );
    directory(
        &current
            .metadata()
            .map_err(|_| ProtectedFileError::Unavailable)?,
        owner,
    )?;
    for (index, name) in names.iter().enumerate() {
        let last = index + 1 == names.len();
        let flags = if !last || leaf_directory {
            dir_flags
        } else {
            // A FIFO must not block before we can reject its file type.
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK
        };
        current = File::from(
            openat(&current, *name, flags, Mode::empty())
                .map_err(|_| ProtectedFileError::Unavailable)?,
        );
        let metadata = current
            .metadata()
            .map_err(|_| ProtectedFileError::Unavailable)?;
        if !last || leaf_directory {
            directory(&metadata, owner)?;
        }
        if last
            && (metadata.uid() != owner
                || metadata.mode() & 0o077 != 0
                || (!leaf_directory && (!metadata.is_file() || metadata.nlink() != 1)))
        {
            return Err(ProtectedFileError::Untrusted);
        }
    }
    Ok(current)
}

/// Check an existing private registration/coordination directory without
/// following a symlink in any component or creating/replacing the directory.
/// # Errors
/// Rejects untrusted ownership, writeable ancestors, non-private final modes.
pub fn check_protected_directory(path: &Path, owner_uid: u32) -> Result<(), ProtectedFileError> {
    open_checked(path, owner_uid, true).map(drop)
}

/// Read one bounded existing regular file through checked directory handles.
/// This does not canonicalize an untrusted path or reopen the leaf after check.
/// The explicit owner comes from the service account, not from the file itself.
/// # Errors
/// Rejects symlinks, extra hard links, other owners, permissive modes or oversize.
pub fn read_protected(
    path: &Path,
    owner_uid: u32,
    max_bytes: usize,
) -> Result<Vec<u8>, ProtectedFileError> {
    if !(1..=MAX_PROTECTED_FILE_BYTES).contains(&max_bytes) {
        return Err(ProtectedFileError::Invalid);
    }
    let file = open_checked(path, owner_uid, false)?;
    if file
        .metadata()
        .map_err(|_| ProtectedFileError::Unavailable)?
        .len()
        > max_bytes as u64
    {
        return Err(ProtectedFileError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ProtectedFileError::Unavailable)?;
    if bytes.len() > max_bytes {
        return Err(ProtectedFileError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };

    fn fixture() -> (tempfile::TempDir, u32) {
        // The test intentionally chooses a real path, not the production loader
        // silently canonicalizing a user-selected alias or symlink.
        let parent = std::env::temp_dir().canonicalize().unwrap();
        let dir = tempfile::tempdir_in(parent).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let owner = fs::metadata(dir.path()).unwrap().uid();
        (dir, owner)
    }
    fn secret(path: &Path) {
        fs::write(path, b"fixture-only").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[cfg(target_os = "linux")]
    fn check_search_only_ancestor(path: &Path, owner: u32) {
        // Root bypasses directory DAC and therefore cannot prove this case.
        assert_ne!(
            owner, 0,
            "permission probe must run without root privileges"
        );
        assert_eq!(
            fs::read_dir(path).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        let child = path.join("private");
        assert_eq!(
            read_protected(&child.join("secret"), owner, 64).unwrap(),
            b"fixture-only"
        );
        check_protected_directory(&child, owner).unwrap();
        // Search permission does not relax the final directory's private mode.
        assert!(check_protected_directory(path, owner).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn search_only_ancestors_do_not_require_directory_read_permission() {
        use std::os::unix::{fs::chown, process::CommandExt};

        const CHILD_ROOT: &str = "SANDPROOF_PROTECTED_SEARCH_ONLY_TEST_ROOT";
        if let Some(path) = std::env::var_os(CHILD_ROOT) {
            let path = std::path::PathBuf::from(path);
            let owner = fs::metadata(path.join("private/secret")).unwrap().uid();
            check_search_only_ancestor(&path, owner);
            return;
        }
        let (dir, owner) = fixture();
        let ancestor = dir.path().join("search-only");
        let child = ancestor.join("private");
        fs::create_dir(&ancestor).unwrap();
        fs::create_dir(&child).unwrap();
        fs::set_permissions(&child, fs::Permissions::from_mode(0o700)).unwrap();
        secret(&child.join("secret"));
        if owner == 0 {
            const SERVICE_UID: u32 = 65534;
            chown(&child, Some(SERVICE_UID), Some(SERVICE_UID)).unwrap();
            chown(child.join("secret"), Some(SERVICE_UID), Some(SERVICE_UID)).unwrap();
            fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o711)).unwrap();
            fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o711)).unwrap();
            // The original test executable may live below a private root target.
            // Copy only this test binary into the scoped fixture for the real
            // unprivileged child; no source/configuration permission is changed.
            let binary = dir.path().join("permission-test");
            fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
            fs::set_permissions(&binary, fs::Permissions::from_mode(0o555)).unwrap();
            let status = std::process::Command::new(binary)
                .args([
                    "--exact",
                    "protected_files::tests::search_only_ancestors_do_not_require_directory_read_permission",
                    "--nocapture",
                ])
                .env(CHILD_ROOT, &ancestor)
                .uid(SERVICE_UID)
                .gid(SERVICE_UID)
                .status()
                .unwrap();
            assert!(status.success(), "unprivileged directory probe failed");
        } else {
            // A user's own 0711 directory remains readable by that owner. Mode
            // 0111 models the same search-only DAC without requiring chown.
            fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o111)).unwrap();
            let result = std::panic::catch_unwind(|| check_search_only_ancestor(&ancestor, owner));
            fs::set_permissions(&ancestor, fs::Permissions::from_mode(0o700)).unwrap();
            result.unwrap();
        }
    }

    #[test]
    fn existing_private_file_is_bounded_and_owner_selected() {
        let (dir, owner) = fixture();
        let path = dir.path().join("secret");
        secret(&path);
        assert_eq!(read_protected(&path, owner, 12).unwrap(), b"fixture-only");
        assert_eq!(
            read_protected(&path, owner, 11),
            Err(ProtectedFileError::TooLarge)
        );
        assert_eq!(
            read_protected(&path, owner, 0),
            Err(ProtectedFileError::Invalid)
        );
        assert!(read_protected(&path, owner.checked_add(1).unwrap(), 64).is_err());
        check_protected_directory(dir.path(), owner).unwrap();
        assert!(read_protected(dir.path(), owner, 64).is_err());
    }

    #[test]
    fn symlink_leaf_or_ancestor_and_traversal_are_rejected() {
        let (dir, owner) = fixture();
        let child = dir.path().join("private");
        fs::create_dir(&child).unwrap();
        fs::set_permissions(&child, fs::Permissions::from_mode(0o700)).unwrap();
        secret(&child.join("secret"));
        symlink(child.join("secret"), dir.path().join("alias")).unwrap();
        symlink(&child, dir.path().join("alias-dir")).unwrap();
        assert!(read_protected(&dir.path().join("alias"), owner, 64).is_err());
        assert!(read_protected(&dir.path().join("alias-dir/secret"), owner, 64).is_err());
        assert!(read_protected(&child.join("../private/secret"), owner, 64).is_err());
        assert!(read_protected(Path::new("relative"), owner, 64).is_err());
    }

    #[test]
    fn permissions_hardlinks_and_untrusted_parent_writes_are_rejected() {
        let (dir, owner) = fixture();
        let path = dir.path().join("secret");
        secret(&path);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            read_protected(&path, owner, 64),
            Err(ProtectedFileError::Untrusted)
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::hard_link(&path, dir.path().join("other")).unwrap();
        assert_eq!(
            read_protected(&path, owner, 64),
            Err(ProtectedFileError::Untrusted)
        );
        fs::remove_file(dir.path().join("other")).unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            read_protected(&path, owner, 64),
            Err(ProtectedFileError::Untrusted)
        );
    }
}
