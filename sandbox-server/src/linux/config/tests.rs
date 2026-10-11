use super::super::preflight::ValidatedDeployment;
use super::super::test_support::{
    check, full_capacity_settings, maintained_settings, partial_settings, resolve, settings,
};
use super::*;
use serde_json::{Value, json};

#[test]
fn request_admission_has_finite_defaults_and_rejects_invalid_overrides() {
    let config: ConfigFile = serde_json::from_value(settings()).unwrap();
    let contract = DeploymentContract::validate(&config).unwrap();
    assert_eq!(contract.requests.max_pending_requests, 64);
    assert_eq!(contract.requests.control_reserve, 8);
    for policy in [
        json!({"max_pending_requests": 2, "control_reserve": 0}),
        json!({"max_pending_requests": 2, "control_reserve": 2}),
        json!({"max_pending_requests": 8193, "control_reserve": 1}),
        json!({"max_pending_requests": 2}),
        json!({"max_pending_requests": 2, "control_reserve": 1, "typo": true}),
        Value::Null,
    ] {
        let mut changed = settings();
        changed["request_admission"] = policy;
        assert!(check(changed).is_err());
    }
    let mut valid = settings();
    valid["request_admission"] = json!({"max_pending_requests": 2, "control_reserve": 1});
    assert!(check(valid).is_ok());
}

#[test]
fn formal_contract_rejects_every_partial_mode_without_filesystem_effects() {
    for path in [
        "/bounded_host",
        "/maintenance",
        "/data_transfer",
        "/bounded_host/physical_storage",
        "/bounded_host/console_policy",
        "/maintenance/capacity",
    ] {
        for null in [false, true] {
            let mut value = settings();
            if null {
                *value.pointer_mut(path).unwrap() = Value::Null;
            } else {
                let (parent, field) = path.rsplit_once('/').unwrap();
                value
                    .pointer_mut(parent)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
            }
            let config = serde_json::from_value(value).unwrap();
            assert!(
                ValidatedDeployment::validate(config).is_err(),
                "accepted {path}, null={null}"
            );
        }
    }
    let mut misspelled = settings();
    misspelled["data_tranfer"] = misspelled["data_transfer"].take();
    assert!(check(misspelled).is_err());
    assert!(check(partial_settings()).is_err());
}

#[test]
fn valid_omissions_keep_lifetime_defaults_and_prebuilt_only_capability() {
    let config: ConfigFile = serde_json::from_value(settings()).unwrap();
    assert!(!config.internal_build_vms);
    assert!(config.build_retry_windows.is_none());
    assert!(config.template_registry.is_none());
    assert!(DeploymentContract::validate(&config).is_ok());
    assert_eq!(config.default_timeout_secs, 300);
    assert_eq!(config.startup_timeout_secs, 300);
    assert_eq!(
        managed_config(&config, 1024)
            .unwrap()
            .limits
            .max_lifetime_secs,
        config.max_timeout_secs
    );
    // This is a capability floor, not an implicit change to an SDK request.
    assert!(!config.public_ipv4_enabled);
    for field in [
        "default_timeout_secs",
        "startup_timeout_secs",
        "max_lifetime_secs",
    ] {
        let mut zero = settings();
        zero[field] = json!(0);
        assert!(check(zero).is_err(), "accepted zero {field}");
    }
}

