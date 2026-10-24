//! Read-only center assembly. No Redis/KVM/real gateway acceptance is claimed.
use super::*;
use crate::test_tls as tls;
use sandproof_controller_service::templates::{ResourceShape, TemplateArtifact, TemplateBinding};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};

#[test]
fn assembly_errors_expose_only_their_fixed_diagnostic_category() {
    let categories = [
        (
            AssemblyError::ProtectedInput,
            "protected center input rejected",
        ),
        (
            AssemblyError::InvalidConfiguration,
            "invalid center deployment configuration",
        ),
        (
            AssemblyError::InvalidTls,
            "invalid registered center TLS material",
        ),
        (
            AssemblyError::IncompleteTemplateRegistration,
            "incomplete trusted template registration",
        ),
        (
            AssemblyError::InvalidPublicBoundary,
            "invalid public center boundary",
        ),
    ];
    for (error, expected) in categories {
        assert_eq!(error.as_str(), expected);
        assert_eq!(error.to_string(), expected);
    }
}

struct Fixture {
    root: tempfile::TempDir,
    owner: u32,
    config: Value,
}
fn write_private(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
fn private_directory(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let owner = fs::metadata(root.path()).unwrap().uid();
        let path = |name: &str| root.path().join(name);
        let tls = tls::certificates_with_dual_roles();
        write_private(&path("center.der"), tls.center.certificate.der());
        write_private(&path("center-key.der"), &tls.center.key.serialize_der());
        write_private(&path("ca.der"), tls.ca.der());
        write_private(&path("host.der"), tls.host.certificate.der());
        write_private(
            &path("token.key"),
            b"center-token-config-fixture-only-secret-00001",
        );
        private_directory(&path("writer"));
        private_directory(&path("registry"));
        private_directory(&path("registry/inbox"));
        private_directory(&path("registry/artifacts"));
        let binding = TemplateBinding {
            guest: None,
            template_id: "base".into(),
            revision: "r1".into(),
            owner: None,
            kernel: TemplateArtifact {
                path: "/trusted-host/kernel".into(),
                sha256: "1".repeat(64),
                size_bytes: 4096,
            },
            rootfs: TemplateArtifact {
                path: "/trusted-host/rootfs".into(),
                sha256: "2".repeat(64),
                size_bytes: 1024 * 1024 * 1024,
            },
            resources: ResourceShape {
                cpu_count: 1,
                memory_mib: 128,
                host_memory_bytes: 129 * 1024 * 1024,
                disk_bytes: 1024 * 1024 * 1024,
            },
            envd_version: "0.9.0".into(),
            startup: None,
        };
        let resources = json!({"instances":4,"cpu":4,"memory_bytes":516*1024*1024u64,"disk_bytes":4*1024*1024*1024u64});
        let config = json!({
            "deployment_id":"assembly-fixture","center_id":"center","bind":"127.0.0.1:3000","domain":"sandbox.test",
            "credentials":[{"tenant":1,"secret":"center-config-user-fixture-only-key-0001"}],
            "data_token_key":path("token.key"),"max_timeout_secs":300,"max_lifetime_secs":3600,
            "startup_timeout_secs":300,"reply_timeout_ms":1000,"poll_interval_ms":10,"expiry_scan_ms":1000,
            "node_observation":{"health_window_ms":5000,"max_clock_skew_ms":100},
            "public_ipv4_enabled":false,"request_admission":{"max_pending_requests":8,"control_reserve":2},
            "data_transfer":{"file_upload_bytes":1_048_576,"file_download_bytes":1_048_576,"command_input_bytes":65_536,
                "rpc_request_bytes":65_536,"rpc_response_bytes":65_536,"max_transfers":8,"max_transfers_per_vm":4,
                "control_reserve":1,"request_idle_seconds":5,"file_idle_seconds":5,"write_idle_seconds":5},
            "redis":{"address":"127.0.0.1:6379","username":"center-fixture","password":"redis-config-fixture-only-key",
                "max_block_bytes":8_388_608,"memory_bytes":67_108_864,"operation_timeout_ms":1000,"writer_root":path("writer")},
            "tls":{"certificate_chain_der":[path("center.der")],"private_key_der":path("center-key.der"),
                "trusted_roots_der":[path("ca.der")],"max_connections":12,"handshake_timeout_ms":500,"request_timeout_ms":1000},
            "bounds":{"max_nodes":2,"max_templates_per_node":4,"max_sandboxes":4,"operations_per_sandbox":4,"max_options_bytes":4096,"result_retention_ms":300000,"platform":resources},
            "tenants":[{"tenant":"1","limit":resources}],
            "nodes":[{"registration":{"identity":{"deployment_id":"assembly-fixture","node_id":"host-a","node_generation":1},
                "role":"runtime","limit":resources,"mode":"accepting","healthy_until_ms":0,"free":{"instances":0,"cpu":0,"memory_bytes":0,"disk_bytes":0},
                "templates":[],"protocol_version":2,"retention_slots":4},
                "address":"127.0.0.1:9000","server_name":"host.test","peer_certificates_der":[path("host.der")]}],
            "template_registry":{"root":path("registry"),"max_templates_global":8,"max_templates_per_tenant":4,
                "max_artifact_bytes_global":16*1024*1024*1024u64,"max_artifact_bytes_per_tenant":8*1024*1024*1024u64,"validation_timeout_secs":30},
            "metadata_budget":{"max_certificate_bytes":65_536},
            "private_builds":null,
            "private_build_boundary":null,
            "templates":{"templates":[binding],"aliases":{"default":"base"},"compute_limits":{
                "max_vcpus_global":4,"max_vcpus_per_tenant":4,
                "max_memory_bytes_global":516*1024*1024u64,"max_memory_bytes_per_tenant":516*1024*1024u64}},
            "template_registrations":[{"revision":"base-r1","template_id":"base"}]
            ,"artifact_service":{"bind":"127.0.0.1:9100","server_name":"center.test","tls":{"certificate_chain_der":[path("center.der")],
                "private_key_der":path("center-key.der"),"trusted_roots_der":[path("ca.der")]},
                "max_connections":4,"handshake_timeout_ms":500,"request_timeout_ms":1000,"stream_timeout_ms":60000}
        });
        Self {
            root,
            owner,
            config,
        }
    }
    fn load(&self, config: &Value) -> Result<CenterAssemblyInputs, AssemblyError> {
        let path = self.root.path().join("center.json");
        write_private(&path, &serde_json::to_vec(config).unwrap());
        CenterAssemblyInputs::load(&path, self.owner)
    }
    fn with_private_builds(&self) -> Value {
        let mut config = self.config.clone();
        let mut builder = config["templates"]["templates"][0].clone();
        builder["template_id"] = json!("builder-base");
        config["templates"]["templates"]
            .as_array_mut()
            .unwrap()
            .push(builder);
        config["template_registrations"]
            .as_array_mut()
            .unwrap()
            .push(json!({"revision":"builder-r1","template_id":"builder-base"}));
        let tls = tls::certificates_with_dual_roles();
        write_private(
            &self.root.path().join("builder.der"),
            tls.other.certificate.der(),
        );
        // A different valid leaf pin suffices for assembly; actual peer trust
        // against its issuer is tested by the transport, not invented here.
        let mut node = config["nodes"][0].clone();
        node["registration"]["identity"]["node_id"] = json!("host-builder");
        node["registration"]["role"] = json!("builder");
        node["address"] = json!("127.0.0.1:9001");
        node["server_name"] = json!("other.test");
        node["peer_certificates_der"] = json!([self.root.path().join("builder.der")]);
        config["nodes"].as_array_mut().unwrap().push(node);
        write_private(
            &self.root.path().join("build.key"),
            b"private-build-config-test-credential-0001",
        );
        config["private_builds"] = json!({"windows":{"max_open_windows":2,"max_duration_ms":60000},
            "window_duration_ms":5000,"max_anchors":1,"builder_revision":"builder-r1",
            "policy":{"max_timeout_secs":600,"max_lifetime_secs":900,
                      "startup_timeout_secs":30,"public_ipv4_enabled":true}});
        config["private_build_boundary"] = json!({"bind":"127.0.0.1:9200","backend_address":"127.0.0.1:9201",
            "credential_file":self.root.path().join("build.key"),"request_timeout_ms":60000,
            "max_request_bytes":65536,"max_response_bytes":409_600});
        config
    }
}

