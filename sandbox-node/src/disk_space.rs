//! Read-only filesystem observation shared by activation and admission.
use std::{error::Error, fs, os::unix::fs::MetadataExt, path::Path};

#[derive(Clone, Copy, Debug)]
pub struct DiskSpace {
    pub device: u64,
    pub available_bytes: u64,
    pub available_inodes: u64,
    pub fragment_bytes: u64,
}

/// # Errors
/// Fails closed on inaccessible filesystems or unrepresentable geometry.
pub fn disk_space(path: &Path) -> Result<DiskSpace, Box<dyn Error>> {
    let value = rustix::fs::statvfs(path)?;
    if value.f_frsize == 0 {
        return Err("invalid filesystem fragment size".into());
    }
    Ok(DiskSpace {
        device: fs::metadata(path)?.dev(),
        available_bytes: value
            .f_bavail
            .checked_mul(value.f_frsize)
            .ok_or("filesystem byte overflow")?,
        available_inodes: value.f_favail,
        fragment_bytes: value.f_frsize,
    })
}
