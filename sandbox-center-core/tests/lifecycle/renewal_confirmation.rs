//! Controlled observations exercise the real pure authority; no Host/Redis claim.
use super::*;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, ConfirmedRenewal, CurrentCreateReadiness,
    TerminalLifecycleObservation, TerminalStatus,
};

fn running(capacity: u16) -> (CenterState, AcceptedLifecycleIntent) {
    let mut config = serde_json::to_value(state()).unwrap();
    config["bounds"]["operations_per_sandbox"] = capacity.into();
    let initial: CenterState = serde_json::from_value(config).unwrap();
    let mut input = create("1", 1);
    let LifecycleSubmission::Create { parameters, .. } = &mut input else {
        unreachable!()
    };
    parameters.max_lifetime_ms = 20_000;
    let (creating, original) = initial
        .accept_lifecycle(&"1".try_into().unwrap(), &input, 10)
        .unwrap();
    let mut view = observed(&original);
    let CurrentCreateReadiness::Observed { facts } = &mut view.readiness else {
        unreachable!()
    };
    facts.max_end_at_ms = 20_100;
    (
        creating.accept_lifecycle_create_observation(&view).unwrap(),
        original,
    )
}

fn renew(
    state: &CenterState,
    original: &AcceptedLifecycleIntent,
    at_ms: u64,
    seconds: u64,
    extend_only: bool,
    sdk: bool,
) -> (CenterState, AcceptedLifecycleIntent) {
    let binding = &original.request.binding;
    let key = if sdk {
        state.next_sdk_operation_key(&binding.key.owner).unwrap()
    } else {
        let mut key = binding.key.clone();
        key.sequence = 99.try_into().unwrap();
        key
    };
    state
        .accept_lifecycle(
            &binding.key.owner,
            &LifecycleSubmission::Renew {
                key,
                session: binding.session.clone(),
                expected_version: state.sandboxes()[0].current_version().unwrap(),
                timeout_secs: seconds,
                extend_only,
            },
            at_ms,
        )
        .unwrap()
}

fn apply(
    state: &CenterState,
    original: &AcceptedLifecycleIntent,
    renewal: &AcceptedLifecycleIntent,
    end_at_ms: u64,
    observed_at_ms: u64,
) -> CenterState {
    let mut view = observed(original);
    view.current_request = renewal.clone();
    let CurrentCreateReadiness::Observed { facts } = &mut view.readiness else {
        unreachable!()
    };
    facts.max_end_at_ms = 20_100;
    facts.end_at_ms = end_at_ms;
    facts.observed_at_ms = observed_at_ms;
    state.accept_lifecycle_create_observation(&view).unwrap()
}

fn ack(state: &CenterState, renewal: &AcceptedLifecycleIntent) -> ConfirmedRenewal {
    ConfirmedRenewal {
        result: state.lifecycle_renewal_confirmation(renewal).unwrap(),
    }
}

fn destroy(
    state: &CenterState,
    original: &AcceptedLifecycleIntent,
) -> (CenterState, AcceptedLifecycleIntent) {
    let binding = &original.request.binding;
    state
        .accept_lifecycle(
            &binding.key.owner,
            &LifecycleSubmission::Destroy {
                key: state.next_sdk_operation_key(&binding.key.owner).unwrap(),
                session: binding.session.clone(),
                expected_version: state.sandboxes()[0].current_version().unwrap(),
            },
            0,
        )
        .unwrap()
}

