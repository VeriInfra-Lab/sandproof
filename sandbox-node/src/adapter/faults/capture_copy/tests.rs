//! Selector/command protocol only; no physical Full, End or permission claimed.
use super::*;
use rustix::fs::{self, Mode, OFlags};
use sandproof_protocol::{ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind, ResourceRef, SandboxId, TenantId};
use std::{fs as disk, os::unix::fs::PermissionsExt, path::Path};
fn action() -> WorkAction<core::EffectId> {
    WorkAction {
        identity: ActionIdentity {
            protocol_version: PROTOCOL_VERSION,
            owner: TenantId::try_from("tenant-1").unwrap(),
            sandbox: SandboxId::try_from("sb-test-1").unwrap(),
            operation: OperationId::try_from("op-test-4").unwrap(),
            epoch: 4,
            generation: 2,
            logical_action: LogicalActionId::try_from("effect-17").unwrap(),
            attempt: 0,
            resource: ResourceRef {
                kind: ResourceKind::Vm,
                id: ResourceId::try_from("vm-test-2").unwrap(),
            },
        },
        command: core::EffectId::initial(core::Command::SealSnapshot {
            sandbox: core::SandboxId(1),
            epoch: 4,
            generation: 2,
            vm: core::VmId(2),
            workspace: core::FsId(1),
            snapshot: core::SnapshotId(1),
            manifest: core::SnapshotManifestRef {
                digest: core::ManifestDigest(7),
            },
        }),
    }
}
fn worker() -> CaptureWorkerBinding {
    CaptureWorkerBinding::new(
        SourceProcessIdentity {
            pid: 41,
            start_time_ticks: 17,
            boot_id: "original-boot".into(),
        },
        ContentDigest::from_sha256([7; 32]),
    )
    .unwrap()
}
fn directory(path: &Path) -> Directory {
    let file = File::from(
        fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap(),
    );
    // Ordinary fixtures use their actual owner. The production open still
    // requires the original root-private directory and protected ancestors.
    Directory::from_opened(path.into(), file, rustix::process::geteuid().as_raw()).unwrap()
}
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("gate");
    disk::create_dir(&path).unwrap();
    disk::set_permissions(&path, disk::Permissions::from_mode(0o700)).unwrap();
    directory(&path)
        .create(
            "config.json",
            &serde_json::to_vec(&serde_json::json!({
                "schema_version": 1, "run_identity": "a".repeat(64), "sandbox": "sb-test-1",
                "generation": 2, "timeout_ms": 1000,
            }))
            .unwrap(),
        )
        .unwrap();
    (root, path)
}
fn source() -> SourceProcessIdentity {
    SourceProcessIdentity { pid: 19, start_time_ticks: 29, boot_id: "original-source-boot".into() }
}
fn instruction(path: &Path, mode: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.join("ready.json").exists() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let bytes = disk::read(path.join("observation.json")).unwrap();
    let observed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(observed["action"], serde_json::to_value(action()).unwrap());
    assert_eq!(observed["worker"], serde_json::to_value(worker()).unwrap());
    assert_eq!(observed["source"], serde_json::to_value(source()).unwrap());
    assert_eq!(observed["copied_bytes"], 65_536);
    assert_eq!(observed["total_bytes"], 131_072);
    let mut value = serde_json::json!({"schema_version":1,"run_identity":"a".repeat(64),
        "observation_sha256":hex_digest(&bytes),"full_work_binding":ContentDigest::from_sha256([3;32]),
        "command":if mode=="fail" {"fail"} else {"release"}});
    match mode {
        "wrong-run" => value["run_identity"]=serde_json::json!("b".repeat(64)),
        "wrong-question" => value["observation_sha256"]=serde_json::json!("c".repeat(64)),
        "wrong-work" => value["full_work_binding"]=serde_json::to_value(ContentDigest::from_sha256([4;32])).unwrap(),
        _ => {}
    }
    directory(path).create("command.json", &serde_json::to_vec(&value).unwrap()).unwrap();
}
#[test]
fn copy_gate_binds_original_question_and_only_consumes_exact_release_or_failure() {
    for mode in ["release", "fail", "wrong-run", "wrong-question", "wrong-work"] {
        let (_root,path)=fixture();
        let gate=CaptureCopyGate::select(directory(&path), &action()).unwrap().unwrap();
        let control=path.clone();
        let name=mode.to_owned();
        let thread=std::thread::spawn(move || instruction(&control,&name));
        let result=gate.wait_question(&action(),ContentDigest::from_sha256([3;32]),&source(),&worker(),(65_536,131_072));
        thread.join().unwrap();
        assert_eq!(result.is_ok(),mode=="release");
        assert_eq!(path.join("consumed.json").exists(),matches!(mode,"release"|"fail"));
    }
}
#[test]
fn copy_gate_rejects_zero_or_finished_copy_before_publishing_a_question() {
    for progress in [(0,131_072),(131_072,131_072),(2,1)] {
        let (_root,path)=fixture();
        let gate=CaptureCopyGate::select(directory(&path), &action()).unwrap().unwrap();
        assert!(gate.wait_question(&action(),ContentDigest::from_sha256([3;32]),&source(),&worker(),progress).is_err());
        assert!(!path.join("ready.json").exists());
    }
}
