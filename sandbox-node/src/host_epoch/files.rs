use super::{KvmDeviceIdentity, Result, error};
use rustix::fs::{self as rfs, Mode, OFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read, Write},
    os::unix::fs::{FileTypeExt, MetadataExt},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    owner: u32,
    group: u32,
    mode: u32,
    links: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Stamp {
    fn read(value: &fs::Metadata) -> Self {
        Self {
            device: value.dev(),
            inode: value.ino(),
            owner: value.uid(),
            group: value.gid(),
            mode: value.mode(),
            links: value.nlink(),
            length: value.len(),
            modified: (value.mtime(), value.mtime_nsec()),
            changed: (value.ctime(), value.ctime_nsec()),
        }
    }
}
#[derive(Clone, Copy)]
enum Kind {
    Payload,
    Executable,
}
impl Kind {
    fn maximum(self) -> u64 {
        match self {
            Self::Payload => 1024 * 1024,
            Self::Executable => 64 * 1024 * 1024,
        }
    }
    fn accepts(self, mode: u32) -> bool {
        match self {
            Self::Payload => mode & 0o7777 == 0o400,
            Self::Executable => mode & 0o022 == 0 && mode & 0o111 != 0,
        }
    }
}
pub(super) struct CheckedFile {
    file: File,
    path: PathBuf,
    stamp: Stamp,
    pub bytes: Vec<u8>,
    pub sha256: [u8; 32],
}
impl CheckedFile {
    pub fn payload(path: &Path) -> Result<Self> {
        sandbox_runtime::check_trusted_path(path)?;
        Self::read_owned(path, 0, Kind::Payload)
    }
    fn read_owned(path: &Path, owner: u32, kind: Kind) -> Result<Self> {
        let mut file = File::from(
            rfs::open(
                path,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(io::Error::from)?,
        );
        let metadata = file.metadata()?;
        if !metadata.is_file()
            || metadata.uid() != owner
            || metadata.nlink() != 1
            || metadata.len() == 0
            || metadata.len() > kind.maximum()
            || !kind.accepts(metadata.mode())
        {
            return Err(error("bootstrap file owner/type/mode/links/length differ"));
        }
        let stamp = Stamp::read(&metadata);
        let mut hasher = Sha256::new();
        let mut bytes = Vec::new();
        let mut length = 0_u64;
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let maximum = usize::try_from((metadata.len() - length + 1).min(buffer.len() as u64))
                .map_err(|_| error("bootstrap read size"))?;
            let count = file.read(&mut buffer[..maximum])?;
            if count == 0 {
                break;
            }
            length += count as u64;
            if length > metadata.len() {
                return Err(error("bootstrap file grew during read"));
            }
            hasher.update(&buffer[..count]);
            if matches!(kind, Kind::Payload) {
                bytes.extend_from_slice(&buffer[..count]);
            }
        }
        if length != metadata.len() {
            return Err(error("bootstrap file truncated during read"));
        }
        let result = Self {
            file,
            path: path.into(),
            stamp,
            bytes,
            sha256: hasher.finalize().into(),
        };
        result.recheck()?;
        Ok(result)
    }
    pub fn recheck(&self) -> Result<()> {
        if Stamp::read(&self.file.metadata()?) != self.stamp
            || Stamp::read(&fs::symlink_metadata(&self.path)?) != self.stamp
        {
            return Err(error("bootstrap file changed or replaced"));
        }
        Ok(())
    }
}
/// Explicit, create-only operator publication. No serve/recovery caller invokes
/// this function, and uncertain failures never remove or replace the new file.
pub(super) fn write_new_payload(path: &Path, bytes: &[u8]) -> Result<()> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(error(
            "bootstrap provisioning requires original root identity",
        ));
    }
    let parent_path = path
        .parent()
        .ok_or_else(|| error("bootstrap parent missing"))?;
    let leaf = path
        .file_name()
        .ok_or_else(|| error("bootstrap filename missing"))?;
    sandbox_runtime::check_trusted_path(parent_path)?;
    let parent = File::from(
        rfs::open(
            parent_path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    recheck_parent(parent_path, &parent)?;
    let file = write_owned_payload(&parent, leaf, bytes, 0)?;
    recheck_parent(parent_path, &parent)?;
    let checked = CheckedFile::payload(path)?;
    if Stamp::read(&file.metadata()?) != checked.stamp || checked.bytes != bytes {
        return Err(error(
            "published bootstrap file differs from original descriptor",
        ));
    }
    checked.recheck()?;
    Ok(())
}

fn recheck_parent(path: &Path, parent: &File) -> Result<()> {
    sandbox_runtime::check_trusted_path(path)?;
    let held = parent.metadata()?;
    let named = fs::symlink_metadata(path)?;
    if !held.is_dir()
        || held.uid() != 0
        || held.mode() & 0o022 != 0
        || held.dev() != named.dev()
        || held.ino() != named.ino()
        || !named.is_dir()
        || named.file_type().is_symlink()
    {
        return Err(error("bootstrap parent changed"));
    }
    Ok(())
}

fn write_owned_payload(
    parent: &File,
    leaf: &std::ffi::OsStr,
    bytes: &[u8],
    owner: u32,
) -> Result<File> {
    let metadata = parent.metadata()?;
    let mut components = Path::new(leaf).components();
    if bytes.is_empty()
        || bytes.len() > Kind::Payload.maximum() as usize
        || !metadata.is_dir()
        || metadata.uid() != owner
        || metadata.mode() & 0o022 != 0
        || !matches!(components.next(), Some(std::path::Component::Normal(_)))
        || components.next().is_some()
    {
        return Err(error(
            "invalid bootstrap publication parent, name or length",
        ));
    }
    let mut file = File::from(
        rfs::openat(
            parent,
            leaf,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR,
        )
        .map_err(io::Error::from)?,
    );
    file.write_all(bytes)?;
    file.sync_all()?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != owner
        || metadata.nlink() != 1
        || metadata.mode() & 0o7777 != 0o400
        || metadata.len() != bytes.len() as u64
    {
        return Err(error("new bootstrap file metadata differs"));
    }
    parent.sync_all()?;
    Ok(file)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ToolIdentity {
    device: u64,
    inode: u64,
    length: u64,
    mode: u32,
    group: u32,
    sha256: [u8; 32],
}
impl ToolIdentity {
    pub(super) fn digest(&self) -> [u8; 32] {
        self.sha256
    }
}
pub(super) fn tool(path: &Path) -> Result<ToolIdentity> {
    sandbox_runtime::check_trusted_path(path)?;
    let file = CheckedFile::read_owned(path, 0, Kind::Executable)?;
    file.recheck()?;
    Ok(ToolIdentity {
        device: file.stamp.device,
        inode: file.stamp.inode,
        length: file.stamp.length,
        mode: file.stamp.mode & 0o7777,
        group: file.stamp.group,
        sha256: file.sha256,
    })
}

/// Fixed kernel paths only; callers never select these from an API request.
/// Reads are byte-bounded, not a hard per-system-call latency guarantee.
pub(super) fn host_text(path: &Path, maximum: u64) -> Result<String> {
    let file = File::from(
        rfs::open(
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    if !file.metadata()?.is_file() {
        return Err(error("host metadata is not a regular kernel file"));
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(error("host metadata exceeded bootstrap bound"));
    }
    let value = String::from_utf8(bytes).map_err(|_| error("host metadata is not UTF-8"))?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(error("host metadata is empty"));
    }
    Ok(value)
}

pub(super) fn kvm_device() -> Result<KvmDeviceIdentity> {
    sandbox_runtime::check_trusted_path(Path::new("/dev"))?;
    let file = File::from(
        rfs::open(
            "/dev/kvm",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    let metadata = file.metadata()?;
    if !metadata.file_type().is_char_device()
        || metadata.uid() != 0
        || metadata.mode() & 0o002 != 0
        || metadata.nlink() != 1
        || rfs::major(metadata.rdev()) != 10
        || rfs::minor(metadata.rdev()) != 232
        || Stamp::read(&fs::symlink_metadata("/dev/kvm")?) != Stamp::read(&metadata)
    {
        return Err(error("fixed KVM device identity differs"));
    }
    Ok(KvmDeviceIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        special_device: metadata.rdev(),
    })
}

#[cfg(test)]
pub(super) fn controlled_payload_leaf(path: &Path, owner: u32) -> Result<CheckedFile> {
    // Controlled leaf tests only. The production entry always checks root
    // ancestors/owner via CheckedFile::payload; this is not a public bypass.
    CheckedFile::read_owned(path, owner, Kind::Payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    };
    fn fixture(mode: u32) -> (tempfile::TempDir, PathBuf, u32) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("payload");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&path)
            .unwrap();
        file.write_all(b"protected original bytes").unwrap();
        let owner = file.metadata().unwrap().uid();
        (root, path, owner)
    }
    #[test]
    fn explicit_publication_is_create_only_private_and_reopenable() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let parent = File::open(directory.path()).unwrap();
        let owner = parent.metadata().unwrap().uid();
        let leaf = std::ffi::OsStr::new("bootstrap");
        let bytes = b"actual publication bytes";
        let file = write_owned_payload(&parent, leaf, bytes, owner).unwrap();
        let path = directory.path().join(leaf);
        let checked = CheckedFile::read_owned(&path, owner, Kind::Payload).unwrap();
        assert_eq!(checked.bytes, bytes);
        assert_eq!(checked.stamp, Stamp::read(&file.metadata().unwrap()));
        assert!(write_owned_payload(&parent, leaf, b"replacement", owner).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::hard_link(&path, directory.path().join("alias")).unwrap();
        assert!(write_owned_payload(&parent, leaf, b"replacement", owner).is_err());
        assert!(CheckedFile::read_owned(&path, owner, Kind::Payload).is_err());
    }

    #[test]
    fn publication_rejects_bad_parent_name_size_or_symlink_without_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let parent = File::open(directory.path()).unwrap();
        let owner = parent.metadata().unwrap().uid();
        for leaf in ["", "..", "../escape", "/absolute", "sub/leaf"] {
            assert!(
                write_owned_payload(&parent, std::ffi::OsStr::new(leaf), b"bytes", owner).is_err()
            );
        }
        let leaf = std::ffi::OsStr::new("bootstrap");
        assert!(write_owned_payload(&parent, leaf, b"", owner).is_err());
        assert!(write_owned_payload(&parent, leaf, &vec![0; 1024 * 1024 + 1], owner).is_err());
        assert!(write_owned_payload(&parent, leaf, b"bytes", owner.wrapping_add(1)).is_err());
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o777)).unwrap();
        assert!(write_owned_payload(&parent, leaf, b"bytes", owner).is_err());
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
        fs::write(directory.path().join("original"), b"retained").unwrap();
        symlink("original", directory.path().join(leaf)).unwrap();
        assert!(write_owned_payload(&parent, leaf, b"replacement", owner).is_err());
        assert_eq!(
            fs::read(directory.path().join("original")).unwrap(),
            b"retained"
        );
    }

    #[test]
    fn leaf_reader_hashes_real_bytes_and_rejects_wrong_owner_mode_and_links() {
        let (root, path, owner) = fixture(0o400);
        let file = CheckedFile::read_owned(&path, owner, Kind::Payload).unwrap();
        assert_eq!(file.bytes, b"protected original bytes");
        assert_eq!(file.sha256, <[u8; 32]>::from(Sha256::digest(&file.bytes)));
        assert!(CheckedFile::read_owned(&path, owner.wrapping_add(1), Kind::Payload).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(CheckedFile::read_owned(&path, owner, Kind::Payload).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        fs::hard_link(&path, root.path().join("alias")).unwrap();
        assert!(CheckedFile::read_owned(&path, owner, Kind::Payload).is_err());
    }
    #[test]
    fn held_registration_rejects_replacement_symlink_and_changed_bytes() {
        let (root, path, owner) = fixture(0o400);
        let held = CheckedFile::read_owned(&path, owner, Kind::Payload).unwrap();
        fs::rename(&path, root.path().join("old")).unwrap();
        fs::write(&path, b"protected original bytes").unwrap();
        assert!(held.recheck().is_err());
        fs::remove_file(&path).unwrap();
        symlink("old", &path).unwrap();
        assert!(CheckedFile::read_owned(&path, owner, Kind::Payload).is_err());
        fs::remove_file(&path).unwrap();
        fs::rename(root.path().join("old"), &path).unwrap();
        let held = CheckedFile::read_owned(&path, owner, Kind::Payload).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, b"changed bytes").unwrap();
        assert!(held.recheck().is_err());
    }
}
