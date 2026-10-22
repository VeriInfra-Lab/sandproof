//! Controlled values/leaf files test the same production guards; they are not
//! actual VM, CPU compatibility or registered-deployment certificates.
use super::*;
use sandproof_protocol::{
    DeploymentId, NodeId, internal::PositiveCounter, lifecycle::ContentDigest,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
};
const BOOT: &str = "01234567-89ab-cdef-0123-456789abcdef";
const NAMESPACE: &str = "0123456789abcdef";
fn node() -> NodeIdentity {
    NodeIdentity {
        deployment_id: DeploymentId::try_from("bootstrap-test").unwrap(),
        node_id: NodeId::try_from("node-test").unwrap(),
        node_generation: PositiveCounter::try_from(1).unwrap(),
    }
}
fn value() -> serde_json::Value {
    serde_json::json!({
        "version":1,"purpose":"host_bootstrap","node":node(),"namespace":NAMESPACE,
        "cold_profile":"rootfs_eth0_mmds_v2_v1","cpu_template":"host_default_v1",
        "host": {
            "boot_id":BOOT,"kernel":{"release":"fixed-kernel","version":"fixed-build","architecture":"x86_64"},
            "cpus":{"online":[0],"manager_allowed":[0],"descriptions":[{"processor":0,"stable":{"vendor_id":"GenuineIntel","microcode":"0x123"}}]},
            "firecracker":{"device":1,"inode":2,"length":64,"mode":448,"group":0,"sha256":vec![1_u8;32]},
            "jailer":{"device":1,"inode":3,"length":64,"mode":448,"group":0,"sha256":vec![2_u8;32]},
            "kvm_device":{"device":4,"inode":5,"special_device":6}
        }
    })
}
fn checked(
    root: &Path,
    body: &serde_json::Value,
    name: &str,
) -> (CheckedFile, HostEpochRegistration) {
    let path = root.join(name);
    let bytes = serde_json::to_vec(body).unwrap();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(&path)
        .unwrap();
    file.write_all(&bytes).unwrap();
    let owner = file.metadata().unwrap().uid();
    drop(file);
    let actual = files::controlled_payload_leaf(&path, owner).unwrap();
    let registration = HostEpochRegistration {
        version: 1,
        boot_id: BOOT.into(),
        policy: ContentDigest::from_sha256(Sha256::digest(&bytes).into()),
    };
    (actual, registration)
}
#[test]
fn original_full_digest_node_namespace_and_boot_are_bound() {
    let root = tempfile::tempdir().unwrap();
    let (file, registration) = checked(root.path(), &value(), "original");
    let payload = Payload::read_bound(&file, &node(), &registration, NAMESPACE).unwrap();
    payload.require_reading(&payload.host).unwrap();
    let wrong = HostEpochRegistration {
        policy: ContentDigest::from_sha256([0; 32]),
        ..registration.clone()
    };
    assert!(Payload::read_bound(&file, &node(), &wrong, NAMESPACE).is_err());
    let other = NodeIdentity {
        node_generation: PositiveCounter::try_from(2).unwrap(),
        ..node()
    };
    assert!(Payload::read_bound(&file, &other, &registration, NAMESPACE).is_err());
    assert!(Payload::read_bound(&file, &node(), &registration, "another-namespace").is_err());
    let wrong = HostEpochRegistration {
        boot_id: "ffffffff-ffff-ffff-ffff-ffffffffffff".into(),
        ..registration.clone()
    };
    assert!(Payload::read_bound(&file, &node(), &wrong, NAMESPACE).is_err());
    let wrong = HostEpochRegistration {
        version: 2,
        ..registration
    };
    assert!(Payload::read_bound(&file, &node(), &wrong, NAMESPACE).is_err());
}
#[test]
fn wrong_purpose_or_missing_unknown_profile_fields_never_form_bootstrap() {
    let root = tempfile::tempdir().unwrap();
    for (index, field, replacement) in [
        (0, "purpose", "capture"),
        (1, "cold_profile", "general_vm"),
        (2, "cpu_template", "custom"),
    ] {
        let mut body = value();
        body[field] = replacement.into();
        let (file, registration) = checked(root.path(), &body, &format!("wrong-{index}"));
        assert!(Payload::read_bound(&file, &node(), &registration, NAMESPACE).is_err());
    }
    let mut body = value();
    body.as_object_mut().unwrap().remove("purpose");
    let (file, registration) = checked(root.path(), &body, "missing");
    assert!(Payload::read_bound(&file, &node(), &registration, NAMESPACE).is_err());
    let mut body = value();
    body["actual_vm_xsave_bytes"] = 4096.into();
    let (file, registration) = checked(root.path(), &body, "unobserved-extra-fact");
    assert!(Payload::read_bound(&file, &node(), &registration, NAMESPACE).is_err());
}
#[test]
fn current_tool_kernel_cpu_and_device_changes_are_refused() {
    let body = value();
    let payload: Payload = serde_json::from_value(body.clone()).unwrap();
    for pointer in [
        "/host/boot_id",
        "/host/kernel/release",
        "/host/kernel/version",
        "/host/cpus/descriptions/0/stable/microcode",
    ] {
        let mut current = body.clone();
        *current.pointer_mut(pointer).unwrap() = "changed".into();
        let current: Payload = serde_json::from_value(current).unwrap();
        assert!(payload.require_reading(&current.host).is_err());
    }
    for pointer in [
        "/host/firecracker/sha256/0",
        "/host/jailer/sha256/0",
        "/host/kvm_device/inode",
        "/host/cpus/manager_allowed/0",
    ] {
        let mut current = body.clone();
        *current.pointer_mut(pointer).unwrap() = 9.into();
        let current: Payload = serde_json::from_value(current).unwrap();
        assert!(payload.require_reading(&current.host).is_err());
    }
}
#[test]
fn configured_path_is_not_a_registration_or_permission() {
    assert!(ProtectedHostEpochSource::at("relative/payload".into()).is_err());
    assert!(ProtectedHostEpochSource::at("/root/../payload".into()).is_err());
    assert!(ProtectedHostEpochSource::at("/".into()).is_err());
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("not-created");
    let source = ProtectedHostEpochSource::at(path.clone()).unwrap();
    let registration = HostEpochRegistration {
        version: 1,
        boot_id: BOOT.into(),
        policy: ContentDigest::from_sha256([0; 32]),
    };
    assert!(
        source
            .observe_original(
                &node(),
                &registration,
                Inputs {
                    namespace: NAMESPACE,
                    firecracker: Path::new("/not-executed"),
                    jailer: Path::new("/not-executed")
                }
            )
            .is_err()
    );
    assert!(!path.exists());
}

