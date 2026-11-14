//! Selector/command protocol only; no physical Full, End or permission claimed.
use super::*;
use rustix::fs::{self, Mode, OFlags};
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
                "schema_version": 2, "run_identity": "a".repeat(64), "sandbox": "sb-test-1",
                "generation": 2, "timeout_ms": 1000,
            }))
            .unwrap(),
        )
        .unwrap();
    (root, path)
}
fn source() -> SourceProcessIdentity {
    SourceProcessIdentity {
        pid: 19,
        start_time_ticks: 29,
        boot_id: "original-source-boot".into(),
    }
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
    assert_eq!(observed["schema_version"], 2);
    assert_eq!(observed["phase"], "measuring-cloned-artifact");
    assert_eq!(observed["measured_bytes"], 65_536);
    assert!(observed.get("copied_bytes").is_none());
    assert_eq!(observed["total_bytes"], 131_072);
    let mut value = serde_json::json!({"schema_version":2,"run_identity":"a".repeat(64),
        "observation_sha256":hex_digest(&bytes),"full_work_binding":ContentDigest::from_sha256([3;32]),
        "command":if mode=="fail" {"fail"} else {"release"}});
    match mode {
        "wrong-run" => value["run_identity"] = serde_json::json!("b".repeat(64)),
        "wrong-question" => value["observation_sha256"] = serde_json::json!("c".repeat(64)),
        "wrong-work" => {
            value["full_work_binding"] =
                serde_json::to_value(ContentDigest::from_sha256([4; 32])).unwrap();
        }
        "old-format" => value["schema_version"] = serde_json::json!(1),
        _ => {}
    }
    directory(path)
        .create("command.json", &serde_json::to_vec(&value).unwrap())
        .unwrap();
}
#[test]
fn copy_gate_binds_original_question_and_only_consumes_exact_release_or_failure() {
    for mode in [
        "release",
        "fail",
        "wrong-run",
        "wrong-question",
        "wrong-work",
        "old-format",
    ] {
        let (_root, path) = fixture();
        let gate = CaptureCopyGate::select(directory(&path), &action())
            .unwrap()
            .unwrap();
        let control = path.clone();
        let name = mode.to_owned();
        let thread = std::thread::spawn(move || instruction(&control, &name));
        let result = gate.wait_question(
            &action(),
            ContentDigest::from_sha256([3; 32]),
            &source(),
            &worker(),
            (65_536, 131_072),
            None,
        );
        thread.join().unwrap();
        assert_eq!(result.is_ok(), mode == "release");
        assert_eq!(
            path.join("consumed.json").exists(),
            matches!(mode, "release" | "fail")
        );
    }
}
#[test]
fn copy_gate_rejects_zero_or_finished_measurement_before_publishing_a_question() {
    for progress in [(0, 131_072), (131_072, 131_072), (2, 1)] {
        let (_root, path) = fixture();
        let gate = CaptureCopyGate::select(directory(&path), &action())
            .unwrap()
            .unwrap();
        assert!(
            gate.wait_question(
                &action(),
                ContentDigest::from_sha256([3; 32]),
                &source(),
                &worker(),
                progress,
                None,
            )
            .is_err()
        );
        assert!(!path.join("ready.json").exists());
    }
}

