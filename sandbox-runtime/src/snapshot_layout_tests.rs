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
    assert!(!root.path().join(PROBE_SOURCE).exists());
    assert!(!root.path().join(PROBE_TARGET).exists());
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
fn unprobed_development_record_is_rejected_without_migration() {
    let (root, owner) = fixture();
    let layout = SnapshotLayout::initialize_for_owner(root.path(), [6; 32], owner).unwrap();
    let profile = root.path().join(PROFILE);
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(&profile).unwrap()).unwrap();
    assert_eq!(record["version"], 2);
    record["version"] = 1.into();
    let original = serde_json::to_vec(&record).unwrap();
    fs::write(&profile, &original).unwrap();
    assert!(SnapshotLayout::reopen_for_owner(root.path(), [6; 32], owner).is_err());
    assert_eq!(fs::read(&profile).unwrap(), original);
    assert!(layout.runtime_root().exists());
    assert!(layout.store_root().exists());
}

#[test]
#[ignore = "requires root and a private mount namespace on the qualified XFS test host"]
fn same_device_mount_alias_is_rejected_by_actual_reopen() {
    use std::process::Command;
    const CHILD: &str = "SANDPROOF_TEST_LAYOUT_MOUNT_CHILD";
    const NAME: &str =
        "linux::snapshot_layout::tests::same_device_mount_alias_is_rejected_by_actual_reopen";
    const COMPLETED: &str = "SANDPROOF_LAYOUT_PRIVATE_MOUNT_CHECK_COMPLETED";
    assert_eq!(rustix::process::geteuid().as_raw(), 0);
    if let Some(path) = std::env::var_os(CHILD) {
        let path = PathBuf::from(path);
        let original = SnapshotLayout::reopen(&path, [0x74; 32]).unwrap();
        let runtime = original.runtime_root();
        let identity = fs::metadata(&runtime).unwrap();
        let profile = fs::read(path.join(PROFILE)).unwrap();
        assert!(
            Command::new("/usr/bin/mount")
                .args([
                    OsStr::new("--bind"),
                    runtime.as_os_str(),
                    runtime.as_os_str()
                ])
                .status()
                .unwrap()
                .success()
        );
        let current = fs::metadata(&runtime).unwrap();
        assert_eq!(
            (identity.dev(), identity.ino()),
            (current.dev(), current.ino())
        );
        let Err(failure) = SnapshotLayout::reopen(&path, [0x74; 32]) else {
            panic!("same-device bind mount must not pass the actual layout guard");
        };
        assert!(
            failure
                .to_string()
                .contains("matching current FD mount identities")
        );
        assert!(
            Command::new("/usr/bin/umount")
                .arg(&runtime)
                .status()
                .unwrap()
                .success()
        );
        SnapshotLayout::reopen(&path, [0x74; 32]).unwrap();
        assert_eq!(fs::read(path.join(PROFILE)).unwrap(), profile);
        println!("{COMPLETED}");
        return;
    }
    let (root, _) = fixture();
    SnapshotLayout::initialize(root.path(), [0x74; 32]).unwrap();
    let output = Command::new("/usr/bin/unshare")
        .args(["--mount", "--propagation", "private"])
        .arg(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", NAME, "--nocapture"])
        .env(CHILD, root.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    print!("{stdout}");
    eprint!("{stderr}");
    assert!(output.status.success(), "private mount child failed");
    assert!(stdout.lines().any(|line| line == "running 1 test"));
    assert!(
        stdout
            .lines()
            .any(|line| { line.starts_with("test result: ok. 1 passed; 0 failed; 0 ignored;") })
    );
    assert_eq!(stdout.matches(COMPLETED).count(), 1);
    SnapshotLayout::reopen(root.path(), [0x74; 32]).unwrap();
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
