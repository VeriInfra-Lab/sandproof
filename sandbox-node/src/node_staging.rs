//! Config-bound deterministic staging for trusted node records. Only an exact
//! reserved stage is recoverable; arbitrary temporary files stay unknown.
use super::{NodeConfig, NodeResult, artifact_digest, check_trusted_path};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub(super) const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
pub(super) const STAGE_FILES: &[&str] = &["identity.json", "payload"];

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Identity {
    version: u32,
    mapping: String,
    target: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct OwnedFile {
    device: u64,
    inode: u64,
    owner: u32,
    size: u64,
    digest: String,
}

#[derive(Clone, Debug)]
struct Stage {
    directory: (u64, u64),
    identity: Option<OwnedFile>,
    payload: Option<OwnedFile>,
}

pub(super) fn path(target: &Path) -> NodeResult<PathBuf> {
    let filename = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("invalid stage target")?;
    Ok(target.with_file_name(format!(".{filename}.stage")))
}

fn identity(config: &NodeConfig, target: &Path) -> NodeResult<Identity> {
    Ok(Identity {
        version: 1,
        mapping: format!("{:x}", Sha256::digest(serde_json::to_vec(config)?)),
        target: target.to_owned(),
    })
}

pub(super) fn identity_bytes(config: &NodeConfig, target: &Path) -> NodeResult<u64> {
    Ok(u64::try_from(
        serde_json::to_vec(&identity(config, target)?)?.len(),
    )?)
}

fn owned_file(path: &Path, owner: u32) -> NodeResult<Option<OwnedFile>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
        || metadata.len() > MAX_PAYLOAD as u64
    {
        return Err("untrusted deterministic stage file".into());
    }
    Ok(Some(OwnedFile {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner,
        size: metadata.len(),
        digest: artifact_digest(File::open(path)?)?,
    }))
}

fn inspect_stage(target: &Path, expected: &Identity, owner: u32) -> NodeResult<Option<Stage>> {
    let directory = path(target)?;
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
    {
        return Err("untrusted deterministic stage directory".into());
    }
    for entry in fs::read_dir(&directory)? {
        if !entry?
            .file_name()
            .to_str()
            .is_some_and(|name| STAGE_FILES.contains(&name))
        {
            return Err("unknown deterministic stage entry".into());
        }
    }
    let identity = owned_file(&directory.join("identity.json"), owner)?;
    let payload = owned_file(&directory.join("payload"), owner)?;
    let expected_bytes = serde_json::to_vec(expected)?;
    let actual = if identity.is_some() {
        fs::read(directory.join("identity.json"))?
    } else {
        Vec::new()
    };
    if !expected_bytes.starts_with(&actual) || (payload.is_some() && actual != expected_bytes) {
        return Err("stage identity does not match exact deployment and target".into());
    }
    Ok(Some(Stage {
        directory: (metadata.dev(), metadata.ino()),
        identity,
        payload,
    }))
}

