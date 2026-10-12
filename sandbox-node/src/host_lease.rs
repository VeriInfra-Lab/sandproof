//! Lifetime single-writer lock acquired before bounded node-state recovery.
use super::{NodeConfig, NodeResult, check_trusted_path};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
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
    pub fn inspect_existing(config: &NodeConfig, required: &LeaseIdentity) -> NodeResult<Self> {
        super::storage_envelope::validate_configuration(config)?;
        let path = config.state_dir.join("bounded-host.lock");
        let marker = serde_json::to_vec(&(1_u32, config))?;
        let lease = inspect_existing(&path, &marker, required, 0)?;
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
    let identity = verify_open_file(&file, path, marker, required, owner)?;
    let content = read_marker(&mut file, marker.len())?;
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

// This descriptor has no write capability. A failed audit cannot create a
// marker, complete an initialization prefix, fsync or recover staged records.
fn inspect_existing(
    path: &Path,
    marker: &[u8],
    required: &LeaseIdentity,
    owner: u32,
) -> NodeResult<HostLease> {
    let flags = rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(i32::try_from(flags.bits())?)
        .open(path)?;
    file.try_lock()?;
    let identity = verify_open_file(&file, path, marker, Some(required), owner)?;
    if read_marker(&mut file, marker.len())? != marker {
        return Err("bounded host lease mapping changed".into());
    }
    Ok(HostLease {
        file,
        identity,
        path: path.to_owned(),
    })
}

fn read_marker(file: &mut File, limit: usize) -> NodeResult<Vec<u8>> {
    let mut bytes = Vec::new();
    file.seek(SeekFrom::Start(0))?;
    file.take(
        u64::try_from(limit)?
            .checked_add(1)
            .ok_or("lease marker limit")?,
    )
    .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn verify_open_file(
    file: &File,
    path: &Path,
    marker: &[u8],
    required: Option<&LeaseIdentity>,
    owner: u32,
) -> NodeResult<LeaseIdentity> {
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
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn owner() -> u32 {
        rustix::process::geteuid().as_raw()
    }
    #[test]
    fn readonly_inspection_keeps_exact_identity_and_has_no_write_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lease");
        let first = acquire(&path, b"mapping", None, owner()).unwrap();
        let identity = first.identity.clone();
        assert!(inspect_existing(&path, b"mapping", &identity, owner()).is_err());
        drop(first);
        let before = fs::metadata(&path).unwrap();
        let mut inspection = inspect_existing(&path, b"mapping", &identity, owner()).unwrap();
        assert_eq!(inspection.identity, identity);
        assert!(inspection.file.write_all(b"cannot-write").is_err());
        let after = fs::metadata(&path).unwrap();
        assert_eq!(
            (before.dev(), before.ino(), before.mtime(), before.ctime()),
            (after.dev(), after.ino(), after.mtime(), after.ctime())
        );
        assert_eq!(fs::read(&path).unwrap(), b"mapping");
    }
    #[test]
    fn readonly_inspection_never_creates_or_repairs_a_missing_or_partial_marker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lease");
        let dummy = LeaseIdentity {
            device: 0,
            inode: 0,
        };
        assert!(inspect_existing(&path, b"mapping", &dummy, owner()).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        let first = acquire(&path, b"mapping", None, owner()).unwrap();
        let identity = first.identity.clone();
        drop(first);
        fs::write(&path, b"map").unwrap();
        assert!(inspect_existing(&path, b"mapping", &identity, owner()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"map");
        fs::write(&path, b"invalid").unwrap();
        assert!(inspect_existing(&path, b"mapping", &identity, owner()).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"invalid");
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