#[test]
fn full_capacity_requires_explicit_console_and_linked_controller_reserve() {
    let value = full_capacity_settings();
    assert!(check(value.clone()).is_ok());
    let mut missing = value.clone();
    missing["bounded_host"]
        .as_object_mut()
        .unwrap()
        .remove("console_policy");
    assert!(check(missing).is_err());
    let mut short = value.clone();
    short["cleanup_reserve_bytes"] = json!(1042_u64 * 1024 * 1024 - 1);
    assert!(check(short).is_err());
    for field in [
        "max_shm_bytes",
        "max_receipt_bytes",
        "max_transaction_inputs",
    ] {
        let mut missing = value.clone();
        missing["maintenance"]["capacity"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(check(missing).is_err());
        let mut zero = value.clone();
        zero["maintenance"]["capacity"][field] = json!(0);
        assert!(check(zero).is_err());
    }
}

#[test]
fn production_maintenance_cannot_accept_work_without_cleanup_reservations() {
    let value = maintained_settings();
    assert!(check(value.clone()).is_err());
    let database = std::env::temp_dir().join(format!(
        "sandproof-unreserved-maintenance-{}.db",
        std::process::id()
    ));
    assert!(DeploymentContract::validate(&serde_json::from_value(value).unwrap()).is_err());
    // Refusal must happen before any database/host initialization.
    assert!(!database.exists());
}

#[test]
fn explicit_console_policy_is_strict_and_not_inferred() {
    let mut missing = settings();
    missing["bounded_host"]
        .as_object_mut()
        .unwrap()
        .remove("console_policy");
    assert!(check(missing).is_err());
    let mut value = settings();
    value["bounded_host"]["console_policy"] = json!("discard");
    assert_eq!(
        resolve(value.clone()).console_policy,
        Some(sandproof_controller_service::ConsolePolicy::Discard)
    );
    value["bounded_host"]["console_policy"] = json!("file");
    assert!(check(value).is_err());
}

#[test]
fn full_private_capacity_never_falls_back_to_legacy_retry_anchors() {
    let mut value = full_capacity_settings();
    value["internal_build_vms"] = json!(true);
    assert!(check(value.clone()).is_err());
    value["build_retry_windows"] = json!({"window_duration_ms": 100, "max_anchors": 2});
    assert!(check(value).is_ok());
}

#[test]
fn private_retry_policy_requires_explicit_maintained_private_pool() {
    let mut value = full_capacity_settings();
    value["internal_build_vms"] = json!(true);
    value["build_retry_windows"] = json!({"window_duration_ms": 100, "max_anchors": 2});
    assert!(check(value.clone()).is_ok());
    for field in ["internal_build_vms", "maintenance", "bounded_host"] {
        let mut invalid = value.clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert!(check(invalid).is_err(), "missing {field}");
    }
    for field in ["window_duration_ms", "max_anchors"] {
        let mut invalid = value.clone();
        invalid["build_retry_windows"][field] = json!(0);
        assert!(check(invalid).is_err());
        invalid = value.clone();
        invalid["build_retry_windows"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(check(invalid).is_err());
    }
    value["build_retry_windows"]["unknown"] = json!(1);
    assert!(check(value).is_err());
}

#[test]
fn maintenance_configuration_is_explicit_strict_and_requires_bounded_host() {
    assert!(check(settings()).is_ok());
    assert!(check(full_capacity_settings()).is_ok());
    for field in [
        "retention_ms",
        "max_checkpoint_bytes",
        "max_records",
        "max_references",
        "max_tail_inputs",
        "max_tail_bytes",
        "max_input_bytes",
        "prune_batch_rows",
    ] {
        for invalid in [
            json!(0),
            json!(-1),
            json!(true),
            json!("3"),
            json!(1.5),
            Value::Null,
            json!(u64::MAX),
        ] {
            let mut value = full_capacity_settings();
            value["maintenance"][field] = invalid;
            assert!(
                check(value).is_err(),
                "accepted invalid maintenance {field}"
            );
        }
        let mut value = full_capacity_settings();
        value["maintenance"].as_object_mut().unwrap().remove(field);
        assert!(
            check(value).is_err(),
            "accepted missing maintenance {field}"
        );
    }
    let mut value = maintained_settings();
    value["maintenance"]["unknown"] = json!(1);
    assert!(check(value).is_err());
    let mut value = maintained_settings();
    value.as_object_mut().unwrap().remove("bounded_host");
    assert!(check(value).is_err());
    let mut value = maintained_settings();
    value["maintenance"]["max_input_bytes"] = json!(8_000_001);
    assert!(check(value).is_err());
}

#[test]
fn bounded_route_capacity_comes_only_from_explicit_transfer_limits() {
    let original = resolve(settings());
    // The control reserve is already included in the per-VM cap.
    assert_eq!(original.max_data_routes_per_slot, 4);
    assert_eq!(original.max_data_routes_global, 8);
    assert_eq!(original.data_route_control_reserve, 1);
    let mut changed = settings();
    changed["data_transfer"]["max_transfers_per_vm"] = json!(5);
    let changed = resolve(changed);
    assert_eq!(changed.max_data_routes_per_slot, 5);
    assert_ne!(changed, original);
    let mut global_only = settings();
    global_only["data_transfer"]["max_transfers"] = json!(16);
    let global_only = resolve(global_only);
    assert_eq!(global_only.max_data_routes_global, 16);
    assert_eq!(
        global_only.max_data_routes_per_slot,
        original.max_data_routes_per_slot
    );
    assert_ne!(global_only, original);
    let mut reserve_only = settings();
    reserve_only["data_transfer"]["control_reserve"] = json!(2);
    let reserve_only = resolve(reserve_only);
    assert_eq!(reserve_only.data_route_control_reserve, 2);
    assert_ne!(reserve_only, original);
    for changed in [changed, global_only, reserve_only] {
        assert_ne!(
            serde_json::to_vec(&changed).unwrap(),
            serde_json::to_vec(&original).unwrap()
        );
    }
    for field in [
        "max_data_routes_per_slot",
        "max_data_routes_global",
        "data_route_control_reserve",
    ] {
        let mut duplicated = settings();
        duplicated["bounded_host"][field] = json!(4);
        assert!(check(duplicated).is_err());
    }
}

#[test]
fn bounded_mode_rejects_missing_or_invalid_transfer_budgets() {
    let mut missing = settings();
    missing.as_object_mut().unwrap().remove("data_transfer");
    assert!(check(missing.clone()).is_err());
    missing.as_object_mut().unwrap().remove("bounded_host");
    assert!(check(missing).is_err());
    let mut absent = settings();
    absent["data_transfer"] = Value::Null;
    assert!(check(absent).is_err());
    for (field, invalid) in [
        ("max_transfers_per_vm", json!(0)),
        ("max_transfers_per_vm", json!(true)),
        ("max_transfers_per_vm", json!(9)),
        ("max_transfers", json!(3)),
        ("control_reserve", json!(0)),
        ("control_reserve", json!(4)),
        ("rpc_request_bytes", json!(8 * 1024 * 1024 + 1)),
        ("file_upload_bytes", json!(0)),
        ("request_idle_seconds", json!(0)),
    ] {
        let mut value = settings();
        value["data_transfer"][field] = invalid;
        assert!(check(value).is_err(), "accepted invalid {field}");
    }
}

#[test]
fn bounded_configuration_is_explicit_and_permanent_ids_do_not_expand_uid_span() {
    assert!(check(settings()).is_ok());
    let mut bounded = settings();
    bounded["max_vm_id"] = json!(0);
    assert!(check(bounded).is_err());
    let mut legacy = settings();
    legacy.as_object_mut().unwrap().remove("bounded_host");
    assert!(check(legacy).is_err());
    let mut value = settings();
    value["bounded_host"] = Value::Null;
    assert!(check(value).is_err());
    let mut value = settings();
    value["uid_base"] = json!(u32::MAX - 2);
    value["bounded_host"]["uid_base"] = json!(u32::MAX - 2);
    assert!(check(value.clone()).is_ok());
    value["bounded_host"]["maintenance"]["max_host_slots"] = json!(3);
    assert!(check(value).is_err());
}

#[test]
fn bounded_configuration_rejects_missing_unknown_untyped_and_invalid_limits() {
    let paths = [
        "/uid_base",
        "/bounded_host/uid_base",
        "/bounded_host/maintenance/max_retained_results",
        "/bounded_host/maintenance/max_host_slots",
        "/bounded_host/maintenance/windows/max_open_windows",
        "/bounded_host/maintenance/windows/max_duration_ms",
    ];
    for path in paths {
        for invalid in [
            json!(0),
            json!(-1),
            json!(true),
            json!("2"),
            json!(1.5),
            Value::Null,
        ] {
            let mut value = settings();
            *value.pointer_mut(path).unwrap() = invalid;
            assert!(check(value).is_err(), "accepted invalid {path}");
        }
        let mut value = settings();
        let (parent, field) = path.rsplit_once('/').unwrap();
        value
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(check(value).is_err(), "accepted missing {path}");
    }
    for path in [
        "/bounded_host",
        "/bounded_host/maintenance",
        "/bounded_host/maintenance/windows",
    ] {
        let mut value = settings();
        value
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("misspelled".into(), json!(1));
        assert!(check(value).is_err(), "accepted unknown field in {path}");
    }
    for invalid in [
        json!(-1),
        json!(true),
        json!("0"),
        json!(1.5),
        json!(1_048_575),
        Value::Null,
    ] {
        let mut value = settings();
        value["bounded_host"]["uplink_base"] = invalid;
        assert!(check(value).is_err());
    }
    let mut value = settings();
    value["bounded_host"]
        .as_object_mut()
        .unwrap()
        .remove("uplink_base");
    assert!(check(value).is_err());
    let mut value = settings();
    value["bounded_host"]["uid_base"] = json!(1001);
    assert!(check(value).is_err());
    let mut value = settings();
    value["bounded_host"]["maintenance"]["max_host_slots"] = json!(1_048_577);
    assert!(check(value).is_err());
}
