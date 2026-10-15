//! Descriptor-based reads of trusted daemon installation/state inputs.
//! Root and the configured owner remain trusted. This does not protect against
//! root, same-owner mutation, mount replacement or rollback of a valid image.
use rustix::fs::{Mode, OFlags, open, openat};
use std::{
    fs::File,
    os::unix::{ffi::OsStrExt, fs::MetadataExt},
    path::{Component, Path},
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidPath,
    Untrusted,
    Io,
}

#[derive(Clone, Copy)]
enum Kind {
    Directory,
    Configuration,
    Worker,
}

/// The effective daemon UID, observed from the kernel rather than config text.
#[must_use]
pub fn current_owner() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// Open an existing private state directory; never create or repair it.
/// # Errors
/// Invalid path/budget, unsafe ancestry/owner/mode, symlink or I/O failure.
pub fn state_directory(path: &Path, owner: u32, maximum: usize) -> Result<File, Error> {
    trusted(path, owner, maximum, Kind::Directory)
}

/// Open a private, single-link regular configuration file without following
/// any symlink. The caller must bound reads and validate its actual contents.
/// # Errors
/// Invalid path/budget, unsafe ancestry/owner/mode, symlink or I/O failure.
pub fn configuration(path: &Path, owner: u32, maximum: usize) -> Result<File, Error> {
    trusted(path, owner, maximum, Kind::Configuration)
}

/// Observe a trusted executable regular file. This is not a binary digest or
/// proof that the later pathname-based exec will use the same bytes. Root and
/// owner replacement remain excluded; template versioning is still required.
/// # Errors
/// Invalid path/budget, unsafe ancestry/owner/mode, symlink or I/O failure.
pub fn worker(path: &Path, owner: u32, maximum: usize) -> Result<File, Error> {
    trusted(path, owner, maximum, Kind::Worker)
}

fn trusted(path: &Path, owner: u32, maximum: usize, kind: Kind) -> Result<File, Error> {
    let bytes = path.as_os_str().as_bytes();
    if maximum == 0
        || maximum > isize::MAX as usize / 16
        || owner == u32::MAX
        || !path.is_absolute()
        || bytes.len() > maximum
        || bytes.contains(&0)
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        return Err(Error::InvalidPath);
    }
    let mut components = path.components().peekable();
    if components.next() != Some(Component::RootDir) || components.peek().is_none() {
        return Err(Error::InvalidPath);
    }
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOFOLLOW;
    let directory =
        File::from(open("/", flags | OFlags::DIRECTORY, Mode::empty()).map_err(|_| Error::Io)?);
    walk(directory, components, owner, kind)
}

fn walk(
    mut directory: File,
    mut components: std::iter::Peekable<std::path::Components<'_>>,
    owner: u32,
    kind: Kind,
) -> Result<File, Error> {
    let flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOFOLLOW;
    check_ancestor(&directory, owner)?;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(Error::InvalidPath);
        };
        let last = components.peek().is_none();
        let directory_entry = !last || matches!(kind, Kind::Directory);
        let fd = openat(
            &directory,
            name,
            flags
                | if directory_entry {
                    OFlags::DIRECTORY
                } else {
                    OFlags::empty()
                },
            Mode::empty(),
        )
        .map_err(|_| Error::Io)?;
        let file = File::from(fd);
        if !last {
            check_ancestor(&file, owner)?;
            directory = file;
            continue;
        }
        let m = file.metadata().map_err(|_| Error::Io)?;
        if m.uid() != owner || m.nlink() == 0 {
            return Err(Error::Untrusted);
        }
        // All six group/other permission bits must be zero.
        let private = m.mode().trailing_zeros() >= 6;
        let valid = match kind {
            Kind::Directory => m.is_dir() && private,
            Kind::Configuration => m.is_file() && m.nlink() == 1 && private,
            Kind::Worker => {
                m.is_file() && m.nlink() == 1 && m.mode() & 0o022 == 0 && m.mode() & 0o111 != 0
            }
        };
        return if valid {
            Ok(file)
        } else {
            Err(Error::Untrusted)
        };
    }
    Err(Error::InvalidPath)
}

fn check_ancestor(file: &File, owner: u32) -> Result<(), Error> {
    let m = file.metadata().map_err(|_| Error::Io)?;
    if !m.is_dir() || (m.uid() != 0 && m.uid() != owner) || m.mode() & 0o022 != 0 || m.nlink() == 0
    {
        return Err(Error::Untrusted);
    }
    Ok(())
}
