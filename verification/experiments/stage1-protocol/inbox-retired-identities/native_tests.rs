//! One full identity contract for original, candidate, wrong and restored Rust.
use super::*;
use crate::controller::lifecycle_inbox::model::{
    Entry, Inbox, LifecycleRetentionLane, Session, proof_validate_retired_identities,
};
use sandproof_protocol::lifecycle::{TerminalLifecycleObservation, TerminalStatus};

fn retired(owner: &str, id: &str) -> TerminalLifecycleObservation {
    let mut request = create(owner, id);
    request.request.binding.expected = ExpectedSession::Version(1);
    request.request.binding.key.sequence = 2.try_into().unwrap();
    request.request.operation = LifecycleOperation::Destroy {};
    request.accepted_version = 2;
    assert!(request.validate(&context()).is_ok());
    TerminalLifecycleObservation {
        request,
        status: TerminalStatus::Cleaned,
    }
}

fn lane(value: TerminalLifecycleObservation) -> LifecycleRetentionLane {
    LifecycleRetentionLane {
        slot: value.request.retention.slot,
        retired_through: 1,
        current_window: Some(2.try_into().unwrap()),
        last_retired: Some(value),
    }
}

fn session() -> Session {
    let intent = create("active-owner", "active-session");
    Session {
        id: intent.request.binding.session.clone(),
        retention: intent.retention,
        owner: intent.request.binding.key.owner.clone(),
        current_version: 1,
        terminal: false,
        refusal: None,
        incarnation: Some(1.try_into().unwrap()),
        local: None,
        entries: vec![Entry {
            intent,
            local_operation: None,
            applied_deadline: None,
            previous_effective_end_at_ms: None,
            unload_resolution: None,
        }],
        confirmed_terminal: None,
        confirmed_renewal: None,
        compacted_sdk_through: None,
        native_retirement: None,
    }
}

fn store() -> Inbox {
    Inbox {
        config: config(),
        host_epoch_observation: None,
        owner_high_water: 0,
        owners: vec![],
        sessions: vec![],
        lanes: vec![lane(retired("retired-owner", "retired-session"))],
    }
}

fn accepted(value: &Inbox) {
    assert!(proof_validate_retired_identities(value).is_ok());
}

fn rejected(value: &Inbox) {
    assert!(matches!(
        proof_validate_retired_identities(value),
        Err(JournalError::CorruptStore)
    ));
}

#[test]
fn empty_and_absent_accepted() {
    let mut value = store();
    value.lanes.clear();
    value.sessions.push(session());
    accepted(&value);
    value.lanes.push(LifecycleRetentionLane::fresh(1));
    accepted(&value);
}

#[test]
fn normal_disjoint_accepted() {
    let mut value = store();
    value.lanes.push(lane(retired("other-owner", "session-b")));
    value.sessions.push(session());
    accepted(&value);
}

#[test]
fn prior_session_rejected() {
    let mut value = store();
    value
        .lanes
        .push(lane(retired("other-owner", "retired-session")));
    rejected(&value);
}

#[test]
fn prior_key_rejected() {
    let mut value = store();
    value
        .lanes
        .push(lane(retired("retired-owner", "other-session")));
    rejected(&value);
}

#[test]
fn active_session_rejected() {
    let mut value = store();
    let mut active = session();
    active.id = "retired-session".try_into().unwrap();
    active.entries.clear();
    value.sessions.push(active);
    rejected(&value);
}

#[test]
fn active_key_rejected() {
    let mut value = store();
    let mut active = session();
    active.entries[0].intent = value.lanes[0]
        .last_retired
        .as_ref()
        .unwrap()
        .request
        .clone();
    value.sessions.push(active);
    rejected(&value);
}

#[test]
fn all_active_key_fields_distinguish() {
    for field in 0..4 {
        let mut value = store();
        let mut active = session();
        active.entries[0].intent = value.lanes[0]
            .last_retired
            .as_ref()
            .unwrap()
            .request
            .clone();
        let key = &mut active.entries[0].intent.request.binding.key;
        match field {
            0 => key.deployment = "other-deployment".try_into().unwrap(),
            1 => key.owner = "other-owner".try_into().unwrap(),
            2 => key.window = 9.try_into().unwrap(),
            _ => key.sequence = 9.try_into().unwrap(),
        }
        value.sessions.push(active);
        accepted(&value);
    }
}

#[test]
fn nonadjacent_prior_rejected() {
    let mut value = store();
    value.lanes.push(LifecycleRetentionLane::fresh(2));
    value
        .lanes
        .push(lane(retired("middle-owner", "middle-session")));
    value
        .lanes
        .push(lane(retired("other-owner", "retired-session")));
    rejected(&value);
}

#[test]
fn late_active_entry_rejected() {
    let mut value = store();
    let mut active = session();
    let mut hit = active.entries[0].clone();
    hit.intent = value.lanes[0]
        .last_retired
        .as_ref()
        .unwrap()
        .request
        .clone();
    active.entries.push(hit);
    value.sessions.push(session());
    value.sessions.push(active);
    rejected(&value);
}

#[test]
fn unrelated_metadata_not_strengthened() {
    let mut value = store();
    let last = value.lanes[0].last_retired.as_mut().unwrap();
    last.request.request.binding.protocol_version = 0;
    last.request.retention.slot = 0;
    last.status = TerminalStatus::Pending;
    value.lanes[0].retired_through = u64::MAX;
    value.lanes[0].current_window = None;
    accepted(&value);
}
