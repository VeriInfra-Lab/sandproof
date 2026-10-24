//! One bounded physical completion beside the original immutable bundle.
//! An interrupted write remains charged; no record grants SQL or Stop rights.
use super::{FileStamp, SnapshotStore, invalid, key_name, matches_entry, os_error};
use crate::RuntimeError;
use rustix::fs::{Mode, OFlags, RenameFlags, openat, renameat_with};
use std::{
    ffi::OsStr,
    fs::{File, Permissions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
};
pub(super) const MAX_COMPLETION_BYTES: u64 = 16_384;

impl SnapshotStore {
    pub(crate) fn publish_completion(
        &self,
        key: [u8; 32],
        bytes: &[u8],
    ) -> Result<(), RuntimeError> {
        if bytes.is_empty() || bytes.len() as u64 > MAX_COMPLETION_BYTES {
            return Err(invalid("capture completion byte bound"));
        }
        let name = key_name("completed-", &key);
        let stage = key_name("completion-stage-", &key);
        let mut file = File::from(
            openat(
                &self.directory,
                stage.as_str(),
                OFlags::CREATE | OFlags::EXCL | OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(os_error)?,
        );
        let meta = file.metadata()?;
        if !meta.is_file() || meta.uid() != self.owner || meta.nlink() != 1 {
            return Err(invalid("capture completion owner/type/link"));
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        file.set_permissions(Permissions::from_mode(0o400))?;
        file.sync_all()?;
        matches_entry(&self.directory, OsStr::new(&stage), &file)?;
        renameat_with(
            &self.directory,
            stage.as_str(),
            &self.directory,
            name.as_str(),
            RenameFlags::NOREPLACE,
        )
        .map_err(os_error)?;
        self.directory.sync_all()?;
        matches_entry(&self.directory, OsStr::new(&name), &file)?;
        Ok(())
    }

    pub(crate) fn read_completion(&self, key: [u8; 32]) -> Result<Vec<u8>, RuntimeError> {
        let name = key_name("completed-", &key);
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