fn remove_file(path: &Path, expected: Option<&OwnedFile>, owner: u32) -> NodeResult<()> {
    let actual = owned_file(path, owner)?;
    if actual.as_ref() != expected {
        return Err("stage file changed during recovery".into());
    }
    if actual.is_some() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn clean_stage(target: &Path, expected: &Identity, owner: u32) -> NodeResult<()> {
    let Some(stage) = inspect_stage(target, expected, owner)? else {
        File::open(target.parent().ok_or("stage parent")?)?.sync_all()?;
        return Ok(());
    };
    let directory = path(target)?;
    remove_file(&directory.join("payload"), stage.payload.as_ref(), owner)?;
    remove_file(
        &directory.join("identity.json"),
        stage.identity.as_ref(),
        owner,
    )?;
    File::open(&directory)?.sync_all()?;
    let metadata = fs::symlink_metadata(&directory)?;
    if (metadata.dev(), metadata.ino()) != stage.directory || metadata.uid() != owner {
        return Err("stage directory changed during recovery".into());
    }
    fs::remove_dir(&directory)?;
    File::open(target.parent().ok_or("stage parent")?)?.sync_all()?;
    Ok(())
}

pub(super) fn inspect(config: &NodeConfig, target: &Path) -> NodeResult<()> {
    let stage = path(target)?;
    if stage.try_exists()? {
        check_trusted_path(&stage)?;
    }
    inspect_stage(target, &identity(config, target)?, 0)?;
    Ok(())
}

pub(super) fn cleanup(config: &NodeConfig, target: &Path) -> NodeResult<()> {
    inspect(config, target)?;
    clean_stage(target, &identity(config, target)?, 0)
}

fn create_file(path: &Path, bytes: &[u8]) -> NodeResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn write_owned(
    target: &Path,
    bytes: &[u8],
    replace: bool,
    expected: &Identity,
    owner: u32,
) -> NodeResult<()> {
    if bytes.len() > MAX_PAYLOAD {
        return Err("node record exceeds staged byte limit".into());
    }
    let identity_bytes = serde_json::to_vec(expected)?;
    if identity_bytes.len() > MAX_PAYLOAD {
        return Err("node stage identity exceeds staged byte limit".into());
    }
    clean_stage(target, expected, owner)?;
    let directory = path(target)?;
    fs::DirBuilder::new().mode(0o700).create(&directory)?;
    File::open(target.parent().ok_or("stage parent")?)?.sync_all()?;
    create_file(&directory.join("identity.json"), &identity_bytes)?;
    File::open(&directory)?.sync_all()?;
    create_file(&directory.join("payload"), bytes)?;
    File::open(&directory)?.sync_all()?;
    if replace {
        fs::rename(directory.join("payload"), target)?;
    } else {
        // The target is published atomically and can never clobber a previous
        // identity. A crash leaves at most another link to the same inode.
        fs::hard_link(directory.join("payload"), target)?;
    }
    File::open(target.parent().ok_or("stage parent")?)?.sync_all()?;
    clean_stage(target, expected, owner)
}

pub(super) fn write(
    config: &NodeConfig,
    target: &Path,
    bytes: &[u8],
    replace: bool,
) -> NodeResult<()> {
    check_trusted_path(target.parent().ok_or("stage parent")?)?;
    write_owned(target, bytes, replace, &identity(config, target)?, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn expected(target: &Path) -> Identity {
        Identity {
            version: 1,
            mapping: "fixed-config".into(),
            target: target.to_owned(),
        }
    }
    fn stage(target: &Path) -> PathBuf {
        let stage = path(target).unwrap();
        fs::DirBuilder::new().mode(0o700).create(&stage).unwrap();
        stage
    }
    fn owner() -> u32 {
        rustix::process::geteuid().as_raw()
    }

    #[test]
    fn every_interrupted_owned_stage_can_resume_without_reusing_identity() {
        for cut in 0..5 {
            let directory = tempfile::tempdir().unwrap();
            let target = directory.path().join("record.json");
            let identity = expected(&target);
            let stage = stage(&target);
            let bytes = serde_json::to_vec(&identity).unwrap();
            if cut >= 1 {
                create_file(
                    &stage.join("identity.json"),
                    if cut == 1 {
                        &bytes[..bytes.len() / 2]
                    } else {
                        &bytes
                    },
                )
                .unwrap();
            }
            if cut >= 3 {
                create_file(&stage.join("payload"), b"partial old payload").unwrap();
            }
            if cut == 4 {
                fs::hard_link(stage.join("payload"), &target).unwrap();
            }
            let restored: Identity =
                serde_json::from_slice(&serde_json::to_vec(&identity).unwrap()).unwrap();
            clean_stage(&target, &restored, owner()).unwrap();
            assert!(!stage.exists());
            if cut == 4 {
                assert_eq!(fs::read(&target).unwrap(), b"partial old payload");
            }
            write_owned(&target, b"next record", cut == 4, &restored, owner()).unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"next record");
            assert!(!stage.exists());
        }
    }

    #[test]
    fn foreign_stage_cannot_be_deleted_or_hide_uncommitted_payload() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("record.json");
        let stage = stage(&target);
        create_file(&stage.join("payload"), b"foreign").unwrap();
        assert!(clean_stage(&target, &expected(&target), owner()).is_err());
        assert!(stage.join("payload").exists());
        fs::remove_file(stage.join("payload")).unwrap();
        create_file(&stage.join("identity.json"), b"wrong identity").unwrap();
        assert!(clean_stage(&target, &expected(&target), owner()).is_err());
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(clean_stage(&target, &expected(&target), owner()).is_err());
    }

    #[test]
    fn new_write_never_overwrites_an_authoritative_record() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("record.json");
        let identity = expected(&target);
        write_owned(&target, b"first", false, &identity, owner()).unwrap();
        assert!(write_owned(&target, b"second", false, &identity, owner()).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"first");
        clean_stage(&target, &identity, owner()).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"first");
    }
    #[test]
    fn exact_stage_cleanup_refuses_replaced_file_and_syncs_absent_retry() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("record.json");
        let stage = stage(&target);
        let payload = stage.join("payload");
        create_file(&payload, b"old").unwrap();
        let observed = owned_file(&payload, owner()).unwrap().unwrap();
        fs::rename(&payload, stage.join("saved")).unwrap();
        create_file(&payload, b"old").unwrap();
        assert!(remove_file(&payload, Some(&observed), owner()).is_err());
        assert_eq!(fs::read(payload).unwrap(), b"old");
        let absent = directory.path().join("absent.json");
        clean_stage(&absent, &expected(&absent), owner()).unwrap();
    }
}
