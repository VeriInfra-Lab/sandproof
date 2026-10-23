//! Original record representation and recovery checks, with controlled times.
//! These tests do not claim a real clock or public adapter has been verified.
use super::*;
use sandproof_center_core::AcceptanceTime;
use sandproof_protocol::lifecycle::{
    ConfirmedLifecycleResult, TerminalLifecycleObservation, TerminalStatus,
};

fn renewal() -> (
    CenterState,
    sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
) {
    let owner = "1".try_into().unwrap();
    let (created, original) = state()
        .accept_lifecycle(&owner, &create("1", 1), 10)
        .unwrap();
    let running = created
        .accept_lifecycle_create_observation(&observed(&original))
        .unwrap();
    let mut key = original.request.binding.key.clone();
    key.sequence = 2.try_into().unwrap();
    running
        .accept_lifecycle(
            &owner,
            &LifecycleSubmission::Renew {
                key,
                session: original.request.binding.session.clone(),
                expected_version: 1,
                timeout_secs: 1,
                extend_only: false,
            },
            150,
        )
        .unwrap()
}

#[test]
fn timed_create_and_renew_require_their_exact_sample_on_recovery() {
    let owner = "1".try_into().unwrap();
    let (created, _) = state()
        .accept_lifecycle(&owner, &create("1", 1), 10)
        .unwrap();
    let (renewed, _) = renewal();
    for original in [created, renewed] {
        let index = original.sandboxes()[0].operations().len() - 1;
        for time in [
            AcceptanceTime::Unsampled,
            AcceptanceTime::Sampled { at_ms: 999 },
        ] {
            let mut value = serde_json::to_value(&original).unwrap();
            value["sandboxes"][0]["operations"][index]["acceptance_time"] =
                serde_json::to_value(time).unwrap();
            let wrong: CenterState = serde_json::from_value(value).unwrap();
            assert!(!wrong.is_valid());
        }
        let mut missing = serde_json::to_value(&original).unwrap();
        missing["sandboxes"][0]["operations"][index]
            .as_object_mut()
            .unwrap()
            .remove("acceptance_time");
        assert!(serde_json::from_value::<CenterState>(missing).is_err());
    }
}

#[test]
fn destroy_is_explicitly_unsampled_for_every_supplied_clock_value() {
    let owner = "1".try_into().unwrap();
    let (creating, original) = state()
        .accept_lifecycle(&owner, &create("1", 1), 10)
        .unwrap();
    let stop = followup(&original, 2, true);
    let expected = creating.accept_lifecycle(&owner, &stop, 0).unwrap();
    for time in [0, 10, 1000, u64::MAX] {
        assert_eq!(
            creating.accept_lifecycle(&owner, &stop, time),
            Ok(expected.clone())
        );
        assert_eq!(
            expected.0.sandboxes()[0].operations()[1].acceptance_time(),
            AcceptanceTime::Unsampled
        );
        assert_eq!(
            expected.0.accept_lifecycle(&owner, &stop, time),
            Ok(expected.clone())
        );
    }
    let mut wrong = serde_json::to_value(expected.0).unwrap();
    wrong["sandboxes"][0]["operations"][1]["acceptance_time"] =
        serde_json::to_value(AcceptanceTime::Sampled { at_ms: 0 }).unwrap();
    assert!(
        !serde_json::from_value::<CenterState>(wrong)
            .unwrap()
            .is_valid()
    );
}

#[test]
fn all_previous_samples_still_bound_retention_after_unsampled_destroy() {
    let owner = "1".try_into().unwrap();
    let (pending, renewed) = renewal();
    let stop = LifecycleSubmission::Destroy {
        key: pending.next_sdk_operation_key(&owner).unwrap(),
        session: renewed.request.binding.session.clone(),
        expected_version: renewed.accepted_version,
    };
    let (terminating, destroy) = pending.accept_lifecycle(&owner, &stop, 0).unwrap();
    let result = TerminalLifecycleObservation {
        request: destroy,
        status: TerminalStatus::Cleaned,
    };
    let completed = terminating
        .accept_lifecycle_terminal_observation(&result, 140)
        .unwrap();
    assert!(completed.is_valid());
    assert_eq!(completed.sandboxes()[0].phase(), SessionPhase::Cleaned);
    assert!(!completed.sandboxes()[0].result_retention_elapsed(u64::MAX));
    let reopened: CenterState =
        serde_json::from_slice(&serde_json::to_vec(&completed).unwrap()).unwrap();
    let recovered = reopened
        .accept_lifecycle_terminal_observation(&result, 160)
        .unwrap();
    assert_eq!(
        recovered.accept_lifecycle_terminal_observation(&result, 200),
        Ok(recovered.clone())
    );
    let confirmed = recovered
        .confirm_terminal(&ConfirmedLifecycleResult {
            result: result.clone(),
        })
        .unwrap();
    assert_eq!(
        confirmed.begin_close(&result, 259),
        Err(DecisionError::Unavailable)
    );
    assert!(confirmed.begin_close(&result, 260).is_ok());
}
