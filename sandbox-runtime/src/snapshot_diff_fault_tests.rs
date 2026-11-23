//! Focused physical Diff-preparation faults. These never drive a VMM or
//! infer a controller result from local files.

use super::*;
use crate::linux::{
    RuntimeConfig, check_trusted_path, snapshot_restore::write_tracked_restore_fixture,
};
use rustix::process::{Pid, PidfdFlags, pidfd_open};
use std::{
    fs::{self, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
};

const RESOURCE: &str = "vm-diff-fault-1";
const MEMORY_BYTES: usize = 2 * 1024 * 1024;

struct Fixture {
    _area: tempfile::TempDir,
    runtime: Runtime,
    source: OriginalVmSource,
    jail: PathBuf,
    claim: PathBuf,
}

impl Fixture {
    fn new(root: &Path) -> Self {
        let area = tempfile::Builder::new()
            .prefix("diff-preparation-")
            .tempdir_in(root)
            .unwrap();
        let owner = rustix::process::geteuid().as_raw();
        let group = rustix::process::getegid().as_raw();
        let runtime = Runtime {
            config: RuntimeConfig {
                guest_instance_id: "diff-fault-fixture".into(),
                console_policy: None,
                host_slot: None,
                jailer: PathBuf::new(),
                firecracker: PathBuf::new(),
                chroot_base: area.path().join("runtime"),
                kernel: PathBuf::new(),
                rootfs: PathBuf::new(),
                vm_uid: owner,
                vm_gid: group,
                vcpu_count: 1,
                memory_mib: 2,
                host_resources: None,
                private_netns: None,
                access_token_hash: None,
            },
        };
        let jail = area.path().join("jail");
        fs::create_dir_all(jail.join("restore")).unwrap();
        let identity = RuntimeProcessIdentity::capture(std::process::id()).unwrap();
        let source = OriginalVmSource {
            resource_id: RESOURCE.into(),
            root: File::open(&jail).unwrap(),
            _pidfd: pidfd_open(
                Pid::from_raw(i32::try_from(identity.pid).unwrap()).unwrap(),
                PidfdFlags::empty(),
            )
            .unwrap(),
            identity,
        };
        let claim = runtime.config.chroot_base.join(".claims").join(RESOURCE);
        Self {
            _area: area,
            runtime,
            source,
            jail,
            claim,
        }
    }

    fn restore_memory(&self, byte: u8) -> FileStamp {
        let path = self.jail.join("restore").join(BACKING_MEMORY);
        fs::write(&path, vec![byte; MEMORY_BYTES]).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        let file = File::open(path).unwrap();
        file.sync_all().unwrap();
        FileStamp::from_metadata(&file.metadata().unwrap())
    }

    fn backing_path(&self) -> PathBuf {
        self.jail.join("restore").join(BACKING_MEMORY)
    }

    fn fresh_capture(&self, name: &str) -> (PathBuf, File) {
        let directory = self.jail.join(name);
        fs::create_dir(&directory).unwrap();
        let descriptor = File::open(&directory).unwrap();
        (directory, descriptor)
    }
}

fn trusted_xfs_root(variable: &str) -> PathBuf {
    assert_eq!(rustix::process::geteuid().as_raw(), 0);
    let root = PathBuf::from(std::env::var_os(variable).expect("explicit isolated XFS test root"));
    check_trusted_path(&root).unwrap();
    require_same_xfs_mount(&File::open(&root).unwrap(), &File::open(&root).unwrap()).unwrap();
    root
}

#[test]
fn absent_or_replaced_parent_cannot_enter_diff_preclone() {
    let root = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(root.path());
    let saved = fixture.restore_memory(14);
    assert!(
        fixture
            .runtime
            .open_diff_parent(&fixture.source, &saved)
            .unwrap()
            .is_some()
    );

    let path = fixture.backing_path();
    let original = path.with_file_name("pinned-original");
    fs::rename(&path, &original).unwrap();
    assert!(
        fixture
            .runtime
            .open_diff_parent(&fixture.source, &saved)
            .unwrap()
            .is_none()
    );

    // A same-size, private regular file with the right owner cannot take
    // over the original descriptor's role even if its contents match.
    let _replacement = fixture.restore_memory(14);
    assert!(
        fixture
            .runtime
            .open_diff_parent(&fixture.source, &saved)
            .unwrap()
            .is_none()
    );
    assert_eq!(fs::read(original).unwrap(), vec![14; MEMORY_BYTES]);
}

#[test]
#[ignore = "requires root on an explicit trusted private XFS reflink mount"]
fn actual_diff_refuses_wrong_loaded_process_and_falls_back_without_lineage() {
    let root = trusted_xfs_root("SANDPROOF_DIFF_FAULT_ROOT");
    let fixture = Fixture::new(&root);
    let saved = fixture.restore_memory(23);
    let mut wrong = fixture.source.identity.clone();
    wrong.start_time_ticks += 1;
    write_tracked_restore_fixture(&fixture.runtime, RESOURCE, wrong, saved);
    let (wrong_directory, wrong_capture) = fixture.fresh_capture("wrong-capture");
    assert!(matches!(
        fixture
            .runtime
            .prepare_capture_memory(&fixture.source, &wrong_capture, [51; 32]),
        Err(RuntimeError::IdentityMismatch(
            "dirty capture loaded process or intent differs"
        ))
    ));
    assert!(!fixture.claim.join(USED_RECORD).exists());
    assert!(fs::read_dir(wrong_directory).unwrap().next().is_none());
    assert_eq!(
        fs::read(fixture.backing_path()).unwrap(),
        vec![23; MEMORY_BYTES]
    );

    let fixture = Fixture::new(&root);
    let saved = fixture.restore_memory(32);
    write_tracked_restore_fixture(
        &fixture.runtime,
        RESOURCE,
        fixture.source.identity.clone(),
        saved,
    );
    fs::rename(fixture.backing_path(), fixture.jail.join("pinned-original")).unwrap();
    let (missing_directory, missing_capture) = fixture.fresh_capture("missing-capture");
    let key = [52; 32];
    assert!(
        fixture
            .runtime
            .prepare_capture_memory(&fixture.source, &missing_capture, key)
            .unwrap()
            .is_none()
    );
    require_capture_request_kind(
        &fixture.claim,
        &fixture.source.identity,
        key,
        Some(CaptureKind::Full),
    )
    .unwrap();
    assert!(
        require_capture_request_kind(
            &fixture.claim,
            &fixture.source.identity,
            key,
            Some(CaptureKind::Diff)
        )
        .is_err()
    );
    assert!(fs::read_dir(missing_directory).unwrap().next().is_none());
    assert_eq!(
        fs::read(fixture.jail.join("pinned-original")).unwrap(),
        vec![32; MEMORY_BYTES]
    );

    let fixture = Fixture::new(&root);
    let saved = fixture.restore_memory(45);
    write_tracked_restore_fixture(
        &fixture.runtime,
        RESOURCE,
        fixture.source.identity.clone(),
        saved.clone(),
    );
    // Precisely model a lost restore reply before the durable LoadedRecord:
    // intent and verified private backing still exist, but can't authorize Diff.
    fs::remove_file(fixture.claim.join("restore-loaded.json")).unwrap();
    let (unrecorded_directory, unrecorded_capture) = fixture.fresh_capture("unrecorded-load");
    let key = [53; 32];
    assert!(
        fixture
            .runtime
            .prepare_capture_memory(&fixture.source, &unrecorded_capture, key)
            .unwrap()
            .is_none()
    );
    require_capture_request_kind(
        &fixture.claim,
        &fixture.source.identity,
        key,
        Some(CaptureKind::Full),
    )
    .unwrap();
    assert!(fs::read_dir(unrecorded_directory).unwrap().next().is_none());
    assert_eq!(
        FileStamp::from_metadata(&fixture.backing_path().metadata().unwrap()),
        saved
    );
}

#[test]
#[ignore = "requires root on an explicit trusted private XFS reflink mount"]
fn actual_interrupted_diff_preclone_preserves_backing_and_never_reuses_epoch() {
    let fixture = Fixture::new(&trusted_xfs_root("SANDPROOF_DIFF_FAULT_ROOT"));
    let saved = fixture.restore_memory(67);
    write_tracked_restore_fixture(
        &fixture.runtime,
        RESOURCE,
        fixture.source.identity.clone(),
        saved.clone(),
    );
    let (partial_directory, partial_capture) = fixture.fresh_capture("partial-capture");
    let key = [61; 32];
    let prepared = fixture
        .runtime
        .prepare_capture_memory(&fixture.source, &partial_capture, key)
        .unwrap()
        .unwrap();
    let candidate = partial_directory.join(CAPTURE_MEMORY);
    assert_ne!(
        candidate.metadata().unwrap().ino(),
        fixture.backing_path().metadata().unwrap().ino()
    );
    // Model an interrupted provider write after clone; there is deliberately
    // no fabricated Firecracker response or completed bundle.
    let mut output = OpenOptions::new().write(true).open(&candidate).unwrap();
    output.seek(SeekFrom::Start(4096)).unwrap();
    output.write_all(&[0; 4096]).unwrap();
    output.sync_all().unwrap();
    drop(prepared);
    assert_eq!(
        fs::read(fixture.backing_path()).unwrap(),
        vec![67; MEMORY_BYTES]
    );
    assert_eq!(
        FileStamp::from_metadata(&fixture.backing_path().metadata().unwrap()),
        saved
    );
    assert_eq!(&fs::read(&candidate).unwrap()[4096..8192], &[0; 4096]);

    let reopened = Runtime {
        config: fixture.runtime.config.clone(),
    };
    let (new_directory, new_capture) = fixture.fresh_capture("new-capture");
    assert!(
        reopened
            .prepare_capture_memory(&fixture.source, &new_capture, [62; 32])
            .unwrap()
            .is_none()
    );
    assert!(fs::read_dir(new_directory).unwrap().next().is_none());
    assert!(candidate.is_file());
    require_capture_request_kind(
        &fixture.claim,
        &fixture.source.identity,
        key,
        Some(CaptureKind::Diff),
    )
    .unwrap();
    assert!(
        require_capture_request_kind(
            &fixture.claim,
            &fixture.source.identity,
            [62; 32],
            Some(CaptureKind::Diff)
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires a separate explicitly prepared sub-MiB quota on the same trusted XFS mount"]
fn actual_diff_preclone_quota_failure_retains_original_backing_and_reserved_attempt() {
    let root = trusted_xfs_root("SANDPROOF_DIFF_FAULT_ROOT");
    let limited = trusted_xfs_root("SANDPROOF_DIFF_FAULT_QUOTA_TARGET");
    assert_ne!(root, limited);
    let fixture = Fixture::new(&root);
    let saved = fixture.restore_memory(79);
    write_tracked_restore_fixture(
        &fixture.runtime,
        RESOURCE,
        fixture.source.identity.clone(),
        saved.clone(),
    );
    let limited = tempfile::Builder::new()
        .prefix("diff-quota-")
        .tempdir_in(limited)
        .unwrap();
    let capture = File::open(limited.path()).unwrap();
    let key = [63; 32];
    match fixture
        .runtime
        .prepare_capture_memory(&fixture.source, &capture, key)
    {
        Err(RuntimeError::Io(error))
            if [
                rustix::io::Errno::NOSPC.raw_os_error(),
                rustix::io::Errno::DQUOT.raw_os_error(),
            ]
            .contains(&error.raw_os_error().unwrap_or_default()) => {}
        Err(other) => panic!("expected kernel ENOSPC or EDQUOT, got {other:?}"),
        Ok(_) => panic!("quota test unexpectedly prepared an image"),
    }
    assert_eq!(
        FileStamp::from_metadata(&fixture.backing_path().metadata().unwrap()),
        saved
    );
    assert_eq!(
        fs::read(fixture.backing_path()).unwrap(),
        vec![79; MEMORY_BYTES]
    );
    assert!(limited.path().join(CAPTURE_MEMORY).is_file());
    require_capture_request_kind(
        &fixture.claim,
        &fixture.source.identity,
        key,
        Some(CaptureKind::Diff),
    )
    .unwrap();

    let reopened = Runtime {
        config: fixture.runtime.config.clone(),
    };
    let (new_directory, new_capture) = fixture.fresh_capture("restart-after-quota");
    assert!(
        reopened
            .prepare_capture_memory(&fixture.source, &new_capture, [64; 32])
            .unwrap()
            .is_none()
    );
    assert!(fs::read_dir(new_directory).unwrap().next().is_none());
}