#[test]
fn old_copy_config_is_rejected_instead_of_becoming_a_measurement_gate() {
    let (_root, path) = fixture();
    let mut config: serde_json::Value =
        serde_json::from_slice(&disk::read(path.join("config.json")).unwrap()).unwrap();
    config["schema_version"] = serde_json::json!(1);
    disk::write(
        path.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    assert!(CaptureCopyGate::select(directory(&path), &action()).is_err());
    assert!(!path.join("ready.json").exists());
}

fn sync_fixture(phase: &str) -> (tempfile::TempDir, PathBuf) {
    let (root, path) = fixture();
    let mut config: serde_json::Value =
        serde_json::from_slice(&disk::read(path.join("config.json")).unwrap()).unwrap();
    config["schema_version"] = serde_json::json!(3);
    config["sync_phase"] = serde_json::json!(phase);
    disk::write(
        path.join("config.json"),
        serde_json::to_vec(&config).unwrap(),
    )
    .unwrap();
    (root, path)
}

#[test]
fn sync_gate_selects_only_original_memory_event_and_has_no_byte_progress_claim() {
    use sandbox_runtime::snapshot_store::{SnapshotArtifact as A, SnapshotArtifactEvent as E};
    for (phase, selected, other) in [
        ("before-sync", E::BeforeSync, E::Synced),
        ("after-sync", E::Synced, E::BeforeSync),
    ] {
        let (_root, path) = sync_fixture(phase);
        let gate = CaptureCopyGate::select(directory(&path), &action())
            .unwrap()
            .unwrap();
        assert_eq!(gate.progress(A::Memory, selected), Some((0, 0)));
        for artifact in [A::State, A::Disk(0)] {
            assert_eq!(gate.progress(artifact, selected), None);
        }
        for event in [
            E::BeforeClone,
            E::Cloned,
            other,
            E::Measuring {
                measured_bytes: 1,
                total_bytes: 2,
            },
        ] {
            assert_eq!(gate.progress(A::Memory, event), None);
        }
        assert!(!path.join("ready.json").exists());
    }
}

#[test]
fn sync_gate_binds_phase_action_worker_source_and_exact_command_before_consumption() {
    for phase in ["before-sync", "after-sync"] {
        for mode in [
            "release",
            "fail",
            "wrong-run",
            "wrong-work",
            "wrong-question",
            "wrong-schema",
            "wrong-command",
        ] {
            let (_root, path) = sync_fixture(phase);
            let gate = CaptureCopyGate::select(directory(&path), &action())
                .unwrap()
                .unwrap();
            let control = path.clone();
            let expected_phase = format!("memory-{phase}");
            let thread = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !control.join("ready.json").exists() {
                    assert!(Instant::now() < deadline);
                    std::thread::yield_now();
                }
                let bytes = disk::read(control.join("observation.json")).unwrap();
                let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(value["schema_version"], 3);
                assert_eq!(value["phase"], expected_phase);
                assert_eq!(value["capture_key"], "d".repeat(64));
                assert_eq!(value["action"], serde_json::to_value(action()).unwrap());
                assert_eq!(value["worker"], serde_json::to_value(worker()).unwrap());
                assert_eq!(value["source"], serde_json::to_value(source()).unwrap());
                assert!(value.get("measured_bytes").is_none());
                assert!(value.get("total_bytes").is_none());
                let mut command = serde_json::json!({"schema_version":3,"run_identity":"a".repeat(64),
                    "observation_sha256":hex_digest(&bytes),"full_work_binding":ContentDigest::from_sha256([3;32]),
                    "command":if mode=="fail" {"fail"} else {"release"}});
                match mode {
                    "wrong-run" => command["run_identity"] = serde_json::json!("b".repeat(64)),
                    "wrong-work" => {
                        command["full_work_binding"] =
                            serde_json::to_value(ContentDigest::from_sha256([4; 32])).unwrap();
                    }
                    "wrong-question" => {
                        command["observation_sha256"] = serde_json::json!("c".repeat(64));
                    }
                    "wrong-schema" => command["schema_version"] = serde_json::json!(2),
                    "wrong-command" => command["command"] = serde_json::json!("published"),
                    _ => {}
                }
                directory(&control)
                    .create("command.json", &serde_json::to_vec(&command).unwrap())
                    .unwrap();
            });
            let result = gate.wait_question(
                &action(),
                ContentDigest::from_sha256([3; 32]),
                &source(),
                &worker(),
                (0, 0),
                Some("d".repeat(64)),
            );
            thread.join().unwrap();
            assert_eq!(result.is_ok(), mode == "release");
            assert_eq!(
                path.join("consumed.json").exists(),
                matches!(mode, "release" | "fail")
            );
            if matches!(mode, "release" | "fail") {
                assert!(
                    CaptureCopyGate::select(directory(&path), &action())
                        .unwrap()
                        .is_none()
                );
            }
        }
    }
}

#[test]
fn sync_config_and_missing_key_are_rejected_without_a_question() {
    for (version, phase) in [
        (2, Some("before-sync")),
        (3, None),
        (3, Some("synced")),
        (4, Some("after-sync")),
    ] {
        let (_root, path) = fixture();
        let mut config: serde_json::Value =
            serde_json::from_slice(&disk::read(path.join("config.json")).unwrap()).unwrap();
        config["schema_version"] = serde_json::json!(version);
        if let Some(phase) = phase {
            config["sync_phase"] = serde_json::json!(phase);
        }
        disk::write(
            path.join("config.json"),
            serde_json::to_vec(&config).unwrap(),
        )
        .unwrap();
        assert!(CaptureCopyGate::select(directory(&path), &action()).is_err());
        assert!(!path.join("ready.json").exists());
    }
    let (_root, path) = sync_fixture("before-sync");
    let gate = CaptureCopyGate::select(directory(&path), &action())
        .unwrap()
        .unwrap();
    for key in [None, Some("wrong".into())] {
        assert!(
            gate.wait_question(
                &action(),
                ContentDigest::from_sha256([3; 32]),
                &source(),
                &worker(),
                (0, 0),
                key
            )
            .is_err()
        );
    }
    assert!(!path.join("ready.json").exists());
}

#[test]
fn one_original_gate_path_only_without_ambient_helper_environment() {
    assert!(select_paths(None, None).unwrap().is_none());
    assert_eq!(
        select_paths(Some("measurement".into()), None).unwrap(),
        Some(PathBuf::from("measurement"))
    );
    assert_eq!(
        select_paths(None, Some("sync".into())).unwrap(),
        Some(PathBuf::from("sync"))
    );
    assert!(select_paths(Some("measurement".into()), Some("sync".into())).is_err());
}
