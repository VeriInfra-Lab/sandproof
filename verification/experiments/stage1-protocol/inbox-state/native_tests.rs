//! Same complete query assertions in original, both wrong, and restored Rust.
use super::*;
use crate::controller::lifecycle_inbox::model::{Inbox, LifecycleRetentionLane};

fn recorded() -> Inbox {
    let config = config();
    let mut inbox = Inbox {
        lanes: (1..=config.max_sessions)
            .map(LifecycleRetentionLane::fresh)
            .collect(),
        config,
        host_epoch_observation: None,
        owner_high_water: 0,
        owners: vec![],
        sessions: vec![],
    };
    let intent = create("owner", "session-a");
    assert!(intent.validate(&context()).is_ok());
    assert_eq!(inbox.record(&intent).unwrap(), (0, 0));
    inbox
}

fn advanced() -> Inbox {
    let mut inbox = recorded();
    let session = &mut inbox.sessions[0];
    session.local = Some(InternalLocalBinding {
        sandbox: core::SandboxId(10),
        owner: core::TenantId(20),
        operation: core::OperationId(30),
    });
    session.entries[0].local_operation = Some(core::OperationId(41));
    let mut intent = session.entries[0].intent.clone();
    intent.accepted_version = 2;
    intent.request.binding.expected = ExpectedSession::Version(1);
    intent.request.binding.key.sequence = 2.try_into().unwrap();
    intent.request.operation = LifecycleOperation::Unload {
        source_incarnation: session.incarnation.unwrap(),
        capture: 1.try_into().unwrap(),
    };
    assert!(intent.validate(&context()).is_ok());
    assert_eq!(inbox.record(&intent).unwrap(), (0, 1));
    inbox
}

fn assert_state(inbox: &Inbox, s: usize, e: usize) {
    let before = inbox.clone();
    let session = &inbox.sessions[s];
    let entry = &session.entries[e];
    let expected = LifecycleWorkState {
        intent: entry.intent.clone(),
        current_session_version: session.current_version,
        current: entry.intent.accepted_version == session.current_version,
        terminal: session.terminal,
        refusal: session.refusal,
        local: session.local,
        local_operation: entry.local_operation,
    };
    assert_eq!(inbox.state(s, e), expected);
    assert_eq!(*inbox, before);
}

#[test]
fn fresh_current_keeps_every_field() {
    assert_state(&recorded(), 0, 0);
}

#[test]
fn retained_entry_is_stale_with_own_operation() {
    let inbox = advanced();
    assert_state(&inbox, 0, 0);
    assert!(!inbox.state(0, 0).current);
    assert_eq!(
        inbox.state(0, 0).local_operation,
        Some(core::OperationId(41))
    );
}

#[test]
fn current_entry_does_not_borrow_session_operation() {
    let inbox = advanced();
    assert_state(&inbox, 0, 1);
    assert!(inbox.state(0, 1).current);
    assert_eq!(inbox.state(0, 1).local_operation, None);
}

#[test]
fn nonzero_session_keeps_flags_and_complete_intent() {
    let mut inbox = recorded();
    let intent = create("second-owner", "session-b");
    assert!(intent.validate(&context()).is_ok());
    assert_eq!(inbox.record(&intent).unwrap(), (1, 0));
    // Query arbitrary represented flags too; this does not establish reachability.
    inbox.sessions[1].terminal = true;
    inbox.sessions[1].refusal = Some(sandproof_protocol::lifecycle::CreateRefusalReason::Capacity);
    assert_state(&inbox, 1, 0);
}
