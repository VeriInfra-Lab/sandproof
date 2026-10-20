use sandproof_btree_set_binding::{
    insert_lane, insert_str, insert_u64, new_then_repeat_str, repeat_str,
};
use std::collections::BTreeSet;

#[test]
fn first_insert_reports_membership_and_writes_original_set() {
    let mut seen = BTreeSet::from(["tenant-a"]);
    assert!(
        insert_str(&mut seen, "tenant-b"),
        "BTreeSet insert flag must report a previously absent key"
    );
    assert_eq!(seen, BTreeSet::from(["tenant-a", "tenant-b"]));
    assert!(!insert_str(&mut seen, "tenant-b"));
    assert_eq!(seen, BTreeSet::from(["tenant-a", "tenant-b"]));
}

#[test]
fn separately_owned_equal_utf8_is_a_duplicate() {
    let original = String::from("租户-é");
    let equal = original.clone();
    let other = String::from("租户-β");
    let mut seen = BTreeSet::new();
    assert!(insert_str(&mut seen, &original));
    let before = seen.clone();
    assert!(!insert_str(&mut seen, &equal));
    assert_eq!(seen, before);
    assert!(insert_str(&mut seen, &other));
    assert_eq!(seen.len(), 2);
}

#[test]
fn lane_identity_includes_the_exact_u16_slot() {
    let mut seen = BTreeSet::new();
    assert!(insert_lane(&mut seen, "node-a", 1));
    assert!(!insert_lane(&mut seen, "node-a", 1));
    assert!(insert_lane(&mut seen, "node-a", 2));
    assert!(insert_lane(&mut seen, "node-b", 1));
    assert!(insert_lane(&mut seen, "node-a", u16::MAX));
    assert_eq!(seen.len(), 4);
}

#[test]
fn two_calls_share_the_written_back_original_collection() {
    let mut seen = BTreeSet::from(["tenant-a"]);
    assert_eq!(repeat_str(&mut seen, "tenant-b"), (true, false));
    assert_eq!(repeat_str(&mut seen, "tenant-b"), (false, false));
    assert_eq!(seen, BTreeSet::from(["tenant-a", "tenant-b"]));
}

#[test]
fn construction_and_repeated_insert_have_nonempty_witnesses() {
    for value in ["a", "租户-é", ""] {
        assert_eq!(new_then_repeat_str(value), (true, false));
    }
}

#[test]
fn unsigned_control_keeps_boundary_values_distinct() {
    let mut seen = BTreeSet::new();
    assert!(insert_u64(&mut seen, u64::MAX));
    assert!(insert_u64(&mut seen, 0));
    assert!(!insert_u64(&mut seen, u64::MAX));
    assert_eq!(seen, BTreeSet::from([0, u64::MAX]));
}
