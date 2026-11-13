use super::*;
use std::{
    fs as disk,
    io::Write,
    os::unix::fs::{OpenOptionsExt, symlink},
};

struct Fixture {
    root: tempfile::TempDir,
    store: SnapshotStore,
    key: [u8; 32],
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        disk::set_permissions(root.path(), Permissions::from_mode(0o700)).unwrap();
        let store = SnapshotStore::open_for_owner(root.path(), rustix::process::geteuid().as_raw())
            .unwrap();
        let key = [23; 32];
        let stage = store.begin(key).unwrap();
        drop(stage);
        for name in ["state.bin", "memory.bin"] {
            Self::write(
                &root.path().join(key_name("staging-", &key)).join(name),
                b"partial",
            );
        }
        Self { root, store, key }
    }
    fn write(path: &std::path::Path, bytes: &[u8]) {
        let mut file = disk::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
    }
    fn stage(&self) -> std::path::PathBuf {
        self.root.path().join(key_name("staging-", &self.key))
    }
}
#[test]
fn never_sealed_partial_files_and_both_marker_stages_retire_with_same_saved_plan() {
    let fixture = Fixture::new();
    for prefix in MARKERS {
        Fixture::write(
            &fixture.root.path().join(key_name(prefix, &fixture.key)),
            b"",
        );
    }
    Fixture::write(&fixture.root.path().join("unrelated"), b"preserve");
    let plan = fixture.store.capture_retirement_plan(fixture.key).unwrap();
    let plan: CaptureStoreRetirement =
        serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    disk::remove_file(fixture.stage().join("state.bin")).unwrap();
    disk::remove_file(fixture.root.path().join(key_name(MARKERS[0], &fixture.key))).unwrap();
    fixture.store.retire_capture_material(&plan).unwrap();
    fixture.store.retire_capture_material(&plan).unwrap();
    assert!(!fixture.stage().exists());
    assert_eq!(
        disk::read(fixture.root.path().join("unrelated")).unwrap(),
        b"preserve"
    );
    assert_eq!(disk::read_dir(fixture.root.path()).unwrap().count(), 1);
}
#[test]
fn changed_files_foreign_entries_and_late_output_are_rejected_before_first_unlink() {
    for variant in 0..6 {
        let fixture = Fixture::new();
        let plan = fixture.store.capture_retirement_plan(fixture.key).unwrap();
        let memory = fixture.stage().join("memory.bin");
        match variant {
            0 => Fixture::write(&fixture.stage().join("disk-0.img"), b"late"),
            1 => {
                disk::rename(&memory, fixture.root.path().join("original")).unwrap();
                Fixture::write(&memory, b"partial");
            }
            2 => {
                disk::remove_file(&memory).unwrap();
                symlink("state.bin", &memory).unwrap();
            }
            3 => disk::hard_link(&memory, fixture.root.path().join("alias")).unwrap(),
            4 => Fixture::write(&fixture.stage().join("foreign"), b"keep"),
            5 => Fixture::write(
                &fixture
                    .root
                    .path()
                    .join(key_name("api-returned-", &fixture.key)),
                b"late response",
            ),
            _ => unreachable!(),
        }
        assert!(fixture.store.retire_capture_material(&plan).is_err());
        assert_eq!(
            disk::read(fixture.stage().join("state.bin")).unwrap(),
            b"partial"
        );
        assert!(memory.symlink_metadata().is_ok());
    }
}
#[test]
fn replaced_directory_and_foreign_store_are_never_adopted_after_partial_cleanup() {
    let fixture = Fixture::new();
    let plan = fixture.store.capture_retirement_plan(fixture.key).unwrap();
    let other = Fixture::new();
    assert!(other.store.retire_capture_material(&plan).is_err());
    disk::rename(fixture.stage(), fixture.root.path().join("original-stage")).unwrap();
    disk::create_dir(fixture.stage()).unwrap();
    disk::set_permissions(fixture.stage(), Permissions::from_mode(0o700)).unwrap();
    assert!(fixture.store.retire_capture_material(&plan).is_err());
    assert!(
        fixture
            .root
            .path()
            .join("original-stage/state.bin")
            .exists()
    );
    assert!(fixture.stage().exists());
}
#[test]
fn absent_output_plan_rejects_new_directory_or_file_instead_of_adopting_it() {
    let fixture = Fixture::new();
    fixture
        .store
        .retire_capture_material(&fixture.store.capture_retirement_plan(fixture.key).unwrap())
        .unwrap();
    let plan = fixture.store.capture_retirement_plan(fixture.key).unwrap();
    let stage = fixture.store.begin(fixture.key).unwrap();
    assert!(fixture.store.retire_capture_material(&plan).is_err());
    drop(stage);
    assert!(fixture.stage().exists());
}
