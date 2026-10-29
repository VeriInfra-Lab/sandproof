//! Gate protocol only. Synthetic bindings here are not physical GC evidence.
use super::*;
use rustix::fs::{self, Mode, OFlags};
use sandproof_protocol::{ResourceId, ResourceKind};
use std::{fs as disk, os::unix::fs::PermissionsExt, path::Path};

fn directory(path: &Path) -> Directory {
    let file = File::from(
        fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap(),
    );
    // Production Directory::open still requires root-private trusted ancestry.
    Directory::from_opened(path.into(), file, rustix::process::geteuid().as_raw()).unwrap()
}
fn material() -> Material {
    Material {
        sandbox: 1,
        generation: 2,
        capture_key: format!("capture-{}", "c".repeat(64)),
        work_binding: "d".repeat(64),
        plan_binding: "e".repeat(64),
        plan_bytes: 1371,
    }
}
fn source() -> ResourceRef {
    ResourceRef {
        kind: ResourceKind::Vm,
        id: ResourceId::try_from("vm-test-2").unwrap(),
    }
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
                "schema_version":1,"run_identity":"a".repeat(64),"sandbox":1,
                "generation":2,"capture_key":material().capture_key,"timeout_ms":1000,
            }))
            .unwrap(),
        )
        .unwrap();
    (root, path)
}
fn instruction(path: &Path, mode: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !path.join("ready.json").exists() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let bytes = disk::read(path.join("observation.json")).unwrap();
    let observed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let ready: serde_json::Value =
        serde_json::from_slice(&disk::read(path.join("ready.json")).unwrap()).unwrap();
    assert_eq!(ready["observation_sha256"], hex_digest(&bytes));
    assert_eq!(observed["phase"], PHASE);
    assert_eq!(
        observed["material"],
        serde_json::to_value(material()).unwrap()
    );
    assert_eq!(
        observed["source_resource"],
        serde_json::to_value(source()).unwrap()
    );
    let consumed: Consumption =
        serde_json::from_slice(&disk::read(path.join("consumed.json")).unwrap()).unwrap();
    assert_eq!(consumed.material, material());
    assert_eq!(consumed.observation_sha256, hex_digest(&bytes));
    let mut value = serde_json::json!({"schema_version":1,"run_identity":"a".repeat(64),
        "observation_sha256":hex_digest(&bytes),"material":material(),
        "command":if mode=="fail" {"fail"} else {"release"}});
    match mode {
        "wrong-run" => value["run_identity"] = serde_json::json!("b".repeat(64)),
        "wrong-observation" => value["observation_sha256"] = serde_json::json!("f".repeat(64)),
        "wrong-capture" => {
            value["material"]["capture_key"] =
                serde_json::json!(format!("capture-{}", "f".repeat(64)))
        }
        "wrong-work" => value["material"]["work_binding"] = serde_json::json!("f".repeat(64)),
        "wrong-plan" => value["material"]["plan_binding"] = serde_json::json!("f".repeat(64)),
        "wrong-length" => value["material"]["plan_bytes"] = serde_json::json!(1372),
        "wrong-command" => value["command"] = serde_json::json!("settle"),
        _ => {}
    }
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(bytes.len() <= 1024);
    // Publish the complete command, rather than racing a named partial write.
    directory(path)
        .create("command-stage.json", &bytes)
        .unwrap();
    disk::rename(path.join("command-stage.json"), path.join("command.json")).unwrap();
    File::open(path).unwrap().sync_all().unwrap();
}

#[test]
fn release_or_failure_requires_the_exact_work_capture_plan_length_and_observation() {
    for mode in [
        "release",
        "fail",
        "wrong-run",
        "wrong-observation",
        "wrong-capture",
        "wrong-work",
        "wrong-plan",
        "wrong-length",
        "wrong-command",
    ] {
        let (_root, path) = fixture();
        let gate = Gate::select(directory(&path), &material())
            .unwrap()
            .unwrap();
        let control = path.clone();
        let thread = std::thread::spawn(move || instruction(&control, mode));
        let result = gate.wait(&material(), &source());
        thread.join().unwrap();
        assert_eq!(result.is_ok(), mode == "release");
        // Reopening skips only this one test cut for the exact same plan.
        assert!(
            Gate::select(directory(&path), &material())
                .unwrap()
                .is_none()
        );
        for variant in 0..4 {
            let mut changed = material();
            match variant {
                0 => changed.work_binding = "f".repeat(64),
                1 => changed.plan_binding = "f".repeat(64),
                2 => changed.plan_bytes += 1,
                3 => changed.capture_key = format!("capture-{}", "f".repeat(64)),
                _ => unreachable!(),
            }
            let selected = Gate::select(directory(&path), &changed);
            if variant == 3 {
                assert!(selected.unwrap().is_none());
            } else {
                assert!(selected.is_err());
            }
        }
    }
}

#[test]
fn another_capture_in_the_same_generation_does_not_select_or_consume_the_gate() {
    let (_root, path) = fixture();
    let mut other = material();
    other.capture_key = format!("capture-{}", "f".repeat(64));
    assert!(Gate::select(directory(&path), &other).unwrap().is_none());
    assert!(!path.join("observation.json").exists());
    assert!(!path.join("consumed.json").exists());
    assert!(!path.join("ready.json").exists());
    assert!(
        Gate::select(directory(&path), &material())
            .unwrap()
            .is_some()
    );
}

#[test]
fn malformed_consumption_is_not_a_successful_restart_bypass() {
    let (_root, path) = fixture();
    directory(&path)
        .create("consumed.json", br#"{"schema_version":1}"#)
        .unwrap();
    assert!(Gate::select(directory(&path), &material()).is_err());
    assert!(!path.join("ready.json").exists());
}