#[test]
fn documented_center_example_loads_with_operator_material_through_the_original_validator() {
    let fixture = Fixture::new();
    let mut example: Value =
        serde_json::from_str(include_str!("../../../deploy/center.example.json")).unwrap();
    // The example has intentionally empty credentials and no private keys.
    // Supply ONLY operator material from the existing protected/TLS fixture;
    // retain its complete shape, identity, bounds, timings and registrations.
    example["credentials"][0]["secret"] = fixture.config["credentials"][0]["secret"].clone();
    example["redis"]["password"] = fixture.config["redis"]["password"].clone();
    example["redis"]["writer_root"] = fixture.config["redis"]["writer_root"].clone();
    example["data_token_key"] = fixture.config["data_token_key"].clone();
    example["template_registry"]["root"] = fixture.config["template_registry"]["root"].clone();
    for field in [
        "certificate_chain_der",
        "private_key_der",
        "trusted_roots_der",
    ] {
        example["tls"][field] = fixture.config["tls"][field].clone();
        example["artifact_service"]["tls"][field] =
            fixture.config["artifact_service"]["tls"][field].clone();
    }
    example["nodes"][0]["peer_certificates_der"] =
        fixture.config["nodes"][0]["peer_certificates_der"].clone();
    let result = fixture.load(&example);
    assert!(result.is_ok(), "original validator: {:?}", result.err());
    for field in [
        "center_id",
        "nodes",
        "artifact_service",
        "node_observation",
        "private_builds",
        "private_build_boundary",
    ] {
        let mut missing = example.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(fixture.load(&missing).is_err(), "missing {field}");
    }
    let mut unknown = example.clone();
    unknown["mode"] = json!("legacy");
    assert!(fixture.load(&unknown).is_err());
    let mut missing_slot = example.clone();
    missing_slot["nodes"][0]["registration"]
        .as_object_mut()
        .unwrap()
        .remove("retention_slots");
    assert!(fixture.load(&missing_slot).is_err());
    let mut fake_healthy = example.clone();
    fake_healthy["nodes"][0]["registration"]["healthy_until_ms"] = json!(60_000);
    assert!(fixture.load(&fake_healthy).is_err());
    example["credentials"][0]["secret"] = json!("");
    assert!(fixture.load(&example).is_err());
}

