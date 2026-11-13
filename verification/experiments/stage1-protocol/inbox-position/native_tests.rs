//! Exercise actual record-return positions and compare the whole query value.
use super::*;
use crate::controller::lifecycle_inbox::model::{Inbox, LifecycleRetentionLane};

fn empty() -> Inbox {
    let config = config();
    Inbox {
        lanes: (1..=config.max_sessions)
            .map(LifecycleRetentionLane::fresh)
            .collect(),
        config,
        host_epoch_observation: None,
        owner_high_water: 0,
        owners: vec![],
        sessions: vec![],
    }
}

fn record_query(inbox: &mut Inbox, intent: &AcceptedLifecycleIntent) -> LifecycleWorkState {
    assert!(intent.validate(&context()).is_ok());
    let (s, e) = inbox.record(intent).unwrap();
    let before_query = inbox.clone();
    let session = &inbox.sessions[s];
    let entry = &session.entries[e];
    assert_eq!(entry.intent, *intent);
    let expected = LifecycleWorkState {
        intent: intent.clone(),
        current_session_version: session.current_version,
        current: intent.accepted_version == session.current_version,
        terminal: session.terminal,
        refusal: session.refusal,
        local: session.local,
        local_operation: entry.local_operation,
    };
    let actual = inbox.state(s, e);
    assert_eq!(actual, expected);
    assert_eq!(*inbox, before_query);
    actual
}

fn running() -> (Inbox, AcceptedLifecycleIntent) {
    let mut inbox = empty();
    let create = create("owner", "session-a");
    record_query(&mut inbox, &create);
    inbox.sessions[0].local = Some(InternalLocalBinding {
        sandbox: core::SandboxId(10),
        owner: core::TenantId(20),
        operation: core::OperationId(30),
    });
    inbox.sessions[0].entries[0].local_operation = Some(core::OperationId(41));
    (inbox, create)
}

fn unload(inbox: &Inbox, create: &AcceptedLifecycleIntent) -> AcceptedLifecycleIntent {
    let mut intent = create.clone();
    intent.accepted_version = 2;
    intent.request.binding.expected = ExpectedSession::Version(1);
    intent.request.binding.key.sequence = 2.try_into().unwrap();
    intent.request.operation = LifecycleOperation::Unload {
        source_incarnation: inbox.sessions[0].incarnation.unwrap(),
        capture: 1.try_into().unwrap(),
    };
    intent
}

#[test]
fn fresh_positions_return_complete_current_values() {
    let mut inbox = empty();
    for (owner, session) in [("first", "session-a"), ("second", "session-b")] {
        let state = record_query(&mut inbox, &create(owner, session));
        assert!(state.current);
        assert_eq!(state.local_operation, None);
    }
    assert_eq!(inbox.sessions.len(), 2);
}

#[test]
fn existing_position_returns_new_entry() {
    let (mut inbox, create) = running();
    let intent = unload(&inbox, &create);
    let state = record_query(&mut inbox, &intent);
    assert!(state.current);
    assert_eq!(state.current_session_version, 2);
    assert_eq!(state.local_operation, None);
    assert_eq!(inbox.sessions[0].entries.len(), 2);
}

#[test]
fn replay_position_returns_stale_entry_with_own_operation() {
    let (mut inbox, create) = running();
    let intent = unload(&inbox, &create);
    record_query(&mut inbox, &intent);
    let before = inbox.clone();
    let state = record_query(&mut inbox, &create);
    assert!(!state.current);
    assert_eq!(state.current_session_version, 2);
    assert_eq!(state.local_operation, Some(core::OperationId(41)));
    assert_eq!(inbox, before);
}

#[test]
fn first_key_conflict_preserves_entire_inbox() {
    let (mut inbox, mut changed) = running();
    let LifecycleOperation::Create {
        target_incarnation, ..
    } = &mut changed.request.operation
    else {
        unreachable!("fixture is Create")
    };
    *target_incarnation = 102.try_into().unwrap();
    assert!(changed.validate(&context()).is_ok());
    let before = inbox.clone();
    assert!(matches!(
        inbox.record(&changed),
        Err(JournalError::Conflict)
    ));
    assert_eq!(inbox, before);
}
