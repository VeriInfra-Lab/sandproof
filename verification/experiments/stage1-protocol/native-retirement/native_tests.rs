//! Exploratory original `validate_saved` checks, not a receipt or physical cleanup proof.
use super::*;
use crate::controller::lifecycle_inbox::model::{Entry, Session};
use crate::controller::lifecycle_inbox::native_retirement::{
    NativeRetirementReceipt, validate_saved,
};
use crate::controller::lifecycle_inbox::source_work;
use sandbox_controller_core::maintenance::retirement::{
    QuiescenceCertificate, QuiescenceStatus, RetireRequest,
};

fn normal() -> Session {
    let initial = create("owner", "session");
    let mut destroy = initial.clone();
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2_u64.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    assert_eq!(initial.validate(&context()), Ok(()));
    assert_eq!(destroy.validate(&context()), Ok(()));
    let local = InternalLocalBinding {
        sandbox: core::SandboxId(10),
        owner: core::TenantId(20),
        operation: core::OperationId(30),
    };
    let retirement = RetireRequest {
        sandbox: local.sandbox,
        owner: local.owner,
        epoch: 5,
        operation: core::OperationId(31),
        retain_until: 100,
        quiescence: QuiescenceCertificate {
            sandbox: local.sandbox,
            owner: local.owner,
            epoch: 5,
            operation: core::OperationId(31),
            workspace: core::FsId(6),
            worker_drained: QuiescenceStatus::Confirmed,
            data_connections_drained: QuiescenceStatus::Confirmed,
            host_resources_reclaimed: QuiescenceStatus::Confirmed,
            external_pins_released: QuiescenceStatus::Confirmed,
        },
    };
    let saved: NativeRetirementReceipt = serde_json::from_value(serde_json::json!({
        "local": local,
        "request_binding": source_work::digest(&serde_json::to_vec(&destroy).unwrap()),
        "retirement": retirement,
    }))
    .unwrap();
    let entry = |intent, operation| Entry {
        intent,
        local_operation: Some(core::OperationId(operation)),
        applied_deadline: None,
        previous_effective_end_at_ms: None,
        unload_resolution: None,
    };
    Session {
        id: initial.request.binding.session.clone(),
        retention: initial.retention,
        owner: initial.request.binding.key.owner.clone(),
        current_version: 2,
        terminal: true,
        refusal: None,
        incarnation: Some(101_u64.try_into().unwrap()),
        local: Some(local),
        entries: vec![entry(initial, 30), entry(destroy, 31)],
        confirmed_terminal: None,
        confirmed_renewal: None,
        compacted_sdk_through: None,
        native_retirement: Some(saved),
    }
}

fn check(value: &Session, accepted: bool) {
    let before = value.clone();
    match (validate_saved(value), accepted) {
        (Ok(()), true) | (Err(JournalError::CorruptStore), false) => {}
        (result, _) => panic!("unexpected saved receipt result: {result:?}; accepted={accepted}"),
    }
    assert_eq!(*value, before);
}

fn change_saved(value: &mut Session, pointer: &str, replacement: serde_json::Value) {
    let mut json = serde_json::to_value(value.native_retirement.as_ref().unwrap()).unwrap();
    *json.pointer_mut(pointer).unwrap() = replacement;
    value.native_retirement = Some(serde_json::from_value(json).unwrap());
}

#[test]
fn normal_saved_receipt_accepted() {
    check(&normal(), true);
}

#[test]
fn absent_receipt_skips_session_checks() {
    let mut value = normal();
    value.native_retirement = None;
    value.entries.clear();
    value.local = None;
    value.terminal = false;
    check(&value, true);
}

#[test]
fn present_receipt_requires_last_entry() {
    let mut value = normal();
    value.entries.clear();
    check(&value, false);
}

#[test]
fn terminal_local_version_operation_guards() {
    let changes: [fn(&mut Session); 7] = [
        |s| s.terminal = false,
        |s| s.local = None,
        |s| s.local.as_mut().unwrap().sandbox = core::SandboxId(11),
        |s| s.local.as_mut().unwrap().owner = core::TenantId(21),
        |s| s.local.as_mut().unwrap().operation = core::OperationId(99),
        |s| s.current_version = 3,
        |s| s.entries.last_mut().unwrap().local_operation = None,
    ];
    for change in changes {
        let mut value = normal();
        change(&mut value);
        check(&value, false);
    }
}

#[test]
fn full_intent_digest_binding_is_observed() {
    let mut value = normal();
    value.entries.last_mut().unwrap().intent.center.center = "other".try_into().unwrap();
    check(&value, false);
}

#[test]
fn receipt_identity_fields_are_observed() {
    for pointer in [
        "/retirement/sandbox",
        "/retirement/owner",
        "/retirement/operation",
        "/retirement/quiescence/sandbox",
        "/retirement/quiescence/owner",
        "/retirement/quiescence/epoch",
        "/retirement/quiescence/operation",
    ] {
        let mut value = normal();
        change_saved(&mut value, pointer, serde_json::json!(99));
        check(&value, false);
    }
}

#[test]
fn all_four_status_tags_are_observed() {
    for field in [
        "worker_drained",
        "data_connections_drained",
        "host_resources_reclaimed",
        "external_pins_released",
    ] {
        let mut value = normal();
        change_saved(
            &mut value,
            &format!("/retirement/quiescence/{field}"),
            serde_json::json!("Unconfirmed"),
        );
        check(&value, false);
    }
}

#[test]
fn workspace_and_retention_are_not_checked_here() {
    let mut value = normal();
    change_saved(
        &mut value,
        "/retirement/quiescence/workspace",
        serde_json::json!(99),
    );
    change_saved(&mut value, "/retirement/retain_until", serde_json::json!(0));
    check(&value, true);
}

#[test]
fn envelope_validity_is_a_separate_outer_obligation() {
    let mut value = normal();
    value
        .entries
        .last_mut()
        .unwrap()
        .intent
        .request
        .binding
        .protocol_version = 3;
    let intent = &value.entries.last().unwrap().intent;
    assert!(intent.validate(&context()).is_err());
    let binding = source_work::digest(&serde_json::to_vec(intent).unwrap());
    change_saved(
        &mut value,
        "/request_binding",
        serde_json::to_value(binding).unwrap(),
    );
    check(&value, true);
}

#[test]
fn non_destroy_rejected_even_with_recomputed_binding() {
    let mut value = normal();
    value.entries.last_mut().unwrap().intent.request.operation =
        value.entries[0].intent.request.operation.clone();
    let intent = &value.entries.last().unwrap().intent;
    let binding = source_work::digest(&serde_json::to_vec(intent).unwrap());
    change_saved(
        &mut value,
        "/request_binding",
        serde_json::to_value(binding).unwrap(),
    );
    check(&value, false);
}
