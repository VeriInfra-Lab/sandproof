//! One full budget contract for original, wrong and restored Rust.
use super::*;
use crate::controller::lifecycle_inbox::model::proof_inbox_required_bytes;

fn small() -> LifecycleInboxConfig {
    let mut value = config();
    value.host_epoch = None;
    value.max_sessions = 1;
    value.max_options_bytes = 1;
    value.max_profile_bytes = 1;
    value.max_manifest_bytes = 1;
    value
}

#[test]
fn legal_small_matches() {
    let value = small();
    assert!(value.valid());
    assert_eq!(proof_inbox_required_bytes(&value), 106_503);
}

#[test]
fn legal_fixture_matches() {
    let mut value = config();
    value.host_epoch = None;
    assert!(value.valid());
    assert_eq!(proof_inbox_required_bytes(&value), 286_720);
}

#[test]
fn zero_positions_keeps_global_reserve() {
    let mut value = config();
    value.max_sessions = 0;
    assert_eq!(proof_inbox_required_bytes(&value), 8192);
}

#[test]
fn options_increment_charges_four_per_position() {
    let mut value = config();
    let before = proof_inbox_required_bytes(&value);
    value.max_options_bytes += 1;
    assert!(value.valid());
    assert_eq!(proof_inbox_required_bytes(&value), before + 8);
}

#[test]
fn operation_increment_charges_8192_per_position() {
    let mut value = config();
    let before = proof_inbox_required_bytes(&value);
    value.max_operations_per_session += 1;
    assert!(value.valid());
    assert_eq!(proof_inbox_required_bytes(&value), before + 16_384);
}

#[test]
fn manifest_increment_charges_two_per_position() {
    let mut value = config();
    let before = proof_inbox_required_bytes(&value);
    value.max_manifest_bytes += 1;
    assert!(value.valid());
    assert_eq!(proof_inbox_required_bytes(&value), before + 4);
}

#[test]
fn profile_increment_charges_once_per_position() {
    let mut value = config();
    let before = proof_inbox_required_bytes(&value);
    value.max_profile_bytes += 1;
    assert!(value.valid());
    assert_eq!(proof_inbox_required_bytes(&value), before + 2);
}

#[test]
fn per_position_fixed_reserve_is_charged() {
    let mut value = small();
    value.max_operations_per_session = 0;
    value.max_options_bytes = 0;
    value.max_profile_bytes = 0;
    value.max_manifest_bytes = 0;
    assert_eq!(proof_inbox_required_bytes(&value), 40_960);
}

#[test]
fn maximum_typed_domain_no_overflow() {
    let mut value = config();
    value.max_sessions = u16::MAX;
    value.max_operations_per_session = u16::MAX;
    value.max_options_bytes = u32::MAX;
    value.max_profile_bytes = u32::MAX;
    value.max_manifest_bytes = u32::MAX;
    assert_eq!(proof_inbox_required_bytes(&value), 2_005_480_217_559_047);
}

#[test]
fn non_capacity_fields_are_ignored() {
    let mut value = config();
    let before = proof_inbox_required_bytes(&value);
    value.center.deployment = "another-deployment".try_into().unwrap();
    value.center.center = "another-center".try_into().unwrap();
    value.node.node_id = "another-node".try_into().unwrap();
    value.node.node_generation = 9.try_into().unwrap();
    value.host_epoch = None;
    assert_eq!(proof_inbox_required_bytes(&value), before);
}
