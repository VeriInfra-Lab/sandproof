use sandproof_center_core::{
    Bounds, CenterState, CreateParameters, DecisionError, LifecycleSubmission, LifecycleWindow,
    NodeMode, NodeRegistration, NodeRole, SessionPhase, TenantBudget,
};
use sandproof_protocol::internal::lifecycle::LifecycleFact;
use sandproof_protocol::lifecycle::{CenterIdentity, LifecycleOperation, OperationKey};
#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
use fixture::*;

fn state() -> CenterState {
    let center = CenterState::new(
        node("host-a").deployment_id,
        Bounds {
            max_nodes: 1,
            max_templates_per_node: 2,
            max_sandboxes: 2,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
            platform: resources(2),
        },
        vec![NodeRegistration {
            identity: node("host-a"),
            retention_slots: 2,
            role: NodeRole::Runtime,
            limit: resources(2),
            mode: NodeMode::Accepting,
            healthy_until_ms: 2000,
            free: resources(2),
            templates: vec![sandproof_center_core::AvailableTemplate {
                revision: binding().template_revision,
                digest: binding().template_digest,
            }],
            protocol_version: 2,
        }],
        ["1", "2"]
            .into_iter()
            .map(|id| TenantBudget {
                tenant: id.try_into().unwrap(),
                limit: resources(2),
            })
            .collect(),
    )
    .unwrap();
    center
        .enable_lifecycle(
            CenterIdentity {
                deployment: node("host-a").deployment_id,
                center: "center".try_into().unwrap(),
            },
            ["1", "2"]
                .into_iter()
                .map(|id| LifecycleWindow {
                    owner: id.try_into().unwrap(),
                    window: 9.try_into().unwrap(),
                })
                .collect(),
        )
        .unwrap()
}
fn create(owner: &str, sequence: u64) -> LifecycleSubmission {
    let binding = binding();
    LifecycleSubmission::Create {
        key: OperationKey {
            deployment: node("host-a").deployment_id,
            owner: owner.try_into().unwrap(),
            window: 9.try_into().unwrap(),
            sequence: sequence.try_into().unwrap(),
        },
        parameters: CreateParameters {
            template_revision: binding.template_revision,
            template_digest: binding.template_digest,
            resources: binding.resources,
            options: binding.options,
            startup_timeout_ms: 1000,
            timeout_ms: 100,
            max_lifetime_ms: 500,
        },
    }
}
fn followup(
    original: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    sequence: u64,
    destroy: bool,
) -> LifecycleSubmission {
    let mut key = original.request.binding.key.clone();
    key.sequence = sequence.try_into().unwrap();
    if destroy {
        LifecycleSubmission::Destroy {
            key,
            session: original.request.binding.session.clone(),
            expected_version: 1,
        }
    } else {
        LifecycleSubmission::Unload {
            key,
            session: original.request.binding.session.clone(),
            expected_version: 1,
        }
    }
}

#[test]
fn two_owners_get_original_allocations_and_same_block_enumerable_outbox() {
    let initial = state();
    let (first, a) = initial
        .accept_lifecycle(&"1".try_into().unwrap(), &create("1", 1), 10)
        .unwrap();
    let (second, b) = first
        .accept_lifecycle(&"2".try_into().unwrap(), &create("2", 1), 10)
        .unwrap();
    assert!(second.is_valid());
    assert_eq!(
        a.request.binding.session.as_protocol_id().as_str(),
        "sandbox-1"
    );
    assert_eq!(
        b.request.binding.session.as_protocol_id().as_str(),
        "sandbox-2"
    );
    assert_eq!(second.identity_frontiers(), (2, 2));
    assert_eq!(second.reserved(None, None), Some(resources(2)));
    assert_eq!(second.pending_lifecycle(), vec![a, b]);
    assert!(second.pending().is_empty());
    let bytes = serde_json::to_vec(&second).unwrap();
    let reopened: CenterState = serde_json::from_slice(&bytes).unwrap();
    assert!(reopened.is_valid());
    assert_eq!(reopened, second);
    assert!(bytes.len() <= second.required_authority_bytes());
}

#[test]
fn replay_precedes_time_and_phase_checks_without_new_ids_or_new_deadlines() {
    let input = create("1", 1);
    let (accepted, intent) = state()
        .accept_lifecycle(&"1".try_into().unwrap(), &input, 10)
        .unwrap();
    let ready = accepted
        .observe_lifecycle_ready(&intent, 1.try_into().unwrap(), 100, 200, 600)
        .unwrap();
    let unload = followup(&intent, 2, false);
    let (unloading, _) = ready
        .accept_lifecycle(&"1".try_into().unwrap(), &unload, 150)
        .unwrap();
    let (same, replay) = unloading
        .accept_lifecycle(&"1".try_into().unwrap(), &input, u64::MAX)
        .unwrap();
    assert_eq!(same, unloading);
    assert_eq!(replay, intent);
    assert_eq!(same.identity_frontiers(), (1, 1));
}

#[test]
fn changed_full_submission_and_foreign_owner_do_not_reuse_key() {
    let input = create("1", 1);
    let (accepted, _) = state()
        .accept_lifecycle(&"1".try_into().unwrap(), &input, 10)
        .unwrap();
    let mut wrong = input.clone();
    let LifecycleSubmission::Create { parameters, .. } = &mut wrong else {
        unreachable!()
    };
    parameters.options.push(b' ');
    assert_eq!(
        accepted.accept_lifecycle(&"1".try_into().unwrap(), &wrong, 10),
        Err(DecisionError::Conflict)
    );
    assert_eq!(
        accepted.accept_lifecycle(&"2".try_into().unwrap(), &input, 10),
        Err(DecisionError::NotFound)
    );
}

