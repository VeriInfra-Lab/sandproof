use super::*;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, CurrentReadyFacts, CurrentResumeReadiness, ResumeObservation,
};

fn source() -> (CenterState, AcceptedLifecycleIntent) {
    let (state, observation) = unloaded::pending();
    let state = state
        .accept_lifecycle_unload_observation(&observation, 160)
        .unwrap();
    let mut wire = serde_json::to_value(&state).unwrap();
    wire["bounds"]["operations_per_sandbox"] = 8.into();
    let state: CenterState = serde_json::from_value(wire).unwrap();
    assert!(state.is_valid());
    (state, observation.request)
}
fn input(unload: &AcceptedLifecycleIntent) -> LifecycleSubmission {
    let mut key = unload.request.binding.key.clone();
    key.sequence = 3.try_into().unwrap();
    LifecycleSubmission::Resume {
        key,
        session: unload.request.binding.session.clone(),
        expected_version: unload.accepted_version,
    }
}
fn ready(intent: &AcceptedLifecycleIntent) -> ResumeObservation {
    ResumeObservation {
        request: intent.clone(),
        current_request: intent.clone(),
        readiness: CurrentResumeReadiness::Observed {
            facts: CurrentReadyFacts {
                started_at_ms: 100,
                end_at_ms: 200,
                max_end_at_ms: 600,
                startup_deadline_ms: 1010,
                observed_at_ms: 180,
            },
        },
    }
}

fn continued_resume() -> (
    CenterState,
    ResumeObservation,
    sandproof_protocol::lifecycle::UnloadObservation,
) {
    let (before, unload) = source();
    let owner = "1".try_into().unwrap();
    let (accepted, origin) = before
        .accept_lifecycle(&owner, &input(&unload), 170)
        .unwrap();
    let running = accepted
        .accept_lifecycle_resume_observation(&ready(&origin))
        .unwrap();
    let mut key = origin.request.binding.key.clone();
    key.sequence = 4.try_into().unwrap();
    let (pending, current) = running
        .accept_lifecycle(
            &owner,
            &LifecycleSubmission::Unload {
                key,
                session: origin.request.binding.session.clone(),
                expected_version: origin.accepted_version,
            },
            181,
        )
        .unwrap();
    let mut facts = running.sandboxes()[0].last_running().unwrap().clone();
    facts.observed_at_ms = 185;
    (
        pending,
        ResumeObservation {
            request: origin,
            current_request: current.clone(),
            readiness: CurrentResumeReadiness::StateLost,
        },
        sandproof_protocol::lifecycle::UnloadObservation {
            request: current,
            outcome: sandproof_protocol::lifecycle::UnloadOutcome::OriginalContinued { facts },
        },
    )
}

#[test]
fn observed_continued_resume_rechecks_facts_without_writing_a_running_field_into_unload() {
    let (pending, mut observation, continued) = continued_resume();
    let running = pending
        .accept_lifecycle_unload_observation(&continued, 185)
        .unwrap();
    let mut facts = continued.outcome.continued().unwrap().clone();
    facts.observed_at_ms = 186;
    observation.readiness = CurrentResumeReadiness::Observed {
        facts: facts.clone(),
    };
    assert_eq!(
        running.accept_lifecycle_resume_observation(&observation),
        Ok(running.clone())
    );
    for field in 0..4 {
        let mut changed = facts.clone();
        match field {
            0 => changed.started_at_ms += 1,
            1 => changed.end_at_ms += 1,
            2 => changed.max_end_at_ms += 1,
            _ => changed.startup_deadline_ms += 1,
        }
        observation.readiness = CurrentResumeReadiness::Observed { facts: changed };
        assert_eq!(
            running.accept_lifecycle_resume_observation(&observation),
            Err(DecisionError::Conflict)
        );
    }
}

