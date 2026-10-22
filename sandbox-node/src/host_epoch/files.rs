use super::{KvmDeviceIdentity, Result, error};
use rustix::fs::{self as rfs, Mode, OFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
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
