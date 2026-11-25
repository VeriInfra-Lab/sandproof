use sandproof_controller_service::application::branching::{
    BranchTarget, PublicBranchBatchRequest, PublicBranchBatchResult, valid_ready_targets,
};
use sandproof_protocol::SandboxId;
use serde_json::json;
use std::num::NonZeroU16;

#[test]
fn batch_count_requires_a_positive_integer_and_no_unrecognized_input() {
    let request = |count| {
        json!({
            "window": 1, "sequence": 2, "expected_version": 3,
            "checkpoint": "checkpoint-001", "count": count,
        })
    };
    let valid = serde_json::from_value::<PublicBranchBatchRequest>(request(json!(4))).unwrap();
    assert_eq!(valid.count.get(), 4);
    for invalid in [json!(0), json!(-1), json!(1.5), json!("4"), json!(65536)] {
        assert!(serde_json::from_value::<PublicBranchBatchRequest>(request(invalid)).is_err());
    }
    let mut invalid = request(json!(4));
    invalid["owner"] = json!("somebody-else");
    assert!(serde_json::from_value::<PublicBranchBatchRequest>(invalid).is_err());
}

#[test]
fn ready_targets_must_match_the_entire_ordered_host_plan() {
    let count = NonZeroU16::new(2).unwrap();
    let a = BranchTarget {
        sandbox: SandboxId::try_from("branch-a").unwrap(),
        port: NonZeroU16::new(7000).unwrap(),
    };
    let b = BranchTarget {
        sandbox: SandboxId::try_from("branch-b").unwrap(),
        port: NonZeroU16::new(7001).unwrap(),
    };
    let plan = [a.clone(), b.clone()];
    assert!(valid_ready_targets(&plan, &plan, count));
    assert!(!valid_ready_targets(&plan, &[b.clone(), a.clone()], count));
    assert!(!valid_ready_targets(&plan, &plan[..1], count));
    let duplicate_id = [
        a.clone(),
        BranchTarget {
            port: b.port,
            ..a.clone()
        },
    ];
    assert!(!valid_ready_targets(&duplicate_id, &duplicate_id, count));
    let duplicate_port = [a.clone(), BranchTarget { port: a.port, ..b }];
    assert!(!valid_ready_targets(
        &duplicate_port,
        &duplicate_port,
        count
    ));
}

#[test]
fn incomplete_batch_states_have_no_branch_handles() {
    for state in ["pending", "cleaning_up", "retired"] {
        assert!(
            serde_json::from_value::<PublicBranchBatchResult>(json!({
                "state": state, "branches": ["branch-a"]
            }))
            .is_err()
        );
    }
    assert!(
        serde_json::from_value::<PublicBranchBatchResult>(json!({
            "state": "failed", "reason": "guest_setup_failed", "branches": ["branch-a"]
        }))
        .is_err()
    );
}
