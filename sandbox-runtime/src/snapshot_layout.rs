//! First-initialize and exact-reopen layout for one stage1 physical profile.
//! Directory identity comes from original handles, never caller-supplied inode
//! values. Unknown partial initialization is retained and cannot be adopted.

use super::{RuntimeError, jail::DirectoryIdentity};
use crate::snapshot_store::{SnapshotStore, matches_entry, open_store};
use rustix::fs::{self, AtFlags, Mode, OFlags, RenameFlags};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, Read, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;
const PROFILE: &str = "layout.json";
const INITIALIZING: &str = ".layout-initializing.json";
const MAX_PROFILE_BYTES: u64 = 16_384;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LayoutRecord {
    version: u32,
    binding: [u8; 32],
    parent: DirectoryIdentity,
    runtime: DirectoryIdentity,
    store: DirectoryIdentity,
}

/// Private construction binds the original parent/v/s, including after restart.
/// The binding is the original protected deployment/profile value, not a field
/// obtained by reading this record or accepting an untrusted capture request.
pub struct SnapshotLayout {
    path: PathBuf,
    parent: File,
    runtime: File,
    store: File,
    record: LayoutRecord,
    owner: u32,
}

fn error(message: &'static str) -> RuntimeError {
    RuntimeError::IdentityMismatch(message)
}
fn errno(error: rustix::io::Errno) -> RuntimeError {
    RuntimeError::Io(io::Error::from(error))
}
fn require_root() -> Result<(), RuntimeError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(error("layout requires root controller"));
    }
    Ok(())
}
fn child(parent: &File, name: &str, owner: u32) -> Result<File, RuntimeError> {
    let opened = File::from(
        fs::openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(errno)?,
    );
    let metadata = opened.metadata()?;
    if !metadata.is_dir() || metadata.uid() != owner || metadata.mode() & 0o077 != 0 {
        return Err(error("layout child type, owner or mode"));
    }
    matches_entry(parent, OsStr::new(name), &opened)?;
    Ok(opened)
}
fn distinct(parent: &File, runtime: &File, store: &File) -> Result<(), RuntimeError> {
    let base = parent.metadata()?;
    let first = runtime.metadata()?;
    let second = store.metadata()?;
    if first.dev() != base.dev()
        || second.dev() != base.dev()
        || first.ino() == second.ino()
        || first.ino() == base.ino()
        || second.ino() == base.ino()
    {
        return Err(error(
            "layout must be distinct original sibling directories",
        ));
    }
    Ok(())
}
fn empty(parent: &File) -> Result<(), RuntimeError> {
    let mut entries = fs::Dir::read_from(parent).map_err(errno)?;
    for entry in &mut entries {
        let entry = entry.map_err(errno)?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            return Err(error("layout initialize refuses existing contents"));
        }
    }
    Ok(())
}
fn read_record(parent: &File, owner: u32) -> Result<LayoutRecord, RuntimeError> {
    let mut file = File::from(
        fs::openat(
            parent,
            PROFILE,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(errno)?,
    );
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
        || metadata.len() == 0
        || metadata.len() > MAX_PROFILE_BYTES
    {
        return Err(error("layout record ownership, type or bounds"));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_PROFILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(error("layout record grew"));
    }
    matches_entry(parent, OsStr::new(PROFILE), &file)?;
    Ok(serde_json::from_slice(&bytes)?)
}
fn publish_record(parent: &File, record: &LayoutRecord) -> Result<(), RuntimeError> {
    let bytes = serde_json::to_vec(record)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_PROFILE_BYTES {
        return Err(error("layout record size"));
    }
    let mut file = File::from(
        fs::openat(
            parent,
            INITIALIZING,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(errno)?,
    );
    file.write_all(&bytes)?;
    file.sync_all()?;
    matches_entry(parent, OsStr::new(INITIALIZING), &file)?;
    fs::renameat_with(
        parent,
        INITIALIZING,
        parent,
        PROFILE,
        RenameFlags::NOREPLACE,
    )
    .map_err(errno)?;
    parent.sync_all()?;
    matches_entry(parent, OsStr::new(PROFILE), &file)
}

impl SnapshotLayout {
    /// Initialize a fresh empty protected parent exactly once. No cleanup on
    /// error: partial directories/record remain under the original work scope.
    /// # Errors
    /// Rejects non-root, nonempty parents, aliases and any create/write/sync error.
    pub fn initialize(path: &Path, binding: [u8; 32]) -> Result<Self, RuntimeError> {
        require_root()?;
        Self::initialize_for_owner(path, binding, 0)
    }
    fn initialize_for_owner(
        path: &Path,
        binding: [u8; 32],
        owner: u32,
    ) -> Result<Self, RuntimeError> {
        let parent = open_store(path, owner)?;
        empty(&parent)?;
        fs::mkdirat(&parent, "v", Mode::RWXU).map_err(errno)?;
        fs::mkdirat(&parent, "s", Mode::RWXU).map_err(errno)?;
        let runtime = child(&parent, "v", owner)?;
        let store = child(&parent, "s", owner)?;
        distinct(&parent, &runtime, &store)?;
        let record = LayoutRecord {
            version: VERSION,
            binding,
            parent: DirectoryIdentity::capture(&parent)?,
            runtime: DirectoryIdentity::capture(&runtime)?,
            store: DirectoryIdentity::capture(&store)?,
        };
        runtime.sync_all()?;
        store.sync_all()?;
        parent.sync_all()?;
        publish_record(&parent, &record)?;
        Ok(Self {
            path: path.to_owned(),
            parent,
            runtime,
            store,
            record,
            owner,
        })
    }
    /// Reopen the original registered layout, including populated runtime/store
    /// directories after manager restart. Missing/partial/replaced records fail
    /// closed; this function never calls initialize or chooses a new number.
    /// # Errors
    /// Rejects any binding, version, path, permission or original identity mismatch.
    pub fn reopen(path: &Path, binding: [u8; 32]) -> Result<Self, RuntimeError> {
        require_root()?;
        Self::reopen_for_owner(path, binding, 0)
    }
    fn reopen_for_owner(path: &Path, binding: [u8; 32], owner: u32) -> Result<Self, RuntimeError> {
        let parent = open_store(path, owner)?;
        let record = read_record(&parent, owner)?;
        if record.version != VERSION
            || record.binding != binding
            || !record.parent.matches(&parent)?
        {
            return Err(error("layout version, binding or parent changed"));
        }
        let runtime = child(&parent, "v", owner)?;
        let store = child(&parent, "s", owner)?;
        distinct(&parent, &runtime, &store)?;
        if !record.runtime.matches(&runtime)? || !record.store.matches(&store)? {
            return Err(error("layout original child identity changed"));
        }
        match fs::statat(&parent, INITIALIZING, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => {}
            Ok(_) => return Err(error("layout has unresolved initialization work")),
            Err(value) => return Err(errno(value)),
        }
        parent.sync_all()?;
        Ok(Self {
            path: path.to_owned(),
            parent,
            runtime,
            store,
            record,
            owner,
        })
    }
    #[must_use]
    pub fn runtime_root(&self) -> PathBuf {
        self.path.join("v")
    }
    #[must_use]
    pub fn store_root(&self) -> PathBuf {
        self.path.join("s")
    }

    /// Read an existing published key; this never grants registration or stop.
    /// # Errors
    /// Rejects changed layout and invalid/incomplete original files.
    pub fn open_capture_bundle(
        &self,
        directory_key: [u8; 32],
        limits: crate::snapshot_store::BundleReadLimits,
    ) -> Result<crate::snapshot_store::SnapshotBundle, RuntimeError> {
        self.store_for_runtime(&self.runtime_root())?
            .open_bundle(directory_key, limits)
    }

    pub(crate) fn store_for_runtime(&self, path: &Path) -> Result<SnapshotStore, RuntimeError> {
        if path != self.runtime_root() {
            return Err(error("runtime root differs from original layout"));
        }
        let current = Self::reopen_for_owner(&self.path, self.record.binding, self.owner)?;
        if current.record != self.record
            || !self.record.parent.matches(&self.parent)?
            || !self.record.runtime.matches(&self.runtime)?
            || !self.record.store.matches(&self.store)?
        {
            return Err(error("cached layout no longer matches original"));
        }
        let store =
            SnapshotStore::from_layout(self.store_root(), self.store.try_clone()?, self.owner);
        // Cheap path rejection is additional; the layout's producer/original
        // identities provide the sibling relation, not this lexical predicate.
        store.require_separate_from(path)?;
        Ok(store)
    }
}

#[cfg(test)]
#[path = "snapshot_layout_tests.rs"]
mod tests;
