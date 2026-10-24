use super::*;
use sandproof_protocol::lifecycle::{ContentDigest, UnloadObservation, UnloadOutcome, UnloadedFacts};

pub(super) fn pending() -> (CenterState, UnloadObservation) {
    let mut config = serde_json::to_value(state()).unwrap();
    for field in ["instances", "cpu"] {
        config["bounds"]["platform"][field] = 1.into();
        config["nodes"][0]["limit"][field] = 1.into();
    }
    config["bounds"]["platform"]["memory_bytes"] = 1024.into();
    config["nodes"][0]["limit"]["memory_bytes"] = 1024.into();
    let image: CenterState = serde_json::from_value(config).unwrap();
    let owner = "1".try_into().unwrap();
    let (creating, create) = image.accept_lifecycle(&owner, &create("1", 1), 10).unwrap();
    let running = creating
        .accept_lifecycle_create_observation(&observed(&create))
        .unwrap();
    let (unloading, request) = running
        .accept_lifecycle(&owner, &followup(&create, 2, false), 150)
        .unwrap();
    let LifecycleOperation::Unload {
        source_incarnation,
        capture,
    } = request.request.operation
    else {
        unreachable!()
    };
    let prior = unloading.sandboxes()[0].last_running().unwrap();
    let facts = UnloadedFacts {
        source_incarnation,
        capture,
        material_bytes: 7,
        manifest_digest: ContentDigest::from_sha256([7; 32]),
        started_at_ms: prior.started_at_ms,
        end_at_ms: prior.end_at_ms,
        max_end_at_ms: prior.max_end_at_ms,
        startup_deadline_ms: prior.startup_deadline_ms,
    };
    (
        unloading,
        UnloadObservation {
            request,
            outcome: UnloadOutcome::Unloaded { facts },
        },
    )
}

#[test]
fn pending_does_not_complete_or_return_running_capacity() {
    let (image, mut observation) = pending();
    observation.outcome = UnloadOutcome::Pending;
    assert_eq!(
        image.accept_lifecycle_unload_observation(&observation, 160),
        Ok(image.clone())
    );
    assert_eq!(image.sandboxes()[0].phase(), SessionPhase::Unloading);
    assert_eq!(
        image.accept_lifecycle(&"2".try_into().unwrap(), &create("2", 1), 160),
        Err(DecisionError::Capacity)
    );
}

#[test]
fn completed_unload_survives_serialization_and_releases_only_running_components() {
    let (image, observation) = pending();
    let completed = image
        .accept_lifecycle_unload_observation(&observation, 160)
        .unwrap();
    let bytes = serde_json::to_vec(&completed).unwrap();
    let reopened: CenterState = serde_json::from_slice(&bytes).unwrap();
    assert!(reopened.is_valid());
    assert_eq!(reopened, completed);
    assert_eq!(reopened.sandboxes()[0].phase(), SessionPhase::Unloaded);
    assert_eq!(
        reopened.sandboxes()[0]
            .operations()
            .last()
            .unwrap()
            .unloaded(),
        Some(&observation)
    );
    let held = reopened.reserved(None, None).unwrap();
    assert_eq!(
        (held.instances, held.cpu, held.memory_bytes, held.disk_bytes),
        (0, 0, 0, 4096)
    );
    assert!(reopened.pending_lifecycle().is_empty());
    assert_eq!(
        reopened.accept_lifecycle_unload_observation(&observation, 170),
        Ok(completed)
    );
    let (second, _) = reopened
        .accept_lifecycle(&"2".try_into().unwrap(), &create("2", 1), 170)
        .unwrap();
    assert_eq!(second.sandboxes().len(), 2);
    assert_eq!(second.reserved(None, None).unwrap().disk_bytes, 8192);
    assert!(
        second
            .lifecycle_session(
                &"2".try_into().unwrap(),
                &observation.request.request.binding.session
            )
            .is_none()
    );
}

#[test]
fn changed_complete_binding_lifetime_or_capture_cannot_publish_unloaded() {
    let (image, original) = pending();
    for field in ["owner", "version", "capture", "lifetime"] {
        let mut changed = original.clone();
        match field {
            "owner" => changed.request.request.binding.key.owner = "2".try_into().unwrap(),
            "version" => changed.request.accepted_version += 1,
            "capture" => unloaded_mut(&mut changed).capture = 2.try_into().unwrap(),
            _ => unloaded_mut(&mut changed).end_at_ms += 1,
        }
        assert!(
            image
                .accept_lifecycle_unload_observation(&changed, 160)
                .is_err(),
            "{field}"
        );
    }
    let completed = image
        .accept_lifecycle_unload_observation(&original, 160)
        .unwrap();
    let mut changed = original.clone();
    unloaded_mut(&mut changed).manifest_digest = ContentDigest::from_sha256([8; 32]);
    assert_eq!(
        completed.accept_lifecycle_unload_observation(&changed, 170),
        Err(DecisionError::Conflict)
    );
}

