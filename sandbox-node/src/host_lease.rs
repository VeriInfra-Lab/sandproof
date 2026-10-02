//! Lifetime single-writer lock acquired before bounded node-state recovery.
use super::{NodeConfig, NodeResult, check_trusted_path};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LeaseIdentity {
    pub device: u64,
    pub inode: u64,
}

pub(super) struct HostLease {
    file: File,
    pub identity: LeaseIdentity,
    path: PathBuf,
}

impl Drop for HostLease {
    fn drop(&mut self) {
        // This controller-only lease is never deliberately handed to a child.
        // Explicit release also ends transient fork-before-exec references to
        // its CLOEXEC open description. Per-slot helper guards do NOT do this.
        let _ = self.file.unlock();
    }
}

impl HostLease {
    pub fn acquire(config: &NodeConfig, required: Option<&LeaseIdentity>) -> NodeResult<Self> {
        super::storage_envelope::validate_configuration(config)?;
        let path = config.state_dir.join("bounded-host.lock");
        let marker = serde_json::to_vec(&(1_u32, config))?;
        let lease = acquire(&path, &marker, required, 0)?;
        check_trusted_path(&path)?;
        Ok(lease)
    }
    pub fn validate(&self) -> NodeResult<()> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.nlink() != 1
            || metadata.uid() != 0
            || metadata.mode() & 0o077 != 0
            || metadata.dev() != self.identity.device
            || metadata.ino() != self.identity.inode
        {
            return Err("bounded host lease identity changed".into());
        }
        Ok(())
    }
}

fn acquire(
    path: &Path,
    marker: &[u8],
    required: Option<&LeaseIdentity>,
    owner: u32,
) -> NodeResult<HostLease> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).mode(0o600);
    let mut file = if required.is_some() {
        options.open(path)?
    } else {
        match options.create_new(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                OpenOptions::new().read(true).write(true).open(path)?
            }
            Err(error) => return Err(error.into()),
        }
    };
    // No content mutation or stage recovery precedes this acquisition.
    file.try_lock()?;
    let metadata = file.metadata()?;
    let current = fs::symlink_metadata(path)?;
    let identity = LeaseIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    if !metadata.is_file()
        || current.file_type().is_symlink()
        || metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() > marker.len() as u64
        || (metadata.dev(), metadata.ino()) != (current.dev(), current.ino())
        || required.is_some_and(|required| *required != identity)
    {
        return Err("untrusted or replaced bounded host lease".into());
    }
    let content = fs::read(path)?;
    if !marker.starts_with(&content) || (required.is_some() && content != marker) {
        return Err("bounded host lease mapping changed".into());
    }
    if content != marker {
        file.seek(SeekFrom::Start(0))?;
        file.write_all(marker)?;
    }
    file.sync_all()?;
    File::open(path.parent().ok_or("host lease parent")?)?.sync_all()?;
    Ok(HostLease {
        file,
        identity,
        path: path.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> u32 {
        rustix::process::geteuid().as_raw()
    }
    #[test]
    fn independent_second_open_cannot_recover_or_mutate_live_host() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lease");
        let first = acquire(&path, b"mapping", None, owner()).unwrap();
        assert!(acquire(&path, b"mapping", None, owner()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"mapping");
        let identity = first.identity.clone();
        drop(first);
        let reopened = acquire(&path, b"mapping", Some(&identity), owner()).unwrap();
        assert_eq!(reopened.identity, identity);
    }
    #[test]
    fn initialized_missing_or_replaced_lease_never_resets_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lease");
        let lease = acquire(&path, b"mapping", None, owner()).unwrap();
        let identity = lease.identity.clone();
        drop(lease);
        fs::rename(&path, dir.path().join("old")).unwrap();
        assert!(acquire(&path, b"mapping", Some(&identity), owner()).is_err());
        let replacement = acquire(&path, b"mapping", None, owner()).unwrap();
        drop(replacement);
        assert!(acquire(&path, b"mapping", Some(&identity), owner()).is_err());
    }
    #[test]
    fn interrupted_initial_lease_marker_resumes_only_exact_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lease");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        drop(file);
        let lease = acquire(&path, b"mapping", None, owner()).unwrap();
        drop(lease);
        assert!(acquire(&path, b"different", None, owner()).is_err());
    }
    #[test]
    fn controller_lease_drop_ends_transient_cloexec_copies() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lease");
        let lease = acquire(&path, b"mapping", None, owner()).unwrap();
        let identity = lease.identity.clone();
        // Model the short-lived open-description reference between an
        // unrelated command's fork and exec without a scheduler race.
        let inherited = lease.file.try_clone().unwrap();
        drop(lease);
        let next = acquire(&path, b"mapping", Some(&identity), owner()).unwrap();
        assert_eq!(next.identity, identity);
        drop(inherited);
        assert!(acquire(&path, b"mapping", Some(&identity), owner()).is_err());
    }
}