#[test]
fn continued_resume_loss_requires_saved_continuation_and_survives_reopen_and_history_replay() {
    let (pending, loss, continued) = continued_resume();
    assert!(loss.matches_request(&loss.request));
    assert_eq!(
        pending.accept_lifecycle_resume_observation(&loss),
        Err(DecisionError::Unavailable)
    );
    let running = pending
        .accept_lifecycle_unload_observation(&continued, 185)
        .unwrap();
    let after_exit = running.accept_lifecycle_resume_observation(&loss).unwrap();
    assert_eq!(after_exit.sandboxes()[0].phase(), SessionPhase::StateLost);
    assert_eq!(after_exit.reserved(None, None), Some(resources(1)));
    assert_eq!(
        after_exit.sandboxes()[0]
            .operations()
            .last()
            .unwrap()
            .unloaded(),
        Some(&continued)
    );
    let reopened: CenterState =
        serde_json::from_slice(&serde_json::to_vec(&after_exit).unwrap()).unwrap();
    assert!(reopened.is_valid());
    assert_eq!(reopened, after_exit);
    assert_eq!(
        reopened.accept_lifecycle_unload_observation(&continued, 185),
        Ok(after_exit.clone())
    );
    let mut old_ready = ready(&loss.request);
    old_ready.current_request = loss.current_request.clone();
    assert_eq!(
        after_exit.accept_lifecycle_resume_observation(&old_ready),
        Err(DecisionError::Conflict)
    );
    let mut corrupted = serde_json::to_value(&after_exit).unwrap();
    corrupted["sandboxes"][0]["operations"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["unloaded"] = serde_json::Value::Null;
    let corrupted: CenterState = serde_json::from_value(corrupted).unwrap();
    assert!(
        !corrupted.is_valid(),
        "a lost Unload cannot invent its own continuation"
    );
}

#[test]
fn continued_resume_loss_rejects_changed_current_and_cannot_override_destroy() {
    let (pending, loss, continued) = continued_resume();
    let running = pending
        .accept_lifecycle_unload_observation(&continued, 185)
        .unwrap();
    let mut changed = loss.clone();
    changed.current_request.accepted_version += 1;
    assert_eq!(
        running.accept_lifecycle_resume_observation(&changed),
        Err(DecisionError::Stale)
    );
    let after_exit = running.accept_lifecycle_resume_observation(&loss).unwrap();
    let current = &loss.current_request;
    let mut key = current.request.binding.key.clone();
    key.sequence = 5.try_into().unwrap();
    let (terminating, _) = after_exit
        .accept_lifecycle(
            &"1".try_into().unwrap(),
            &LifecycleSubmission::Destroy {
                key,
                session: current.request.binding.session.clone(),
                expected_version: current.accepted_version,
            },
            0,
        )
        .unwrap();
    assert!(terminating.is_valid());
    assert_eq!(
        terminating.sandboxes()[0].phase(),
        SessionPhase::Terminating
    );
    assert_eq!(
        terminating.accept_lifecycle_resume_observation(&loss),
        Err(DecisionError::Stale)
    );
}

#[test]
fn resume_derives_one_candidate_and_reacquires_running_capacity_before_dispatch() {
    let (before, unload) = source();
    let input = input(&unload);
    let owner = "1".try_into().unwrap();
    let (accepted, intent) = before.accept_lifecycle(&owner, &input, 170).unwrap();
    assert_eq!(accepted.sandboxes()[0].phase(), SessionPhase::Resuming);
    assert_eq!(accepted.reserved(None, None), Some(resources(1)));
    assert_eq!(accepted.pending_lifecycle(), vec![intent.clone()]);
    let LifecycleOperation::Resume {
        source_incarnation,
        capture,
        target_incarnation,
        manifest_digest,
    } = intent.request.operation
    else {
        panic!("Resume")
    };
    assert_eq!(
        (
            source_incarnation.get(),
            capture.get(),
            target_incarnation.get()
        ),
        (1, 1, 2)
    );
    assert_eq!(
        manifest_digest,
        sandproof_protocol::lifecycle::ContentDigest::from_sha256([7; 32])
    );
    assert_eq!(
        accepted.accept_lifecycle(&owner, &input, u64::MAX),
        Ok((accepted.clone(), intent))
    );
    let mut wrong = input;
    let LifecycleSubmission::Resume {
        expected_version, ..
    } = &mut wrong
    else {
        unreachable!()
    };
    *expected_version += 1;
    assert_eq!(
        accepted.accept_lifecycle(&owner, &wrong, 170),
        Err(DecisionError::Conflict)
    );
    let bytes = serde_json::to_vec(&accepted).unwrap();
    let recovered: CenterState = serde_json::from_slice(&bytes).unwrap();
    assert!(recovered.is_valid());
    assert_eq!(recovered, accepted);
}

#[test]
fn resume_rejects_expired_source_and_busy_running_pool_without_mutation() {
    let (before, unload) = source();
    let owner = "1".try_into().unwrap();
    assert_eq!(
        before.accept_lifecycle(&owner, &input(&unload), 200),
        Err(DecisionError::Stale)
    );
    let (occupied, _) = before
        .accept_lifecycle(&"2".try_into().unwrap(), &create("2", 1), 170)
        .unwrap();
    assert_eq!(
        occupied.accept_lifecycle(&owner, &input(&unload), 170),
        Err(DecisionError::Capacity)
    );
    assert!(occupied.is_valid());
}

#[test]
fn actual_ready_keeps_original_lifetime_and_next_capture_uses_new_incarnation() {
    let (before, unload) = source();
    let owner = "1".try_into().unwrap();
    let (accepted, intent) = before
        .accept_lifecycle(&owner, &input(&unload), 170)
        .unwrap();
    let mut observation = ready(&intent);
    // The initial fixture's exact original deadline is derived from Create.
    let source = before.sandboxes()[0]
        .operations()
        .last()
        .unwrap()
        .unloaded()
        .unwrap()
        .outcome
        .unloaded()
        .unwrap();
    let CurrentResumeReadiness::Observed { facts } = &mut observation.readiness else {
        unreachable!()
    };
    facts.startup_deadline_ms = source.startup_deadline_ms;
    let running = accepted
        .accept_lifecycle_resume_observation(&observation)
        .unwrap();
    assert_eq!(running.sandboxes()[0].phase(), SessionPhase::Running);
    assert_eq!(
        running.sandboxes()[0].last_running().unwrap().end_at_ms,
        source.end_at_ms
    );
    let mut key = intent.request.binding.key.clone();
    key.sequence = 4.try_into().unwrap();
    let next = LifecycleSubmission::Unload {
        key,
        session: intent.request.binding.session.clone(),
        expected_version: 3,
    };
    let (_, capture) = running.accept_lifecycle(&owner, &next, 185).unwrap();
    assert!(
        matches!(capture.request.operation, LifecycleOperation::Unload { source_incarnation, capture } if source_incarnation.get() == 2 && capture.get() == 2)
    );
    let CurrentResumeReadiness::Observed { facts } = &mut observation.readiness else {
        unreachable!()
    };
    facts.end_at_ms += 1;
    assert_eq!(
        accepted.accept_lifecycle_resume_observation(&observation),
        Err(DecisionError::Conflict)
    );
}

#[test]
fn confirmed_loss_is_irreversible_and_old_ready_cannot_resurrect_destroy() {
    let (before, unload) = source();
    let owner = "1".try_into().unwrap();
    let (accepted, intent) = before
        .accept_lifecycle(&owner, &input(&unload), 170)
        .unwrap();
    let mut observation = ready(&intent);
    observation.readiness = CurrentResumeReadiness::Pending;
    assert_eq!(
        accepted.accept_lifecycle_resume_observation(&observation),
        Err(DecisionError::Unavailable)
    );
    observation.readiness = CurrentResumeReadiness::StateLost;
    let lost = accepted
        .accept_lifecycle_resume_observation(&observation)
        .unwrap();
    assert_eq!(lost.sandboxes()[0].phase(), SessionPhase::StateLost);
    assert_eq!(lost.reserved(None, None), Some(resources(1)));
    assert!(
        lost.accept_lifecycle_resume_observation(&ready(&intent))
            .is_err()
    );
    let mut destroy = followup(&intent, 4, true);
    let LifecycleSubmission::Destroy {
        expected_version, ..
    } = &mut destroy
    else {
        unreachable!()
    };
    *expected_version = intent.accepted_version;
    let (terminating, _) = lost.accept_lifecycle(&owner, &destroy, 0).unwrap();
    assert_eq!(
        terminating.sandboxes()[0].phase(),
        SessionPhase::Terminating
    );
    assert_eq!(
        terminating.accept_lifecycle_resume_observation(&observation),
        Err(DecisionError::Stale)
    );
}

#[test]
fn renewal_observes_the_resumed_origin_after_the_old_deadline() {
    let (before, unload) = source();
    let owner = "1".try_into().unwrap();
    let (accepted, intent) = before
        .accept_lifecycle(&owner, &input(&unload), 170)
        .unwrap();
    let source = before.sandboxes()[0]
        .operations()
        .last()
        .unwrap()
        .unloaded()
        .unwrap()
        .outcome
        .unloaded()
        .unwrap();
    let mut observation = ready(&intent);
    let CurrentResumeReadiness::Observed { facts } = &mut observation.readiness else {
        unreachable!()
    };
    facts.startup_deadline_ms = source.startup_deadline_ms;
    let running = accepted
        .accept_lifecycle_resume_observation(&observation)
        .unwrap();
    let mut key = intent.request.binding.key.clone();
    key.sequence = 4.try_into().unwrap();
    let input = LifecycleSubmission::Renew {
        key,
        session: intent.request.binding.session.clone(),
        expected_version: intent.accepted_version,
        timeout_secs: 1,
        extend_only: false,
    };
    let (renewing, renewal) = running.accept_lifecycle(&owner, &input, 185).unwrap();
    assert_eq!(
        renewing.sandboxes()[0].current_execution_origin(),
        Some(&intent)
    );
    observation.current_request = renewal;
    let CurrentResumeReadiness::Observed { facts } = &mut observation.readiness else {
        unreachable!()
    };
    facts.end_at_ms = source.max_end_at_ms;
    facts.observed_at_ms = source.end_at_ms + 1;
    let renewed = renewing
        .accept_lifecycle_resume_observation(&observation)
        .unwrap();
    assert!(renewed.is_valid());
    assert_eq!(renewed.sandboxes()[0].phase(), SessionPhase::Running);
    assert_eq!(
        renewed.sandboxes()[0].last_running().unwrap().started_at_ms,
        source.started_at_ms
    );
    assert_eq!(
        renewed.sandboxes()[0].last_running().unwrap().max_end_at_ms,
        source.max_end_at_ms
    );
}
