//! Private selector protocol only; no Full, VM or service outcome is asserted.
use super::*;
use rustix::fs::{self, Mode, OFlags};
use sandproof_controller_service::capture::SourceProcessIdentity;
use sandproof_protocol::{
    ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
    ResourceRef, SandboxId, TenantId,
};
use std::{fs as disk, os::unix::fs::PermissionsExt, path::Path};

fn action() -> WorkAction<core::EffectId> {
    WorkAction {
        identity: ActionIdentity {
            protocol_version: PROTOCOL_VERSION,
            owner: TenantId::try_from("tenant-1").unwrap(),
            sandbox: SandboxId::try_from("sb-test-1").unwrap(),
            operation: OperationId::try_from("op-test-4").unwrap(),
            epoch: 4, generation: 2,
            logical_action: LogicalActionId::try_from("effect-17").unwrap(),
            attempt: 0,
            resource: ResourceRef { kind: ResourceKind::Vm, id: ResourceId::try_from("vm-test-2").unwrap() },
        },
        command: core::EffectId::initial(core::Command::SealSnapshot {
            sandbox: core::SandboxId(1), epoch: 4, generation: 2, vm: core::VmId(2),
            workspace: core::FsId(1), snapshot: core::SnapshotId(1),
            manifest: core::SnapshotManifestRef { digest: core::ManifestDigest(7) },
        }),
    }
}
fn worker() -> CaptureWorkerBinding {
    CaptureWorkerBinding::new(SourceProcessIdentity {
        pid: 41, start_time_ticks: 17, boot_id: "original-boot".into(),
    }, ContentDigest::from_sha256([7; 32])).unwrap()
}
fn directory(path: &Path) -> Directory {
    let file = File::from(fs::open(path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty()).unwrap());
    // Ordinary fixtures use their actual owner. The production open still
    // requires the original root-private directory and protected ancestors.
    Directory::from_opened(path.into(), file, rustix::process::geteuid().as_raw()).unwrap()
}
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("gate");
    disk::create_dir(&path).unwrap();
    disk::set_permissions(&path, disk::Permissions::from_mode(0o700)).unwrap();
    directory(&path).create("config.json", &serde_json::to_vec(&serde_json::json!({
        "schema_version": 1, "run_identity": "a".repeat(64), "sandbox": "sb-test-1",
        "generation": 2, "timeout_ms": 1000,
    })).unwrap()).unwrap();
    (root, path)
}
fn instruction(path: &Path, variant: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let ready = loop {
        if let Some(bytes) = directory(path).read("ready.json").unwrap() { break bytes; }
        assert!(Instant::now() < deadline, "no original question published");
        std::thread::yield_now();
    };
    let ready: serde_json::Value = serde_json::from_slice(&ready).unwrap();
    let bytes = disk::read(path.join("observation.json")).unwrap();
    let question: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(ready["observation_sha256"], hex_digest(&bytes));
    assert_eq!(question["action"], serde_json::to_value(action()).unwrap());
    assert_eq!(question["worker"], serde_json::to_value(worker()).unwrap());
    assert_eq!(disk::metadata(path.join("observation.json")).unwrap().mode() & 0o7777, 0o600);
    let mut command = serde_json::json!({
        "schema_version": 1, "run_identity": "a".repeat(64),
        "observation_sha256": hex_digest(&bytes), "full_work_binding": vec![9_u8; 32], "command": "cancel",
    });
    match variant {
        "run" => command["run_identity"] = "b".repeat(64).into(),
        "digest" => command["observation_sha256"] = "c".repeat(64).into(),
        "full" => command["full_work_binding"] = serde_json::json!(vec![8_u8; 32]),
        "release" => command["command"] = "release".into(),
        "replace" => {
            disk::remove_file(path.join("observation.json")).unwrap();
            directory(path).create("observation.json", &bytes).unwrap();
        }
        "none" => {}
        _ => unreachable!(),
    }
    directory(path).create("command-stage.json", &serde_json::to_vec(&command).unwrap()).unwrap();
    disk::rename(path.join("command-stage.json"), path.join("command.json")).unwrap();
    File::open(path).unwrap().sync_all().unwrap();
}
#[test]
fn only_complete_bound_cancel_is_consumed_once() {
    let (_root, path) = fixture();
    let gate = CaptureCancelGate::select(directory(&path), &action()).unwrap().unwrap();
    let consumer_path = path.clone();
    let consumer = std::thread::spawn(move || instruction(&consumer_path, "none"));
    gate.wait_for(&action(), &ContentDigest::from_sha256([9; 32]), &worker(),
        Instant::now() + Duration::from_secs(5)).unwrap();
    consumer.join().unwrap();
    assert!(path.join("consumed.json").exists());
    assert!(CaptureCancelGate::select(directory(&path), &action()).unwrap().is_none());
    assert_eq!(disk::read_dir(path).unwrap().count(), 5);
}
#[test]
fn changed_run_question_full_or_release_command_never_consumes_a_cancel() {
    for variant in ["run", "digest", "full", "release", "replace"] {
        let (_root, path) = fixture();
        let gate = CaptureCancelGate::select(directory(&path), &action()).unwrap().unwrap();
        let consumer_path = path.clone();
        let consumer = std::thread::spawn(move || instruction(&consumer_path, variant));
        assert!(gate.wait_for(&action(), &ContentDigest::from_sha256([9; 32]), &worker(),
            Instant::now() + Duration::from_secs(5)).is_err(), "accepted {variant}");
        consumer.join().unwrap();
        assert!(!path.join("consumed.json").exists());
    }
}
#[test]
fn wrong_scope_never_arms_and_changed_action_is_rejected_before_publication() {
    let (_root, path) = fixture();
    let mut other = action();
    other.identity.generation += 1;
    assert!(CaptureCancelGate::select(directory(&path), &other).unwrap().is_none());
    let gate = CaptureCancelGate::select(directory(&path), &action()).unwrap().unwrap();
    assert!(gate.wait_for(&other, &ContentDigest::from_sha256([9; 32]), &worker(),
        Instant::now() + Duration::from_secs(1)).is_err());
    assert_eq!(disk::read_dir(&path).unwrap().count(), 1);
}
#[test]
fn expired_cancel_barrier_remains_unknown_without_consuming_or_sending_release() {
    let (_root, path) = fixture();
    let gate = CaptureCancelGate::select(directory(&path), &action()).unwrap().unwrap();
    assert!(gate.wait_for(&action(), &ContentDigest::from_sha256([9; 32]), &worker(), Instant::now()).is_err());
    assert!(!path.join("command.json").exists());
    assert!(!path.join("consumed.json").exists());
}
