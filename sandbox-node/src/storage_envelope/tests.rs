use super::*;
use sandbox_controller_core::{
    PolicyDigest,
    maintenance::{MaintenanceLimits, windows::WindowLimits},
};
use sandproof_controller_service::BoundedHostConfig;
use std::path::PathBuf;

fn config() -> NodeConfig {
    NodeConfig {
        namespace: "0123456789abcdef".into(),
        state_dir: "/host/node".into(),
        jailer: "/usr/bin/jailer".into(),
        firecracker: "/usr/bin/firecracker".into(),
        chroot_base: "/host/jail".into(),
        kernel: "/images/kernel".into(),
        rootfs: "/images/rootfs".into(),
        bounded_host: BoundedHostConfig {
            physical_storage: Some(super::super::test_support::physical_profile(
                "0123456789abcdef",
                std::path::Path::new("/host/node"),
                std::path::Path::new("/host/jail"),
                2,
            )),
            console_policy: Some(sandproof_controller_service::ConsolePolicy::Discard),
            maintenance: MaintenanceLimits {
                max_host_slots: 2,
                max_retained_results: 16,
                windows: WindowLimits::new(2, 1000).unwrap(),
            },
            uid_base: 10_000,
            uplink_base: 100,
            max_data_routes_per_slot: 4,
            max_data_routes_global: 5,
            data_route_control_reserve: 1,
        },
        vcpu_count: 1,
        memory_mib: 128,
        host_resources: None,
        file_policy_digest: PolicyDigest(1),
    }
}

#[test]
fn exact_closed_layout_caps_include_free_cells_stages_and_idle_routes() {
    let config = config();
    let (identity, lease) = validate_configuration(&config).unwrap();
    let bounds = HostMetadataEnvelope::derive(&config, 1).unwrap();
    // Guard the audited layout. Additions must update the derived expectation.
    assert_eq!(CLAIM_RECORD_NAMES.len(), 13);
    assert_eq!(
        host_slots::retirement_paths(&config, host_slots::maximum_binding()).len(),
        5
    );
    assert_eq!(node_staging::STAGE_FILES, ["identity.json", "payload"]);
    // N=2, L=1, D=4: T=10; routes use all N*D fixed cells, not global=5.
    assert_eq!(
        bounds.payload_bytes,
        10 * (2 * node_staging::MAX_PAYLOAD as u64 + identity)
            + lease
            + 26 * MAX_CLAIM_RECORD_BYTES
            + 20 * MAX_HELPER_MARKER_BYTES
    );
    assert_eq!(bounds.file_count, 77);
    assert_eq!(bounds.directory_count, 19);
    assert_eq!(bounds.inode_count, 96);
    let all_live = HostMetadataEnvelope::derive(&config, u64::MAX).unwrap();
    assert_eq!(all_live.file_count, 118);
    assert_eq!(all_live.directory_count, 25);
    assert_eq!(all_live.inode_count, 143);
}

#[test]
fn configured_path_escaping_and_maximum_incarnation_are_serialized_exactly() {
    let mut config = config();
    config.state_dir = PathBuf::from("/host/quotes-\"-backslash-\\-newline-\n");
    config.bounded_host.physical_storage = Some(super::super::test_support::physical_profile(
        &config.namespace,
        &config.state_dir,
        &config.chroot_base,
        config.bounded_host.maintenance.max_host_slots,
    ));
    let (actual_identity, actual_lease) = validate_configuration(&config).unwrap();
    let mut targets = host_slots::retirement_paths(&config, host_slots::maximum_binding());
    targets.extend([
        config.state_dir.join("host-slot-config.json"),
        host_slots::host_position_path(&config, HostSlotId(1)),
        host_slots::gate_path(&config, HostSlotId(1)),
    ]);
    let expected = targets
        .iter()
        .map(|target| {
            serde_json::to_vec(
                &serde_json::json!({"version":1,"mapping":"0".repeat(64),"target":target}),
            )
            .unwrap()
            .len() as u64
        })
        .max()
        .unwrap();
    assert_eq!(actual_identity, expected);
    assert_eq!(
        actual_lease,
        serde_json::to_vec(&(1_u32, &config)).unwrap().len() as u64
    );
    assert!(
        targets
            .iter()
            .any(|path| path.to_string_lossy().contains(&u64::MAX.to_string()))
    );
    assert!(HostMetadataEnvelope::derive(&config, 1).is_ok());
}

