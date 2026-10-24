//! Actual pure profile/parameter guards with controlled data. Not a live VM,
//! CPU certificate, durable plan, protected-layout success or Full completion.
use super::super::capture_profile::ProtectedCaptureSettings;
use super::*;
use sandproof_controller_service::capture::SourceProcessIdentity;
use sandproof_controller_service::templates::{ResourceShape, TemplateArtifact};
use sandproof_protocol::{ResourceKind, ResourceRef};
use serde_json::json;

fn template() -> TemplateBinding {
    TemplateBinding {
        template_id: "base".into(),
        revision: "pinned".into(),
        owner: Some(7),
        kernel: TemplateArtifact {
            path: "/protected/kernel".into(),
            sha256: "02".repeat(32),
            size_bytes: 1024,
        },
        rootfs: TemplateArtifact {
            path: "/protected/rootfs".into(),
            sha256: "03".repeat(32),
            size_bytes: 8192,
        },
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            disk_bytes: 8192,
            host_memory_bytes: 256 * 1_048_576,
        },
        envd_version: "fixed".into(),
        guest: None,
        startup: None,
    }
}
fn profile() -> VmCompatibility {
    expected_compatibility(
        &template(),
        [4; 32],
        [1; 32],
        ContentDigest::from_sha256([5; 32]),
    )
    .unwrap()
}
fn raw() -> RawSourceObservation {
    RawSourceObservation {
        work_binding: ContentDigest::from_sha256([7; 32]),
        resource: ResourceRef {
            kind: ResourceKind::Vm,
            id: ResourceId::try_from("vm-0123456789abcdef-0").unwrap(),
        },
        source: SourceProcessIdentity {
            pid: 11,
            start_time_ticks: 13,
            boot_id: "01234567-89ab-cdef-0123-456789abcdef".into(),
        },
        configuration: b"visible fields already checked by the real Runtime provider".to_vec(),
        executable_sha256: [1; 32],
        kernel_sha256: [2; 32],
        kernel_bytes: 1024,
        disk_bytes: 8192,
    }
}
fn request() -> FullCaptureWorkRequest {
    serde_json::from_value(json!({
        "pause":{"binding":{"operation":{"protocol_version":sandproof_protocol::lifecycle::PROTOCOL_VERSION,
            "node":{"deployment_id":"deployment","node_id":"node","node_generation":1},
            "session":"session","key":{"deployment":"deployment","owner":"owner","window":1,"sequence":1},
            "expected":{"state":"version","version":3}},"source_incarnation":2,"capture":4},
            "source_work_binding":ContentDigest::from_sha256([7;32]),
            "node":{"deployment_id":"deployment","node_id":"node","node_generation":1},
            "source_resource":raw().resource,"source":raw().source},
        "compatibility":profile(),"state_bytes":1_000_000,"memory_bytes":128*1_048_576,
        "disk_bytes":8192,"api_timeout_ms":1000,"worker_timeout_ms":300000
    })).unwrap()
}
#[test]
fn independent_profile_binds_exact_template_device_tool_cpu_and_fixed_format() {
    let template = template();
    let expected = profile();
    assert_eq!(expected.snapshot_format, 10);
    assert_eq!(expected.tool.as_bytes(), &[1; 32]);
    assert_eq!(expected.devices.as_bytes(), &[4; 32]);
    assert_eq!(expected.cpu.as_bytes(), &[5; 32]);
    assert_eq!(
        expected.writable_drives[0].jail_relative_path,
        "rootfs.ext4"
    );
    let mut changed = template.clone();
    changed.revision = "other".into();
    assert_ne!(
        expected_compatibility(&changed, [4; 32], [1; 32], expected.cpu).unwrap(),
        expected
    );
    for (device, tool, cpu) in [
        ([8; 32], [1; 32], expected.cpu),
        ([4; 32], [8; 32], expected.cpu),
        ([4; 32], [1; 32], ContentDigest::from_sha256([8; 32])),
    ] {
        assert_ne!(
            expected_compatibility(&template, device, tool, cpu).unwrap(),
            expected
        );
    }
    changed = template.clone();
    changed.resources.cpu_count = 33;
    assert!(expected_compatibility(&changed, [4; 32], [1; 32], expected.cpu).is_err());
    changed.kernel.sha256 = "bad".into();
    assert!(expected_compatibility(&changed, [4; 32], [1; 32], expected.cpu).is_err());
}
#[test]
fn actual_source_file_guard_refuses_executable_kernel_or_disk_mismatch() {
    assert!(check_measured_source(&template(), &raw(), &profile()).is_ok());
    for change in 0..4 {
        let mut actual = raw();
        match change {
            0 => actual.executable_sha256 = [9; 32],
            1 => actual.kernel_sha256 = [9; 32],
            2 => actual.kernel_bytes += 1,
            _ => actual.disk_bytes += 1,
        }
        assert!(check_measured_source(&template(), &actual, &profile()).is_err());
    }
}
#[test]
fn work_dto_cannot_replace_independent_profile_binding_or_any_copy_parameter() {
    let expected = request();
    assert!(expected.valid());
    assert!(require_expected_request(&expected, &expected).is_ok());
    for field in [
        "state_bytes",
        "memory_bytes",
        "disk_bytes",
        "api_timeout_ms",
        "worker_timeout_ms",
    ] {
        let mut actual = serde_json::to_value(&expected).unwrap();
        actual[field] = json!(actual[field].as_u64().unwrap() + 1);
        let actual = serde_json::from_value(actual).unwrap();
        assert!(
            require_expected_request(&actual, &expected).is_err(),
            "accepted {field}"
        );
    }
    let mut changed = expected.clone();
    changed.compatibility.cpu = ContentDigest::from_sha256([9; 32]);
    assert!(changed.valid());
    assert!(require_expected_request(&changed, &expected).is_err());
    changed = expected.clone();
    changed.pause.source.pid += 1;
    assert!(require_expected_request(&changed, &expected).is_err());
    changed = expected.clone();
    changed.pause.binding.capture = sandproof_protocol::lifecycle::CaptureId::try_from(9).unwrap();
    assert!(require_expected_request(&changed, &expected).is_err());
}
#[test]
fn committed_read_scope_binds_persisted_work_without_old_vm_state() {
    let expected = request();
    let caps = BundleReadLimits {
        state_bytes: expected.state_bytes,
        memory_bytes: expected.memory_bytes,
        disk_bytes: expected.disk_bytes,
    };
    let node = &expected.pause.node;
    let epoch = expected.compatibility.cpu;
    assert!(require_committed_read_scope(&expected, node, epoch, caps).is_ok());
    let mut foreign = expected.clone();
    foreign.pause.node.node_generation = foreign.pause.node.node_generation.checked_next().unwrap();
    assert!(require_committed_read_scope(&foreign, node, epoch, caps).is_err());
    foreign = expected.clone();
    foreign.compatibility.cpu = ContentDigest::from_sha256([9; 32]);
    assert!(require_committed_read_scope(&foreign, node, epoch, caps).is_err());
    for field in ["state_bytes", "memory_bytes", "disk_bytes"] {
        let mut altered = serde_json::to_value(&expected).unwrap();
        altered[field] = json!(altered[field].as_u64().unwrap() + 1);
        let altered = serde_json::from_value(altered).unwrap();
        assert!(
            require_committed_read_scope(&altered, node, epoch, caps).is_err(),
            "accepted {field}"
        );
    }
    foreign = expected.clone();
    foreign.memory_bytes = 0;
    assert!(require_committed_read_scope(&foreign, node, epoch, caps).is_err());
}
#[test]
fn protected_copy_settings_do_not_accept_unbounded_or_zero_values() {
    let good = ProtectedCaptureSettings {
        layout_parent: "/protected/layout".into(),
        layout_binding: ContentDigest::from_sha256([1; 32]),
        state_copy_bytes: 10_000_000,
        memory_copy_bytes: 128 * 1_048_576,
        disk_copy_bytes: 8192,
        api_timeout_ms: 1000,
        worker_timeout_ms: 300_000,
    };
    assert!(good.validate().is_ok());
    for (field, value) in [
        ("state_copy_bytes", 0),
        ("state_copy_bytes", 10_000_001),
        ("memory_copy_bytes", 0),
        ("memory_copy_bytes", u64::MAX),
        ("disk_copy_bytes", 0),
        ("api_timeout_ms", 999),
        ("api_timeout_ms", 300_001),
        ("worker_timeout_ms", 999),
        ("worker_timeout_ms", 300_001),
    ] {
        let mut changed = serde_json::to_value(&good).unwrap();
        changed[field] = json!(value);
        let changed: ProtectedCaptureSettings = serde_json::from_value(changed).unwrap();
        assert!(changed.validate().is_err());
    }
    let mut missing = serde_json::to_value(&good).unwrap();
    missing.as_object_mut().unwrap().remove("worker_timeout_ms");
    assert!(serde_json::from_value::<ProtectedCaptureSettings>(missing).is_err());
    let shorter_than_api = ProtectedCaptureSettings {
        api_timeout_ms: 2_000,
        worker_timeout_ms: 1_000,
        ..good.clone()
    };
    assert!(shorter_than_api.validate().is_err());
    let bad = ProtectedCaptureSettings {
        layout_parent: "relative".into(),
        ..good
    };
    assert!(bad.validate().is_err());
}
