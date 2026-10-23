//! Pure deployment refusals; no privileged activation or real TLS listener.
use super::*;
use crate::linux::test_support::settings;
use serde_json::{Value, json};

fn host_settings() -> Value {
    let mut native = settings();
    // Original immutable catalog, not a test/production alternate parser.
    native["host_resources"] = json!({
        "cgroup_parent": "/sys/fs/cgroup/test-only-host",
        "memory_max_bytes": 134_221_824
    });
    let descriptor = json!({
        "template_id": "base", "revision": "r1", "owner": null,
        "kernel": {"path": "/images/kernel", "sha256": "a".repeat(64), "size_bytes": 64},
        "rootfs": {"path": "/images/rootfs", "sha256": "b".repeat(64), "size_bytes": 1024},
        "resources": {"cpu_count": 1, "memory_mib": 128, "host_memory_bytes": 134_221_824, "disk_bytes": 1024},
        "envd_version": "0.9.0",
        "startup": {"workdir": "/home/user", "argv": [], "env_vars": {}, "user": "user"}
    });
    native["templates"] = json!({
        "templates": [descriptor], "aliases": {"base": "base"},
        "compute_limits": {
            "max_vcpus_global": 2, "max_vcpus_per_tenant": 2,
            "max_memory_bytes_global": 536_870_912, "max_memory_bytes_per_tenant": 536_870_912
        }
    });
    let cache = json!({"namespace":native["namespace"],"root":"/fixture/template-cache","max_entries":2,"max_downloads":1,
        "max_entry_bytes":4096,"artifact_bytes":8192,"temporary_bytes":4096,
        "metadata_bytes":282_624,"inode_budget":19});
    native["bounded_host"]["physical_storage"]["host_template_caches"] = json!([cache.clone()]);
    native["bounded_host"]["physical_storage"]["filesystem_overhead_inodes"] = json!(128);
    json!({
        "native": native,
        "inbox": {
            "center": {"deployment": "deployment-a", "center": "center-a"},
            "node": {"deployment_id": "deployment-a", "node_id": "host-a", "node_generation": 1},
            "host_epoch": null,
            "max_sessions": 2, "max_operations_per_session": 8, "max_options_bytes": 1024,
            "max_profile_bytes": 4096, "max_manifest_bytes": 16384
        },
        "protected_bootstrap_path": "/fixture/host-bootstrap.json",
        "source_observation": {"max_work_bytes":4096,"max_configuration_bytes":1024,
            "max_executable_bytes":67108864,"max_kernel_bytes":268435456,"max_disk_bytes":4096},
        "internal": {
            "bind": "127.0.0.1:3044", "max_connections": 10,
            "handshake_timeout_ms": 1000, "request_timeout_ms": 5000,
            "tls": {"identity_chain": ["/private/host.der"], "private_key": "/private/host.key",
                "roots": ["/private/root.der"], "center_leaves": ["/private/center.der"]}
        },
        "template_cache": cache,
        "artifact_source": {"address":"127.0.0.1:3045","server_name":"artifact.local",
            "max_connections":1,"handshake_timeout_ms":1000,"request_timeout_ms":5000,
            "download_timeout_ms":60000,"idle_timeout_ms":10000,
            "tls":{"identity_chain":["/private/host.der"],"private_key":"/private/host.key",
                "roots":["/private/root.der"],"artifact_leaves":["/private/artifact.der"]}}
    })
}

fn check(value: Value) -> bool {
    serde_json::from_value::<HostConfig>(value).is_ok_and(|config| config.validate_static().is_ok())
}

#[test]
fn sole_host_schema_has_no_public_authorization_or_implicit_modes() {
    // Unknown and duplicate keys are rejected by typed serde, including inside
    // nested settings; no Value/flatten reparse can discard a duplicate.
    for field in ["credentials", "bind", "domain", "request_admission", "mode"] {
        let mut value = host_settings();
        value[field] = json!("not-accepted");
        assert!(!check(value));
    }
    let encoded = serde_json::to_string(&host_settings()).unwrap();
    let duplicate = encoded.replacen(
        "\"max_connections\":10",
        "\"max_connections\":10,\"max_connections\":11",
        1,
    );
    assert!(serde_json::from_str::<HostConfig>(&duplicate).is_err());
}

