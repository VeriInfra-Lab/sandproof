use super::*;
use config::{Configuration, DecodeError};
use serde_json::{Value, json};
mod composition;

fn value() -> Value {
    let mut value: Value = serde_json::from_str(include_str!("tests/configuration.json")).unwrap();
    value["paths"]["owner_uid"] = json!(sandproof_guest_runtime::startup_files::current_owner());
    value
}
fn decode(value: &Value) -> Configuration {
    Configuration::decode(&serde_json::to_vec(value).unwrap()).unwrap()
}

#[test]
fn explicit_configuration_prepares_original_pools_without_opening_installation_paths() {
    let mut value = value();
    for key in ["pid_directory", "gate_directory", "worker"] {
        value["paths"][key] = json!("/not-installed-sandproof-input");
    }
    value["snapshots"]["timezone"] = json!("/not-installed-sandproof-zone");
    let prepared = Prepared::new(decode(&value)).unwrap();
    assert_eq!(prepared.config.paths.max_pid, 4_194_304);
    // Neither an absent installation nor an unavailable MMDS is consulted by
    // check/prepare. The consuming start path performs those observations.
}

#[test]
fn complete_strict_schema_rejects_missing_unknown_duplicate_and_malformed_fields() {
    let original = value();
    let text = serde_json::to_string(&original).unwrap();
    assert!(matches!(
        Configuration::decode(
            text.replace("\"version\":2", "\"version\":2,\"version\":2")
                .as_bytes()
        ),
        Err(DecodeError::Json)
    ));
    assert!(matches!(
        Configuration::decode(format!("{text} true").as_bytes()),
        Err(DecodeError::Json)
    ));
    let mut unsupported = original.clone();
    unsupported["version"] = json!(1);
    assert!(matches!(
        Configuration::decode(&serde_json::to_vec(&unsupported).unwrap()),
        Err(DecodeError::Version)
    ));
    // Every object here is a required typed section, including duration units.
    check_objects(&original, "");
    let mut malformed = original;
    malformed["runtime"]["worker_threads"] = json!(-1);
    assert!(matches!(
        Configuration::decode(&serde_json::to_vec(&malformed).unwrap()),
        Err(DecodeError::Json)
    ));
    assert!(matches!(
        Configuration::decode(&vec![b' '; config::CONFIG_BYTES + 1]),
        Err(DecodeError::Size)
    ));
}

#[test]
fn invalid_budgets_and_cross_component_context_fail_before_external_work() {
    let original = value();
    for pointer in [
        "/runtime/worker_threads",
        "/runtime/blocking_threads",
        "/runtime/thread_stack_bytes",
        "/paths/max_pid",
        "/worker/artifact/bytes",
        "/startup/identity_bytes",
        "/metadata/body_bytes",
        "/initialization/body_bytes",
        "/transport/connections",
        "/accounts/in_flight",
        "/accounts/source/groups",
        "/commands/context/limits/bytes",
        "/commands/process/chunk_bytes",
        "/commands/owners/records",
        "/commands/registry/records",
        "/commands/ordinary/in_flight",
        "/commands/control/in_flight",
        "/commands/response/account_headers",
        "/files/context/limits/path_bytes",
        "/files/requests/in_flight",
        "/files/rpc_requests/in_flight",
        "/files/operations/in_flight",
        "/files/transfers/in_flight",
        "/files/upload/headers/bytes",
        "/files/upload/encoding/items",
        "/files/upload/gzip/decode/members",
        "/files/upload/multipart/frame_bytes",
        "/files/upload/batch/files",
        "/files/download/response/response_header_bytes",
        "/files/upload_response/wire_bytes",
        "/files/rpc_response/wire_bytes",
        "/snapshots/mime/entries",
        "/snapshots/timezone_limits/path_bytes",
        "/snapshots/timezone_limits/snapshot/transitions",
    ] {
        let mut bad = original.clone();
        *bad.pointer_mut(pointer).unwrap() = json!(0);
        assert!(
            matches!(Prepared::new(decode(&bad)), Err(Error::Configuration)),
            "{pointer}"
        );
    }
    for (pointer, replacement) in [
        ("/listen", json!("127.0.0.1:0")),
        ("/worker/package_version", json!("0.9.0")),
        ("/worker/protocol_version", json!(1)),
        ("/worker/artifact/sha256", json!("bad")),
        ("/paths/worker", json!("relative")),
        ("/paths/gate_directory", json!("/root/../gate")),
        ("/paths/pid_directory", json!("/nul\0state")),
        ("/paths/owner_uid", json!(u32::MAX)),
        ("/files/context/daemon_workdir", json!("/different")),
        ("/runtime/shutdown_timeout", json!({"secs":0,"nanos":0})),
        ("/commands/registry/retention", json!({"secs":0,"nanos":0})),
        (
            "/commands/registry/retention",
            json!({"secs":29,"nanos":999_999_999}),
        ),
        ("/commands/registry/retention", json!({"secs":30,"nanos":1})),
        ("/commands/registry/retention", json!({"secs":31,"nanos":0})),
        (
            "/commands/admission/cleanup_timeout",
            json!({"secs":0,"nanos":0}),
        ),
    ] {
        let mut bad = original.clone();
        *bad.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            matches!(Prepared::new(decode(&bad)), Err(Error::Configuration)),
            "{pointer}"
        );
    }
}

fn check_objects(root: &Value, path: &str) {
    let object = root.pointer(path).unwrap().as_object().unwrap();
    let mut extra = root.clone();
    extra
        .pointer_mut(path)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("unknown_field".into(), json!(0));
    assert!(
        matches!(
            Configuration::decode(&serde_json::to_vec(&extra).unwrap()),
            Err(DecodeError::Json)
        ),
        "unknown at {path}"
    );
    for (key, field) in object {
        let mut missing = root.clone();
        missing
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(
            matches!(
                Configuration::decode(&serde_json::to_vec(&missing).unwrap()),
                Err(DecodeError::Json)
            ),
            "missing {path}/{key}"
        );
        if field.is_object() {
            check_objects(root, &format!("{path}/{key}"));
        }
    }
}
