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
fn clone_uses_independent_inodes_with_bidirectional_write_and_unlink_isolation() {
    let fixture = Fixture::new();
    let payload = b"original independent contents";
    let mut source = fixture.source("clone-source", payload);
    let mut target = fixture.source("clone-target", b"");
    let directory = File::open(fixture.root.path()).unwrap();
    require_same_xfs_mount(&directory, &source).unwrap();
    require_same_xfs_mount(&directory, &target).unwrap();
    let source_before = FileStamp::from_metadata(&source.metadata().unwrap());
    let target_before = target.metadata().unwrap();
    let source_position = source.stream_position().unwrap();
    clone_whole_file(&source, &target).unwrap();
    assert_eq!(
        FileStamp::from_metadata(&source.metadata().unwrap()),
        source_before
    );
    assert_eq!(source.stream_position().unwrap(), source_position);
    assert_eq!(target.stream_position().unwrap(), 0);
    let cloned = target.metadata().unwrap();
    assert_eq!(
        (cloned.dev(), cloned.ino()),
        (target_before.dev(), target_before.ino())
    );
    assert_ne!(source_before.inode, cloned.ino());
    assert_eq!((cloned.nlink(), cloned.mode() & 0o777), (1, 0o600));
    assert_eq!(cloned.len(), payload.len() as u64);
    source.seek(SeekFrom::Start(0)).unwrap();
    source.write_all(b"S").unwrap();
    assert_eq!(
        fs::read(fixture.root.path().join("clone-target")).unwrap(),
        payload
    );
    target.write_all(b"T").unwrap();
    let mut expected_source = payload.to_vec();
    expected_source[0] = b'S';
    let mut expected_target = payload.to_vec();
    expected_target[0] = b'T';
    assert_eq!(
        fs::read(fixture.root.path().join("clone-source")).unwrap(),
        expected_source
    );
    fs::remove_file(fixture.root.path().join("clone-source")).unwrap();
    assert_eq!(
        fs::read(fixture.root.path().join("clone-target")).unwrap(),
        expected_target
    );
    fs::remove_file(fixture.root.path().join("clone-target")).unwrap();
    source.seek(SeekFrom::Start(0)).unwrap();
    let mut still_open = Vec::new();
    source.read_to_end(&mut still_open).unwrap();
    assert_eq!(still_open, expected_source);
}

#[test]
fn whole_clone_preserves_unaligned_lengths_and_sparse_logical_bytes() {
    let fixture = Fixture::new();
    for length in [1_u64, 10_003, 2 * 1024 * 1024 + 17] {
        let mut source = fixture.source(&format!("sparse-source-{length}"), b"");
        let target = fixture.source(&format!("sparse-target-{length}"), b"");
        source.set_len(length).unwrap();
        source.write_all(b"A").unwrap();
        source.seek(SeekFrom::Start(length - 1)).unwrap();
        source.write_all(b"Z").unwrap();
        let before = FileStamp::from_metadata(&source.metadata().unwrap());
        clone_whole_file(&source, &target).unwrap();
        assert_eq!(
            FileStamp::from_metadata(&source.metadata().unwrap()),
            before
        );
        let left = fs::read(fixture.root.path().join(format!("sparse-source-{length}"))).unwrap();
        let right = fs::read(fixture.root.path().join(format!("sparse-target-{length}"))).unwrap();
        assert_eq!(left, right);
        assert_eq!(target.metadata().unwrap().len(), length);
        if length > 1024 * 1024 {
            assert!(source.metadata().unwrap().blocks() * 512 < length / 2);
            assert!(target.metadata().unwrap().blocks() * 512 < length / 2);
        }
    }
}