#[test]
#[ignore = "requires root and /dev/kvm; only protected metadata and read-only facts, never a VM"]
fn root_protected_bootstrap_roundtrip_uses_real_host_facts_without_a_vm() {
    assert_eq!(rustix::process::geteuid().as_raw(), 0);
    let root = tempfile::Builder::new()
        .prefix("sandproof-bootstrap-")
        .tempdir_in("/run")
        .unwrap();
    let firecracker = root.path().join("synthetic-firecracker");
    let jailer = root.path().join("synthetic-jailer");
    for path in [&firecracker, &jailer] {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(path)
            .unwrap();
        file.write_all(b"synthetic tool bytes, never executed")
            .unwrap();
    }
    let inputs = Inputs {
        namespace: NAMESPACE,
        firecracker: &firecracker,
        jailer: &jailer,
    };
    let host = HostFacts::read(inputs).unwrap();
    let payload = Payload {
        version: 1,
        purpose: "host_bootstrap".into(),
        node: node(),
        namespace: NAMESPACE.into(),
        cold_profile: ColdProfile::RootfsEth0MmdsV2V1,
        cpu_template: CpuTemplate::HostDefaultV1,
        host,
    };
    let bytes = serde_json::to_vec(&payload).unwrap();
    let path = root.path().join("registered.json");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(&path)
        .unwrap();
    file.write_all(&bytes).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let registration = HostEpochRegistration {
        version: 1,
        boot_id: payload.host.boot_id.clone(),
        policy: ContentDigest::from_sha256(Sha256::digest(&bytes).into()),
    };
    let source = ProtectedHostEpochSource::at(path).unwrap();
    let observed = source
        .observe_original(&node(), &registration, inputs)
        .unwrap();
    assert_eq!(
        observed,
        HostEpochObservation {
            node: node(),
            registration: registration.clone()
        }
    );
    let replaceable = root.path().join("replaceable");
    fs::create_dir(&replaceable).unwrap();
    fs::set_permissions(
        &replaceable,
        std::os::unix::fs::PermissionsExt::from_mode(0o777),
    )
    .unwrap();
    let untrusted_path = replaceable.join("registered.json");
    let mut untrusted = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(&untrusted_path)
        .unwrap();
    untrusted.write_all(&bytes).unwrap();
    drop(untrusted);
    let untrusted = ProtectedHostEpochSource::at(untrusted_path).unwrap();
    assert!(
        untrusted
            .observe_original(&node(), &registration, inputs)
            .is_err()
    );
    fs::set_permissions(
        &firecracker,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .unwrap();
    fs::write(&firecracker, b"different synthetic bytes").unwrap();
    assert!(
        source
            .observe_original(&node(), &registration, inputs)
            .is_err()
    );
    // All fixture files are below this owned /run directory; drop removes only
    // those files. No tool was executed and no VM, mount or quota was created.
}
