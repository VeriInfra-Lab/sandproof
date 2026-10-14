use super::{Budget, FileError, FileFailure};
use crate::file_download::LinuxErrno;
use crate::file_protocol::{FaultContext, FaultStage};
use rustix::fd::{AsFd, BorrowedFd};
use rustix::fs::{AtFlags, Dir, DirEntry, Mode, OFlags, openat, readlinkat_raw, unlinkat};
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
    first_error: Option<FileFailure>,
}

fn internal<T>(result: Result<T, Errno>) -> Result<T, FileFailure> {
    result.map_err(|_| FileError::Internal.into())
}

fn observed(stage: FaultStage, name: &OsStr, errno: Errno) -> FileFailure {
    FileFailure::observed(
        FileError::Internal,
        stage,
        Path::new(name),
        None,
        Some(LinuxErrno(errno.raw_os_error())),
    )
}

// Prefix only a captured error, without normalizing separators or re-reading
// paths. Check before allocation: diagnostic exhaustion retains partial effects.
fn prepend(mut error: FileFailure, parent: &OsStr, budget: &Budget) -> FileFailure {
    if let Some(fault) = &mut error.fault {
        let parent = parent.as_bytes();
        let Some(size) = parent
            .len()
            .checked_add(1)
            .and_then(|n| n.checked_add(fault.path.len()))
        else {
            return FileError::ResourceExhausted.into();
        };
        if size > budget.limits.path_bytes {
            return FileError::ResourceExhausted.into();
        }
        let mut path = Vec::with_capacity(size);
        path.extend_from_slice(parent);
        path.push(b'/');
        path.extend_from_slice(&fault.path);
        fault.path = path;
    }
    error
}

impl Frame {
    fn failed_child(&mut self, error: FileFailure, budget: &Budget) {
        self.batch_failures += 1;
        if self.first_error.is_none() {
            self.first_error = Some(prepend(error, &self.name, budget));
        }
    }
}

fn enter(parent: BorrowedFd<'_>, name: &OsStr) -> Result<Option<Frame>, FileFailure> {
    let unlink_error = match unlinkat(parent, name, AtFlags::empty()) {
        Ok(()) | Err(Errno::NOENT) => return Ok(None),
        Err(error @ (Errno::ISDIR | Errno::PERM | Errno::ACCESS)) => error,
        Err(error) => return Err(observed(FaultStage::RemoveUnlink, name, error)),
    };
    let fd = match openat(parent, name, OPEN_DIR, Mode::empty()) {
        Ok(fd) => fd,
        Err(Errno::NOENT) => return Ok(None),
        Err(mut error) => {
            // Go rootOpenDir distinguishes a replacement link from an ordinary
            // non-directory. Only link existence matters here, not its target.
            if matches!(error, Errno::NOTDIR | Errno::LOOP)
                && readlinkat_raw(parent, name, &mut [0_u8; 1]).is_ok()
            {
                error = unlink_error;
            } else if matches!(error, Errno::NOTDIR | Errno::OPNOTSUPP) {
                return Err(observed(FaultStage::RemoveUnlink, name, unlink_error));
            }
            // Even an unreadable empty child can be removed by its parent.
            return match unlinkat(parent, name, AtFlags::REMOVEDIR) {
                Ok(()) | Err(Errno::NOENT) => Ok(None),
                Err(_) => Err(observed(FaultStage::RemoveOpenDir, name, error)),
            };
        }
    };
    Ok(Some(Frame {
        directory: internal(Dir::new(fd))?,
        name: name.to_owned(),
        read: 0,
        batch_failures: 0,
        first_error: None,
    }))
}

fn split_path(mut bytes: &[u8]) -> (&OsStr, &OsStr) {
    while bytes.starts_with(b"//") {
        bytes = &bytes[1..];
    }
    let end = bytes.iter().rposition(|b| *b != b'/').map_or(1, |n| n + 1);
    let bytes = &bytes[..end];
    if bytes == b"/" {
        (OsStr::new("."), OsStr::new("/"))
    } else {
        let at = bytes.iter().rposition(|b| *b == b'/').unwrap_or(0);
        (
            OsStr::from_bytes(&bytes[..at.max(1)]),
            OsStr::from_bytes(&bytes[at + 1..]),
        )
    }
}