#[test]
fn clone_rejects_aliases_links_nonprivate_and_nonempty_files_without_truncating_them() {
    for defect in [
        "empty-source",
        "nonempty-target",
        "source-link",
        "target-link",
        "source-mode",
        "target-mode",
        "same-inode",
    ] {
        let fixture = Fixture::new();
        let source = fixture.source(
            "checked-source",
            if defect == "empty-source" {
                b""
            } else {
                b"source"
            },
        );
        let target = fixture.source(
            "checked-target",
            if defect == "nonempty-target" {
                b"keep target"
            } else {
                b""
            },
        );
        match defect {
            "source-link" => fs::hard_link(
                fixture.root.path().join("checked-source"),
                fixture.root.path().join("alias"),
            )
            .unwrap(),
            "target-link" => fs::hard_link(
                fixture.root.path().join("checked-target"),
                fixture.root.path().join("alias"),
            )
            .unwrap(),
            "source-mode" => source
                .set_permissions(Permissions::from_mode(0o644))
                .unwrap(),
            "target-mode" => target
                .set_permissions(Permissions::from_mode(0o400))
                .unwrap(),
            _ => {}
        }
        let source_before = FileStamp::from_metadata(&source.metadata().unwrap());
        let target_before = FileStamp::from_metadata(&target.metadata().unwrap());
        let result = clone_whole_file(
            &source,
            if defect == "same-inode" {
                &source
            } else {
                &target
            },
        );
        assert!(
            matches!(result, Err(RuntimeError::IdentityMismatch(_))),
            "{defect}"
        );
        assert_eq!(
            FileStamp::from_metadata(&source.metadata().unwrap()),
            source_before,
            "{defect}"
        );
        assert_eq!(
            FileStamp::from_metadata(&target.metadata().unwrap()),
            target_before,
            "{defect}"
        );
    }
}

#[test]
fn read_only_destination_returns_real_clone_error_and_retains_both_files() {
    let fixture = Fixture::new();
    let source = fixture.source("readable-source", b"source bytes");
    drop(fixture.source("read-only-fd", b""));
    let target = File::open(fixture.root.path().join("read-only-fd")).unwrap();
    let before = FileStamp::from_metadata(&source.metadata().unwrap());
    let RuntimeError::Io(error) = clone_whole_file(&source, &target).unwrap_err() else {
        panic!("valid metadata must reach the real FICLONE descriptor check");
    };
    assert_eq!(
        error.raw_os_error(),
        Some(rustix::io::Errno::BADF.raw_os_error())
    );
    assert_eq!(target.metadata().unwrap().len(), 0);
    assert_eq!(
        FileStamp::from_metadata(&source.metadata().unwrap()),
        before
    );
}

#[test]
fn cross_device_clone_is_rejected_without_read_write_fallback() {
    let fixture = Fixture::new();
    let source = fixture.source("cross-device-source", b"original data");
    let other = tempfile::tempdir_in("/dev/shm").unwrap();
    let target = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(other.path().join("target"))
        .unwrap();
    assert_ne!(
        source.metadata().unwrap().dev(),
        target.metadata().unwrap().dev(),
        "this Linux check requires the configured data FS and independent tmpfs"
    );
    assert!(matches!(
        clone_whole_file(&source, &target),
        Err(RuntimeError::IdentityMismatch(_))
    ));
    assert_eq!(target.metadata().unwrap().len(), 0);
    assert_eq!(
        fs::read(fixture.root.path().join("cross-device-source")).unwrap(),
        b"original data"
    );
}