#[test]
fn private_build_capability_is_explicit_role_bound_and_uses_the_registered_digest() {
    let fixture = Fixture::new();
    let config = fixture.with_private_builds();
    let loaded = fixture.load(&config).unwrap();
    let private = loaded.provisioning.metadata.private_build_config().unwrap();
    let binding: TemplateBinding =
        serde_json::from_value(config["templates"]["templates"][1].clone()).unwrap();
    assert_eq!(private.builder_digest, binding.canonical_digest().unwrap());
    assert!(loaded.private_build_boundary.is_some());
    let mut missing = config.clone();
    missing["private_builds"]
        .as_object_mut()
        .unwrap()
        .insert("builder_digest".into(), json!(vec![0; 32]));
    assert!(fixture.load(&missing).is_err());
    for path in ["private_builds", "private_build_boundary"] {
        let mut changed = config.clone();
        changed[path] = Value::Null;
        assert!(fixture.load(&changed).is_err());
        changed.as_object_mut().unwrap().remove(path);
        assert!(fixture.load(&changed).is_err());
    }
    for revision in ["missing", "base-r1"] {
        let mut changed = config.clone();
        changed["private_builds"]["builder_revision"] = json!(revision);
        assert!(fixture.load(&changed).is_err());
    }
    let mut no_builder = config.clone();
    no_builder["nodes"][1]["registration"]["role"] = json!("runtime");
    assert!(fixture.load(&no_builder).is_err());
}

#[test]
fn private_build_transport_preserves_large_input_bounds_and_refuses_unsafe_boundaries() {
    let fixture = Fixture::new();
    let config = fixture.with_private_builds();
    for bytes in [65536_u64, 16 * 1024 * 1024 * 1024 + 64 * 1024] {
        let mut changed = config.clone();
        changed["private_build_boundary"]["max_request_bytes"] = json!(bytes);
        assert!(fixture.load(&changed).is_ok());
    }
    for (field, value) in [
        ("bind", json!("0.0.0.0:9200")),
        ("bind", json!("127.0.0.1:3000")),
        ("backend_address", json!("127.0.0.1:9200")),
        ("max_response_bytes", json!(409_599)),
        (
            "max_request_bytes",
            json!(16_u64 * 1024 * 1024 * 1024 + 64 * 1024 + 1),
        ),
    ] {
        let mut changed = config.clone();
        changed["private_build_boundary"][field] = value;
        assert!(fixture.load(&changed).is_err());
    }
}

