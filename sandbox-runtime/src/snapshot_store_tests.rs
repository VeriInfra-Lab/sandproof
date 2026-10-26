use super::*;
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{OpenOptionsExt, symlink},
};

struct Fixture {
    root: tempfile::TempDir,
    store: std::path::PathBuf,
    owner: u32,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), Permissions::from_mode(0o700)).unwrap();
        let store = root.path().join("store");
        fs::create_dir(&store).unwrap();
        fs::set_permissions(&store, Permissions::from_mode(0o700)).unwrap();
        Self {
            root,
            store,
            owner: rustix::process::geteuid().as_raw(),
        }
    }
    fn open(&self) -> SnapshotStore {
        SnapshotStore::open_for_owner(&self.store, self.owner).unwrap()
    }
    fn source(&self, name: &str, bytes: &[u8]) -> File {
        let path = self.root.path().join(name);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
        file
    }
    fn full_stage(&self, key: [u8; 32]) -> SnapshotStage {
        let mut stage = self.open().begin(key).unwrap();
        for (index, role) in [
            SnapshotArtifact::State,
            SnapshotArtifact::Memory,
            SnapshotArtifact::Disk(0),
        ]
        .into_iter()
        .enumerate()
        {
            let mut input = self.source(&format!("source-{index}"), b"actual bytes");
            stage
                .copy_artifact(&mut input, self.owner, role, 100)
                .unwrap();
        }
        stage
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Restore our own fixture directories for non-root tempdir reclamation.
        for entry in fs::read_dir(&self.store).unwrap().flatten() {
            if entry.file_type().unwrap().is_dir() {
                fs::set_permissions(entry.path(), Permissions::from_mode(0o700)).unwrap();
            }
        }
    }
}

#[test]
fn copies_actual_bytes_and_publishes_exact_files_without_reusing_names() {
    let fixture = Fixture::new();
    let key = [1; 32];
    let stage = fixture.full_stage(key);
    let measurements = stage.measurements();
    let published = stage.publish(b"{\"bound\":true}", &measurements).unwrap();
    assert_eq!(published.files, measurements);
    assert_eq!(
        published.manifest_sha256,
        <[u8; 32]>::from(Sha256::digest(b"{\"bound\":true}"))
    );
    let bundle = fixture.store.join(key_name("capture-", &key));
    assert!(!fixture.store.join(key_name("staging-", &key)).exists());
    for entry in &published.files {
        let path = bundle.join(entry.artifact.relative_path());
        let bytes = fs::read(&path).unwrap();
        assert_eq!(bytes, b"actual bytes");
        assert_eq!(entry.sha256, <[u8; 32]>::from(Sha256::digest(&bytes)));
        assert_eq!(entry.length, bytes.len() as u64);
        assert_eq!(path.metadata().unwrap().mode() & 0o777, 0o400);
    }
    assert_eq!(bundle.metadata().unwrap().mode() & 0o777, 0o500);
    assert!(fixture.open().begin(key).is_err());
}

#[test]
fn stage_drop_and_copy_error_keep_original_and_partial_materials() {
    let fixture = Fixture::new();
    let key = [2; 32];
    let mut original = fixture.source("original", b"preserve live state");
    original.seek(SeekFrom::Start(0)).unwrap();
    let source_path = fixture.root.path().join("original");
    let mut write_only = OpenOptions::new().write(true).open(&source_path).unwrap();
    let mut stage = fixture.open().begin(key).unwrap();
    assert!(
        stage
            .copy_artifact(
                &mut write_only,
                fixture.owner,
                SnapshotArtifact::Memory,
                100
            )
            .is_err()
    );
    drop(stage);
    assert_eq!(fs::read(source_path).unwrap(), b"preserve live state");
    assert!(
        fixture
            .store
            .join(key_name("staging-", &key))
            .join("memory.bin")
            .exists()
    );
    assert!(!fixture.store.join(key_name("capture-", &key)).exists());
    assert!(fixture.open().begin(key).is_err());
}

