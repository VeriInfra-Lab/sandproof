//! Injected only into the original private test module in isolated source copies.
use super::*;
use crate::controller::lifecycle_inbox::model::{Entry, Inbox, LifecycleRetentionLane};

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

fn original() -> AcceptedLifecycleIntent {
    let intent = create("alpha", "session-a");
    assert!(intent.validate(&context()).is_ok());
    intent
}

fn entry(intent: AcceptedLifecycleIntent) -> Entry {
    Entry {
        intent,
        local_operation: None,
        applied_deadline: None,
        previous_effective_end_at_ms: None,
        unload_resolution: None,
    }
}

#[test]
fn original_lookup_keeps_position_and_ignores_current_metadata() {
    let intent = original();
    let mut inbox = empty();
    assert_eq!(inbox.find(&intent).unwrap(), None);
    inbox.record(&intent).unwrap();
    assert_eq!(inbox.find(&intent).unwrap(), Some((0, 0)));
    inbox.sessions[0].current_version = intent.accepted_version + 100;
    inbox.sessions[0].terminal = true;
    inbox.sessions[0].entries.insert(
        0,
        entry({
            let mut other = intent.clone();
            other.request.binding.key.sequence = 99.try_into().unwrap();
            other
        }),
    );
    let mut preceding = inbox.sessions[0].clone();
    preceding.entries.clear();
    inbox.sessions.insert(0, preceding);
    assert_eq!(inbox.find(&intent).unwrap(), Some((1, 1)));
}

#[test]
fn first_key_conflict_is_not_hidden_by_a_later_exact_replay() {
    let intent = original();
    let mut inbox = empty();
    inbox.record(&intent).unwrap();
    let mut changed = intent.clone();
    changed.accepted_version += 1;
    inbox.sessions[0].entries = vec![entry(changed.clone()), entry(intent.clone())];
    assert!(matches!(inbox.find(&intent), Err(JournalError::Conflict)));
    inbox.sessions[0].entries = vec![entry(intent.clone()), entry(changed.clone())];
    assert_eq!(inbox.find(&intent).unwrap(), Some((0, 0)));
    let mut later = inbox.sessions[0].clone();
    later.entries = vec![entry(intent.clone())];
    inbox.sessions[0].entries = vec![entry(changed)];
    inbox.sessions.push(later);
    assert!(matches!(inbox.find(&intent), Err(JournalError::Conflict)));
}

#[test]
fn same_key_changed_full_envelope_is_always_a_conflict() {
    let intent = original();
    let mut inbox = empty();
    inbox.record(&intent).unwrap();
    let mut variants = vec![];
    macro_rules! change {
        ($name:expr, $value:ident, $body:block) => {{
            let mut $value = intent.clone();
            $body
            assert_ne!($value, intent, "mutation must change {}", $name);
            assert_eq!($value.request.binding.key, intent.request.binding.key);
            variants.push(($name, $value));
        }};
    }
    change!("center", v, {
        v.center.center = "other-center".try_into().unwrap();
    });
    change!("center deployment", v, {
        v.center.deployment = "other-deployment".try_into().unwrap();
    });
    change!("accepted version", v, {
        v.accepted_version += 1;
    });
    change!("retention slot", v, {
        v.retention.slot += 1;
    });
    change!("retention window", v, {
        v.retention.window = 7.try_into().unwrap();
    });
    change!("protocol", v, {
        v.request.binding.protocol_version += 1;
    });
    change!("node", v, {
        v.request.binding.node.node_id = "other-node".try_into().unwrap();
    });
    change!("node deployment", v, {
        v.request.binding.node.deployment_id = "other-deployment".try_into().unwrap();
    });
    change!("node generation", v, {
        v.request.binding.node.node_generation = 7.try_into().unwrap();
    });
    change!("session", v, {
        v.request.binding.session = SessionId::try_from("other-session").unwrap();
    });
    change!("expected", v, {
        v.request.binding.expected = ExpectedSession::Version(0);
    });
    change!("operation", v, {
        v.request.operation = LifecycleOperation::Destroy {};
    });
    change!("create parameters", v, {
        let LifecycleOperation::Create { create, .. } = &mut v.request.operation else {
            panic!()
        };
        create.options.push(0);
    });
    for (name, value) in variants {
        assert!(
            matches!(inbox.find(&value), Err(JournalError::Conflict)),
            "{name}"
        );
    }
}

#[test]
fn changed_complete_key_is_not_a_replay() {
    let intent = original();
    let mut inbox = empty();
    inbox.record(&intent).unwrap();
    for part in 0..4 {
        let mut value = intent.clone();
        let key = &mut value.request.binding.key;
        match part {
            0 => key.deployment = "other-deployment".try_into().unwrap(),
            1 => key.owner = "other-owner".try_into().unwrap(),
            2 => key.window = 7.try_into().unwrap(),
            _ => key.sequence = 7.try_into().unwrap(),
        }
        assert_ne!(value.request.binding.key, intent.request.binding.key);
        assert_eq!(inbox.find(&value).unwrap(), None);
    }
}