#[test]
fn sdk_history_reuses_four_slots_through_version_twenty_five_and_keeps_destroy_reserved() {
    let (mut state, original) = running(4);
    let reservation = state.reserved(None, None);
    for version in 2..=25 {
        let (pending, renewal) = renew(&state, &original, 150, 1, false, true);
        assert_eq!(renewal.accepted_version, version);
        assert!(pending.sandboxes()[0].operations().len() <= 3);
        let applied = apply(&pending, &original, &renewal, 1150, 150);
        let confirmed = ack(&applied, &renewal);
        state = applied.confirm_lifecycle_renewal(&confirmed).unwrap();
        assert_eq!(state.sandboxes()[0].operations().len(), 2);
        assert_eq!(state.sandboxes()[0].operations()[0].intent(), &original);
        assert_eq!(
            state.sandboxes()[0].confirmed_renewal.as_ref(),
            Some(&confirmed.result)
        );
        assert_eq!(state.reserved(None, None), reservation);
        assert_eq!(state.identity_frontiers(), (1, version));
        assert!(state.pending_lifecycle().is_empty());
        let encoded = serde_json::to_vec(&state).unwrap();
        assert!(encoded.len() <= state.required_authority_bytes());
        let restored: CenterState = serde_json::from_slice(&encoded).unwrap();
        assert!(restored.is_valid());
        assert_eq!(restored, state);
    }
    let (terminating, stop) = destroy(&state, &original);
    assert_eq!(stop.accepted_version, 26);
    assert_eq!(terminating.reserved(None, None), reservation);
    assert_eq!(terminating.sandboxes()[0].operations().len(), 3);
}

#[test]
fn a_public_operation_cannot_hide_an_unconfirmed_sdk_and_is_never_pruned() {
    let (state, original) = running(8);
    let (pending, sdk) = renew(&state, &original, 150, 1, false, true);
    let applied = apply(&pending, &original, &sdk, 1150, 150);
    let first_ack = ack(&applied, &sdk);
    let (pending, public) = renew(&applied, &original, 160, 1, false, false);
    let applied = apply(&pending, &original, &public, 1160, 160);
    assert_eq!(applied.sandboxes()[0].unconfirmed_sdk_renewal(), Some(&sdk));
    let input = LifecycleSubmission::Renew {
        key: applied
            .next_sdk_operation_key(&original.request.binding.key.owner)
            .unwrap(),
        session: original.request.binding.session.clone(),
        expected_version: public.accepted_version,
        timeout_secs: 1,
        extend_only: false,
    };
    assert_eq!(
        applied.accept_lifecycle(&original.request.binding.key.owner, &input, 170),
        Err(DecisionError::Unavailable)
    );
    let confirmed = applied.confirm_lifecycle_renewal(&first_ack).unwrap();
    assert_eq!(
        confirmed.sandboxes()[0]
            .current_running()
            .unwrap()
            .end_at_ms,
        1160
    );
    let public_record = serde_json::to_value(&confirmed.sandboxes()[0].operations()[2]).unwrap();
    let (pending, next_sdk) = renew(&confirmed, &original, 170, 1, false, true);
    let applied = apply(&pending, &original, &next_sdk, 1170, 170);
    let pruned = applied
        .confirm_lifecycle_renewal(&ack(&applied, &next_sdk))
        .unwrap();
    assert_eq!(pruned.sandboxes()[0].operations().len(), 3);
    assert_eq!(
        serde_json::to_value(&pruned.sandboxes()[0].operations()[1]).unwrap(),
        public_record
    );
    assert_eq!(pruned.sandboxes()[0].operations()[1].intent(), &public);
}

#[test]
fn an_unknown_new_result_and_late_ack_keep_the_pending_request_and_destroy_path() {
    let (state, original) = running(4);
    let (pending, first) = renew(&state, &original, 150, 1, false, true);
    let applied = apply(&pending, &original, &first, 1150, 150);
    let first_ack = ack(&applied, &first);
    let confirmed = applied.confirm_lifecycle_renewal(&first_ack).unwrap();
    let (pending, second) = renew(&confirmed, &original, 160, 1, false, true);
    assert_eq!(
        pending.lifecycle_renewal_confirmation(&second),
        Err(DecisionError::Unavailable)
    );
    assert_eq!(
        pending.confirm_lifecycle_renewal(&first_ack),
        Ok(pending.clone())
    );
    assert_eq!(pending.pending_lifecycle(), vec![second]);
    assert_eq!(pending.sandboxes()[0].operations().len(), 3);
    let (terminating, _) = destroy(&pending, &original);
    assert!(terminating.sandboxes()[0].terminating());
    assert_eq!(terminating.sandboxes()[0].operations().len(), 4);
}