#[test]
fn non_xfs_clone_is_rejected_before_content_creation() {
    let root = tempfile::tempdir_in("/dev/shm").unwrap();
    let mut source = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(root.path().join("source"))
        .unwrap();
    let target = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(root.path().join("target"))
        .unwrap();
    source
        .write_all(b"unsupported FS retains the original")
        .unwrap();
    let before = FileStamp::from_metadata(&source.metadata().unwrap());
    assert!(matches!(
        clone_whole_file(&source, &target),
        Err(RuntimeError::IdentityMismatch(_))
    ));
    assert_eq!(target.metadata().unwrap().len(), 0);
    assert_eq!(
        FileStamp::from_metadata(&source.metadata().unwrap()),
        before
    );
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
fn actual_clone_size_limit_error_child() {
    let Some(root) = std::env::var_os("SANDPROOF_UNIT_CLONE_FSIZE_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let owner = rustix::process::geteuid().as_raw();
    let store = SnapshotStore::open_for_owner(&root.join("store"), owner).unwrap();
    let mut source = File::open(root.join("large-source")).unwrap();
    let original = FileStamp::from_metadata(&source.metadata().unwrap());
    let key = [27; 32];
    let mut stage = store.begin(key).unwrap();
    let result = stage.copy_artifact(&mut source, owner, SnapshotArtifact::Memory, 196_608);
    let RuntimeError::Io(error) = result.unwrap_err() else {
        panic!("expected actual FICLONE size-limit failure");
    };
    // The fixed Linux/XFS operator sample returns EINVAL from whole FICLONE
    // under this exact limit, before any target bytes become established.
    assert_eq!(
        error.raw_os_error(),
        Some(rustix::io::Errno::INVAL.raw_os_error())
    );
    assert!(
        stage.measurements().is_empty(),
        "a failed clone is not a completed artifact"
    );
    drop(stage);
    let length = fs::metadata(
        root.join("store")
            .join(key_name("staging-", &key))
            .join("memory.bin"),
    )
    .unwrap()
    .len();
    assert_eq!(
        length, 0,
        "the actual clone size-limit failure precedes content creation"
    );
    assert_eq!(
        FileStamp::from_metadata(&source.metadata().unwrap()),
        original
    );
    assert!(!root.join("store").join(key_name("capture-", &key)).exists());
    assert!(
        store.begin(key).is_err(),
        "same original partial work cannot be silently reused"
    );
}

#[test]
fn actual_clone_size_limit_failure_keeps_empty_target_and_original_source() {
    let fixture = Fixture::new();
    let payload = vec![b'L'; 196_608];
    drop(fixture.source("large-source", &payload));
    // Keep the limit/ignored signal local to this exact child. CoW has no
    // userspace mid-write boundary: check the real ioctl refusal, not a fake
    // partial-copy count or an assumed EFBIG from the old write loop.
    let output = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            "trap '' XFSZ; ulimit -f 128; exec \"$@\"",
            "clone-fsize",
        ])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "snapshot_store::tests::actual_clone_size_limit_error_child",
            "--nocapture",
        ])
        .env("SANDPROOF_UNIT_CLONE_FSIZE_ROOT", fixture.root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(fixture.root.path().join("large-source")).unwrap(),
        payload
    );
    assert_eq!(
        fixture
            .store
            .join(key_name("staging-", &[27; 32]))
            .join("memory.bin")
            .metadata()
            .unwrap()
            .len(),
        0
    );
}