#[test]
fn unload_requires_actual_ready_binding_preserves_reservations_and_destroy_capacity() {
    let (creating, intent) = state()
        .accept_lifecycle(&"1".try_into().unwrap(), &create("1", 1), 10)
        .unwrap();
    let unload = followup(&intent, 2, false);
    assert_eq!(
        creating.accept_lifecycle(&"1".try_into().unwrap(), &unload, 100),
        Err(DecisionError::Conflict)
    );
    let ready = creating
        .observe_lifecycle_ready(&intent, 1.try_into().unwrap(), 100, 200, 600)
        .unwrap();
    let (unloading, accepted) = ready
        .accept_lifecycle(&"1".try_into().unwrap(), &unload, 150)
        .unwrap();
    assert_eq!(accepted.accepted_version, 2);
    assert_eq!(unloading.reserved(None, None), ready.reserved(None, None));
    let session = unloading
        .lifecycle_session(&"1".try_into().unwrap(), &intent.request.binding.session)
        .unwrap();
    assert_eq!(session.phase(), SessionPhase::Unloading);
    let mut stop = followup(&intent, 3, true);
    let LifecycleSubmission::Destroy {
        expected_version, ..
    } = &mut stop
    else {
        unreachable!()
    };
    *expected_version = 2;
    let (terminating, destroy) = unloading
        .accept_lifecycle(&"1".try_into().unwrap(), &stop, 150)
        .unwrap();
    assert_eq!(destroy.accepted_version, 3);
    assert!(matches!(
        destroy.request.operation,
        LifecycleOperation::Destroy {}
    ));
    assert_eq!(terminating.reserved(None, None), ready.reserved(None, None));
    assert!(terminating.sandboxes()[0].terminating);
    assert_eq!(terminating.pending_lifecycle(), vec![destroy]);
    assert_eq!(
        terminating.observe_lifecycle_ready(&intent, 2.try_into().unwrap(), 100, 200, 600),
        Err(DecisionError::Stale)
    );
}

#[test]
fn stale_version_and_window_refuse_before_effects_and_legacy_writes_cannot_bypass() {
    let (creating, intent) = state()
        .accept_lifecycle(&"1".try_into().unwrap(), &create("1", 1), 10)
        .unwrap();
    let ready = creating
        .observe_lifecycle_ready(&intent, 1.try_into().unwrap(), 100, 200, 600)
        .unwrap();
    let mut input = followup(&intent, 2, false);
    let LifecycleSubmission::Unload {
        expected_version, ..
    } = &mut input
    else {
        unreachable!()
    };
    *expected_version = 0;
    assert_eq!(
        ready.accept_lifecycle(&"1".try_into().unwrap(), &input, 150),
        Err(DecisionError::Stale)
    );
    let mut wrong_window = create("1", 3);
    let LifecycleSubmission::Create { key, .. } = &mut wrong_window else {
        unreachable!()
    };
    key.window = 10.try_into().unwrap();
    assert_eq!(
        ready.accept_lifecycle(&"1".try_into().unwrap(), &wrong_window, 150),
        Err(DecisionError::Stale)
    );
    assert_eq!(
        ready.create(&"1".try_into().unwrap(), binding(), 10),
        Err(DecisionError::Conflict)
    );
    assert_eq!(
        ready.destroy(
            &"1".try_into().unwrap(),
            intent.request.binding.session.as_protocol_id()
        ),
        Err(DecisionError::Conflict)
    );
    let old = request(
        1,
        1,
        sandproof_protocol::internal::lifecycle::Intent::Create,
    );
    assert_eq!(
        ready.apply_response(&response(&old, 2, LifecycleFact::Stopped)),
        Err(DecisionError::Conflict)
    );
    assert!(ready.pending().is_empty());
}

#[test]
fn recovery_refuses_corrupt_versions_keys_or_original_mapping() {
    let (accepted, _) = state()
        .accept_lifecycle(&"1".try_into().unwrap(), &create("1", 1), 10)
        .unwrap();
    for field in ["accepted_version", "original_sandbox", "created_at_ms"] {
        let mut json = serde_json::to_value(&accepted).unwrap();
        let session = &mut json["lifecycle"]["sessions"][0];
        match field {
            "accepted_version" => session["operations"][0]["intent"][field] = 2.into(),
            "original_sandbox" => session[field] = "sandbox-2".into(),
            _ => session[field] = 999.into(),
        }
        let wrong: CenterState = serde_json::from_value(json).unwrap();
        assert!(!wrong.is_valid(), "corrupt {field} accepted");
    }
}

#[test]
fn new_versions_preserve_successor_and_reject_stale_or_exhausted() {
    use sandproof_center_core::next_lifecycle_version;
    assert_eq!(next_lifecycle_version(0, 1), Err(DecisionError::Stale));
    assert_eq!(next_lifecycle_version(7, 7), Ok(8));
    assert_eq!(next_lifecycle_version(0, 0), Ok(1));
    assert_eq!(
        next_lifecycle_version(u64::MAX, u64::MAX),
        Err(DecisionError::Exhausted)
    );
}

#[test]
fn snapshot_resource_has_a_distinct_wire_role() {
    assert_eq!(
        serde_json::to_string(&sandproof_protocol::ResourceKind::Snapshot).unwrap(),
        "\"snapshot\""
    );
    assert_ne!(
        sandproof_protocol::ResourceKind::Snapshot,
        sandproof_protocol::ResourceKind::Workspace
    );
}
