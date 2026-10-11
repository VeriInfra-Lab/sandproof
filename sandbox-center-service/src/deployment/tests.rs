//! Read-only center assembly. No Redis/KVM/real gateway acceptance is claimed.
use super::*;
#[path = "../../../sandbox-internal/tests/support/tls.rs"]
mod tls;
use sandproof_controller_service::templates::{ResourceShape, TemplateArtifact, TemplateBinding};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
};

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
        let tls = tls::certificates();
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
        let template =
            PublicTemplate::from_binding(binding.clone(), "base-r1".try_into().unwrap()).unwrap();
        let resources = json!({"instances":4,"cpu":4,"memory_bytes":516*1024*1024u64,"disk_bytes":4*1024*1024*1024u64});
        let config = json!({
            "deployment_id":"assembly-fixture","bind":"127.0.0.1:3000","domain":"sandbox.test",
            "credentials":[{"tenant":1,"secret":"center-config-user-fixture-only-key-0001"}],
            "data_token_key":path("token.key"),"max_timeout_secs":300,"max_lifetime_secs":3600,
            "startup_timeout_secs":300,"reply_timeout_ms":1000,"poll_interval_ms":10,"expiry_scan_ms":1000,
            "public_ipv4_enabled":false,"request_admission":{"max_pending_requests":8,"control_reserve":2},
            "data_transfer":{"file_upload_bytes":1_048_576,"file_download_bytes":1_048_576,"command_input_bytes":65_536,
                "rpc_request_bytes":65_536,"rpc_response_bytes":65_536,"max_transfers":8,"max_transfers_per_vm":4,
                "control_reserve":1,"request_idle_seconds":5,"file_idle_seconds":5,"write_idle_seconds":5},
            "redis":{"address":"127.0.0.1:6379","username":"center-fixture","password":"redis-config-fixture-only-key",
                "max_block_bytes":2_097_152,"memory_bytes":67_108_864,"operation_timeout_ms":1000,"writer_root":path("writer")},
            "tls":{"certificate_chain_der":[path("center.der")],"private_key_der":path("center-key.der"),
                "trusted_roots_der":[path("ca.der")],"max_connections":12,"handshake_timeout_ms":500,"request_timeout_ms":1000},
            "bounds":{"max_nodes":2,"max_templates_per_node":4,"max_sandboxes":4,"operations_per_sandbox":4,"max_options_bytes":4096,"platform":resources},
            "tenants":[{"tenant":"1","limit":resources}],
            "nodes":[{"registration":{"identity":{"deployment_id":"assembly-fixture","node_id":"host-a","node_generation":1},
                "limit":resources,"mode":"accepting","healthy_until_ms":100_000,"free":resources,
                "templates":[{"revision":"base-r1","digest":template.digest()}],"protocol_version":1},
                "address":"127.0.0.1:9000","server_name":"host.test","peer_certificates_der":[path("host.der")]}],
            "template_registry":{"root":path("registry"),"max_templates_global":8,"max_templates_per_tenant":4,
                "max_artifact_bytes_global":16*1024*1024*1024u64,"max_artifact_bytes_per_tenant":8*1024*1024*1024u64,"validation_timeout_secs":30},
            "templates":{"templates":[binding],"aliases":{"default":"base"},"compute_limits":{
                "max_vcpus_global":4,"max_vcpus_per_tenant":4,
                "max_memory_bytes_global":516*1024*1024u64,"max_memory_bytes_per_tenant":516*1024*1024u64}},
            "template_registrations":[{"revision":"base-r1","template_id":"base"}]
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
}

#[test]
fn center_inputs_load_protected_original_bindings_without_host_artifact_io() {
    let fixture = Fixture::new();
    let loaded = fixture.load(&fixture.config).unwrap();
    assert_eq!(loaded.hosts.len(), 1);
    assert_eq!(loaded.hosts[0].identity.node_id.as_str(), "host-a");
    assert_eq!(loaded.templates[0].template_id(), "base");
    assert_eq!(
        loaded.catalog.resolve(1, "default").unwrap().template_id,
        "base"
    );
    assert!(loaded.registration.is_valid());
    assert_eq!(loaded.registration.bounds().operations_per_sandbox, 4);
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
    digest["nodes"][0]["registration"]["templates"][0]["digest"] = json!(vec![0; 32]);
    assert!(matches!(
        fixture.load(&digest),
        Err(AssemblyError::IncompleteTemplateRegistration)
    ));
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
