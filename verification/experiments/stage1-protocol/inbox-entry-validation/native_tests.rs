//! One contract and the same assertions for original, candidate, wrong and restored views.
use super::*;
use crate::controller::lifecycle_inbox::model::{Entry, Inbox, Session, proof_entry_kind};

fn entry(operation: LifecycleOperation) -> Entry {
    let mut intent = create("owner", "session");
    intent.request.binding.expected = if matches!(operation, LifecycleOperation::Create { .. }) {
        ExpectedSession::Absent
    } else {
        ExpectedSession::Version(0)
    };
    intent.request.operation = operation;
    assert!(intent.validate(&context()).is_ok());
    Entry {
        intent,
        local_operation: None,
        applied_deadline: None,
        previous_effective_end_at_ms: None,
        unload_resolution: None,
    }
}

pub(super) fn create_entry() -> Entry {
    entry(create("owner", "session").request.operation)
}

fn renewal() -> Entry {
    Entry {
        local_operation: Some(core::OperationId::new(30)),
        applied_deadline: Some(20),
        previous_effective_end_at_ms: Some(20),
        ..entry(LifecycleOperation::Renew {
            incarnation: 1.try_into().unwrap(),
            end_at_ms: 10,
            extend_only: true,
        })
    }
}

pub(super) fn session(value: &Entry) -> Session {
    Session {
        id: value.intent.request.binding.session.clone(),
        retention: value.intent.retention,
        owner: value.intent.request.binding.key.owner.clone(),
        current_version: 1,
        terminal: false,
        refusal: None,
        incarnation: Some(1.try_into().unwrap()),
        local: Some(crate::InternalLocalBinding {
            sandbox: core::SandboxId::new(10),
            owner: core::TenantId::new(20),
            operation: core::OperationId::new(30),
        }),
        entries: vec![value.clone()],
        confirmed_terminal: None,
        confirmed_renewal: None,
        compacted_sdk_through: None,
        native_retirement: None,
    }
}

pub(super) fn inbox(value: &Entry) -> Inbox {
    Inbox {
        config: config(),
        host_epoch_observation: None,
        owner_high_water: 0,
        owners: vec![],
        sessions: vec![session(value)],
        lanes: vec![],
    }
}

fn accepted(value: &Entry, kind: usize) {
    assert!(
        matches!(proof_entry_kind(&inbox(value), &session(value), value, &context()), Ok(actual) if actual == kind)
    );
}

fn rejected(store: &Inbox, session: &Session, value: &Entry) {
    assert!(matches!(
        proof_entry_kind(store, session, value, &context()),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn normal_five_operations() {
    accepted(&create_entry(), 0);
    accepted(
        &entry(LifecycleOperation::Unload {
            source_incarnation: 1.try_into().unwrap(),
            capture: 1.try_into().unwrap(),
        }),
        1,
    );
    accepted(&entry(LifecycleOperation::Destroy {}), 2);
    accepted(&renewal(), 3);
    accepted(
        &entry(LifecycleOperation::Resume {
            source_incarnation: 1.try_into().unwrap(),
            capture: 1.try_into().unwrap(),
            target_incarnation: 9.try_into().unwrap(),
            manifest_digest: sandproof_protocol::lifecycle::ContentDigest::from_sha256([0; 32]),
        }),
        4,
    );
}

#[test]
fn duplicate_across_sessions_rejected() {
    let value = create_entry();
    let mut store = inbox(&value);
    store.sessions.push(session(&value));
    rejected(&store, &session(&value), &value);
    store.sessions.pop();
    store.sessions[0].entries.push(value.clone());
    rejected(&store, &session(&value), &value);
}

#[test]
fn every_key_field_distinguishes_rows() {
    let value = create_entry();
    for field in 0..4 {
        let mut other = value.clone();
        let key = &mut other.intent.request.binding.key;
        match field {
            0 => key.deployment = "other-deployment".try_into().unwrap(),
            1 => key.owner = "other-owner".try_into().unwrap(),
            2 => key.window = 11.try_into().unwrap(),
            _ => key.sequence = 2.try_into().unwrap(),
        }
        let mut store = inbox(&value);
        store.sessions.push(session(&other));
        assert!(matches!(
            proof_entry_kind(&store, &session(&value), &value, &context()),
            Ok(0)
        ));
    }
}

#[test]
fn renewal_keeps_prior_maximum() {
    let mut value = renewal();
    value.applied_deadline = Some(10);
    rejected(&inbox(&value), &session(&value), &value);
}

#[test]
fn retention_session_owner_match() {
    let value = create_entry();
    for field in 0..4 {
        let mut row = session(&value);
        match field {
            0 => row.retention.slot = 2,
            1 => row.retention.window = 2.try_into().unwrap(),
            2 => row.id = "other-session".try_into().unwrap(),
            _ => row.owner = "other-owner".try_into().unwrap(),
        }
        rejected(&inbox(&value), &row, &value);
    }
}

#[test]
fn envelope_and_independent_context_checked() {
    let value = create_entry();
    let mut config = config();
    config.node.node_generation = 2.try_into().unwrap();
    assert!(matches!(
        proof_entry_kind(&inbox(&value), &session(&value), &value, &config.context()),
        Err(JournalError::CorruptStore)
    ));
    let mut bad = value.clone();
    bad.intent.accepted_version = 2;
    rejected(&inbox(&bad), &session(&bad), &bad);
    bad = value;
    bad.intent.request.binding.protocol_version = 3;
    rejected(&inbox(&bad), &session(&bad), &bad);
}

#[test]
fn local_operation_requires_binding() {
    let mut value = create_entry();
    value.local_operation = Some(core::OperationId::new(30));
    let mut row = session(&value);
    row.local = None;
    rejected(&inbox(&value), &row, &value);
}

#[test]
fn nonrenew_metadata_rejected() {
    let mut value = create_entry();
    value.applied_deadline = Some(10);
    rejected(&inbox(&value), &session(&value), &value);
    value.applied_deadline = None;
    value.previous_effective_end_at_ms = Some(10);
    rejected(&inbox(&value), &session(&value), &value);
}

#[test]
fn renewal_pair_and_nonnegative_checked() {
    let mut value = renewal();
    value.applied_deadline = None;
    rejected(&inbox(&value), &session(&value), &value);
    value = renewal();
    value.previous_effective_end_at_ms = None;
    rejected(&inbox(&value), &session(&value), &value);
    value.previous_effective_end_at_ms = Some(-1);
    rejected(&inbox(&value), &session(&value), &value);
    value.previous_effective_end_at_ms = None;
    value.applied_deadline = None;
    accepted(&value, 3);
}

#[test]
fn operation_specific_limits_checked() {
    let value = create_entry();
    let mut store = inbox(&value);
    store.config.max_options_bytes = 0;
    rejected(&store, &session(&value), &value);
    let value = entry(LifecycleOperation::Unload {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    });
    let mut row = session(&value);
    row.local = None;
    rejected(&inbox(&value), &row, &value);
}

#[test]
fn this_method_does_not_prove_membership() {
    let value = create_entry();
    let store = inbox(&value);
    let mut other = value.clone();
    let LifecycleOperation::Create { create, .. } = &mut other.intent.request.operation else {
        unreachable!()
    };
    create.startup_deadline_ms += 1;
    assert_ne!(other, value);
    assert!(matches!(
        proof_entry_kind(&store, &session(&value), &other, &context()),
        Ok(0)
    ));
    assert_ne!(
        value.intent.request.binding.key.window.get(),
        value.intent.retention.window.get()
    );
}