#[test]
fn artifact_server_role_and_bounded_status_freshness_are_required() {
    let fixture = Fixture::new();
    let mut changed = fixture.config.clone();
    changed["artifact_service"]["server_name"] = json!("not-center.test");
    assert!(matches!(
        fixture.load(&changed),
        Err(AssemblyError::InvalidTls)
    ));
    for (window, skew) in [(0, 0), (1000, 0), (5000, 5001), (86_400_001, 0)] {
        let mut changed = fixture.config.clone();
        changed["node_observation"] = json!({"health_window_ms":window,"max_clock_skew_ms":skew});
        assert!(matches!(
            fixture.load(&changed),
            Err(AssemblyError::InvalidConfiguration)
        ));
    }
    let one_role = tls::certificates();
    write_private(&fixture.root.path().join("ca.der"), one_role.ca.der());
    write_private(
        &fixture.root.path().join("center.der"),
        one_role.center.certificate.der(),
    );
    write_private(
        &fixture.root.path().join("center-key.der"),
        &one_role.center.key.serialize_der(),
    );
    // with_single_cert alone accepts this client-only identity. Production
    // protected load must reject it BEFORE binding the artifact listener.
    assert!(matches!(
        fixture.load(&fixture.config),
        Err(AssemblyError::InvalidTls)
    ));
}

#[test]
fn center_inputs_load_protected_original_bindings_without_host_artifact_io() {
    let fixture = Fixture::new();
    let loaded = fixture.load(&fixture.config).unwrap();
    assert_eq!(loaded.hosts.len(), 1);
    assert_eq!(loaded.hosts[0].identity.node_id.as_str(), "host-a");
    assert!(loaded.provisioning.is_valid());
    assert_eq!(
        loaded.provisioning.center.bounds().operations_per_sandbox,
        4
    );
    assert!(loaded.provisioning.metadata.registry().records().is_empty());
    assert!(
        loaded
            .admission
            .try_acquire(sandproof_api::admission::RequestClass::Control)
            .is_some()
    );
    // Artifact paths deliberately do not exist on this center test machine.
    assert!(!Path::new("/trusted-host/rootfs").exists());
}

#[test]
fn unknown_host_fields_missing_budgets_and_invalid_defaults_fail_before_serving() {
    let fixture = Fixture::new();
    for (key, value) in [
        ("database", json!("/host.db")),
        ("jailer", json!("/bin/jailer")),
        ("bind", json!("0.0.0.0:3000")),
        ("max_timeout_secs", json!(299)),
        ("expiry_scan_ms", json!(0)),
        ("credentials", json!([])),
    ] {
        let mut config = fixture.config.clone();
        config[key] = value;
        assert!(matches!(
            fixture.load(&config),
            Err(AssemblyError::InvalidConfiguration)
        ));
    }
    let mut missing = fixture.config.clone();
    missing.as_object_mut().unwrap().remove("data_transfer");
    assert!(matches!(
        fixture.load(&missing),
        Err(AssemblyError::InvalidConfiguration)
    ));
    let mut missing_retention = fixture.config.clone();
    missing_retention["bounds"]
        .as_object_mut()
        .unwrap()
        .remove("result_retention_ms");
    assert!(matches!(
        fixture.load(&missing_retention),
        Err(AssemblyError::InvalidConfiguration)
    ));
    let mut zero_retention = fixture.config.clone();
    zero_retention["bounds"]["result_retention_ms"] = json!(0);
    assert!(matches!(
        fixture.load(&zero_retention),
        Err(AssemblyError::InvalidConfiguration)
    ));
    let mut foreign_user = fixture.config.clone();
    foreign_user["credentials"][0]["tenant"] = json!(2);
    assert!(matches!(
        fixture.load(&foreign_user),
        Err(AssemblyError::InvalidConfiguration)
    ));
    let mut no_control_room = fixture.config.clone();
    no_control_room["tls"]["max_connections"] = json!(8);
    assert!(matches!(
        fixture.load(&no_control_room),
        Err(AssemblyError::InvalidConfiguration)
    ));
    // Production must retain create/current intent plus confirmation/destroy
    // headroom for repeated renewal. Smaller pure-core diagnostic bounds are
    // still supported by that component, not silently deployed here.
    for operations in [2, 3] {
        let mut insufficient_history = fixture.config.clone();
        insufficient_history["bounds"]["operations_per_sandbox"] = json!(operations);
        assert!(matches!(
            fixture.load(&insufficient_history),
            Err(AssemblyError::InvalidConfiguration)
        ));
    }
}

