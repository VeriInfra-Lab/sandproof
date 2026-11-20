//! The same complete configuration assertions for original/candidate/wrong Rust.
use super::*;
use crate::controller::lifecycle_inbox::model::{
    proof_inbox_config_valid, proof_inbox_required_bytes,
};

fn small() -> LifecycleInboxConfig {
    let mut value = config();
    value.host_epoch = None;
    value.max_sessions = 1;
    value.max_options_bytes = 1;
    value.max_profile_bytes = 1;
    value.max_manifest_bytes = 1;
    value
}

fn limit() -> LifecycleInboxConfig {
    let mut value = small();
    value.max_sessions = 128;
    value.max_profile_bytes = 32_698;
    value
}

#[test]
fn normal_none() {
    assert!(proof_inbox_config_valid(&small()));
}

#[test]
fn normal_host() {
    let mut value = config();
    value.host_epoch.as_mut().unwrap().boot_id = "!~".into();
    assert!(proof_inbox_config_valid(&value));
}

#[test]
fn deployment_mismatch_rejected() {
    let mut value = small();
    value.node.deployment_id = "different".try_into().unwrap();
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn host_version_zero_rejected() {
    let mut value = config();
    value.host_epoch.as_mut().unwrap().version = 0;
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn host_version_two_rejected() {
    let mut value = config();
    value.host_epoch.as_mut().unwrap().version = 2;
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn host_empty_rejected() {
    let mut value = config();
    value.host_epoch.as_mut().unwrap().boot_id.clear();
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn host_ascii_rejected() {
    for boot in [" ", "\t", "\x7f", "é", "a b"] {
        let mut value = config();
        value.host_epoch.as_mut().unwrap().boot_id = boot.into();
        assert!(!proof_inbox_config_valid(&value), "{boot:?}");
    }
}

#[test]
fn host_length_boundary() {
    let mut value = config();
    value.host_epoch.as_mut().unwrap().boot_id = "a".repeat(128);
    assert!(proof_inbox_config_valid(&value));
    value.host_epoch.as_mut().unwrap().boot_id.push('a');
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn policy_is_not_checked() {
    let mut value = config();
    for byte in [0, 255] {
        value.host_epoch.as_mut().unwrap().policy =
            sandproof_protocol::lifecycle::ContentDigest::from_sha256([byte; 32]);
        assert!(proof_inbox_config_valid(&value));
    }
}

#[test]
fn sessions_zero_rejected() {
    let mut value = small();
    value.max_sessions = 0;
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn sessions_upper_rejected() {
    let mut value = small();
    value.max_sessions = 4097;
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn operations_boundaries() {
    let mut value = small();
    value.max_operations_per_session = 7;
    assert!(!proof_inbox_config_valid(&value));
    value.max_operations_per_session = 128;
    assert!(proof_inbox_config_valid(&value));
    value.max_operations_per_session = 129;
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn options_boundaries() {
    let mut value = small();
    value.max_options_bytes = 0;
    assert!(!proof_inbox_config_valid(&value));
    value.max_options_bytes = 65_536;
    assert!(proof_inbox_config_valid(&value));
    value.max_options_bytes = 65_537;
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn profile_zero_rejected() {
    let mut value = small();
    value.max_profile_bytes = 0;
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn profile_upper_rejected() {
    let mut value = small();
    value.max_profile_bytes = 65_536;
    assert!(proof_inbox_config_valid(&value));
    value.max_profile_bytes = 65_537;
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn manifest_boundaries() {
    let mut value = small();
    value.max_manifest_bytes = 0;
    assert!(!proof_inbox_config_valid(&value));
    value.max_manifest_bytes = 262_144;
    assert!(proof_inbox_config_valid(&value));
    value.max_manifest_bytes = 262_145;
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn exact_budget_accepted() {
    let value = limit();
    assert_eq!(value.required_bytes(), 16 * 1024 * 1024);
    assert!(proof_inbox_config_valid(&value));
}

#[test]
fn over_budget_rejected() {
    let mut value = limit();
    value.max_profile_bytes += 1;
    assert_eq!(value.required_bytes(), 16 * 1024 * 1024 + 128);
    assert!(!proof_inbox_config_valid(&value));
}

#[test]
fn other_identity_fields_are_not_checked() {
    let mut value = small();
    value.center.center = "another-center".try_into().unwrap();
    value.node.node_id = "another-node".try_into().unwrap();
    value.node.node_generation = 99.try_into().unwrap();
    assert!(proof_inbox_config_valid(&value));
}

#[test]
fn capacity_forwarder_is_original() {
    assert_eq!(proof_inbox_required_bytes(&small()), 106_503);
    assert_eq!(proof_inbox_required_bytes(&limit()), 16 * 1024 * 1024);
}