#[test]
#[ignore = "requires root and independently prepared source/store XFS project quotas"]
fn actual_project_quota_clone_failure_keeps_original_source_and_staging() {
    assert_eq!(rustix::process::geteuid().as_raw(), 0, "root required");
    let source_path = std::path::PathBuf::from(
        std::env::var_os("SANDPROOF_UNIT_CLONE_QUOTA_SOURCE")
            .expect("explicit prepared quota source required"),
    );
    let store_path = std::path::PathBuf::from(
        std::env::var_os("SANDPROOF_UNIT_CLONE_QUOTA_STORE")
            .expect("explicit prepared quota store required"),
    );
    assert!(source_path.is_absolute() && store_path.is_absolute());
    assert_eq!(source_path.canonicalize().unwrap(), source_path);
    assert_eq!(store_path.canonicalize().unwrap(), store_path);
    assert!(fs::read_dir(&store_path).unwrap().next().is_none());
    let store = SnapshotStore::open(&store_path).unwrap();
    let mut source = File::from(
        open(
            &source_path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap(),
    );
    let original = source.metadata().unwrap();
    assert_eq!(original.uid(), 0);
    assert_eq!(original.len(), 2 * 1024 * 1024);
    assert!(
        original.blocks() * 512 >= original.len(),
        "dense source required"
    );
    let source_before = FileStamp::from_metadata(&original);
    let digest_before: [u8; 32] = Sha256::digest(fs::read(&source_path).unwrap()).into();
    let key = [33; 32];
    let partial = store_path.join(key_name("staging-", &key));
    let mut stage = store.begin(key).unwrap();
    let staging_before = fs::symlink_metadata(&partial).unwrap();
    let result = stage.copy_artifact(&mut source, 0, SnapshotArtifact::Memory, original.len());
    let RuntimeError::Io(error) = result.unwrap_err() else {
        panic!("expected actual kernel project space/quota refusal");
    };
    let errno = error.raw_os_error().expect("actual kernel errno required");
    assert!(
        errno == rustix::io::Errno::NOSPC.raw_os_error()
            || errno == rustix::io::Errno::DQUOT.raw_os_error(),
        "expected ENOSPC or EDQUOT, got {errno}"
    );
    assert!(stage.measurements().is_empty());
    assert_eq!(
        FileStamp::from_metadata(&source.metadata().unwrap()),
        source_before
    );
    assert_eq!(
        FileStamp::from_metadata(&fs::symlink_metadata(&source_path).unwrap()),
        source_before
    );
    let digest_after: [u8; 32] = Sha256::digest(fs::read(&source_path).unwrap()).into();
    assert_eq!(digest_after, digest_before);
    drop(stage);
    let retained = fs::symlink_metadata(&partial).unwrap();
    assert!(retained.is_dir());
    assert_eq!(
        (retained.dev(), retained.ino()),
        (staging_before.dev(), staging_before.ino())
    );
    let target = fs::symlink_metadata(partial.join("memory.bin")).unwrap();
    assert!(target.is_file());
    assert_eq!(
        target.len(),
        0,
        "this quota refusal precedes complete cloning"
    );
    assert_eq!(
        (target.uid(), target.nlink(), target.mode() & 0o777),
        (0, 1, 0o600)
    );
    assert_eq!(target.dev(), original.dev());
    assert_ne!(target.ino(), original.ino());
    assert!(!store_path.join(key_name("capture-", &key)).exists());
    assert!(!store_path.join(key_name("completed-", &key)).exists());
    assert!(
        matches!(store.begin(key), Err(RuntimeError::Io(error))
            if error.raw_os_error() == Some(rustix::io::Errno::EXIST.raw_os_error())),
        "same partial work cannot be adopted"
    );
    eprintln!(
        "quota clone refusal errno={errno} source_device={} source_inode={} source_bytes={} source_sha256={digest_after:?} target_inode={} target_bytes={}",
        original.dev(),
        original.ino(),
        original.len(),
        target.ino(),
        target.len()
    );
}

#[test]
fn real_parent_sync_error_after_publish_keeps_original_bundle_reopenable() {
    let fixture = Fixture::new();
    let key = [28; 32];
    let mut stage = fixture.full_stage(key);
    let measured = stage.measurements();
    let before = stage.parent.metadata().unwrap();
    // O_PATH is the SAME original directory inode: renameat is valid, but
    // fsync on this descriptor returns EBADF after the actual publication.
    // This is fixture-only FD substitution, never a production sync bypass.
    stage.parent = File::from(
        open(
            &fixture.store,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap(),
    );
    assert_eq!(
        (
            stage.parent.metadata().unwrap().dev(),
            stage.parent.metadata().unwrap().ino()
        ),
        (before.dev(), before.ino())
    );
    assert!(matches!(
        stage.publish(b"original bound manifest", &measured),
        Err(RuntimeError::Io(_))
    ));
    let bundle = fixture
        .open()
        .open_bundle(
            key,
            BundleReadLimits {
                state_bytes: 100,
                memory_bytes: 100,
                disk_bytes: 100,
            },
        )
        .unwrap();
    assert_eq!(bundle.manifest_bytes(), b"original bound manifest");
    assert_eq!(bundle.measurements(), measured);
    assert!(
        fixture.open().begin(key).is_err(),
        "visible publication must not be repeated"
    );
    for index in 0..3 {
        assert_eq!(
            fs::read(fixture.root.path().join(format!("source-{index}"))).unwrap(),
            b"actual bytes"
        );
    }
}

#[test]
fn measurement_observer_follows_full_clone_and_failure_never_seals_the_artifact() {
    let fixture = Fixture::new();
    let key = [29; 32];
    let payload = vec![7; COPY_BUFFER_BYTES * 2];
    let mut source = fixture.source("observed-source", &payload);
    let mut stage = fixture.open().begin(key).unwrap();
    let partial = fixture
        .store
        .join(key_name("staging-", &key))
        .join("memory.bin");
    let mut events = Vec::new();
    let result = stage.copy_artifact_observing(
        &mut source,
        fixture.owner,
        SnapshotArtifact::Memory,
        payload.len() as u64,
        &mut |role, event| {
            events.push(event);
            assert_eq!(role, SnapshotArtifact::Memory);
            if event == SnapshotArtifactEvent::BeforeClone {
                assert_eq!(partial.metadata().unwrap().len(), 0);
                return Ok(());
            }
            // The whole independent clone already exists, even though the
            // SHA-256 reader has not measured all logical bytes yet.
            assert_eq!(fs::read(&partial).unwrap(), payload);
            if event == SnapshotArtifactEvent::Cloned {
                return Ok(());
            }
            assert_eq!(
                event,
                SnapshotArtifactEvent::Measuring {
                    measured_bytes: COPY_BUFFER_BYTES as u64,
                    total_bytes: payload.len() as u64
                }
            );
            Err(RuntimeError::Io(io::Error::other(
                "controlled target measurement boundary failure",
            )))
        },
    );
    assert!(result.is_err());
    assert_eq!(
        events,
        vec![
            SnapshotArtifactEvent::BeforeClone,
            SnapshotArtifactEvent::Cloned,
            SnapshotArtifactEvent::Measuring {
                measured_bytes: COPY_BUFFER_BYTES as u64,
                total_bytes: payload.len() as u64
            }
        ]
    );
    assert!(stage.measurements().is_empty());
    assert_eq!(
        fs::read(fixture.root.path().join("observed-source")).unwrap(),
        payload
    );
    assert!(!fixture.store.join(key_name("capture-", &key)).exists());
}

#[test]
fn clone_measurement_and_sync_window_failures_retain_the_same_unsealed_work() {
    for stop in [
        SnapshotArtifactEvent::BeforeClone,
        SnapshotArtifactEvent::Cloned,
        SnapshotArtifactEvent::BeforeSync,
        SnapshotArtifactEvent::Synced,
    ] {
        let fixture = Fixture::new();
        let key = [30; 32];
        let payload = vec![9; COPY_BUFFER_BYTES * 2];
        let mut source = fixture.source("window-source", &payload);
        let before = FileStamp::from_metadata(&source.metadata().unwrap());
        let mut stage = fixture.open().begin(key).unwrap();
        let target = fixture
            .store
            .join(key_name("staging-", &key))
            .join("memory.bin");
        let result = stage.copy_artifact_observing(
            &mut source,
            fixture.owner,
            SnapshotArtifact::Memory,
            payload.len() as u64,
            &mut |_, event| {
                if event != stop {
                    return Ok(());
                }
                assert_eq!(
                    target.metadata().unwrap().len(),
                    if stop == SnapshotArtifactEvent::BeforeClone {
                        0
                    } else {
                        payload.len() as u64
                    }
                );
                if stop == SnapshotArtifactEvent::Synced {
                    assert_eq!(target.metadata().unwrap().mode() & 0o777, 0o400);
                }
                Err(RuntimeError::Io(io::Error::other(
                    "controlled real materialization boundary failure",
                )))
            },
        );
        assert!(result.is_err(), "{stop:?}");
        assert!(stage.measurements().is_empty());
        drop(stage);
        assert_eq!(
            FileStamp::from_metadata(&source.metadata().unwrap()),
            before
        );
        assert!(target.exists());
        assert!(!fixture.store.join(key_name("capture-", &key)).exists());
        assert!(fixture.open().begin(key).is_err());
    }
}

#[test]
fn source_or_clone_mutation_during_measurement_or_sync_prevents_sealing() {
    for mutate_source in [true, false] {
        for stop in [
            SnapshotArtifactEvent::BeforeClone,
            SnapshotArtifactEvent::Cloned,
            SnapshotArtifactEvent::BeforeSync,
            SnapshotArtifactEvent::Synced,
        ] {
            let fixture = Fixture::new();
            let key = [31; 32];
            let payload = vec![8; COPY_BUFFER_BYTES * 2];
            let mut source = fixture.source("mutable-source", &payload);
            let mut stage = fixture.open().begin(key).unwrap();
            let target = fixture
                .store
                .join(key_name("staging-", &key))
                .join("memory.bin");
            let result = stage.copy_artifact_observing(
                &mut source,
                fixture.owner,
                SnapshotArtifact::Memory,
                payload.len() as u64,
                &mut |_, event| {
                    if event == stop {
                        let path = if mutate_source {
                            fixture.root.path().join("mutable-source")
                        } else {
                            target.clone()
                        };
                        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
                        let mut writer = OpenOptions::new().write(true).open(path).unwrap();
                        writer.write_all(b"X").unwrap();
                        // Deterministic identity/length change; do not rely on
                        // the filesystem timestamp clock advancing in this test.
                        writer.set_len(payload.len() as u64 - 1).unwrap();
                    }
                    Ok(())
                },
            );
            assert!(
                matches!(result, Err(RuntimeError::IdentityMismatch(_))),
                "{mutate_source}/{stop:?}"
            );
            assert!(stage.measurements().is_empty());
            assert!(!fixture.store.join(key_name("capture-", &key)).exists());
            if mutate_source {
                assert_eq!(
                    fs::read(target).unwrap(),
                    if stop == SnapshotArtifactEvent::BeforeClone {
                        Vec::new()
                    } else {
                        payload
                    }
                );
            } else {
                assert_eq!(
                    fs::read(fixture.root.path().join("mutable-source")).unwrap(),
                    payload
                );
            }
        }
    }
}

#[test]
fn second_artifact_kernel_clone_failure_keeps_the_first_and_all_material_responsibility() {
    let fixture = Fixture::new();
    let key = [32; 32];
    let mut stage = fixture.open().begin(key).unwrap();
    let mut state_source = fixture.source("first-state", b"original state");
    stage
        .copy_artifact(
            &mut state_source,
            fixture.owner,
            SnapshotArtifact::State,
            100,
        )
        .unwrap();
    drop(fixture.source("second-memory", b"original memory"));
    let mut write_only = OpenOptions::new()
        .write(true)
        .open(fixture.root.path().join("second-memory"))
        .unwrap();
    assert!(matches!(
        stage.copy_artifact(
            &mut write_only,
            fixture.owner,
            SnapshotArtifact::Memory,
            100
        ),
        Err(RuntimeError::Io(_))
    ));
    assert_eq!(stage.measurements().len(), 1);
    let measured = stage.measurements();
    assert!(
        stage
            .publish(b"incomplete bundle cannot publish", &measured)
            .is_err()
    );
    let partial = fixture.store.join(key_name("staging-", &key));
    assert_eq!(
        fs::read(partial.join("state.bin")).unwrap(),
        b"original state"
    );
    assert_eq!(partial.join("memory.bin").metadata().unwrap().len(), 0);
    assert_eq!(
        fs::read(fixture.root.path().join("second-memory")).unwrap(),
        b"original memory"
    );
    assert!(!fixture.store.join(key_name("capture-", &key)).exists());
    assert!(fixture.open().begin(key).is_err());
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