pub(super) fn remove(path: &Path, budget: &mut Budget) -> Result<(), FileFailure> {
    let bytes = path.as_os_str().as_bytes();
    // Go rejects a literal final '.', before any unlink; '/./' is different.
    if bytes.ends_with(b"/.") {
        return Err(observed(
            FaultStage::RemoveAll,
            path.as_os_str(),
            Errno::INVAL,
        ));
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
    let (parent_name, name) = split_path(bytes);
    // Go OpenFile(O_RDONLY) also opens a regular-file parent; unlinkat then
    // reports ENOTDIR. O_DIRECTORY here would change the observed operation.
    let parent =
        match rustix::fs::open(parent_name, OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty()) {
            Ok(fd) => fd,
            Err(Errno::NOENT) => return Ok(()),
            Err(error) => return Err(observed(FaultStage::RemoveParentOpen, parent_name, error)),
        };
    let result = enter(parent.as_fd(), name).and_then(|root| match root {
        Some(root) => walk(parent.as_fd(), root, budget),
        None => Ok(()),
    });
    result.map_err(|error| prepend(error, parent_name, budget))
}

fn walk(parent: BorrowedFd<'_>, root: Frame, budget: &mut Budget) -> Result<(), FileFailure> {
    walk_with(parent, root, budget, &mut enter, &mut Dir::next)
}

fn walk_with(
    parent: BorrowedFd<'_>,
    root: Frame,
    budget: &mut Budget,
    enter_child: &mut impl FnMut(BorrowedFd<'_>, &OsStr) -> Result<Option<Frame>, FileFailure>,
    read: &mut impl FnMut(&mut Dir) -> Option<Result<DirEntry, Errno>>,
) -> Result<(), FileFailure> {
    let mut stack = vec![root];
    while let Some(frame) = stack.last_mut() {
        // Re-open the same inode after each bounded batch: directory deletion
        // can reshuffle offsets. Never re-open a concurrently replaced pathname.
        if frame.read == 1024 {
            budget.visit()?;
            if frame.batch_failures != 1024 {
                // Adapter failures have no proven Go stage. Keep them coarse.
                frame.directory = internal(Dir::read_from(internal(frame.directory.fd())?))?;
            }
            frame.read = 0;
            frame.batch_failures = 0;
        }
        let outcome = match read(&mut frame.directory) {
            Some(Ok(child)) => {
                let name = child.file_name().to_bytes();
                if name == b"." || name == b".." {
                    continue;
                }
                budget.visit()?;
                budget.path(Path::new(OsStr::from_bytes(name)))?;
                frame.read += 1;
                match enter_child(internal(frame.directory.fd())?, OsStr::from_bytes(name)) {
                    Ok(Some(child)) => stack.push(child),
                    Ok(None) => {}
                    Err(error) => frame.failed_child(error, budget),
                }
                continue;
            }
            // A vanished directory at read time succeeds without another rmdir.
            Some(Err(Errno::NOENT)) => Ok(()),
            Some(Err(error)) => {
                let mut failure = observed(FaultStage::RemoveRead, &frame.name, error);
                failure.fault.as_mut().unwrap().context =
                    Some(FaultContext::RemoveReadName(frame.name.as_bytes().to_vec()));
                Err(failure)
            }
            None => {
                let finished = stack.last().ok_or(FileError::Internal)?;
                let parent_fd = if stack.len() > 1 {
                    internal(stack[stack.len() - 2].directory.fd())?
                } else {
                    parent.as_fd()
                };
                match unlinkat(parent_fd, &finished.name, AtFlags::REMOVEDIR) {
                    Ok(()) | Err(Errno::NOENT) => Ok(()),
                    Err(error) => {
                        let frame = stack.last_mut().ok_or(FileError::Internal)?;
                        Err(frame.first_error.take().unwrap_or_else(|| {
                            observed(FaultStage::RemoveUnlink, &frame.name, error)
                        }))
                    }
                }
            }
        };
        // Successful final removal overrides earlier child failures. Otherwise
        // keep the first recursive failure while still attempting siblings.
        stack.pop();
        if let Err(error) = outcome {
            if let Some(parent) = stack.last_mut() {
                parent.failed_child(error, budget);
            } else {
                return Err(error);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "remove_tests.rs"]
mod tests;
