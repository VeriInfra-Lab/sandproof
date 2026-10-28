//! Same assertions for the original, candidate, wrong and restored actual methods.
use super::*;
use crate::controller::lifecycle_inbox::model::{
    LifecycleRetentionLane, proof_retention_lane_valid,
};
use sandproof_protocol::lifecycle::{TerminalLifecycleObservation, TerminalStatus};

pub(super) fn normal() -> LifecycleRetentionLane {
    let mut request = create("owner", "session");
    request.request.binding.expected = ExpectedSession::Version(0);
    request.request.operation = LifecycleOperation::Destroy {};
    assert_eq!(request.validate(&context()), Ok(()));
    LifecycleRetentionLane {
        slot: request.retention.slot,
        retired_through: request.retention.window.get(),
        current_window: Some(2_u64.try_into().unwrap()),
        last_retired: Some(TerminalLifecycleObservation {
            request,
            status: TerminalStatus::Cleaned,
        }),
    }
}
fn last(lane: &mut LifecycleRetentionLane) -> &mut TerminalLifecycleObservation {
    lane.last_retired.as_mut().unwrap()
}
fn valid(lane: &LifecycleRetentionLane) -> bool {
    proof_retention_lane_valid(lane, &context())
}

#[test]
fn normal_retired_accepted() {
    let lane = normal();
    let bytes = serde_json::to_vec(lane.last_retired.as_ref().unwrap()).unwrap();
    assert!(bytes.len() <= 8192);
    assert!(valid(&lane));
}
#[test]
fn fresh_slot_is_not_checked_here() {
    for slot in [0, 1, u16::MAX] {
        let lane = LifecycleRetentionLane {
            slot,
            retired_through: 0,
            current_window: Some(1_u64.try_into().unwrap()),
            last_retired: None,
        };
        assert!(valid(&lane));
    }
}
#[test]
fn highest_floor_has_no_successor() {
    let mut lane = normal();
    lane.retired_through = u64::MAX;
    lane.current_window = None;
    last(&mut lane).request.retention.window = u64::MAX.try_into().unwrap();
    assert!(valid(&lane));
}
#[test]
fn key_window_need_not_equal_retention_window() {
    let mut lane = normal();
    lane.retired_through = 9;
    lane.current_window = Some(10_u64.try_into().unwrap());
    last(&mut lane).request.retention.window = 9_u64.try_into().unwrap();
    assert_ne!(
        last(&mut lane).request.request.binding.key.window.get(),
        lane.retired_through
    );
    assert!(valid(&lane));
}
#[test]
fn wrong_successor_rejected() {
    for current in [None, Some(3_u64.try_into().unwrap())] {
        let mut lane = normal();
        lane.current_window = current;
        assert!(!valid(&lane));
    }
}
#[test]
fn missing_last_requires_zero_floor() {
    let mut lane = normal();
    lane.last_retired = None;
    assert!(!valid(&lane));
}
#[test]
fn present_last_requires_positive_floor() {
    let mut lane = normal();
    lane.retired_through = 0;
    lane.current_window = Some(1_u64.try_into().unwrap());
    assert!(!valid(&lane));
}
#[test]
fn pending_rejected() {
    let mut lane = normal();
    last(&mut lane).status = TerminalStatus::Pending;
    assert!(!valid(&lane));
}
#[test]
fn foreign_center_rejected() {
    let lane = normal();
    let mut other = config();
    other.center.center = "other".try_into().unwrap();
    assert!(!proof_retention_lane_valid(&lane, &other.context()));
}
#[test]
fn foreign_node_rejected() {
    let lane = normal();
    let mut other = config();
    other.node.node_id = "other".try_into().unwrap();
    assert!(!proof_retention_lane_valid(&lane, &other.context()));
}
#[test]
fn old_protocol_rejected() {
    let mut lane = normal();
    last(&mut lane).request.request.binding.protocol_version = 3;
    assert!(!valid(&lane));
}
#[test]
fn nonterminal_operation_rejected() {
    let mut lane = normal();
    last(&mut lane).request.request.operation = LifecycleOperation::Unload {
        source_incarnation: 1_u64.try_into().unwrap(),
        capture: 1_u64.try_into().unwrap(),
    };
    assert!(!valid(&lane));
}
#[test]
fn version_overflow_rejected() {
    let mut lane = normal();
    let request = &mut last(&mut lane).request;
    request.request.binding.expected = ExpectedSession::Version(u64::MAX);
    request.accepted_version = u64::MAX;
    assert!(!valid(&lane));
}
#[test]
fn wrong_slot_rejected() {
    let mut lane = normal();
    lane.slot = 2;
    assert!(!valid(&lane));
}
#[test]
fn wrong_window_rejected() {
    let mut lane = normal();
    last(&mut lane).request.retention.window = 2_u64.try_into().unwrap();
    assert!(!valid(&lane));
}
#[test]
fn invalid_retention_slot_rejected() {
    for slot in [0, 4097] {
        let mut lane = normal();
        lane.slot = slot;
        last(&mut lane).request.retention.slot = slot;
        assert!(!valid(&lane));
    }
}