#[test]
fn destruction_and_expiry_dominate_late_unload() {
    let (image, observation) = pending();
    assert_eq!(
        image.accept_lifecycle_unload_observation(&observation, 200),
        Err(DecisionError::Stale)
    );
    let terminated = image.terminate_expired(200).unwrap();
    assert_eq!(terminated.sandboxes()[0].phase(), SessionPhase::Terminating);
    assert_eq!(
        terminated.accept_lifecycle_unload_observation(&observation, 160),
        Err(DecisionError::Stale)
    );
    let completed = image
        .accept_lifecycle_unload_observation(&observation, 160)
        .unwrap();
    let terminated = completed.terminate_expired(200).unwrap();
    assert_eq!(terminated.sandboxes()[0].phase(), SessionPhase::Terminating);
    assert_eq!(terminated.reserved(None, None).unwrap().instances, 0);
    assert_eq!(terminated.reserved(None, None).unwrap().disk_bytes, 4096);
}

#[test]
fn checkpoint_requires_the_new_field_and_rechecks_original_lifetime() {
    let (image, observation) = pending();
    let complete = image
        .accept_lifecycle_unload_observation(&observation, 160)
        .unwrap();
    let mut data = serde_json::to_value(&complete).unwrap();
    data["sandboxes"][0]["operations"][1]["unloaded"]["outcome"]["facts"]["end_at_ms"] = 201.into();
    let invalid: CenterState = serde_json::from_value(data).unwrap();
    assert!(!invalid.is_valid());
    let mut data = serde_json::to_value(image).unwrap();
    data["sandboxes"][0]["operations"][1]
        .as_object_mut()
        .unwrap()
        .remove("unloaded");
    assert!(serde_json::from_value::<CenterState>(data).is_err());
}

fn unloaded_mut(observation: &mut UnloadObservation) -> &mut UnloadedFacts {
    let UnloadOutcome::Unloaded { facts } = &mut observation.outcome else { unreachable!() };
    facts
}

fn continued() -> (CenterState, UnloadObservation) {
    let (image, mut observation) = pending();
    let mut facts = image.sandboxes()[0].last_running().unwrap().clone();
    facts.observed_at_ms = 160;
    observation.outcome = UnloadOutcome::OriginalContinued { facts };
    (image, observation)
}

#[test]
fn original_continued_retains_current_identity_lifetime_and_capacity() {
    let (image, observation) = continued();
    let next = image.accept_lifecycle_unload_observation(&observation, 160).unwrap();
    let record = &next.sandboxes()[0];
    assert_eq!(record.phase(), SessionPhase::Running);
    assert_eq!(record.operations().last().unwrap().intent(), &observation.request);
    assert_eq!(record.current_running(), observation.outcome.continued());
    assert_eq!(next.reserved(None, None), image.reserved(None, None));
    assert!(next.pending_lifecycle().is_empty());
    assert_eq!(next.accept_lifecycle_unload_observation(&observation, 170), Ok(next.clone()));
    let encoded = serde_json::to_value(&next).unwrap();
    assert!(encoded["sandboxes"][0]["operations"][1]["running"].is_null());
    let reopened: CenterState = serde_json::from_value(encoded).unwrap();
    assert_eq!(next, reopened);
    assert!(reopened.is_valid());
}

#[test]
fn continued_and_unloaded_are_conflicting_final_results() {
    let (image, continued) = continued();
    let (_, unloaded) = pending();
    for (first, second) in [(&continued, &unloaded), (&unloaded, &continued)] {
        let next = image.accept_lifecycle_unload_observation(first, 160).unwrap();
        assert_eq!(next.accept_lifecycle_unload_observation(second, 170), Err(DecisionError::Conflict));
    }
}

#[test]
fn continued_rejects_changed_identity_lifetime_and_unobserved_time() {
    let (image, original) = continued();
    for field in ["version", "incarnation", "capture", "start", "end", "max", "startup", "before", "future"] {
        let mut changed = original.clone();
        match field {
            "version" => changed.request.accepted_version += 1,
            "incarnation" => {
                let LifecycleOperation::Unload { source_incarnation, .. } = &mut changed.request.request.operation else { unreachable!() };
                *source_incarnation = 2.try_into().unwrap();
            }
            "capture" => {
                let LifecycleOperation::Unload { capture, .. } = &mut changed.request.request.operation else { unreachable!() };
                *capture = 2.try_into().unwrap();
            }
            _ => {
                let UnloadOutcome::OriginalContinued { facts } = &mut changed.outcome else { unreachable!() };
                match field {
                    "start" => facts.started_at_ms += 1,
                    "end" => facts.end_at_ms += 1,
                    "max" => facts.max_end_at_ms += 1,
                    "startup" => facts.startup_deadline_ms += 1,
                    "before" => facts.observed_at_ms = 149,
                    _ => facts.observed_at_ms = 161,
                }
            }
        }
        assert!(image.accept_lifecycle_unload_observation(&changed, 160).is_err(), "{field}");
    }
}

