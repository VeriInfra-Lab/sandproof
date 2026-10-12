//! Bounded observation of pre-provisioned service material, not a new quota.
use sandproof_controller_service::physical_storage::{PhysicalServiceData, PhysicalStorageProfile};
use std::{
    collections::BTreeSet,
    error::Error,
    fs,
    os::unix::fs::MetadataExt,
    path::PathBuf,
    time::{Duration, Instant},
};

type Result = std::result::Result<(), Box<dyn Error>>;
struct ObservedDirectory {
    path: PathBuf,
    identity: (u64, u64),
    stamp: (i64, i64, i64, i64),
}

pub(super) fn validate(profile: &PhysicalStorageProfile, device: u64, trusted: bool) -> Result {
    let deadline = Instant::now() + Duration::from_secs(10);
    for service in &profile.service_data {
        inventory(service, device, trusted, deadline)?;
    }
    Ok(())
}

fn exact(metadata: &fs::Metadata, service: &PhysicalServiceData, device: u64) -> Result {
    if ![metadata.is_dir(), metadata.is_file()].contains(&true)
        || metadata.uid() != service.owner_uid
        || metadata.dev() != device
        || metadata.mode() & 0o077 != 0
        || (metadata.is_file() && metadata.nlink() != 1)
    {
        return Err("service material must have its exact private owner/filesystem/type".into());
    }
    Ok(())
}

fn identity(metadata: &fs::Metadata) -> (u64, u64) {
    (metadata.dev(), metadata.ino())
}
fn directory_stamp(metadata: &fs::Metadata) -> (i64, i64, i64, i64) {
    (
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

fn inventory(
    service: &PhysicalServiceData,
    device: u64,
    trusted: bool,
    deadline: Instant,
) -> Result {
    if !fs::symlink_metadata(&service.root)?.is_dir() {
        return Err("service material root must be a registered directory".into());
    }
    let mut pending = vec![service.root.clone()];
    let mut observed = BTreeSet::new();
    let mut directories = Vec::new();
    let mut bytes = 0u64;
    while let Some(path) = pending.pop() {
        if Instant::now() >= deadline {
            return Err("service material observation deadline exceeded".into());
        }
        if trusted {
            sandbox_runtime::check_trusted_path_for_owner(&path, service.owner_uid)?;
        }
        let before = fs::symlink_metadata(&path)?;
        exact(&before, service, device)?;
        if !observed.insert(identity(&before)) || observed.len() as u64 > service.inode_budget {
            return Err("service material aliases or exceeds its complete inode provision".into());
        }
        if before.is_dir() {
            for entry in fs::read_dir(&path)? {
                if (observed.len() + pending.len()) as u64 >= service.inode_budget {
                    return Err("service material exceeds its complete inode provision".into());
                }
                pending.push(entry?.path());
            }
            directories.push(ObservedDirectory {
                path,
                identity: identity(&before),
                stamp: directory_stamp(&before),
            });
        } else {
            let file = fs::OpenOptions::new()
                .read(true)
                .custom_flags(i32::try_from(
                    (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits(),
                )?)
                .open(&path)?;
            let opened = file.metadata()?;
            let current = fs::symlink_metadata(&path)?;
            exact(&opened, service, device)?;
            exact(&current, service, device)?;
            if identity(&before) != identity(&opened) || identity(&opened) != identity(&current) {
                return Err("service material file identity changed during observation".into());
            }
            bytes = bytes
                .checked_add(before.len().max(opened.len()).max(current.len()))
                .ok_or("service material byte count overflow")?;
            if bytes > service.peak_bytes {
                return Err("service material exceeds its complete logical byte provision".into());
            }
        }
    }
    for directory in directories {
        if Instant::now() >= deadline {
            return Err("service material observation deadline exceeded".into());
        }
        let current = fs::symlink_metadata(directory.path)?;
        exact(&current, service, device)?;
        if identity(&current) != directory.identity || directory_stamp(&current) != directory.stamp
        {
            return Err("service material directory changed during observation".into());
        }
    }
    Ok(())
}

use std::os::unix::fs::OpenOptionsExt;
