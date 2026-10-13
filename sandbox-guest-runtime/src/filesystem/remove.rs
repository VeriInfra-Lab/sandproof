use super::{Budget, FileError};
use rustix::fd::{AsFd, BorrowedFd};
use rustix::fs::{AtFlags, Dir, Mode, OFlags, openat, unlinkat};
use rustix::io::Errno;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const OPEN_DIR: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

struct Frame {
    directory: Dir,
    name: OsString,
    read: usize,
    batch_failures: usize,
}

fn internal<T>(result: Result<T, Errno>) -> Result<T, FileError> {
    result.map_err(|_| FileError::Internal)
}

fn enter(parent: BorrowedFd<'_>, name: &OsStr) -> Result<Option<Frame>, FileError> {
    match unlinkat(parent, name, AtFlags::empty()) {
        Ok(()) | Err(Errno::NOENT) => return Ok(None),
        Err(Errno::ISDIR | Errno::PERM | Errno::ACCESS) => {}
        Err(_) => return Err(FileError::Internal),
    }
    let fd = match openat(parent, name, OPEN_DIR, Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(Errno::NOTDIR) => return Err(FileError::Internal),
        Err(_) => {
            // RemoveAll still tries rmdir when opening the directory fails.
            // Removing an empty child requires parent write/search access,
            // not read permission on that child (e.g. mode 000).
            return match unlinkat(parent, name, AtFlags::REMOVEDIR) {
                Ok(()) | Err(Errno::NOENT) => Ok(None),
                Err(_) => Err(FileError::Internal),
            };
        }
    };
    Ok(Some(Frame {
        directory: internal(Dir::new(fd))?,
        name: name.to_owned(),
        read: 0,
        batch_failures: 0,
    }))
}

pub(super) fn remove(path: &Path, budget: &mut Budget) -> Result<(), FileError> {
    let bytes = path.as_os_str().as_bytes();
    // Go rejects a literal final '.', before any unlink; '/./' is different.
    if bytes.ends_with(b"/.") {
        return Err(FileError::Internal);
    }
    budget.visit()?;
    match std::fs::remove_file(path) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {}
    }
    match std::fs::remove_dir(path) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => {}
    }
    let end = bytes.iter().rposition(|b| *b != b'/').map_or(1, |n| n + 1);
    let bytes = &bytes[..end];
    let at = bytes.iter().rposition(|b| *b == b'/').unwrap_or(0);
    let (parent, name) = if bytes == b"/" {
        (OsStr::new("."), OsStr::new("/"))
    } else {
        (
            OsStr::from_bytes(&bytes[..at.max(1)]),
            OsStr::from_bytes(&bytes[at + 1..]),
        )
    };
    let parent = match rustix::fs::open(
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(()),
        Err(_) => return Err(FileError::Internal),
    };
    let Some(root) = enter(parent.as_fd(), name)? else {
        return Ok(());
    };
    walk(parent.as_fd(), root, budget)
}

fn walk(parent: BorrowedFd<'_>, root: Frame, budget: &mut Budget) -> Result<(), FileError> {
    let mut stack = vec![root];
    while let Some(frame) = stack.last_mut() {
        // Re-open the same inode after each bounded batch: directory deletion
        // can reshuffle offsets. Never re-open a concurrently replaced pathname.
        if frame.read == 1024 {
            budget.visit()?;
            if frame.batch_failures != 1024 {
                frame.directory = internal(Dir::read_from(internal(frame.directory.fd())?))?;
            }
            frame.read = 0;
            frame.batch_failures = 0;
        }
        match frame.directory.next() {
            Some(Ok(child)) => {
                let name = child.file_name().to_bytes();
                if name == b"." || name == b".." {
                    continue;
                }
                budget.visit()?;
                budget.path(Path::new(OsStr::from_bytes(name)))?;
                frame.read += 1;
                match enter(internal(frame.directory.fd())?, OsStr::from_bytes(name)) {
                    Ok(Some(child)) => stack.push(child),
                    Ok(None) => {}
                    Err(_) => frame.batch_failures += 1,
                }
                continue;
            }
            Some(Err(error)) if error != Errno::NOENT => {
                stack.pop();
                if let Some(parent) = stack.last_mut() {
                    parent.batch_failures += 1;
                    continue;
                }
                return Err(FileError::Internal);
            }
            Some(Err(_)) | None => {}
        }
        let finished = stack.pop().ok_or(FileError::Internal)?;
        let parent_fd = match stack.last() {
            Some(p) => internal(p.directory.fd())?,
            None => parent.as_fd(),
        };
        let removed = matches!(
            unlinkat(parent_fd, &finished.name, AtFlags::REMOVEDIR),
            Ok(()) | Err(Errno::NOENT)
        );
        // As in RemoveAll, a vanished/removed directory wins over earlier
        // entry errors; otherwise siblings are attempted before failure.
        if !removed {
            if let Some(parent) = stack.last_mut() {
                parent.batch_failures += 1;
            } else {
                return Err(FileError::Internal);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::FileLimits;

    #[test]
    fn replacement_link_never_redirects_an_open_removal_walk() {
        let root = tempfile::tempdir().unwrap();
        let tree = root.path().join("tree");
        let displaced = root.path().join("displaced");
        let outside = root.path().join("outside");
        std::fs::create_dir(&tree).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(tree.join("old"), b"old").unwrap();
        std::fs::write(outside.join("keep"), b"safe").unwrap();
        let parent = rustix::fs::open(root.path(), OPEN_DIR, Mode::empty()).unwrap();
        let opened = enter(parent.as_fd(), OsStr::new("tree")).unwrap().unwrap();
        // Deterministic interleaving immediately after the real no-follow open.
        std::fs::rename(&tree, &displaced).unwrap();
        std::os::unix::fs::symlink(&outside, &tree).unwrap();
        let mut budget = Budget {
            limits: FileLimits {
                path_bytes: 4096,
                entries: 10,
                retained_bytes: 4096,
                visited: 100,
                xattr_bytes: 4096,
                account_bytes: 4096,
            },
            visited: 0,
            entries: 0,
            bytes: 0,
        };
        assert_eq!(
            walk(parent.as_fd(), opened, &mut budget),
            Err(FileError::Internal)
        );
        assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"safe");
        assert!(!displaced.join("old").exists());
        assert!(
            std::fs::symlink_metadata(&tree)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // An unopened child link is removed itself, without traversing target.
        assert!(enter(parent.as_fd(), OsStr::new("tree")).unwrap().is_none());
        assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"safe");
    }
}