#[test]
fn destruction_and_expiry_dominate_continuation() {
    let (image, observation) = continued();
    assert_eq!(image.accept_lifecycle_unload_observation(&observation, 200), Err(DecisionError::Stale));
    let terminated = image.terminate_expired(200).unwrap();
    assert_eq!(terminated.accept_lifecycle_unload_observation(&observation, 160), Err(DecisionError::Stale));
    let next = image.accept_lifecycle_unload_observation(&observation, 160).unwrap();
    let mut key = observation.request.request.binding.key.clone();
    key.sequence = 3.try_into().unwrap();
    let destroy = LifecycleSubmission::Destroy { key, session: observation.request.request.binding.session.clone(), expected_version: observation.request.accepted_version };
    let (terminated, _) = next.accept_lifecycle(&key_owner(&observation), &destroy, 170).unwrap();
    assert_eq!(terminated.sandboxes()[0].phase(), SessionPhase::Terminating);
    assert_eq!(terminated.accept_lifecycle_unload_observation(&observation, 170), Err(DecisionError::Stale));
    assert_eq!(terminated.reserved(None, None).unwrap().instances, 1);
}
fn key_owner(observation: &UnloadObservation) -> sandproof_protocol::TenantId {
    observation.request.request.binding.key.owner.clone()
}

#[test]
fn continued_allows_new_unload_and_renew_with_current_version_and_same_incarnation() {
    let (image, observation) = continued();
    let mut value = serde_json::to_value(image).unwrap();
    value["bounds"]["operations_per_sandbox"] = 5.into();
    let image: CenterState = serde_json::from_value(value).unwrap();
    let next = image.accept_lifecycle_unload_observation(&observation, 160).unwrap();
    let mut key = observation.request.request.binding.key.clone();
    key.sequence = 3.try_into().unwrap();
    let session = observation.request.request.binding.session.clone();
    let expected_version = observation.request.accepted_version;
    let unload = LifecycleSubmission::Unload { key: key.clone(), session: session.clone(), expected_version };
    let (again, intent) = next.accept_lifecycle(&key.owner, &unload, 170).unwrap();
    assert!(again.is_valid());
    assert_eq!(again.sandboxes()[0].phase(), SessionPhase::Unloading);
    let LifecycleOperation::Unload { source_incarnation: old, capture: old_capture } = observation.request.request.operation else { unreachable!() };
    let LifecycleOperation::Unload { source_incarnation, capture } = intent.request.operation else { unreachable!() };
    assert_eq!(source_incarnation, old);
    assert!(capture > old_capture);
    assert!(intent.accepted_version > expected_version);
    let renew = LifecycleSubmission::Renew { key: key.clone(), session, expected_version, timeout_secs: 1, extend_only: true };
    let (renewing, intent) = next.accept_lifecycle(&key.owner, &renew, 170).unwrap();
    assert!(renewing.is_valid());
    let LifecycleOperation::Renew { incarnation, .. } = intent.request.operation else { unreachable!() };
    assert_eq!(incarnation, old);
    assert_eq!(renewing.sandboxes()[0].phase(), SessionPhase::Renewing);
}

#[test]
fn checkpoint_rejects_duplicate_running_pending_and_mutated_continuation() {
    let (image, observation) = continued();
    let next = image.accept_lifecycle_unload_observation(&observation, 160).unwrap();
    for field in ["duplicate", "pending", "lifetime", "time"] {
        let mut encoded = serde_json::to_value(&next).unwrap();
        let operation = &mut encoded["sandboxes"][0]["operations"][1];
        match field {
            "duplicate" => operation["running"] = serde_json::to_value(observation.outcome.continued().unwrap()).unwrap(),
            "pending" => operation["unloaded"]["outcome"] = serde_json::json!({"state":"pending"}),
            "lifetime" => operation["unloaded"]["outcome"]["facts"]["end_at_ms"] = 201.into(),
            _ => operation["unloaded"]["outcome"]["facts"]["observed_at_ms"] = 149.into(),
        }
        let invalid: CenterState = serde_json::from_value(encoded).unwrap();
        assert!(!invalid.is_valid(), "{field}");
    }
}
