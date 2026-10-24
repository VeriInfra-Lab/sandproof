//! One bounded physical completion beside the original immutable bundle.
//! An interrupted write remains charged; no record grants SQL or Stop rights.
use super::{FileStamp, SnapshotStore, invalid, key_name, matches_entry, os_error};
use crate::RuntimeError;
use rustix::fs::{Mode, OFlags, RenameFlags, openat, renameat_with};
use std::{
    ffi::OsStr,
    fs::{File, Permissions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
};
pub(super) const MAX_COMPLETION_BYTES: u64 = 16_384;

impl SnapshotStore {
    pub(crate) fn publish_completion(
        &self,
        key: [u8; 32],
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        self.publish_record("completed-", "completion-stage-", key, bytes)
    }

    #[cfg(test)]
    pub(crate) fn publish_api_end(&self, key: [u8; 32], bytes: &[u8]) -> Result<(), RuntimeError> {
        self.publish_record("api-returned-", "api-return-stage-", key, bytes)
    }

    pub(crate) fn publish_retained_api_end(
        &self,
        key: [u8; 32],
        bytes: &[u8],
        source_alive: impl FnMut() -> Result<bool, RuntimeError>,
    ) -> Result<(), RuntimeError> {
        self.publish_record_retained("api-returned-", "api-return-stage-", key, bytes, source_alive)
    }

    fn publish_record(
        &self,
        prefix: &str,
        stage_prefix: &str,
        key: [u8; 32],
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        self.publish_record_retained(prefix, stage_prefix, key, bytes, || Ok(false))
    }

    fn publish_record_retained(
        &self,
        prefix: &str,
        stage_prefix: &str,
        key: [u8; 32],
        bytes: &[u8],
        mut source_alive: impl FnMut() -> Result<bool, RuntimeError>,
    ) -> Result<(), RuntimeError> {
        if bytes.is_empty() || bytes.len() as u64 > MAX_COMPLETION_BYTES {
            return Err(invalid("capture completion byte bound"));
        }
        let name = key_name(prefix, &key);
        let stage = key_name(stage_prefix, &key);
        let mut file = retry_publication(|| Ok(File::from(
            openat(
                &self.directory,
                stage.as_str(),
                OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(os_error)?,
        )), &mut source_alive)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.uid() != self.owner || meta.nlink() != 1 {
            return Err(invalid("capture completion owner/type/link"));
        }
        retry_publication(|| {
            matches_entry(&self.directory, OsStr::new(&stage), &file)?;
            file.seek(SeekFrom::Start(0))?;
            file.write_all(bytes)?;
            Ok(())
        }, &mut source_alive)?;
        retry_publication(|| Ok(file.sync_all()?), &mut source_alive)?;
        retry_publication(|| Ok(file.set_permissions(Permissions::from_mode(0o400))?), &mut source_alive)?;
        retry_publication(|| Ok(file.sync_all()?), &mut source_alive)?;
        matches_entry(&self.directory, OsStr::new(&stage), &file)?;
        retry_publication(|| Ok(renameat_with(
            &self.directory,
            stage.as_str(),
            &self.directory,
            name.as_str(),
            RenameFlags::NOREPLACE,
        )
        .map_err(os_error)?), &mut source_alive)?;
        retry_publication(|| Ok(self.directory.sync_all()?), &mut source_alive)?;
        matches_entry(&self.directory, OsStr::new(&name), &file)?;
        Ok(())
    }

    pub(crate) fn read_completion(&self, key: [u8; 32]) -> Result<Vec<u8>, RuntimeError> {
        self.read_record("completed-", key)
    }
    pub(crate) fn read_api_end(&self, key: [u8; 32]) -> Result<Vec<u8>, RuntimeError> {
        self.read_record("api-returned-", key)
    }
    fn read_record(&self, prefix: &str, key: [u8; 32]) -> Result<Vec<u8>, RuntimeError> {
        let name = key_name(prefix, &key);
        let mut file = File::from(
            openat(
                &self.directory,
                name.as_str(),
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(os_error)?,
        );
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.uid() != self.owner
            || meta.nlink() != 1
            || meta.mode() & 0o777 != 0o400
            || meta.len() == 0
            || meta.len() > MAX_COMPLETION_BYTES
        {
            return Err(invalid("capture completion original file"));
        }
        let before = FileStamp::from_metadata(&meta);
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(MAX_COMPLETION_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != meta.len() || before != FileStamp::from_metadata(&file.metadata()?)
        {
            return Err(invalid("capture completion changed while reading"));
        }
        matches_entry(&self.directory, OsStr::new(&name), &file)?;
        // Resolve a previous uncertain publication sync on this same directory.
        self.directory.sync_all()?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn api_end_is_separate_from_material_completion_and_partial_stage() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), Permissions::from_mode(0o700)).unwrap();
        let owner = rustix::process::geteuid().as_raw();
        let store = SnapshotStore::open_for_owner(root.path(), owner).unwrap();
        let key = [13; 32];
        store.publish_api_end(key, b"original API-end").unwrap();
        assert_eq!(store.read_api_end(key).unwrap(), b"original API-end");
        assert!(store.read_completion(key).is_err());
        assert!(store.publish_api_end(key, b"replacement").is_err());
        assert_eq!(store.read_api_end(key).unwrap(), b"original API-end");
        let interrupted = [14; 32];
        std::fs::write(
            root.path()
                .join(key_name("api-return-stage-", &interrupted)),
            b"partial",
        )
        .unwrap();
        assert!(store.read_api_end(interrupted).is_err());
        assert!(store.publish_api_end(interrupted, b"new").is_err());
    }

    #[test]
    fn original_completion_survives_reopen_and_never_overwrites() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), Permissions::from_mode(0o700)).unwrap();
        let owner = rustix::process::geteuid().as_raw();
        let store = SnapshotStore::open_for_owner(root.path(), owner).unwrap();
        let key = [7; 32];
        store
            .publish_completion(key, b"original physical completion")
            .unwrap();
        drop(store);
        let store = SnapshotStore::open_for_owner(root.path(), owner).unwrap();
        assert_eq!(
            store.read_completion(key).unwrap(),
            b"original physical completion"
        );
        assert!(store.publish_completion(key, b"replacement").is_err());
        assert_eq!(
            store.read_completion(key).unwrap(),
            b"original physical completion"
        );
        assert!(store.read_completion([8; 32]).is_err());
    }

    #[test]
    fn staging_symlink_oversize_and_mutable_completion_never_attest_success() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), Permissions::from_mode(0o700)).unwrap();
        let store = SnapshotStore::open_for_owner(root.path(), rustix::process::geteuid().as_raw())
            .unwrap();
        let key = [9; 32];
        assert!(store.publish_completion(key, &vec![0; 16_385]).is_err());
        let original = root.path().join("original");
        std::fs::write(&original, b"do not change").unwrap();
        symlink(
            &original,
            root.path().join(key_name("completion-stage-", &key)),
        )
        .unwrap();
        assert!(store.publish_completion(key, b"new").is_err());
        assert_eq!(std::fs::read(&original).unwrap(), b"do not change");
        assert!(store.read_completion(key).is_err());
        let other = [10; 32];
        store.publish_completion(other, b"complete").unwrap();
        std::fs::set_permissions(
            root.path().join(key_name("completed-", &other)),
            Permissions::from_mode(0o600),
        )
        .unwrap();
        assert!(store.read_completion(other).is_err());
    }
}

