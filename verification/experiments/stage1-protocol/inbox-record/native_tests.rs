//! Same complete state assertions in original, candidate, both wrong and restored views.
use super::*;
use crate::controller::lifecycle_inbox::model::{Entry, Inbox, LifecycleRetentionLane, Session};

fn empty() -> Inbox {
    let config = config();
    assert!(config.valid());
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

fn expected_session(intent: &AcceptedLifecycleIntent) -> Session {
    let (terminal, incarnation) = match intent.request.operation {
        LifecycleOperation::Create {
            target_incarnation, ..
        } => (false, Some(target_incarnation)),
        LifecycleOperation::Destroy {} => (true, None),
        _ => panic!("fixture must select a supported fresh operation"),
    };
    Session {
        id: intent.request.binding.session.clone(),
        retention: intent.retention,
        owner: intent.request.binding.key.owner.clone(),
        current_version: intent.accepted_version,
        terminal,
        refusal: None,
        incarnation,
        local: None,
        entries: vec![Entry {
            intent: intent.clone(),
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

#[test]
fn normal_create_keeps_complete_intent_and_entire_frame() {
    let intent = create("alpha", "session-a");
    assert!(intent.validate(&context()).is_ok());
    let mut inbox = empty();
    let mut expected = inbox.clone();
    expected.sessions.push(expected_session(&intent));
    assert_eq!(inbox.record(&intent).unwrap(), (0, 0));
    assert_eq!(inbox, expected);
}

#[test]
fn foreign_retention_window_rejected_without_effect() {
    let mut intent = create("alpha", "session-a");
    intent.retention.window = 2.try_into().unwrap();
    assert!(intent.validate(&context()).is_ok());
    let mut inbox = empty();
    let before = inbox.clone();
    assert!(matches!(inbox.record(&intent), Err(JournalError::Conflict)));
    assert_eq!(inbox, before);
}

#[test]
fn occupied_slot_precedes_unsupported_operation() {
    let mut inbox = empty();
    inbox.record(&create("alpha", "session-a")).unwrap();
    let mut intent = create("beta", "session-b");
    intent.retention.slot = 1;
    intent.request.operation = LifecycleOperation::Unload {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    };
    intent.request.binding.expected = ExpectedSession::Version(0);
    assert!(intent.validate(&context()).is_ok());
    let before = inbox.clone();
    assert!(matches!(
        inbox.record(&intent),
        Err(JournalError::MaintenanceCapacity)
    ));
    assert_eq!(inbox, before);
}

#[test]
fn missing_lane_precedes_capacity_and_unsupported_operation() {
    let mut inbox = empty();
    inbox.record(&create("alpha", "session-a")).unwrap();
    inbox.config.max_sessions = 1;
    inbox.lanes.clear();
    let mut intent = create("beta", "session-b");
    intent.retention.slot = 1;
    intent.request.operation = LifecycleOperation::Unload {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    };
    intent.request.binding.expected = ExpectedSession::Version(0);
    assert!(intent.validate(&context()).is_ok());
    let before = inbox.clone();
    assert!(matches!(inbox.record(&intent), Err(JournalError::Conflict)));
    assert_eq!(inbox, before);
}

#[test]
fn fresh_destroy_keeps_complete_tombstone_without_native_resources() {
    let mut intent = create("alpha", "session-a");
    intent.request.operation = LifecycleOperation::Destroy {};
    intent.request.binding.expected = ExpectedSession::Version(0);
    assert!(intent.validate(&context()).is_ok());
    let mut inbox = empty();
    let mut expected = inbox.clone();
    expected.sessions.push(expected_session(&intent));
    assert_eq!(inbox.record(&intent).unwrap(), (0, 0));
    assert_eq!(inbox, expected);
}

#[test]
fn retained_replay_precedes_changed_current_metadata() {
    let intent = create("alpha", "session-a");
    let mut inbox = empty();
    inbox.record(&intent).unwrap();
    inbox.sessions[0].terminal = true;
    inbox.sessions[0].current_version = 100;
    inbox.config.max_sessions = 0;
    inbox.lanes.clear();
    let before = inbox.clone();
    assert_eq!(inbox.record(&intent).unwrap(), (0, 0));
    assert_eq!(inbox, before);
}
