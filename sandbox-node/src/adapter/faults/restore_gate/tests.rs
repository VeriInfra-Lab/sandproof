//! Controlled private-file protocol tests, not VM/guest authorization evidence.
use super::*;
use sandproof_protocol::{
    ActionIdentity, LogicalActionId, OperationId, PROTOCOL_VERSION, ResourceId, ResourceKind,
    ResourceRef, SandboxId, TenantId,
};
use std::{fs as disk, os::unix::fs::PermissionsExt};
fn action() -> WorkAction<core::EffectId> {
    WorkAction {
        identity: ActionIdentity {
            protocol_version: PROTOCOL_VERSION,
            owner: TenantId::try_from("tenant-7").unwrap(),
            sandbox: SandboxId::try_from("sb-test-1").unwrap(),
            operation: OperationId::try_from("op-test-1").unwrap(),
            epoch: 2,
            generation: 3,
            logical_action: LogicalActionId::try_from("effect-1").unwrap(),
            attempt: 0,
            resource: ResourceRef {
                kind: ResourceKind::Vm,
                id: ResourceId::try_from("vm-test-4").unwrap(),
            },
        },
        command: core::EffectId::initial(core::Command::ActivateVm {
            sandbox: core::SandboxId(1),
            epoch: 2,
            generation: 3,
            vm: core::VmId(4),
        }),
    }
}
struct Fixture {
    root: tempfile::TempDir,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("gate");
        disk::create_dir(&path).unwrap();
        disk::set_permissions(&path, disk::Permissions::from_mode(0o700)).unwrap();
        let value = Self { root, path };
        value.directory().create("config.json",&serde_json::to_vec(&serde_json::json!({"schema_version":1,"run_identity":"a".repeat(64),"sandbox":"sb-test-1","generation":3,"timeout_ms":1000})).unwrap()).unwrap();
        value
    }
    fn directory(&self) -> Directory {
        let fd = File::from(
            fs::open(
                &self.path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap(),
        );
        // The fixture supplies its real private FD and current owner. Production
        // separately checks protected ancestors and requires UID 0.
        Directory::from_opened(self.path.clone(), fd, rustix::process::geteuid().as_raw()).unwrap()
    }
    fn gate(&self) -> RestoreActivationGate {
        RestoreActivationGate::select(self.directory(), &action())
            .unwrap()
            .unwrap()
    }
}
fn command(directory: &Directory, run: &str, digest: &str, value: &str) {
    let file=directory.create("command-stage.json",&serde_json::to_vec(&serde_json::json!({"schema_version":1,"run_identity":run,"observation_sha256":digest,"command":value})).unwrap()).unwrap();
    fs::renameat_with(
        &directory.file,
        "command-stage.json",
        &directory.file,
        "command.json",
        RenameFlags::NOREPLACE,
    )
    .unwrap();
    directory.file.sync_all().unwrap();
    identity_matches(&directory.file, "command.json", &file).unwrap();
}
#[test]
fn actual_record_is_private_and_ready_exposes_only_digest_before_bound_continue() {
    let fixture = Fixture::new();
    let gate = fixture.gate();
    let directory = fixture.directory();
    let consumer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let ready = loop {
            if let Some(bytes) = directory.read("ready.json").unwrap() {
                break bytes;
            }
            assert!(Instant::now() < deadline, "ready never published");
            std::thread::yield_now();
        };
        let ready: serde_json::Value = serde_json::from_slice(&ready).unwrap();
        assert_eq!(ready.as_object().unwrap().len(), 2);
        let path = directory.path.join("observation.json");
        let bytes = disk::read(&path).unwrap();
        assert!(bytes.len() <= OBSERVATION_LIMIT);
        let digest = hex_digest(&bytes);
        assert_eq!(ready["observation_sha256"], digest);
        let observation: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(observation.as_object().unwrap().len(), 6);
        assert_eq!(
            observation["action"],
            serde_json::to_value(action()).unwrap()
        );
        assert_eq!(observation["candidate"]["resource_id"], "vm-test-4");
        assert_eq!(observation["token"], "t".repeat(64));
        assert_eq!(disk::metadata(&path).unwrap().mode() & 0o7777, 0o600);
        command(&directory, &"a".repeat(64), &digest, "continue");
    });
    let process = RuntimeProcessIdentity::capture(std::process::id()).unwrap();
    let namespace = File::open("/proc/self/ns/net").unwrap();
    gate.wait(
        &action(),
        "vm-test-4",
        &process,
        &namespace,
        &"t".repeat(64),
    )
    .unwrap();
    consumer.join().unwrap();
    assert_eq!(disk::read_dir(&fixture.path).unwrap().count(), 4);
    assert!(fixture.path.join("observation.json").exists());
    assert!(
        gate.wait(
            &action(),
            "vm-test-4",
            &process,
            &namespace,
            &"t".repeat(64)
        )
        .is_err()
    );
}
#[test]
fn foreign_generation_selects_no_gate_and_creates_no_observation() {
    let fixture = Fixture::new();
    let mut other = action();
    other.identity.generation = 4;
    assert!(
        RestoreActivationGate::select(fixture.directory(), &other)
            .unwrap()
            .is_none()
    );
    assert_eq!(disk::read_dir(&fixture.path).unwrap().count(), 1);
}
#[test]
fn mismatched_run_digest_fail_or_unknown_command_cannot_release_waiter() {
    for variant in 0..4 {
        let fixture = Fixture::new();
        let gate = fixture.gate();
        let record = gate
            .directory
            .create("observation.json", b"private actual record")
            .unwrap();
        let original = record.metadata().unwrap();
        let digest = hex_digest(b"private actual record");
        let run = if variant == 0 {
            "b".repeat(64)
        } else {
            "a".repeat(64)
        };
        let bound = if variant == 1 {
            "c".repeat(64)
        } else {
            digest.clone()
        };
        command(
            &gate.directory,
            &run,
            &bound,
            match variant {
                2 => "fail",
                3 => "go",
                _ => "continue",
            },
        );
        assert!(
            gate.wait_until(
                &digest,
                &record,
                &original,
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
        assert_eq!(
            disk::read(fixture.path.join("observation.json")).unwrap(),
            b"private actual record"
        );
    }
}
#[test]
fn timeout_keeps_original_observation_and_does_not_imply_success() {
    let fixture = Fixture::new();
    let gate = fixture.gate();
    let record = gate
        .directory
        .create("observation.json", b"private actual record")
        .unwrap();
    assert!(
        gate.wait_until(
            &hex_digest(b"private actual record"),
            &record,
            &record.metadata().unwrap(),
            Instant::now()
        )
        .is_err()
    );
    assert!(fixture.path.join("observation.json").exists());
    assert!(!fixture.path.join("command.json").exists());
}
#[test]
fn replacement_mode_change_and_rebound_config_are_refused() {
    for variant in 0..4 {
        let fixture = Fixture::new();
        let gate = fixture.gate();
        let record = gate
            .directory
            .create("observation.json", b"private actual record")
            .unwrap();
        let original = record.metadata().unwrap();
        match variant {
            0 => {
                disk::rename(
                    fixture.path.join("observation.json"),
                    fixture.root.path().join("original-record"),
                )
                .unwrap();
                gate.directory
                    .create("observation.json", b"private actual record")
                    .unwrap();
            }
            1 => disk::set_permissions(
                fixture.path.join("observation.json"),
                disk::Permissions::from_mode(0o400),
            )
            .unwrap(),
            2 => {
                disk::rename(
                    fixture.path.join("config.json"),
                    fixture.root.path().join("original-config"),
                )
                .unwrap();
                gate.directory
                    .create("config.json", &gate.config_bytes)
                    .unwrap();
            }
            3 => {
                disk::rename(&fixture.path, fixture.root.path().join("original-gate")).unwrap();
                disk::create_dir(&fixture.path).unwrap();
                disk::set_permissions(&fixture.path, disk::Permissions::from_mode(0o700)).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            gate.wait_until(
                &hex_digest(b"private actual record"),
                &record,
                &original,
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
    }
}
#[test]
fn partial_oversized_or_nonprivate_control_is_not_a_release() {
    for variant in 0..3 {
        let fixture = Fixture::new();
        let gate = fixture.gate();
        let record = gate
            .directory
            .create("observation.json", b"private actual record")
            .unwrap();
        let bytes = if variant == 0 {
            b"{".to_vec()
        } else {
            vec![b'x'; 1025]
        };
        let file = gate.directory.create("command.json", &bytes).unwrap();
        if variant == 2 {
            file.set_permissions(disk::Permissions::from_mode(0o644))
                .unwrap();
        }
        assert!(
            gate.wait_until(
                &hex_digest(b"private actual record"),
                &record,
                &record.metadata().unwrap(),
                Instant::now() + Duration::from_secs(1)
            )
            .is_err()
        );
    }
}