/// One failed syscall per scheduling interval. The caller retains the exact
/// stage/bytes and never repeats a Full request. Exact VMM exit drains ownership.
fn retry_publication<T>(
    mut step: impl FnMut() -> Result<T, RuntimeError>,
    mut source_alive: impl FnMut() -> Result<bool, RuntimeError>,
) -> Result<T, RuntimeError> {
    loop {
        match step() {
            Err(RuntimeError::Io(error)) if matches!(error.raw_os_error(), Some(4 | 28 | 122)) => {
                if !source_alive()? { return Err(RuntimeError::Io(error)); }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod retained_tests {
    use super::*;
    #[test]
    fn temporary_space_failure_keeps_known_result_until_same_step_succeeds() {
        let mut attempts = 0;
        let original = b"original returned response";
        let result = retry_publication(|| {
            attempts += 1;
            if attempts == 1 { return Err(RuntimeError::Io(std::io::Error::from_raw_os_error(28))); }
            Ok(original.as_slice())
        }, || Ok(true)).unwrap();
        assert_eq!(result, original);
        assert_eq!(attempts, 2);
    }
    #[test]
    fn exact_source_exit_ends_space_wait_without_fabricating_publication() {
        let mut attempts = 0;
        let result: Result<(), _> = retry_publication(|| {
            attempts += 1;
            Err(RuntimeError::Io(std::io::Error::from_raw_os_error(28)))
        }, || Ok(false));
        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }
    #[test]
    fn foreign_identity_errors_are_not_retried() {
        let result: Result<(), _> = retry_publication(|| Err(invalid("foreign stage")), || panic!("not transient"));
        assert!(result.is_err());
    }
}
