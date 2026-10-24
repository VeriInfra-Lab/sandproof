use super::*;
use sandproof_protocol::lifecycle::{ContentDigest, UnloadObservation, UnloadedFacts};

fn pending() -> (CenterState, UnloadObservation) {
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
    let running = creating.accept_lifecycle_create_observation(&observed(&create)).unwrap();
    let (unloading, request) = running.accept_lifecycle(&owner, &followup(&create, 2, false), 150).unwrap();
    let LifecycleOperation::Unload { source_incarnation, capture } = request.request.operation else { unreachable!() };
    let prior = unloading.sandboxes()[0].last_running().unwrap();
    let facts = UnloadedFacts {
        source_incarnation, capture, material_bytes: 7, manifest_digest: ContentDigest::from_sha256([7; 32]),
        started_at_ms: prior.started_at_ms, end_at_ms: prior.end_at_ms,
        max_end_at_ms: prior.max_end_at_ms, startup_deadline_ms: prior.startup_deadline_ms,
    };
    (unloading, UnloadObservation { request, unloaded: Some(facts) })
}

#[test]
fn pending_does_not_complete_or_return_running_capacity() {
    let (image, mut observation) = pending();
    observation.unloaded = None;
    assert_eq!(image.accept_lifecycle_unload_observation(&observation, 160), Ok(image.clone()));
    assert_eq!(image.sandboxes()[0].phase(), SessionPhase::Unloading);
    assert_eq!(image.accept_lifecycle(&"2".try_into().unwrap(), &create("2", 1), 160), Err(DecisionError::Capacity));
}

#[test]
fn completed_unload_survives_serialization_and_releases_only_running_components() {
    let (image, observation) = pending();
    let completed = image.accept_lifecycle_unload_observation(&observation, 160).unwrap();
    let bytes = serde_json::to_vec(&completed).unwrap();
    let reopened: CenterState = serde_json::from_slice(&bytes).unwrap();
    assert!(reopened.is_valid());
    assert_eq!(reopened, completed);
    assert_eq!(reopened.sandboxes()[0].phase(), SessionPhase::Unloaded);
    assert_eq!(reopened.sandboxes()[0].operations().last().unwrap().unloaded(), Some(&observation));
    let held = reopened.reserved(None, None).unwrap();
    assert_eq!((held.instances, held.cpu, held.memory_bytes, held.disk_bytes), (0, 0, 0, 4096));
    assert!(reopened.pending_lifecycle().is_empty());
    assert_eq!(reopened.accept_lifecycle_unload_observation(&observation, 170), Ok(completed));
    let (second, _) = reopened.accept_lifecycle(&"2".try_into().unwrap(), &create("2", 1), 170).unwrap();
    assert_eq!(second.sandboxes().len(), 2);
    assert_eq!(second.reserved(None, None).unwrap().disk_bytes, 8192);
    assert!(second.lifecycle_session(&"2".try_into().unwrap(), &observation.request.request.binding.session).is_none());
}

#[test]
fn changed_complete_binding_lifetime_or_capture_cannot_publish_unloaded() {
    let (image, original) = pending();
    for field in ["owner", "version", "capture", "lifetime"] {
        let mut changed = original.clone();
        match field {
            "owner" => changed.request.request.binding.key.owner = "2".try_into().unwrap(),
            "version" => changed.request.accepted_version += 1,
            "capture" => changed.unloaded.as_mut().unwrap().capture = 2.try_into().unwrap(),
            _ => changed.unloaded.as_mut().unwrap().end_at_ms += 1,
        }
        assert!(image.accept_lifecycle_unload_observation(&changed, 160).is_err(), "{field}");
    }
    let completed = image.accept_lifecycle_unload_observation(&original, 160).unwrap();
    let mut changed = original.clone();
    changed.unloaded.as_mut().unwrap().manifest_digest = ContentDigest::from_sha256([8; 32]);
    assert_eq!(completed.accept_lifecycle_unload_observation(&changed, 170), Err(DecisionError::Conflict));
}

#[test]
fn destruction_and_expiry_dominate_late_unload() {
    let (image, observation) = pending();
    assert_eq!(image.accept_lifecycle_unload_observation(&observation, 200), Err(DecisionError::Stale));
    let terminated = image.terminate_expired(200).unwrap();
    assert_eq!(terminated.sandboxes()[0].phase(), SessionPhase::Terminating);
    assert_eq!(terminated.accept_lifecycle_unload_observation(&observation, 160), Err(DecisionError::Stale));
    let completed = image.accept_lifecycle_unload_observation(&observation, 160).unwrap();
    let terminated = completed.terminate_expired(200).unwrap();
    assert_eq!(terminated.sandboxes()[0].phase(), SessionPhase::Terminating);
    assert_eq!(terminated.reserved(None, None).unwrap().instances, 0);
    assert_eq!(terminated.reserved(None, None).unwrap().disk_bytes, 4096);
}

#[test]
fn checkpoint_requires_the_new_field_and_rechecks_original_lifetime() {
    let (image, observation) = pending();
    let complete = image.accept_lifecycle_unload_observation(&observation, 160).unwrap();
    let mut data = serde_json::to_value(&complete).unwrap();
    data["sandboxes"][0]["operations"][1]["unloaded"]["unloaded"]["end_at_ms"] = 201.into();
    let invalid: CenterState = serde_json::from_value(data).unwrap();
    assert!(!invalid.is_valid());
    let mut data = serde_json::to_value(image).unwrap();
    data["sandboxes"][0]["operations"][1].as_object_mut().unwrap().remove("unloaded");
    assert!(serde_json::from_value::<CenterState>(data).is_err());
}
