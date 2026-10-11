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
            "node": {"deployment_id": "deployment-a", "node_id": "host-a", "node_generation": 1},
            "max_sandboxes": 2, "operations_per_sandbox": 4,
            "max_options_bytes": 1024
        },
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
        ("/inbox/operations_per_sandbox", json!(3)),
        ("/internal/request_timeout_ms", json!(0)),
    ] {
        let mut settings = host_settings();
        *settings.pointer_mut(path).unwrap() = value;
        assert!(!check(settings));
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
