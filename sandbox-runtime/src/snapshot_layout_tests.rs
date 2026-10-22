use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};

fn fixture() -> (tempfile::TempDir, u32) {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    (root, rustix::process::geteuid().as_raw())
}
#[test]
fn initialize_then_reopen_populated_original_layout() {
    let (root, owner) = fixture();
    let original = SnapshotLayout::initialize_for_owner(root.path(), [1; 32], owner).unwrap();
    fs::write(original.runtime_root().join("existing-vm"), b"original").unwrap();
    fs::write(original.store_root().join("existing-bundle"), b"retained").unwrap();
    let recorded = original.record.clone();
    drop(original);
    let recovered = SnapshotLayout::reopen_for_owner(root.path(), [1; 32], owner).unwrap();
    assert_eq!(recovered.record, recorded);
    assert!(
        recovered
            .store_for_runtime(&recovered.runtime_root())
            .is_ok()
    );
    assert_eq!(
        fs::read(recovered.runtime_root().join("existing-vm")).unwrap(),
        b"original"
    );
    assert_eq!(
        fs::read(recovered.store_root().join("existing-bundle")).unwrap(),
        b"retained"
    );
    assert!(SnapshotLayout::initialize_for_owner(root.path(), [1; 32], owner).is_err());
}
#[test]
fn unknown_partial_initialize_is_not_adopted_or_automatically_retried() {
    let (root, owner) = fixture();
    fs::create_dir(root.path().join("v")).unwrap();
    assert!(SnapshotLayout::initialize_for_owner(root.path(), [2; 32], owner).is_err());
    assert!(SnapshotLayout::reopen_for_owner(root.path(), [2; 32], owner).is_err());
    assert!(root.path().join("v").exists());
    assert!(!root.path().join("layout.json").exists());
}
#[test]
fn original_binding_and_child_directory_identity_are_required() {
    let (root, owner) = fixture();
    let original = SnapshotLayout::initialize_for_owner(root.path(), [3; 32], owner).unwrap();
    assert!(SnapshotLayout::reopen_for_owner(root.path(), [4; 32], owner).is_err());
    fs::rename(original.runtime_root(), root.path().join("old-v")).unwrap();
    fs::create_dir(original.runtime_root()).unwrap();
    fs::set_permissions(original.runtime_root(), fs::Permissions::from_mode(0o700)).unwrap();
    assert!(SnapshotLayout::reopen_for_owner(root.path(), [3; 32], owner).is_err());
    assert!(
        original
            .store_for_runtime(&original.runtime_root())
            .is_err()
    );
    assert!(root.path().join("old-v").exists());
}
#[test]
fn symlink_replacement_missing_or_corrupt_profile_fails_closed() {
    let (root, owner) = fixture();
    let layout = SnapshotLayout::initialize_for_owner(root.path(), [5; 32], owner).unwrap();
    fs::rename(layout.store_root(), root.path().join("old-s")).unwrap();
    symlink(root.path().join("old-s"), layout.store_root()).unwrap();
    assert!(SnapshotLayout::reopen_for_owner(root.path(), [5; 32], owner).is_err());
    fs::remove_file(layout.store_root()).unwrap();
    fs::rename(root.path().join("old-s"), layout.store_root()).unwrap();
    fs::rename(root.path().join(PROFILE), root.path().join("old-profile")).unwrap();
    assert!(SnapshotLayout::reopen_for_owner(root.path(), [5; 32], owner).is_err());
    fs::write(root.path().join(PROFILE), b"partial").unwrap();
    assert!(SnapshotLayout::reopen_for_owner(root.path(), [5; 32], owner).is_err());
    assert!(layout.runtime_root().exists());
    assert!(layout.store_root().exists());
}