#[test]
fn nodes_require_exact_deployment_original_template_digest_and_tls_role_pins() {
    let fixture = Fixture::new();
    let mut foreign = fixture.config.clone();
    foreign["nodes"][0]["registration"]["identity"]["deployment_id"] = json!("other");
    assert!(matches!(
        fixture.load(&foreign),
        Err(AssemblyError::InvalidConfiguration)
    ));
    let mut digest = fixture.config.clone();
    digest["template_registrations"][0]["template_id"] = json!("missing-template");
    assert!(matches!(
        fixture.load(&digest),
        Err(AssemblyError::IncompleteTemplateRegistration)
    ));
    for field in ["healthy_until_ms", "free", "templates"] {
        let mut invented = fixture.config.clone();
        invented["nodes"][0]["registration"][field] = match field {
            "healthy_until_ms" => json!(100_000),
            "free" => json!({"instances":1,"cpu":1,"memory_bytes":1,"disk_bytes":1}),
            _ => json!([{"revision":"base-r1","digest":vec![0;32]}]),
        };
        assert!(matches!(
            fixture.load(&invented),
            Err(AssemblyError::InvalidConfiguration)
        ));
    }
    let mut center_pin = fixture.config.clone();
    center_pin["nodes"][0]["peer_certificates_der"] =
        json!([fixture.root.path().join("center.der")]);
    assert!(matches!(
        fixture.load(&center_pin),
        Err(AssemblyError::InvalidTls)
    ));
    let mut missing_registry = fixture.config.clone();
    missing_registry["template_registrations"] = json!([]);
    assert!(matches!(
        fixture.load(&missing_registry),
        Err(AssemblyError::IncompleteTemplateRegistration)
    ));
}

#[test]
fn tls_key_and_global_token_files_use_same_protected_owner_and_size_checks() {
    let fixture = Fixture::new();
    let token = fixture.root.path().join("token.key");
    write_private(&token, b"short-key");
    assert!(matches!(
        fixture.load(&fixture.config),
        Err(AssemblyError::InvalidConfiguration)
    ));
    write_private(&token, b"center-token-config-fixture-only-secret-00001");
    let key = fixture.root.path().join("center-key.der");
    fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        fixture.load(&fixture.config),
        Err(AssemblyError::ProtectedInput)
    ));
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = fixture.root.path().join("key-alias.der");
    symlink(&key, &alias).unwrap();
    let mut config = fixture.config.clone();
    config["tls"]["private_key_der"] = json!(alias);
    assert!(matches!(
        fixture.load(&config),
        Err(AssemblyError::ProtectedInput)
    ));
    write_private(&key, b"not-a-key");
    assert!(matches!(
        fixture.load(&fixture.config),
        Err(AssemblyError::InvalidTls)
    ));
}

#[test]
fn provisioning_requires_whole_business_future_budget_without_separate_storage_mode() {
    let fixture = Fixture::new();
    let loaded = fixture.load(&fixture.config).unwrap();
    let required = loaded.provisioning.required_block_bytes();
    assert!(required > loaded.provisioning.center.bounds().required_block_bytes());
    for budget in [required - 1, 2_097_152] {
        let mut insufficient = fixture.config.clone();
        insufficient["redis"]["max_block_bytes"] = json!(budget);
        assert!(matches!(
            fixture.load(&insufficient),
            Err(AssemblyError::InvalidConfiguration)
        ));
    }
    let mut missing = fixture.config.clone();
    missing.as_object_mut().unwrap().remove("metadata_budget");
    assert!(matches!(
        fixture.load(&missing),
        Err(AssemblyError::InvalidConfiguration)
    ));
    for certificate in [0, 65_535, 16 * 1024 * 1024 + 1] {
        let mut invalid = fixture.config.clone();
        invalid["metadata_budget"]["max_certificate_bytes"] = json!(certificate);
        assert!(matches!(
            fixture.load(&invalid),
            Err(AssemblyError::InvalidConfiguration)
        ));
    }
}