#[test]
fn future_retirement_is_preflighted_before_any_lease_or_gate_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config();
    config.state_dir = dir.path().to_owned();
    // The immutable config/gate still fit, but retirement stores RuntimeConfig
    // twice, including its operator-selected jailer path.
    config.jailer = PathBuf::from(format!("/{}", "x".repeat(node_staging::MAX_PAYLOAD / 2)));
    let (mapping, gate, retired) = host_slots::maximum_record_bytes(&config).unwrap();
    assert!(mapping < node_staging::MAX_PAYLOAD as u64);
    assert!(gate < node_staging::MAX_PAYLOAD as u64);
    assert!(retired > node_staging::MAX_PAYLOAD as u64);
    assert!(HostMetadataEnvelope::derive(&config, 1).is_err());
    assert!(super::super::host_lease::HostLease::acquire(&config, None).is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn missing_mapping_zero_capacity_and_arithmetic_overflow_fail_closed() {
    let mut config = config();
    assert!(HostMetadataEnvelope::derive(&config, 0).is_err());
    let bounded = &mut config.bounded_host;
    bounded.max_data_routes_per_slot = u64::MAX;
    bounded.max_data_routes_global = u64::MAX;
    assert!(HostMetadataEnvelope::derive(&config, 1).is_err());
    let mut encoded = serde_json::to_value(&config).unwrap();
    encoded.as_object_mut().unwrap().remove("bounded_host");
    assert!(serde_json::from_value::<NodeConfig>(encoded.clone()).is_err());
    encoded["bounded_host"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<NodeConfig>(encoded).is_err());
}

#[test]
fn old_logical_id_limits_and_uid_override_are_rejected() {
    let encoded = serde_json::to_value(config()).unwrap();
    for (field, value) in [("max_vm_id", 10), ("uid_base", 10_000)] {
        let mut old = encoded.clone();
        old[field] = serde_json::json!(value);
        assert!(serde_json::from_value::<NodeConfig>(old).is_err());
    }
}

#[test]
fn partial_physical_deployment_is_rejected_without_host_io() {
    let mut config = config();
    assert!(config.validate_deployment().is_ok());
    config.bounded_host.physical_storage = None;
    config.bounded_host.console_policy = None;
    assert!(config.validate_deployment().is_err());
    config.bounded_host.console_policy = Some(sandproof_controller_service::ConsolePolicy::Discard);
    assert!(config.validate_deployment().is_err());
}

#[test]
fn template_path_budget_is_sourced_from_the_accepted_options_cap() {
    let config = config();
    let (_, _, base) = host_slots::maximum_record_bytes(&config).unwrap();
    let mut longer = config.clone();
    longer.kernel = PathBuf::from(format!("{}suffix", config.kernel.display()));
    let (_, _, updated) = host_slots::maximum_record_bytes(&longer).unwrap();
    // RuntimeConfig appears once directly and once inside the runtime manifest.
    assert_eq!(updated - base, 2 * "suffix".len() as u64);
    assert!(base > 2 * sandproof_controller_service::CreateOptions::MAX_ENCODED_BYTES as u64);
}

#[test]
fn future_helper_markers_cannot_outgrow_the_smaller_runtime_cap() {
    let mut config = config();
    config.state_dir = PathBuf::from(format!(
        "/{}",
        "x".repeat(usize::try_from(MAX_HELPER_MARKER_BYTES).unwrap())
    ));
    assert!(serde_json::to_vec(&config).unwrap().len() < node_staging::MAX_PAYLOAD);
    assert!(HostMetadataEnvelope::derive(&config, 1).is_err());
}

#[test]
fn admitted_options_bound_secret_startup_and_request_payloads() {
    use sandproof_controller_service::{
        CreateOptions,
        templates::{ResourceShape, TemplateArtifact, TemplateBinding, TemplateStartup},
    };
    use std::collections::BTreeMap;
    let artifact = TemplateArtifact {
        path: "/images/image".into(),
        sha256: "f".repeat(64),
        size_bytes: 4096,
    };
    let template = TemplateBinding {
        guest: Some(
            serde_json::from_str(include_str!(
                "../../../tests/compat/guest-template-identity.json"
            ))
            .unwrap(),
        ),
        template_id: "template".into(),
        revision: "revision".into(),
        owner: Some(u64::MAX),
        kernel: artifact.clone(),
        rootfs: artifact,
        resources: ResourceShape {
            cpu_count: 1,
            memory_mib: 128,
            disk_bytes: 4096,
            host_memory_bytes: 256 * 1024 * 1024,
        },
        envd_version: "0.9.0".into(),
        startup: Some(TemplateStartup {
            env_vars: BTreeMap::new(),
            user: "root".into(),
            workdir: "/".into(),
            argv: vec!["x".repeat(8192); 4],
        }),
    };
    let mut options = CreateOptions {
        template: Some(template.clone()),
        env_vars: BTreeMap::from([("VALUE".into(), String::new())]),
        ..CreateOptions::default()
    };
    let remaining = CreateOptions::MAX_ENCODED_BYTES - serde_json::to_vec(&options).unwrap().len();
    options
        .env_vars
        .insert("VALUE".into(), "x".repeat(remaining));
    options.validate().unwrap();
    assert_eq!(
        serde_json::to_vec(&options).unwrap().len(),
        CreateOptions::MAX_ENCODED_BYTES
    );
    let secret = super::super::GuestSecret {
        generation: u64::MAX,
        token: "x".repeat(128),
        sha512: "f".repeat(128),
        env_vars: options.env_vars.clone(),
        template: options.template.clone(),
    };
    let startup = crate::startup::StartupRecord {
        sandbox: u64::MAX,
        vm: u64::MAX,
        vmm_pid: u32::MAX,
        start_ticks: u64::MAX,
        template: template.clone(),
        env_vars: options.env_vars.clone(),
        guest_pid: Some(u32::MAX),
    };
    for bytes in [
        serde_json::to_vec(&secret).unwrap(),
        serde_json::to_vec(&startup).unwrap(),
        crate::startup::start_request(template.startup.as_ref().unwrap()).unwrap(),
    ] {
        assert!(bytes.len() < node_staging::MAX_PAYLOAD);
    }
    options.env_vars.get_mut("VALUE").unwrap().push('x');
    assert!(options.validate().is_err());
}
