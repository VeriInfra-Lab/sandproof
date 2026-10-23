//! Controlled host facts exercise the original center decisions. They are not
//! physical no-admission evidence or a substitute for the Journal provider.
use super::*;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, CreateObservation, CreateRefusalReason, CurrentCreateReadiness,
    TerminalLifecycleObservation, TerminalStatus,
};

fn refused(original: &AcceptedLifecycleIntent) -> CreateObservation {
    CreateObservation {
        request: original.clone(),
        current_request: original.clone(),
        historical_create_succeeded: false,
        readiness: CurrentCreateReadiness::Refused { reason: CreateRefusalReason::Capacity },
    }
}

#[test]
fn exact_refusal_and_destroy_are_saved_together_without_early_refund() {
    let owner = "1".try_into().unwrap();
    let input = create("1", 1);
    let (creating, original) = state().accept_lifecycle(&owner, &input, 10).unwrap();
    let (terminating, destroy) = creating
        .accept_lifecycle_create_refusal(&refused(&original), 20).unwrap();
    assert_eq!(terminating.identity_frontiers(), (1, 2));
    assert_eq!(terminating.reserved(None, None), Some(resources(1)));
    let record = &terminating.sandboxes()[0];
    assert_eq!(record.phase(), SessionPhase::Terminating);
    assert_eq!(record.operations().len(), 2);
    assert_eq!(record.operations()[0].intent(), &original);
    assert_eq!(record.operations()[0].create_refusal(), Some(CreateRefusalReason::Capacity));
    assert_eq!(record.operations()[1].intent(), &destroy);
    assert_eq!(record.operations()[1].create_refusal(), None);
    assert_eq!(serde_json::to_value(&terminating).unwrap()["sandboxes"][0]["operations"][1]["accepted_at_ms"], 20);
    assert_eq!(terminating.pending_lifecycle(), vec![destroy.clone()]);

    let reopened: CenterState = serde_json::from_slice(&serde_json::to_vec(&terminating).unwrap()).unwrap();
    assert!(reopened.is_valid());
    assert_eq!(reopened, terminating);
    assert_eq!(reopened.accept_lifecycle(&owner, &input, u64::MAX), Ok((reopened.clone(), original)));
    let pending = TerminalLifecycleObservation { request: destroy.clone(), status: TerminalStatus::Pending };
    assert_eq!(reopened.accept_lifecycle_terminal_observation(&pending, 30), Ok(reopened.clone()));
    let completed = reopened.accept_lifecycle_terminal_observation(
        &TerminalLifecycleObservation { request: destroy, status: TerminalStatus::Cleaned }, 30,
    ).unwrap();
    assert_eq!(completed.reserved(None, None), Some(resources(0)));
    assert_eq!(completed.sandboxes()[0].phase(), SessionPhase::Cleaned);
    assert_eq!(completed.sandboxes()[0].operations()[0].create_refusal(), Some(CreateRefusalReason::Capacity));
}

#[test]
fn repeated_refusal_reuses_the_current_destroy_and_original_timestamp() {
    let owner = "1".try_into().unwrap();
    let (creating, original) = state().accept_lifecycle(&owner, &create("1", 1), 10).unwrap();
    let (terminating, destroy) = creating.accept_lifecycle_create_refusal(&refused(&original), 20).unwrap();
    let mut replay = refused(&original);
    // An observation taken before the concurrent Destroy is stale, even though
    // its original Create and refusal reason remain the same.
    assert_eq!(terminating.accept_lifecycle_create_refusal(&replay, 40), Err(DecisionError::Stale));
    replay.current_request = destroy.clone();
    assert_eq!(terminating.accept_lifecycle_create_refusal(&replay, u64::MAX), Ok((terminating.clone(), destroy)));
    assert_eq!(terminating.identity_frontiers(), (1, 2));
}

#[test]
fn exact_existing_destroy_accepts_the_cause_without_allocating_again() {
    let owner = "1".try_into().unwrap();
    let (creating, original) = state().accept_lifecycle(&owner, &create("1", 1), 10).unwrap();
    let (terminating, destroy) = creating.accept_lifecycle(&owner, &followup(&original, 2, true), 15).unwrap();
    let mut observation = refused(&original);
    observation.current_request = destroy.clone();
    let (saved, reused) = terminating.accept_lifecycle_create_refusal(&observation, 20).unwrap();
    assert_eq!(reused, destroy);
    assert_eq!(saved.identity_frontiers(), terminating.identity_frontiers());
    assert_eq!(saved.reserved(None, None), terminating.reserved(None, None));
    assert_eq!(serde_json::to_value(saved).unwrap()["sandboxes"][0]["operations"][1]["accepted_at_ms"], 15);
}

#[test]
fn historical_running_cannot_be_relabelled_as_pre_admission_refusal() {
    let owner = "1".try_into().unwrap();
    let (creating, original) = state().accept_lifecycle(&owner, &create("1", 1), 10).unwrap();
    let running = creating.accept_lifecycle_create_observation(&observed(&original)).unwrap();
    assert_eq!(running.accept_lifecycle_create_refusal(&refused(&original), 110), Err(DecisionError::Conflict));
    assert_eq!(running.sandboxes()[0].operations()[0].create_refusal(), None);
}

#[test]
fn pending_and_changed_original_are_not_definite_capacity_refusals() {
    let owner = "1".try_into().unwrap();
    let (creating, original) = state().accept_lifecycle(&owner, &create("1", 1), 10).unwrap();
    for readiness in [CurrentCreateReadiness::Pending, CurrentCreateReadiness::NotRunning, CurrentCreateReadiness::NotCurrent] {
        let mut observation = refused(&original);
        observation.readiness = readiness;
        assert_eq!(creating.accept_lifecycle_create_refusal(&observation, 20), Err(DecisionError::Invalid));
    }
    let mut changed = refused(&original);
    let LifecycleOperation::Create { create, .. } = &mut changed.request.request.operation else { unreachable!() };
    create.options.push(b' ');
    assert_eq!(creating.accept_lifecycle_create_refusal(&changed, 20), Err(DecisionError::Stale));
    let mut historical = refused(&original);
    historical.historical_create_succeeded = true;
    assert_eq!(creating.accept_lifecycle_create_refusal(&historical, 20), Err(DecisionError::Invalid));
    assert_eq!(creating.reserved(None, None), Some(resources(1)));
    assert_eq!(creating.identity_frontiers(), (1, 1));
}

#[test]
fn refusal_can_use_the_original_last_reserved_termination_number() {
    let mut value = serde_json::to_value(state()).unwrap();
    value["operation_highwater"] = (u64::MAX - 2).into();
    let initial: CenterState = serde_json::from_value(value).unwrap();
    assert!(initial.is_valid());
    let owner = "1".try_into().unwrap();
    let (creating, original) = initial.accept_lifecycle(&owner, &create("1", 1), 10).unwrap();
    assert_eq!(creating.identity_frontiers(), (1, u64::MAX - 1));
    let (terminating, destroy) = creating.accept_lifecycle_create_refusal(&refused(&original), 20).unwrap();
    assert_eq!(terminating.identity_frontiers(), (1, u64::MAX));
    assert_eq!(destroy.request.binding.key.sequence.get(), u64::MAX);
    assert!(terminating.is_valid());
    assert_eq!(terminating.reserved(None, None), Some(resources(1)));
    let mut replay = refused(&original);
    replay.current_request = destroy.clone();
    assert_eq!(terminating.accept_lifecycle_create_refusal(&replay, 30), Ok((terminating.clone(), destroy)));
}
