use super::*;
use crate::snapshot_store::SnapshotStage;
use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
};

struct Fixture {
    temporary: tempfile::TempDir,
    store: SnapshotStore,
    key: [u8; 32],
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let root = temporary.path().join("store");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let owner = rustix::process::geteuid().as_raw();
        let store = SnapshotStore::open_for_owner(&root, owner).unwrap();
        let key = [19; 32];
        let mut stage: SnapshotStage = store.begin(key).unwrap();
        for (index, role) in [
            SnapshotArtifact::State,
            SnapshotArtifact::Memory,
            SnapshotArtifact::Disk(0),
        ]
        .into_iter()
        .enumerate()
        {
            let path = temporary.path().join(format!("source-{index}"));
            let mut source = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .unwrap();
            source
                .write_all(&[u8::try_from(index + 1).unwrap(); 17])
                .unwrap();
            stage.copy_artifact(&mut source, owner, role, 17).unwrap();
        }
        let measured = stage.measurements();
        stage.publish(b"actual manifest bytes", &measured).unwrap();
        Self {
            temporary,
            store,
            key,
        }
    }
    fn bundle_path(&self) -> std::path::PathBuf {
        self.store.path.join(key_name("capture-", &self.key))
    }
    fn open(&self) -> Result<SnapshotBundle, RuntimeError> {
        self.store.open_bundle(
            self.key,
            BundleReadLimits {
                state_bytes: 17,
                memory_bytes: 17,
                disk_bytes: 17,
            },
        )
    }
    fn writable_directory(&self) {
        fs::set_permissions(self.bundle_path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    fn seal_directory(&self) {
        fs::set_permissions(self.bundle_path(), fs::Permissions::from_mode(0o500)).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for entry in fs::read_dir(&self.store.path).unwrap().flatten() {
            if entry.file_type().unwrap().is_dir() {
                fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
    }
}

#[test]
fn reopens_original_bytes_and_hashes_without_republishing_or_changing_modes() {
    let fixture = Fixture::new();
    let bundle = fixture.open().unwrap();
    assert_eq!(bundle.manifest_bytes(), b"actual manifest bytes");
    assert_eq!(
        bundle.manifest_sha256(),
        <[u8; 32]>::from(Sha256::digest(b"actual manifest bytes"))
    );
    for (index, file) in bundle.measurements().iter().enumerate() {
        assert_eq!(file.length, 17);
        assert_eq!(
            file.sha256,
            <[u8; 32]>::from(Sha256::digest([u8::try_from(index + 1).unwrap(); 17]))
        );
        assert_eq!(
            fs::metadata(fixture.bundle_path().join(file.artifact.relative_path()))
                .unwrap()
                .mode()
                & 0o777,
            0o400
        );
    }
    bundle.verify_unchanged().unwrap();
    drop(bundle);
    assert!(fixture.bundle_path().join("memory.bin").exists());
    assert!(
        fixture
            .store
            .open_bundle(
                [20; 32],
                BundleReadLimits {
                    state_bytes: 17,
                    memory_bytes: 17,
                    disk_bytes: 17
                }
            )
            .is_err()
    );
    assert_eq!(fs::read_dir(&fixture.store.path).unwrap().count(), 1);
}

#[test]
fn extra_missing_symlink_and_hardlink_files_are_rejected() {
    for variant in 0..4 {
        let fixture = Fixture::new();
        fixture.writable_directory();
        let memory = fixture.bundle_path().join("memory.bin");
        match variant {
            0 => {
                fs::write(fixture.bundle_path().join("extra"), b"unknown").unwrap();
            }
            1 => {
                fs::remove_file(memory).unwrap();
            }
            2 => {
                fs::remove_file(&memory).unwrap();
                symlink(fixture.temporary.path().join("source-1"), memory).unwrap();
            }
            3 => {
                fs::hard_link(memory, fixture.temporary.path().join("extra-link")).unwrap();
            }
            _ => unreachable!(),
        }
        fixture.seal_directory();
        assert!(fixture.open().is_err());
    }
}

#[test]
fn loose_mode_or_byte_budget_cannot_be_accepted_as_a_sealed_source() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .store
            .open_bundle(
                fixture.key,
                BundleReadLimits {
                    state_bytes: 17,
                    memory_bytes: 16,
                    disk_bytes: 17
                }
            )
            .is_err()
    );
    fs::set_permissions(
        fixture.bundle_path().join("disk-0.img"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(fixture.open().is_err());
}

#[test]
fn retained_original_file_and_directory_identities_reject_later_replacement() {
    let fixture = Fixture::new();
    let bundle = fixture.open().unwrap();
    fixture.writable_directory();
    fs::rename(
        fixture.bundle_path().join("manifest.json"),
        fixture.bundle_path().join("old-manifest"),
    )
    .unwrap();
    fs::write(
        fixture.bundle_path().join("manifest.json"),
        b"actual manifest bytes",
    )
    .unwrap();
    fixture.seal_directory();
    assert!(bundle.verify_unchanged().is_err());

    let fixture = Fixture::new();
    let bundle = fixture.open().unwrap();
    fs::rename(fixture.bundle_path(), fixture.store.path.join("old-bundle")).unwrap();
    fs::create_dir(fixture.bundle_path()).unwrap();
    fixture.seal_directory();
    assert!(bundle.verify_unchanged().is_err());
}

#[test]
fn original_content_mutation_is_not_hidden_by_a_retained_descriptor() {
    let fixture = Fixture::new();
    let bundle = fixture.open().unwrap();
    let path = fixture.bundle_path().join("memory.bin");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, b"different content").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    assert!(bundle.verify_unchanged().is_err());
}

#[test]
fn private_restore_copies_preserve_sealed_sources_and_are_independent() {
    let fixture = Fixture::new();
    let bundle = fixture.open().unwrap();
    let private = fixture.temporary.path().join("private-jail");
    fs::create_dir(&private).unwrap();
    let directory = File::open(&private).unwrap();
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    for role in [
        SnapshotArtifact::State,
        SnapshotArtifact::Memory,
        SnapshotArtifact::Disk(0),
    ] {
        bundle.copy_for_restore(&directory, role, uid, gid).unwrap();
    }
    for (name, bytes, mode) in [
        ("restore-state.bin", 1, 0o400),
        ("restore-memory.bin", 2, 0o400),
        ("rootfs.ext4", 3, 0o600),
    ] {
        let metadata = fs::metadata(private.join(name)).unwrap();
        assert_eq!(fs::read(private.join(name)).unwrap(), vec![bytes; 17]);
        assert_eq!(metadata.mode() & 0o777, mode);
        assert_eq!(metadata.uid(), uid);
        assert_eq!(metadata.nlink(), 1);
    }
    fs::write(private.join("rootfs.ext4"), b"new guest writes").unwrap();
    assert_eq!(
        fs::read(fixture.bundle_path().join("disk-0.img")).unwrap(),
        vec![3; 17]
    );
    bundle.verify_unchanged().unwrap();
    assert!(
        bundle
            .copy_for_restore(&directory, SnapshotArtifact::Disk(0), uid, gid)
            .is_err()
    );
    assert_eq!(
        fs::read(private.join("rootfs.ext4")).unwrap(),
        b"new guest writes"
    );
}

#[test]
fn restore_copy_rejects_substituted_source_and_destination_symlink() {
    let fixture = Fixture::new();
    let bundle = fixture.open().unwrap();
    let private = fixture.temporary.path().join("private-jail");
    fs::create_dir(&private).unwrap();
    let directory = File::open(&private).unwrap();
    let sentinel = fixture.temporary.path().join("sentinel");
    fs::write(&sentinel, b"unchanged").unwrap();
    symlink(&sentinel, private.join("restore-memory.bin")).unwrap();
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    assert!(
        bundle
            .copy_for_restore(&directory, SnapshotArtifact::Memory, uid, gid)
            .is_err()
    );
    assert_eq!(fs::read(&sentinel).unwrap(), b"unchanged");
    fixture.writable_directory();
    fs::rename(
        fixture.bundle_path().join("state.bin"),
        fixture.bundle_path().join("old-state"),
    )
    .unwrap();
    fs::write(fixture.bundle_path().join("state.bin"), vec![1; 17]).unwrap();
    fixture.seal_directory();
    assert!(
        bundle
            .copy_for_restore(&directory, SnapshotArtifact::State, uid, gid)
            .is_err()
    );
    assert!(!private.join("restore-state.bin").exists());
}

#[test]
fn exact_bundle_retirement_is_restartable_and_never_deletes_a_replacement() {
    let fixture = Fixture::new();
    let bundle = fixture.open().unwrap();
    let plan = bundle.retirement_plan().unwrap();
    let plan: crate::snapshot_store::BundleRetirement =
        serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    assert_eq!(plan.directory_key(), &fixture.key);
    assert_eq!(plan.manifest_sha256(), &bundle.manifest_sha256());
    // A service crash after unlinking one exact original file resumes using
    // its original persisted plan, without reopening an incomplete Full.
    fixture.writable_directory();
    fs::remove_file(fixture.bundle_path().join("state.bin")).unwrap();
    fixture.store.retire_bundle(&plan).unwrap();
    assert!(!fixture.bundle_path().exists());
    fixture.store.retire_bundle(&plan).unwrap();
    fs::create_dir(fixture.bundle_path()).unwrap();
    fixture.seal_directory();
    assert!(fixture.store.retire_bundle(&plan).is_err());
    assert!(fixture.bundle_path().is_dir());
}

#[test]
fn retirement_refuses_unknown_files_changed_inodes_and_other_store() {
    for variant in 0..3 {
        let fixture = Fixture::new();
        let bundle = fixture.open().unwrap();
        let plan = bundle.retirement_plan().unwrap();
        fixture.writable_directory();
        match variant {
            0 => fs::write(fixture.bundle_path().join("unknown"), b"keep").unwrap(),
            1 => {
                fs::remove_file(fixture.bundle_path().join("memory.bin")).unwrap();
                fs::write(fixture.bundle_path().join("memory.bin"), vec![2; 17]).unwrap();
                fs::set_permissions(
                    fixture.bundle_path().join("memory.bin"),
                    fs::Permissions::from_mode(0o400),
                )
                .unwrap();
            }
            2 => {
                let other = Fixture::new();
                assert!(other.store.retire_bundle(&plan).is_err());
            }
            _ => unreachable!(),
        }
        fixture.seal_directory();
        if variant != 2 {
            assert!(fixture.store.retire_bundle(&plan).is_err());
        }
        assert!(fixture.bundle_path().join("manifest.json").exists());
        assert!(fixture.bundle_path().join("disk-0.img").exists());
    }
}

#[test]
fn retirement_also_reclaims_the_exact_completion_and_interrupted_stage() {
    let fixture = Fixture::new();
    fixture
        .store
        .publish_completion(fixture.key, b"exact original worker completion")
        .unwrap();
    let stage = fixture
        .store
        .path
        .join(key_name("completion-stage-", &fixture.key));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stage)
        .unwrap();
    file.write_all(b"partial original completion").unwrap();
    file.sync_all().unwrap();
    let plan = fixture.open().unwrap().retirement_plan().unwrap();
    fixture.store.retire_bundle(&plan).unwrap();
    assert_eq!(fs::read_dir(&fixture.store.path).unwrap().count(), 0);
    fixture.store.retire_bundle(&plan).unwrap();
}