#[test]
fn host_budget_retains_management_and_destroy_capacity() {
    assert!(check(host_settings()));
    for (path, value) in [
        ("/internal/max_connections", json!(8)),
        ("/internal/max_connections", json!(8193)),
        ("/inbox/max_sessions", json!(0)),
        ("/inbox/max_operations_per_session", json!(7)),
        ("/inbox/max_operations_per_session", json!(129)),
        ("/inbox/max_profile_bytes", json!(0)),
        ("/inbox/max_manifest_bytes", json!(262145)),
        ("/internal/request_timeout_ms", json!(0)),
    ] {
        let mut settings = host_settings();
        *settings.pointer_mut(path).unwrap() = value;
        assert!(!check(settings));
    }
}

#[test]
fn current_bootstrap_settings_reject_foreign_missing_and_old_shapes() {
    for (pointer, replacement) in [
        ("/inbox/center/deployment", json!("another-deployment")),
        ("/protected_bootstrap_path", json!("relative/bootstrap.json")),
        ("/source_observation/max_work_bytes", json!(1023)),
        ("/source_observation/max_configuration_bytes", json!(0)),
    ] {
        let mut value = host_settings();
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert!(!check(value), "accepted invalid current input {pointer}");
    }
    for field in ["max_sandboxes", "operations_per_sandbox"] {
        let mut value = host_settings();
        value["inbox"][field] = json!(4);
        assert!(!check(value), "accepted retired inbox field {field}");
    }
}

#[test]
fn cache_is_one_reserved_source_not_a_static_availability_fallback() {
    let mut value = host_settings();
    value["template_registrations"] = json!([{ "revision":"base-r1","template_id":"base" }]);
    assert!(!check(value));
    value = host_settings();
    value["template_cache"]["artifact_bytes"] = json!(16384);
    assert!(!check(value));
    value = host_settings();
    value["artifact_source"]["idle_timeout_ms"] = json!(60001);
    assert!(!check(value));
}

#[test]
fn full_retained_inbox_requires_its_original_single_value_reservation() {
    let mut value = host_settings();
    value["inbox"]["max_sessions"] = json!(5);
    value["inbox"]["max_options_bytes"] = json!(4096);
    value["inbox"]["max_manifest_bytes"] = json!(32768);
    let config: HostConfig = serde_json::from_value(value.clone()).unwrap();
    let required = config.inbox.required_bytes();
    // The former single-host fixture's row cap is not a distributed inbox cap.
    assert!(required > 192 * 1024 && required <= 1024 * 1024);
    let original_total = value["native"]["maintenance"]["capacity"]["max_sql_bytes"].clone();
    for cap in [192 * 1024, required - 1, required, 1024 * 1024] {
        value["native"]["maintenance"]["capacity"]["max_sql_value_bytes"] = json!(cap);
        assert_eq!(check(value.clone()), cap >= required);
        assert_eq!(
            value["native"]["maintenance"]["capacity"]["max_sql_bytes"],
            original_total
        );
    }
}

#[test]
fn documented_host_example_uses_the_original_validator_and_rejects_missing_or_old_inputs() {
    let value: Value =
        serde_json::from_str(include_str!("../../../../deploy/host.example.json")).unwrap();
    assert!(check(value.clone()));
    let mut insufficient_inodes = value.clone();
    insufficient_inodes["native"]["bounded_host"]["physical_storage"]["filesystem_overhead_inodes"] =
        json!(4096);
    assert!(!check(insufficient_inodes));
    for field in [
        "native",
        "inbox",
        "protected_bootstrap_path",
        "source_observation",
        "internal",
        "template_cache",
        "artifact_source",
    ] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(!check(missing), "missing required {field}");
    }
    for field in [
        "credentials",
        "template_registry",
        "internal_build_vms",
        "build_retry_windows",
    ] {
        let mut retired = value.clone();
        retired["native"][field] = json!(true);
        assert!(!check(retired), "retired native input {field}");
    }
    let mut unreserved = value.clone();
    unreserved["template_cache"]["artifact_bytes"] = json!(1);
    assert!(!check(unreserved));
    let config: HostConfig = serde_json::from_value(value).unwrap();
    config
        .native
        .templates
        .as_ref()
        .unwrap()
        .validate()
        .unwrap();
    let (contract, _) = config.validate_static().unwrap();
    let peak = contract
        .bounded
        .physical_storage
        .unwrap()
        .peak_bytes()
        .unwrap();
    assert!(peak.checked_mul(2).unwrap() < 200 * 1024 * 1024 * 1024);
}
