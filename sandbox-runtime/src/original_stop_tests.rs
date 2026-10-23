//! Ordinary Linux fixtures: own gated children and private files, no VM/root.
use super::*;
use crate::linux::{PROCESS_TEST_LOCK, RuntimeConfig};
use std::{
    fs,
    process::{Child, Command, Stdio},
};

const ID: &str = "original-stop-test";

struct OwnedTestProcess(Child);
impl Drop for OwnedTestProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn process() -> (OwnedTestProcess, RuntimeProcessIdentity) {
    let child = Command::new("/bin/sh")
        .args(["-c", "IFS= read -r finish", "stop-fixture", "--id", ID])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let source = RuntimeProcessIdentity::capture(child.id()).unwrap();
    (OwnedTestProcess(child), source)
}

fn record(source: &RuntimeProcessIdentity) -> ProcessRecord {
    ProcessRecord {
        resource_id: ID.into(),
        pid: source.pid,
        start_time_ticks: source.start_time_ticks,
        boot_id: source.boot_id.clone(),
        launch_cpu_domain: vec![0],
    }
}

fn save(runtime: &Runtime, record: &ProcessRecord) {
    fs::write(
        runtime
            .config
            .chroot_base
            .join(".claims")
            .join(ID)
            .join("record.json"),
        serde_json::to_vec(record).unwrap(),
    )
    .unwrap();
}

fn save_without_boot(runtime: &Runtime, record: &ProcessRecord) -> Vec<u8> {
    let mut value = serde_json::to_value(record).unwrap();
    value.as_object_mut().unwrap().remove("boot_id");
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(
        runtime
            .config
            .chroot_base
            .join(".claims")
            .join(ID)
            .join("record.json"),
        &bytes,
    )
    .unwrap();
    bytes
}

fn fixture(source: &RuntimeProcessIdentity) -> (tempfile::TempDir, Runtime) {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir_all(temp.path().join(".claims").join(ID)).unwrap();
    let runtime = Runtime {
        // Private controlled fixture only. Production constructors retain all
        // root/KVM/deployment checks, with no test switch in the public API.
        config: RuntimeConfig {
            console_policy: None,
            host_slot: None,
            jailer: "/not-invoked/jailer".into(),
            firecracker: "/not-invoked/firecracker".into(),
            chroot_base: temp.path().into(),
            kernel: "/not-read/kernel".into(),
            rootfs: "/not-read/rootfs".into(),
            vm_uid: 64001,
            vm_gid: 64001,
            vcpu_count: 1,
            memory_mib: 512,
            host_resources: None,
            private_netns: None,
            access_token_hash: None,
        },
    };
    save(&runtime, &record(source));
    (temp, runtime)
}

fn alternate_boot(source: &RuntimeProcessIdentity) -> String {
    if source.boot_id == "ffffffff-ffff-ffff-ffff-ffffffffffff" {
        "00000000-0000-0000-0000-000000000000".into()
    } else {
        "ffffffff-ffff-ffff-ffff-ffffffffffff".into()
    }
}

#[test]
fn exact_source_stops_and_replays_without_claim_or_material_deletion() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let (mut child, source) = process();
    let (temp, runtime) = fixture(&source);
    let claim = temp.path().join(".claims").join(ID).join("record.json");
    let saved = fs::read(&claim).unwrap();
    for path in ["s/capture/original-material", "v/original-jail/root/disk"] {
        let path = temp.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"must remain").unwrap();
    }
    assert!(runtime.observe_original_stopped(ID, &source).is_err());
    let stopped = runtime.stop_original_vm(ID, &source).unwrap();
    assert_eq!(stopped.state, VmState::Stopped);
    assert_eq!(stopped.pid, source.pid);
    assert_eq!(stopped.start_time_ticks, source.start_time_ticks);
    assert_eq!(
        runtime.observe_original_stopped(ID, &source).unwrap(),
        stopped
    );
    // Stop confirms no further execution. It does not itself reap an own
    // child or justify releasing this still-occupied process capacity cell.
    assert!(source.is_alive().unwrap());
    child.0.wait().unwrap();
    assert!(!source.is_alive().unwrap());
    assert_eq!(runtime.stop_original_vm(ID, &source).unwrap(), stopped);
    assert_eq!(fs::read(claim).unwrap(), saved);
    for path in ["s/capture/original-material", "v/original-jail/root/disk"] {
        assert_eq!(fs::read(temp.path().join(path)).unwrap(), b"must remain");
    }
}

