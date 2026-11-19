use super::*;
use std::{
    fs as disk,
    io::Write,
    os::unix::fs::{OpenOptionsExt, symlink},
    path::Path,
    process::Command,
};

const STAGED_KEY: [u8; 32] = [23; 32];
const OTHER_KEY: [u8; 32] = [24; 32];

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
        let key = STAGED_KEY;
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

fn file_stamp(path: &Path) -> FileStamp {
    FileStamp::from_metadata(&path.symlink_metadata().unwrap())
}

fn file_mount_id(path: &Path) -> u64 {
    let file = File::open(path).unwrap();
    let stat = fs::statx(&file, "", AtFlags::EMPTY_PATH, fs::StatxFlags::MNT_ID).unwrap();
    assert_ne!(stat.stx_mask & fs::StatxFlags::MNT_ID.bits(), 0);
    stat.stx_mnt_id
}

fn retire_after_mountpoint_refusal(root: &Path) {
    let store = SnapshotStore::open(root).unwrap();
    let stage = root.join(key_name("staging-", &STAGED_KEY));
    let snapshot_state = stage.join("state.bin");
    let memory = stage.join("memory.bin");
    let disk_image = stage.join("disk-0.img");
    let marker = root.join(key_name("api-returned-", &STAGED_KEY));
    let other = root
        .join(key_name("staging-", &OTHER_KEY))
        .join("state.bin");
    assert!(snapshot_state.is_file());
    let memory_stamp = file_stamp(&memory);
    let disk_stamp = file_stamp(&disk_image);
    let marker_stamp = file_stamp(&marker);
    let other_stamp = file_stamp(&other);
    let original_mount = file_mount_id(&memory);
    let plan = store.capture_retirement_plan(STAGED_KEY).unwrap();
    let saved = serde_json::to_vec(&plan).unwrap();

    // Mount the file over itself. The original inode and bytes stay the same,
    // but Linux refuses unlinkat on the mountpoint.
    assert!(
        Command::new("/usr/bin/mount")
            .arg("--bind")
            .arg(&memory)
            .arg(&memory)
            .status()
            .unwrap()
            .success()
    );
    assert_ne!(file_mount_id(&memory), original_mount);
    assert_eq!(file_stamp(&memory), memory_stamp);
    store.verify_capture_retirement(&plan).unwrap();
    let RuntimeError::Io(error) = store.retire_capture_material(&plan).unwrap_err() else {
        panic!("retirement must reach the original unlink of the mounted file");
    };
    assert_eq!(
        error.raw_os_error(),
        Some(rustix::io::Errno::BUSY.raw_os_error())
    );
    assert_eq!(
        snapshot_state.symlink_metadata().unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    assert_eq!(file_stamp(&memory), memory_stamp);
    assert_eq!(file_stamp(&disk_image), disk_stamp);
    assert_eq!(file_stamp(&marker), marker_stamp);
    assert_eq!(file_stamp(&other), other_stamp);
    store.verify_capture_retirement(&plan).unwrap();
    drop(store);

    assert!(
        Command::new("/usr/bin/umount")
            .arg(&memory)
            .status()
            .unwrap()
            .success()
    );
    assert_eq!(file_mount_id(&memory), original_mount);
    assert_eq!(file_stamp(&memory), memory_stamp);
    let reopened = SnapshotStore::open(root).unwrap();
    let original = serde_json::from_slice(&saved).unwrap();
    reopened.retire_capture_material(&original).unwrap();
    reopened.retire_capture_material(&original).unwrap();
    for absent in [stage, marker] {
        assert_eq!(
            absent.symlink_metadata().unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
    }
    assert_eq!(file_stamp(&other), other_stamp);
    assert_eq!(disk::read(other).unwrap(), b"other capture");
}

#[test]
#[ignore = "requires root and a private mount namespace on the approved Linux test host"]
fn actual_unlink_busy_retries_the_same_partial_retirement_plan() {
    const PARENT_MOUNT: &str = "SANDPROOF_TEST_RETIREMENT_BUSY_PARENT_MOUNT";
    const NAME: &str = "snapshot_store::material_retirement::tests::actual_unlink_busy_retries_the_same_partial_retirement_plan";
    const COMPLETED: &str = "E11 unlink EBUSY preserved original files and retried the same plan";
    assert_eq!(rustix::process::geteuid().as_raw(), 0);
    let namespace = disk::read_link("/proc/self/ns/mnt").unwrap();
    if let Some(parent_mount) = std::env::var_os(PARENT_MOUNT) {
        assert_ne!(namespace, std::path::PathBuf::from(parent_mount));
        let fixture = Fixture::new();
        let root = fixture.root.path();
        Fixture::write(&fixture.stage().join("disk-0.img"), b"original disk");
        Fixture::write(
            &root.join(key_name("api-returned-", &STAGED_KEY)),
            b"original marker",
        );
        drop(fixture.store.begin(OTHER_KEY).unwrap());
        Fixture::write(
            &root
                .join(key_name("staging-", &OTHER_KEY))
                .join("state.bin"),
            b"other capture",
        );
        retire_after_mountpoint_refusal(root);
        println!("{COMPLETED}");
        return;
    }

    // The parent owns this disposable root. On a child failure, the namespace
    // drops before the parent can remove the remaining fixture.
    let root = tempfile::tempdir().unwrap();
    let output = Command::new("/usr/bin/unshare")
        .args(["--mount", "--propagation", "private"])
        .arg(std::env::current_exe().unwrap())
        .args([
            NAME,
            "--exact",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("TMPDIR", root.path())
        .env(PARENT_MOUNT, namespace)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    print!("{stdout}");
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "private mount child failed");
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with("test result: ok. 1 passed; 0 failed;"))
    );
    assert_eq!(stdout.matches(COMPLETED).count(), 1);
    assert_eq!(disk::read_dir(root.path()).unwrap().count(), 0);
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
