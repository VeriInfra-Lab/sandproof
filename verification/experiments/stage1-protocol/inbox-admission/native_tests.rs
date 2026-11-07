//! Same assertions injected into the original private test module for every variant.
use super::*;
use crate::controller::lifecycle_inbox::model::{Inbox, LifecycleRetentionLane};

fn inbox() -> Inbox {
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

fn original() -> AcceptedLifecycleIntent {
    let intent = create("alpha", "session-a");
    assert!(intent.validate(&context()).is_ok());
    intent
}

#[test]
fn normal_and_distinct_retention_window_are_accepted() {
    let inbox = inbox();
    let mut intent = original();
    assert!(authenticated(&inbox, &intent, &context()).is_ok());
    intent.retention.window = 17.try_into().unwrap();
    assert!(intent.validate(&context()).is_ok());
    assert!(authenticated(&inbox, &intent, &context()).is_ok());
}

#[test]
fn complete_peer_node_must_match_saved_configuration() {
    let intent = original();
    for field in 0..3 {
        let mut inbox = inbox();
        match field {
            0 => inbox.config.node.node_id = "other-host".try_into().unwrap(),
            1 => inbox.config.node.node_generation = 2.try_into().unwrap(),
            _ => inbox.config.node.deployment_id = "other-deployment".try_into().unwrap(),
        }
        assert!(matches!(
            authenticated(&inbox, &intent, &context()),
            Err(JournalError::Conflict)
        ));
    }
}

#[test]
fn complete_center_must_match_saved_configuration() {
    let intent = original();
    for field in 0..2 {
        let mut inbox = inbox();
        if field == 0 {
            inbox.config.center.center = "other-center".try_into().unwrap();
        } else {
            inbox.config.center.deployment = "other-deployment".try_into().unwrap();
        }
        assert!(matches!(
            authenticated(&inbox, &intent, &context()),
            Err(JournalError::Conflict)
        ));
    }
}

#[test]
fn envelope_shape_errors_are_conflicts_with_matching_context() {
    let inbox = inbox();
    for field in 0..8 {
        let mut intent = original();
        match field {
            0 => intent.request.binding.protocol_version = 3,
            1 => intent.request.binding.node.node_id = "other-host".try_into().unwrap(),
            2 => intent.request.binding.key.deployment = "other-deployment".try_into().unwrap(),
            3 => intent.request.binding.expected = ExpectedSession::Version(0),
            4 => {
                let LifecycleOperation::Create { create, .. } = &mut intent.request.operation
                else {
                    panic!("original create fixture changed");
                };
                create.options.clear();
            }
            5 => intent.accepted_version = 0,
            6 => intent.retention.slot = 0,
            _ => intent.center.center = "other-center".try_into().unwrap(),
        }
        assert!(intent.validate(&context()).is_err(), "field {field}");
        assert!(matches!(
            authenticated(&inbox, &intent, &context()),
            Err(JournalError::Conflict)
        ));
    }
}

#[test]
fn foreign_context_cannot_be_repaired_by_matching_body() {
    let inbox = inbox();
    let mut foreign_node = inbox.config.node.clone();
    foreign_node.node_generation = 2.try_into().unwrap();
    let peer = AuthenticatedCenterContext::from_verified_peer_registration(
        inbox.config.center.clone(),
        foreign_node.clone(),
    );
    let mut intent = original();
    intent.request.binding.node = foreign_node;
    assert!(intent.validate(&peer).is_ok());
    assert!(matches!(
        authenticated(&inbox, &intent, &peer),
        Err(JournalError::Conflict)
    ));
}