#[test]
fn actual_copy_write_error_child() {
    let Some(root) = std::env::var_os("SANDPROOF_UNIT_COPY_FSIZE_ROOT") else { return; };
    let root = std::path::PathBuf::from(root);
    let owner = rustix::process::geteuid().as_raw();
    let store = SnapshotStore::open_for_owner(&root.join("store"), owner).unwrap();
    let mut source = File::open(root.join("large-source")).unwrap();
    let original = FileStamp::from_metadata(&source.metadata().unwrap());
    let key = [27;32];
    let mut stage = store.begin(key).unwrap();
    let result = stage.copy_artifact(&mut source, owner, SnapshotArtifact::Memory, 196_608);
    let RuntimeError::Io(error) = result.unwrap_err() else { panic!("expected actual write I/O failure"); };
    assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
    assert!(stage.measurements().is_empty(), "partial bytes are not a completed artifact");
    drop(stage);
    let length = fs::metadata(root.join("store").join(key_name("staging-",&key)).join("memory.bin")).unwrap().len();
    assert!(length>0 && length<196_608, "kernel must reject an actual mid-copy write");
    assert_eq!(FileStamp::from_metadata(&source.metadata().unwrap()),original);
    assert!(!root.join("store").join(key_name("capture-",&key)).exists());
    assert!(store.begin(key).is_err(), "same original partial work cannot be silently reused");
}