#[test]
fn mismatched_original_source_is_refused_before_signalling() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let (mut child, source) = process();
    let (_temp, runtime) = fixture(&source);
    for wrong in [
        RuntimeProcessIdentity {
            pid: 0,
            ..source.clone()
        },
        RuntimeProcessIdentity {
            start_time_ticks: source.start_time_ticks + 1,
            ..source.clone()
        },
        RuntimeProcessIdentity {
            boot_id: alternate_boot(&source),
            ..source.clone()
        },
    ] {
        assert!(runtime.stop_original_vm(ID, &wrong).is_err());
        assert!(runtime.observe_original_stopped(ID, &wrong).is_err());
        assert!(child.0.try_wait().unwrap().is_none());
    }
    save_without_boot(&runtime, &record(&source));
    assert!(runtime.stop_original_vm(ID, &source).is_err());
    assert!(runtime.observe_original_stopped(ID, &source).is_err());
    assert!(child.0.try_wait().unwrap().is_none());
}

#[test]
fn replaced_claim_and_unknown_record_never_authorize_original_stop() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let (mut original, source) = process();
    let (mut replacement, other) = process();
    let (temp, runtime) = fixture(&source);
    save(&runtime, &record(&other));
    assert!(runtime.stop_original_vm(ID, &source).is_err());
    assert!(runtime.observe_original_stopped(ID, &source).is_err());
    let path = temp.path().join(".claims").join(ID).join("record.json");
    fs::write(&path, b"incomplete").unwrap();
    assert!(runtime.stop_original_vm(ID, &source).is_err());
    assert!(runtime.observe_original_stopped(ID, &source).is_err());
    fs::remove_file(&path).unwrap();
    assert!(matches!(
        runtime.stop_original_vm(ID, &source),
        Err(RuntimeError::MissingRecord)
    ));
    assert!(runtime.observe_original_stopped(ID, &source).is_err());
    assert!(runtime.stop_original_vm("../foreign", &source).is_err());
    assert!(original.0.try_wait().unwrap().is_none());
    assert!(replacement.0.try_wait().unwrap().is_none());
}

#[test]
fn a_foreign_boot_is_unresolved_and_does_not_signal_the_current_pid_occupant() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let (mut child, current) = process();
    let old = RuntimeProcessIdentity {
        boot_id: alternate_boot(&current),
        ..current.clone()
    };
    let (_temp, runtime) = fixture(&old);
    assert!(runtime.stop_original_vm(ID, &old).is_err());
    assert!(runtime.observe_original_stopped(ID, &old).is_err());
    assert!(child.0.try_wait().unwrap().is_none());
    assert!(current.is_alive().unwrap());
}

#[test]
fn readonly_configuration_refuses_wrong_or_missing_source_without_effects() {
    let _serial = PROCESS_TEST_LOCK.lock().unwrap();
    let (mut child, source) = process();
    let (temp, runtime) = fixture(&source);
    let limits = crate::SourceConfigurationLimits {
        executable_bytes: 64 * 1024 * 1024,
        kernel_bytes: 256 * 1024 * 1024,
        disk_bytes: 1024,
        configuration_bytes: 1024,
    };
    let claim = temp.path().join(".claims").join(ID).join("record.json");
    let saved = fs::read(&claim).unwrap();
    let canary = temp.path().join("preserved-original-state");
    fs::write(&canary, b"unchanged").unwrap();
    let wrong = RuntimeProcessIdentity {
        start_time_ticks: source.start_time_ticks + 1,
        ..source.clone()
    };
    assert!(
        runtime
            .observe_original_configuration(ID, &wrong, limits)
            .is_err()
    );
    assert_eq!(fs::read(&claim).unwrap(), saved);
    let missing_boot = save_without_boot(&runtime, &record(&source));
    assert!(
        runtime
            .observe_original_configuration(ID, &source, limits)
            .is_err()
    );
    assert_eq!(fs::read(&claim).unwrap(), missing_boot);
    assert_eq!(fs::read(&canary).unwrap(), b"unchanged");
    assert!(!runtime.root_path(ID).exists());
    assert!(child.0.try_wait().unwrap().is_none());
}
