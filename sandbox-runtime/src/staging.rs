//! Fixed, bounded staging names for claim records. Only an exact prefix of
//! the caller's canonical intended bytes can be resumed after interruption.

use super::{Runtime, RuntimeError, check_trusted_path};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub const MAX_CLAIM_RECORD_BYTES: u64 = 1_048_576;
const STAGING_PREFIX: &str = ".staging-";
pub const CLAIM_RECORD_NAMES: &[&str] = &[
    "record.json",
    "jail.json",
    "aborted.json",
    "failed-prepare.json",
    "network-intent.json",
    "network.json",
    "cgroup-intent.json",
    "cgroup.json",
    "uplink-intent.json",
    "uplink-prepared.json",
    "uplink-aborted.json",
];

pub(super) fn claim_file_names() -> impl Iterator<Item = String> {
    std::iter::once("console.log".to_owned()).chain(
        CLAIM_RECORD_NAMES
            .iter()
            .flat_map(|name| [(*name).to_owned(), format!("{STAGING_PREFIX}{name}")]),
    )
}

pub(super) fn known_claim_file(name: &str) -> bool {
    name == "console.log"
        || CLAIM_RECORD_NAMES.contains(&name)
        || name
            .strip_prefix(STAGING_PREFIX)
            .is_some_and(|target| CLAIM_RECORD_NAMES.contains(&target))
}

fn stage_path(path: &Path) -> Result<PathBuf, RuntimeError> {
    let name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or(RuntimeError::InvalidId)?;
    if !CLAIM_RECORD_NAMES.contains(&name) {
        return Err(RuntimeError::IdentityMismatch(
            "unknown staged claim record target",
        ));
    }
    Ok(path.with_file_name(format!("{STAGING_PREFIX}{name}")))
}

impl Runtime {
    pub(crate) fn write_new_record<T: Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), RuntimeError> {
        if self.config.host_slot.is_some() {
            let parent = path.parent().ok_or(RuntimeError::InvalidId)?;
            check_trusted_path(parent)?;
            write_staged_record(path, value, 0)
        } else {
            super::write_new_record(path, value)
        }
    }
}

fn write_staged_record<T: Serialize>(
    path: &Path,
    value: &T,
    owner: u32,
) -> Result<(), RuntimeError> {
    let stage = stage_path(path)?;
    if super::path_present(path)? {
        return Err(
            io::Error::new(io::ErrorKind::AlreadyExists, "claim record already exists").into(),
        );
    }
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() as u64 > MAX_CLAIM_RECORD_BYTES {
        return Err(RuntimeError::InvalidConfiguration(
            "claim record exceeds bounded staging limit",
        ));
    }
    let parent = path.parent().ok_or(RuntimeError::InvalidId)?;
    let mut file = match File::options()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stage)
    {
        Ok(file) => {
            File::open(parent)?.sync_all()?;
            file
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let descriptor = rustix::fs::open(
                &stage,
                rustix::fs::OFlags::RDWR
                    | rustix::fs::OFlags::CLOEXEC
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            )
            .map_err(io::Error::from)?;
            File::from(descriptor)
        }
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() > bytes.len() as u64
    {
        return Err(RuntimeError::IdentityMismatch(
            "staged claim record identity or size",
        ));
    }
    let mut prefix = Vec::new();
    (&mut file)
        .take(MAX_CLAIM_RECORD_BYTES + 1)
        .read_to_end(&mut prefix)?;
    if !bytes.starts_with(&prefix) {
        return Err(RuntimeError::IdentityMismatch(
            "staged claim record intent changed",
        ));
    }
    file.seek(SeekFrom::End(0))?;
    file.write_all(&bytes[prefix.len()..])?;
    file.sync_all()?;
    let current = fs::symlink_metadata(&stage)?;
    if !current.is_file() || current.dev() != metadata.dev() || current.ino() != metadata.ino() {
        return Err(RuntimeError::IdentityMismatch(
            "staged claim record replaced",
        ));
    }
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        &stage,
        rustix::fs::CWD,
        path,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(io::Error::from)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_interrupted_prefix_resumes_only_the_original_record() {
        let value = serde_json::json!({"resource_id":"vm-test-1","pid":17,"start_time_ticks":88});
        let bytes = serde_json::to_vec(&value).unwrap();
        for length in 0..=bytes.len() {
            let temp = tempfile::tempdir().unwrap();
            let target = temp.path().join("record.json");
            let stage = stage_path(&target).unwrap();
            File::options()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(&stage)
                .unwrap()
                .write_all(&bytes[..length])
                .unwrap();
            write_staged_record(&target, &value, rustix::process::geteuid().as_raw()).unwrap();
            assert_eq!(fs::read(&target).unwrap(), bytes);
            assert!(!stage.exists());
            assert!(
                write_staged_record(&target, &value, rustix::process::geteuid().as_raw()).is_err()
            );
        }
    }

    #[test]
    fn changed_intent_foreign_links_and_unknown_targets_are_never_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("network.json");
        let stage = stage_path(&target).unwrap();
        File::options()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&stage)
            .unwrap()
            .write_all(b"{\"foreign\"")
            .unwrap();
        let value = serde_json::json!({"resource_id":"vm-test-1"});
        assert!(write_staged_record(&target, &value, rustix::process::geteuid().as_raw()).is_err());
        assert_eq!(fs::read(&stage).unwrap(), b"{\"foreign\"");
        fs::remove_file(&stage).unwrap();
        let foreign = temp.path().join("foreign");
        fs::write(&foreign, b"do not change").unwrap();
        std::os::unix::fs::symlink(&foreign, &stage).unwrap();
        assert!(write_staged_record(&target, &value, rustix::process::geteuid().as_raw()).is_err());
        assert_eq!(fs::read(&foreign).unwrap(), b"do not change");
        assert!(
            write_staged_record(
                &temp.path().join("unrecognized.json"),
                &value,
                rustix::process::geteuid().as_raw()
            )
            .is_err()
        );
    }

    #[test]
    fn retirement_staging_names_are_finite_and_cannot_escape_the_claim() {
        for name in CLAIM_RECORD_NAMES {
            assert!(known_claim_file(name));
            assert!(known_claim_file(&format!("{STAGING_PREFIX}{name}")));
        }
        for name in [
            ".tmp-random",
            ".staging-console.log",
            ".staging-../record.json",
            ".staging-unknown.json",
        ] {
            assert!(!known_claim_file(name));
        }
    }
}