#[test]
fn actual_mid_copy_write_error_keeps_partial_bytes_and_original_source() {
    let fixture = Fixture::new();
    let payload=vec![b'L';196_608];
    drop(fixture.source("large-source", &payload));
    // The inherited ignored SIGXFSZ makes the kernel return EFBIG to write(2).
    // Without it the child could die, which would be Unknown, not a returned
    // ordinary I/O error. This limit applies only to this exact child process.
    let output=std::process::Command::new("/bin/sh")
        .args(["-c", "trap '' XFSZ; ulimit -f 128; exec \"$@\"", "copy-fsize"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact","snapshot_store::tests::actual_copy_write_error_child","--nocapture"])
        .env("SANDPROOF_UNIT_COPY_FSIZE_ROOT",fixture.root.path())
        .output().unwrap();
    assert!(output.status.success(), "{}",String::from_utf8_lossy(&output.stderr));
    assert_eq!(fs::read(fixture.root.path().join("large-source")).unwrap(),payload);
    assert!(fixture.store.join(key_name("staging-",&[27;32])).join("memory.bin").metadata().unwrap().len()>0);
}

#[test]
fn real_parent_sync_error_after_publish_keeps_original_bundle_reopenable() {
    let fixture=Fixture::new();
    let key=[28;32];
    let mut stage=fixture.full_stage(key);
    let measured=stage.measurements();
    let before=stage.parent.metadata().unwrap();
    // O_PATH is the SAME original directory inode: renameat is valid, but
    // fsync on this descriptor returns EBADF after the actual publication.
    // This is fixture-only FD substitution, never a production sync bypass.
    stage.parent=File::from(open(&fixture.store,OFlags::PATH|OFlags::DIRECTORY|OFlags::NOFOLLOW|OFlags::CLOEXEC,Mode::empty()).unwrap());
    assert_eq!((stage.parent.metadata().unwrap().dev(),stage.parent.metadata().unwrap().ino()),(before.dev(),before.ino()));
    assert!(matches!(stage.publish(b"original bound manifest",&measured),Err(RuntimeError::Io(_))));
    let bundle=fixture.open().open_bundle(key,BundleReadLimits{state_bytes:100,memory_bytes:100,disk_bytes:100}).unwrap();
    assert_eq!(bundle.manifest_bytes(),b"original bound manifest");
    assert_eq!(bundle.measurements(),measured);
    assert!(fixture.open().begin(key).is_err(),"visible publication must not be repeated");
    for index in 0..3 { assert_eq!(fs::read(fixture.root.path().join(format!("source-{index}"))).unwrap(),b"actual bytes"); }
}

#[test]
fn copy_observer_is_after_real_block_write_and_failure_never_seals_partial_bytes() {
    let fixture=Fixture::new();
    let key=[29;32];
    let payload=vec![7;COPY_BUFFER_BYTES*2];
    let mut source=fixture.source("observed-source",&payload);
    let mut stage=fixture.open().begin(key).unwrap();
    let partial=fixture.store.join(key_name("staging-",&key)).join("memory.bin");
    let mut calls=0;
    let result=stage.copy_artifact_observing(&mut source,fixture.owner,SnapshotArtifact::Memory,payload.len() as u64,&mut |role,copied,total| {
        calls+=1;
        assert_eq!(role,SnapshotArtifact::Memory);
        assert_eq!(copied,COPY_BUFFER_BYTES as u64);
        assert_eq!(total,payload.len() as u64);
        assert_eq!(fs::read(&partial).unwrap(),payload[..COPY_BUFFER_BYTES]);
        Err(RuntimeError::Io(io::Error::other("controlled block boundary failure")))
    });
    assert!(result.is_err());
    assert_eq!(calls,1);
    assert!(stage.measurements().is_empty());
    assert_eq!(fs::read(fixture.root.path().join("observed-source")).unwrap(),payload);
    assert!(!fixture.store.join(key_name("capture-",&key)).exists());
}

#[test]
fn rejects_hardlinked_non_private_wrong_owner_empty_and_over_budget_sources() {
    let fixture = Fixture::new();
    let mut stage = fixture.open().begin([3; 32]).unwrap();
    let mut source = fixture.source("source", b"payload");
    let role = SnapshotArtifact::State;
    assert!(
        stage
            .copy_artifact(&mut source, fixture.owner, role, 6)
            .is_err()
    );
    assert!(
        stage
            .copy_artifact(&mut source, fixture.owner.wrapping_add(1), role, 100)
            .is_err()
    );
    source
        .set_permissions(Permissions::from_mode(0o644))
        .unwrap();
    assert!(
        stage
            .copy_artifact(&mut source, fixture.owner, role, 100)
            .is_err()
    );
    source
        .set_permissions(Permissions::from_mode(0o600))
        .unwrap();
    fs::hard_link(
        fixture.root.path().join("source"),
        fixture.root.path().join("alias"),
    )
    .unwrap();
    assert!(
        stage
            .copy_artifact(&mut source, fixture.owner, role, 100)
            .is_err()
    );
    let mut empty = fixture.source("empty", b"");
    assert!(
        stage
            .copy_artifact(&mut empty, fixture.owner, role, 100)
            .is_err()
    );
    assert!(stage.measurements().is_empty());
}

#[test]
fn rejects_symlink_ancestor_and_non_private_store() {
    let fixture = Fixture::new();
    let link = fixture.root.path().join("alias");
    symlink(&fixture.store, &link).unwrap();
    assert!(SnapshotStore::open_for_owner(&link, fixture.owner).is_err());
    fs::set_permissions(&fixture.store, Permissions::from_mode(0o755)).unwrap();
    assert!(SnapshotStore::open_for_owner(&fixture.store, fixture.owner).is_err());
}

#[test]
fn missing_or_forged_measurements_cannot_publish() {
    let fixture = Fixture::new();
    let key = [4; 32];
    let stage = fixture.full_stage(key);
    let mut expected = stage.measurements();
    expected[0].sha256[0] ^= 1;
    assert!(stage.publish(b"bound manifest", &expected).is_err());
    assert!(!fixture.store.join(key_name("capture-", &key)).exists());
    assert!(fixture.store.join(key_name("staging-", &key)).exists());
}

#[test]
fn missing_disk_cannot_publish_and_duplicate_role_is_rejected() {
    let fixture = Fixture::new();
    let key = [5; 32];
    let mut stage = fixture.open().begin(key).unwrap();
    let mut source = fixture.source("source", b"actual bytes");
    stage
        .copy_artifact(&mut source, fixture.owner, SnapshotArtifact::State, 100)
        .unwrap();
    assert!(
        stage
            .copy_artifact(&mut source, fixture.owner, SnapshotArtifact::State, 100)
            .is_err()
    );
    stage
        .copy_artifact(&mut source, fixture.owner, SnapshotArtifact::Memory, 100)
        .unwrap();
    let expected = stage.measurements();
    assert!(stage.publish(b"manifest", &expected).is_err());
}

#[test]
fn replacement_or_modification_of_copied_file_cannot_publish() {
    for replace in [false, true] {
        let fixture = Fixture::new();
        let key = [6; 32];
        let stage = fixture.full_stage(key);
        let expected = stage.measurements();
        let path = fixture
            .store
            .join(key_name("staging-", &key))
            .join("state.bin");
        if replace {
            fs::rename(&path, path.with_extension("old")).unwrap();
            fs::write(&path, b"substitute").unwrap();
        } else {
            fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
            fs::write(&path, b"changed after copy").unwrap();
        }
        assert!(stage.publish(b"manifest", &expected).is_err());
        assert!(!fixture.store.join(key_name("capture-", &key)).exists());
    }
}

#[test]
fn store_must_not_overlap_runtime_even_with_parent_components() {
    let fixture = Fixture::new();
    let store = fixture.open();
    assert!(
        store
            .require_separate_from(&fixture.root.path().join("v"))
            .is_ok()
    );
    assert!(store.require_separate_from(&fixture.store).is_err());
    assert!(
        store
            .require_separate_from(&fixture.store.join("v"))
            .is_err()
    );
    assert!(store.require_separate_from(fixture.root.path()).is_err());
    assert!(
        store
            .require_separate_from(&fixture.store.join("subdir/.."))
            .is_err()
    );
}
