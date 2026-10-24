//! The core intentionally has no JSON dependency. Exercise its real persisted
//! wire boundary here using the service's existing pinned `serde_json` version.
use core::checkpoint::{self, Checkpoint, CheckpointLimits, CheckpointReject};
use core::maintenance::{
    MaintenanceInput, MaintenanceLimits,
    windows::{WindowInput, WindowLimits},
};
use sandbox_controller_core as core;
use serde_json::{Value, json};

const LIMITS: CheckpointLimits = CheckpointLimits {
    max_records: 100,
    max_references: 100,
};

fn apply(world: &core::World, input: core::ControllerInput) -> core::World {
    core::handle_controller(world, input).unwrap().next
}
fn pending_world() -> core::World {
    let world = apply(
        &core::initial_world(),
        core::ControllerInput::Maintenance(MaintenanceInput::Enable {
            limits: MaintenanceLimits {
                max_retained_results: 10,
                max_host_slots: 2,
                windows: WindowLimits::new(2, 100).unwrap(),
            },
            now_ms: 0,
        }),
    );
    let world = apply(
        &world,
        core::ControllerInput::Request(core::Request::Create(core::CreateRequest {
            actor: core::Actor::Tenant(core::TenantId(7)),
            owner: core::TenantId(7),
            sandbox: core::SandboxId(9),
            workspace: core::FsId(9),
            file_policy: core::FilePolicyRef {
                digest: core::PolicyDigest(3),
            },
            policy: core::NetworkPolicy::Ipv4 {
                internet_access: false,
                allow_out: vec![core::Ipv4Cidr {
                    network: 0,
                    prefix_length: 0,
                }],
                deny_out: Vec::new(),
            },
            operation: core::OperationId(1),
        })),
    );
    let id = world.effect_ids()[0].clone();
    let world = apply(
        &world,
        core::ControllerInput::Effect(core::EffectInput::Dispatch(id.clone())),
    );
    let world = apply(
        &world,
        core::ControllerInput::Effect(core::EffectInput::Timeout(id)),
    );
    apply(
        &world,
        core::ControllerInput::Maintenance(MaintenanceInput::Window(WindowInput::Issue {
            tenant: core::TenantId(7),
            now_ms: 5,
            duration_ms: 10,
        })),
    )
}
fn wire() -> Value {
    serde_json::to_value(checkpoint::capture(&pending_world(), LIMITS).unwrap()).unwrap()
}
fn decode(value: &Value) -> Result<Checkpoint, serde_json::Error> {
    // Parse actual bytes rather than serde_json::Value's alternate deserializer.
    serde_json::from_slice(&serde_json::to_vec(value).unwrap())
}

#[test]
fn json_roundtrip_preserves_unknown_work_windows_and_exact_order() {
    let world = pending_world();
    let checkpoint = checkpoint::capture(&world, LIMITS).unwrap();
    let bytes = serde_json::to_vec(&checkpoint).unwrap();
    let decoded: Checkpoint = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
    let restored = checkpoint::restore(decoded, LIMITS).unwrap();
    assert_eq!(restored, world);
    assert_eq!(
        restored.effect_status(&world.effect_ids()[0]),
        Some(core::EffectStatus::OutcomeUnknown)
    );
}

#[test]
fn unknown_fields_reject_recursively_in_records_enum_variants_and_private_window_values() {
    for pointer in [
        "",
        "/maintenance",
        "/sandboxes/0/1",
        "/sandboxes/0/1/file_policy",
        "/sandboxes/0/1/policy/Ipv4",
        "/sandboxes/0/1/policy/Ipv4/allow_out/0",
        "/operations/0/1",
        "/operations/0/1/input/Create",
        "/operation_plans/0/1",
        "/logical_effects/0/0/command/InstallFence",
        "/logical_effects/0/1",
        "/effects/0/0/logical/command/InstallFence",
        "/inflight/0/logical/command/InstallFence",
        "/maintenance/windows",
        "/maintenance/windows/limits",
        "/maintenance/windows/open/0",
    ] {
        let mut value = wire();
        value
            .pointer_mut(pointer)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("future_field".into(), json!(true));
        assert!(
            decode(&value).is_err(),
            "unknown field accepted at {pointer}"
        );
    }
}

#[test]
fn duplicate_json_fields_and_duplicate_record_keys_fail_closed() {
    let bytes =
        serde_json::to_string(&checkpoint::capture(&pending_world(), LIMITS).unwrap()).unwrap();
    let version = checkpoint::CHECKPOINT_VERSION;
    let duplicate_version = bytes.replacen(
        &format!("\"version\":{version}"),
        &format!("\"version\":{version},\"version\":{version}"),
        1,
    );
    assert_ne!(duplicate_version, bytes);
    assert!(serde_json::from_str::<Checkpoint>(&duplicate_version).is_err());
    let duplicate_epoch = bytes.replacen("\"epoch\":1", "\"epoch\":1,\"epoch\":1", 1);
    assert_ne!(duplicate_epoch, bytes);
    assert!(serde_json::from_str::<Checkpoint>(&duplicate_epoch).is_err());
    let mut value = wire();
    let duplicate = value["sandboxes"][0].clone();
    value["sandboxes"].as_array_mut().unwrap().push(duplicate);
    assert_eq!(
        checkpoint::restore(decode(&value).unwrap(), LIMITS),
        Err(CheckpointReject::DuplicateKey)
    );
}

#[test]
fn current_supported_version_restores_its_actual_captured_world() {
    let world = pending_world();
    let captured = checkpoint::capture(&world, LIMITS).unwrap();
    let value = serde_json::to_value(captured).unwrap();
    assert_eq!(value["version"], json!(checkpoint::CHECKPOINT_VERSION));
    assert_eq!(
        checkpoint::restore(decode(&value).unwrap(), LIMITS),
        Ok(world)
    );
}

#[test]
fn valid_json_does_not_bypass_version_references_or_record_limits() {
    for unsupported in [1, checkpoint::CHECKPOINT_VERSION.checked_add(1).unwrap()] {
        assert_ne!(unsupported, checkpoint::CHECKPOINT_VERSION);
        let mut value = wire();
        value["version"] = json!(unsupported);
        assert_eq!(
            checkpoint::restore(decode(&value).unwrap(), LIMITS),
            Err(CheckpointReject::UnsupportedVersion)
        );
    }
    let mut value = wire();
    value["operations"][0][1]["sandbox"] = json!(99);
    assert!(checkpoint::restore(decode(&value).unwrap(), LIMITS).is_err());
    let low = CheckpointLimits {
        max_records: 1,
        ..LIMITS
    };
    assert_eq!(
        checkpoint::restore(decode(&wire()).unwrap(), low),
        Err(CheckpointReject::RecordLimit)
    );
    let low = CheckpointLimits {
        max_references: 1,
        ..LIMITS
    };
    assert_eq!(
        checkpoint::restore(decode(&wire()).unwrap(), low),
        Err(CheckpointReject::ReferenceLimit)
    );
}