#[test]
fn extend_only_after_compaction_uses_the_actual_predecessor_not_create_end() {
    let (state, original) = running(4);
    let (pending, first) = renew(&state, &original, 150, 6, false, true);
    let applied = apply(&pending, &original, &first, 6150, 150);
    let old_ack = ack(&applied, &first);
    let confirmed = applied.confirm_lifecycle_renewal(&old_ack).unwrap();
    let (pending, second) = renew(&confirmed, &original, 160, 1, true, true);
    let applied = apply(&pending, &original, &second, 6150, 160);
    let pruned = applied
        .confirm_lifecycle_renewal(&ack(&applied, &second))
        .unwrap();
    assert_eq!(pruned.sandboxes()[0].operations().len(), 2);
    assert_eq!(
        pruned.sandboxes()[0].current_running().unwrap().end_at_ms,
        6150
    );
    assert_eq!(
        pruned.confirm_lifecycle_renewal(&old_ack),
        Err(DecisionError::Stale)
    );
    let mut wrong = serde_json::to_value(&pruned).unwrap();
    wrong["sandboxes"][0]["operations"][1]["previous_effective_end_at_ms"] = 200.into();
    let wrong: CenterState = serde_json::from_value(wrong).unwrap();
    assert!(
        !wrong.is_valid(),
        "Create.end is not the deleted SDK's actual effective end"
    );
}

#[test]
fn removed_sample_max_survives_clock_rollback_and_blocks_early_retention() {
    let (state, original) = running(4);
    let (pending, first) = renew(&state, &original, 150, 6, false, true);
    let applied = apply(&pending, &original, &first, 6150, 150);
    let confirmed = applied
        .confirm_lifecycle_renewal(&ack(&applied, &first))
        .unwrap();
    let (pending, second) = renew(&confirmed, &original, 110, 1, false, true);
    let applied = apply(&pending, &original, &second, 1110, 110);
    let pruned = applied
        .confirm_lifecycle_renewal(&ack(&applied, &second))
        .unwrap();
    assert_eq!(
        pruned.sandboxes()[0].compacted_sampled_acceptance_max_ms,
        Some(150)
    );
    let (terminating, stop) = destroy(&pruned, &original);
    let result = TerminalLifecycleObservation {
        request: stop,
        status: TerminalStatus::Cleaned,
    };
    let completed = terminating
        .accept_lifecycle_terminal_observation(&result, 120)
        .unwrap();
    assert!(!completed.sandboxes()[0].result_retention_elapsed(u64::MAX));
    let restored: CenterState =
        serde_json::from_slice(&serde_json::to_vec(&completed).unwrap()).unwrap();
    let recovered = restored
        .accept_lifecycle_terminal_observation(&result, 160)
        .unwrap();
    assert!(!recovered.sandboxes()[0].result_retention_elapsed(259));
    assert!(recovered.sandboxes()[0].result_retention_elapsed(260));
}

#[test]
fn zero_is_a_real_removed_sample_and_new_optional_fields_are_not_missing_defaults() {
    let (state, original) = running(4);
    let (pending, first) = renew(&state, &original, 0, 1, false, true);
    let applied = apply(&pending, &original, &first, 1000, 100);
    let confirmed = applied
        .confirm_lifecycle_renewal(&ack(&applied, &first))
        .unwrap();
    let (pending, second) = renew(&confirmed, &original, 110, 1, false, true);
    let applied = apply(&pending, &original, &second, 1110, 110);
    let pruned = applied
        .confirm_lifecycle_renewal(&ack(&applied, &second))
        .unwrap();
    assert_eq!(
        pruned.sandboxes()[0].compacted_sampled_acceptance_max_ms,
        Some(0)
    );
    for field in [
        "confirmed_renewal",
        "compacted_sdk_through",
        "compacted_sampled_acceptance_max_ms",
    ] {
        let mut missing = serde_json::to_value(&pruned).unwrap();
        missing["sandboxes"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            serde_json::from_value::<CenterState>(missing).is_err(),
            "missing {field} is not a current-format record"
        );
    }
    let mut missing = serde_json::to_value(&pruned).unwrap();
    missing["sandboxes"][0]["operations"][1]
        .as_object_mut()
        .unwrap()
        .remove("previous_effective_end_at_ms");
    assert!(serde_json::from_value::<CenterState>(missing).is_err());
}
