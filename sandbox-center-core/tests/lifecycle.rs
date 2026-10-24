use sandproof_center_core::{
    Bounds, CenterState, CreateParameters, DecisionError, LifecycleSubmission, LifecycleWindow,
    NodeMode, NodeRegistration, NodeRole, SessionPhase, TenantBudget,
};
use sandproof_protocol::lifecycle::{CenterIdentity, LifecycleOperation, OperationKey};
#[path = "../../sandbox-protocol/tests/support/lifecycle.rs"]
mod fixture;
use fixture::*;
#[path = "lifecycle/acceptance_time.rs"]
mod acceptance_time;
#[path = "lifecycle/refusal.rs"]
mod refusal;
#[path = "lifecycle/renewal_confirmation.rs"]
mod renewal_confirmation;
#[path = "lifecycle/unloaded.rs"]
mod unloaded;

fn state() -> CenterState {
    CenterState::new(
        node("host-a").deployment_id,
        Bounds {
            max_nodes: 1,
            max_templates_per_node: 2,
            max_sandboxes: 2,
            operations_per_sandbox: 3,
            max_options_bytes: 128,
            result_retention_ms: 100,
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

#[test]
fn protected_builder_role_uses_original_allocator_without_runtime_replay() {
    let mut encoded = serde_json::to_value(state()).unwrap();
    encoded["nodes"][0]["role"] = serde_json::json!("builder");
    let initial: CenterState = serde_json::from_value(encoded).unwrap();
    assert!(initial.is_valid());
    let owner = "1".try_into().unwrap();
    let input = create("1", 1);
    assert_eq!(
        initial.accept_lifecycle(&owner, &input, 10),
        Err(DecisionError::Unavailable)
    );
    let (accepted, intent) = initial
        .accept_builder_lifecycle(&owner, &input, 10)
        .unwrap();
    assert!(accepted.is_valid());
    assert_eq!(accepted.identity_frontiers(), (1, 1));
    assert_eq!(accepted.reserved(None, None), Some(resources(1)));
    assert_eq!(
        accepted.accept_builder_lifecycle(&owner, &input, u64::MAX),
        Ok((accepted.clone(), intent.clone()))
    );
    assert_eq!(
        accepted.accept_lifecycle(&owner, &input, 10),
        Err(DecisionError::Conflict)
    );
    let runtime = state();
    let (runtime, _) = runtime.accept_lifecycle(&owner, &input, 10).unwrap();
    assert_eq!(
        runtime.accept_builder_lifecycle(&owner, &input, 10),
        Err(DecisionError::Conflict)
    );
    assert_eq!(
        accepted.accept_builder_lifecycle(&owner, &followup(&intent, 2, true), 10),
        Err(DecisionError::Invalid)
    );
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
    let encoded = serde_json::to_value(&second).unwrap();
    assert!(encoded["lifecycle"].get("sessions").is_none());
    assert_eq!(encoded["sandboxes"].as_array().unwrap().len(), 2);
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
        .accept_lifecycle_create_observation(&observed(&intent))
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
    assert_eq!(same.identity_frontiers(), (1, 2));
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
        .accept_lifecycle_create_observation(&observed(&intent))
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
    assert!(terminating.sandboxes()[0].terminating());
    assert_eq!(terminating.pending_lifecycle(), vec![destroy]);
    assert_eq!(
        terminating.accept_lifecycle_create_observation(&observed(&intent)),
        Err(DecisionError::Stale)
    );
}

#[test]
fn stale_version_and_window_refuse_before_effects_and_missing_registration_rejects() {
    let (creating, intent) = state()
        .accept_lifecycle(&"1".try_into().unwrap(), &create("1", 1), 10)
        .unwrap();
    let ready = creating
        .accept_lifecycle_create_observation(&observed(&intent))
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
    let mut missing = serde_json::to_value(&ready).unwrap();
    missing.as_object_mut().unwrap().remove("lifecycle");
    assert!(serde_json::from_value::<CenterState>(missing).is_err());
    let mut old_envelope = serde_json::to_value(&ready).unwrap();
    old_envelope["sandboxes"][0]["operations"][0]["response"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<CenterState>(old_envelope).is_err());
}

#[test]
fn recovery_refuses_corrupt_versions_keys_or_original_mapping() {
    let (accepted, _) = state()
        .accept_lifecycle(&"1".try_into().unwrap(), &create("1", 1), 10)
        .unwrap();
    for field in ["accepted_version", "session", "acceptance_time"] {
        let mut json = serde_json::to_value(&accepted).unwrap();
        let operation = &mut json["sandboxes"][0]["operations"][0];
        match field {
            "accepted_version" => operation["intent"][field] = 2.into(),
            "session" => operation["intent"]["request"]["binding"][field] = "sandbox-2".into(),
            _ => operation[field] = serde_json::json!({"kind":"sampled","at_ms":999}),
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

// Controlled input to the V3 save contract, not physical-host evidence.
fn observed(
    intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
) -> sandproof_protocol::lifecycle::CreateObservation {
    use sandproof_protocol::lifecycle::{
        CreateObservation, CurrentCreateReadiness, CurrentReadyFacts, LifecycleOperation,
    };
    let LifecycleOperation::Create { create, .. } = &intent.request.operation else {
        unreachable!()
    };
    CreateObservation {
        request: intent.clone(),
        current_request: intent.clone(),
        historical_create_succeeded: true,
        readiness: CurrentCreateReadiness::Observed {
            facts: CurrentReadyFacts {
                started_at_ms: 100,
                end_at_ms: 200,
                max_end_at_ms: 600,
                startup_deadline_ms: create.startup_deadline_ms,
                observed_at_ms: 100,
            },
        },
    }
}

#[test]
fn renewal_keeps_the_original_execution_and_replays_the_original_relative_input() {
    let owner = "1".try_into().unwrap();
    let (created, original) = state()
        .accept_lifecycle(&owner, &create("1", 1), 10)
        .unwrap();
    let running = created
        .accept_lifecycle_create_observation(&observed(&original))
        .unwrap();
    let mut key = original.request.binding.key.clone();
    key.sequence = 2.try_into().unwrap();
    let input = LifecycleSubmission::Renew {
        key,
        session: original.request.binding.session.clone(),
        expected_version: 1,
        timeout_secs: 1,
        extend_only: false,
    };
    let (pending, renewed) = running.accept_lifecycle(&owner, &input, 150).unwrap();
    assert_eq!(pending.sandboxes()[0].phase(), SessionPhase::Renewing);
    assert!(pending.sandboxes()[0].current_running().is_none());
    assert!(matches!(
        renewed.request.operation,
        LifecycleOperation::Renew { end_at_ms: 600, .. }
    ));
    let mut observation = observed(&original);
    observation.current_request = renewed.clone();
    let sandproof_protocol::lifecycle::CurrentCreateReadiness::Observed { facts } =
        &mut observation.readiness
    else {
        unreachable!()
    };
    facts.end_at_ms = 600;
    facts.observed_at_ms = 160;
    let committed = pending
        .accept_lifecycle_create_observation(&observation)
        .unwrap();
    let current = committed.sandboxes()[0].current_running().unwrap();
    assert_eq!(
        (
            current.started_at_ms,
            current.max_end_at_ms,
            current.end_at_ms
        ),
        (100, 600, 600)
    );
    let (same, replay) = committed
        .accept_lifecycle(&owner, &input, u64::MAX)
        .unwrap();
    assert_eq!(same, committed);
    assert_eq!(replay, renewed);
    let mut changed = input;
    let LifecycleSubmission::Renew { timeout_secs, .. } = &mut changed else {
        unreachable!()
    };
    *timeout_secs = 2;
    assert_eq!(
        committed.accept_lifecycle(&owner, &changed, 150),
        Err(DecisionError::Conflict)
    );
}

fn cleaned(
    accepted: &CenterState,
    original: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    sdk_destroy: bool,
    now: u64,
) -> (
    CenterState,
    sandproof_protocol::lifecycle::TerminalLifecycleObservation,
) {
    use sandproof_protocol::lifecycle::{TerminalLifecycleObservation, TerminalStatus};
    let owner = &original.request.binding.key.owner;
    let mut input = followup(original, 3, true);
    if sdk_destroy {
        let LifecycleSubmission::Destroy { key, .. } = &mut input else {
            unreachable!()
        };
        *key = accepted.next_sdk_operation_key(owner).unwrap();
    }
    let (terminating, intent) = accepted.accept_lifecycle(owner, &input, 20).unwrap();
    let result = TerminalLifecycleObservation {
        request: intent,
        status: TerminalStatus::Cleaned,
    };
    let state = terminating
        .accept_lifecycle_terminal_observation(&result, now)
        .unwrap();
    (state, result)
}

#[test]
fn exact_confirmation_expiry_and_close_ack_are_separate_retirement_gates() {
    use sandproof_protocol::lifecycle::{ClosedLifecycleWindow, ConfirmedLifecycleResult};
    let owner = "1".try_into().unwrap();
    let (accepted, intent) = state()
        .accept_lifecycle(&owner, &create("1", 1), 10)
        .unwrap();
    let (completed, result) = cleaned(&accepted, &intent, false, 100);
    assert!(completed.sandboxes()[0].released());
    assert!(completed.retirement_candidates(200).is_empty());
    assert_eq!(
        completed.begin_close(&result, 200),
        Err(DecisionError::Conflict)
    );
    let confirmed = completed
        .confirm_terminal(&ConfirmedLifecycleResult {
            result: result.clone(),
        })
        .unwrap();
    assert_eq!(
        confirmed.begin_close(&result, 199),
        Err(DecisionError::Unavailable)
    );
    let (closing, request) = confirmed.begin_close(&result, 200).unwrap();
    let wrong = ClosedLifecycleWindow {
        terminal: result.clone(),
        retired_through: 2.try_into().unwrap(),
    };
    assert_eq!(
        closing.apply_close_ack(&wrong, 200),
        Err(DecisionError::Stale)
    );
    let exact = ClosedLifecycleWindow {
        terminal: result,
        retired_through: request.terminal.request.retention.window,
    };
    assert_eq!(
        closing.apply_close_ack(&exact, 199),
        Err(DecisionError::Unavailable)
    );
    let retired = closing.apply_close_ack(&exact, 200).unwrap();
    assert!(retired.sandboxes().is_empty());
    assert_eq!(retired.apply_close_ack(&exact, 201).unwrap(), retired);
    assert!(retired.is_valid());
}

#[test]
fn public_out_of_order_keys_and_old_live_replay_survive_a_peer_retirement() {
    use sandproof_protocol::lifecycle::{ClosedLifecycleWindow, ConfirmedLifecycleResult};
    let owner = "1".try_into().unwrap();
    let first_input = create("1", 2);
    let second_input = create("1", 1);
    let (first, a) = state().accept_lifecycle(&owner, &first_input, 10).unwrap();
    let (both, b) = first.accept_lifecycle(&owner, &second_input, 10).unwrap();
    // Last Destroy is SDK2. Deletion must still close the public Create window.
    let (completed, result) = cleaned(&both, &a, true, 100);
    let confirmed = completed
        .confirm_terminal(&ConfirmedLifecycleResult {
            result: result.clone(),
        })
        .unwrap();
    let (closing, close) = confirmed.begin_close(&result, 200).unwrap();
    let retired = closing
        .apply_close_ack(
            &ClosedLifecycleWindow {
                terminal: close.terminal.clone(),
                retired_through: close.terminal.request.retention.window,
            },
            200,
        )
        .unwrap();
    assert_eq!(
        retired
            .public_request_window(&owner)
            .unwrap()
            .unwrap()
            .get(),
        11
    );
    let (same, replay) = retired
        .accept_lifecycle(&owner, &second_input, u64::MAX)
        .unwrap();
    assert_eq!(same, retired);
    assert_eq!(replay, b);
    assert_eq!(
        retired.accept_lifecycle(&owner, &first_input, 200),
        Err(DecisionError::Stale)
    );
    let mut fresh = create("1", 1);
    let LifecycleSubmission::Create { key, .. } = &mut fresh else {
        unreachable!()
    };
    key.window = 11.try_into().unwrap();
    assert!(retired.accept_lifecycle(&owner, &fresh, 200).is_ok());
}

#[test]
fn duplicate_terminal_observation_does_not_extend_retention() {
    use sandproof_protocol::lifecycle::ConfirmedLifecycleResult;
    let owner = "1".try_into().unwrap();
    let (accepted, original) = state()
        .accept_lifecycle(&owner, &create("1", 1), 10)
        .unwrap();
    let (completed, result) = cleaned(&accepted, &original, false, 100);
    let replay = completed
        .accept_lifecycle_terminal_observation(&result, 199)
        .unwrap();
    assert_eq!(replay, completed);
    let reopened: CenterState =
        serde_json::from_slice(&serde_json::to_vec(&replay).unwrap()).unwrap();
    let confirmed = reopened
        .confirm_terminal(&ConfirmedLifecycleResult {
            result: result.clone(),
        })
        .unwrap();
    assert!(confirmed.begin_close(&result, 200).is_ok());
}

#[test]
fn retention_clock_faults_keep_the_completed_fact_and_block_deletion() {
    use sandproof_protocol::lifecycle::ConfirmedLifecycleResult;
    for (period, clock) in [(u64::MAX, 100), (100, 0)] {
        let mut value = serde_json::to_value(state()).unwrap();
        value["bounds"]["result_retention_ms"] = period.into();
        let initial: CenterState = serde_json::from_value(value).unwrap();
        let owner = "1".try_into().unwrap();
        let (accepted, original) = initial
            .accept_lifecycle(&owner, &create("1", 1), 10)
            .unwrap();
        let (completed, result) = cleaned(&accepted, &original, false, clock);
        assert_eq!(completed.sandboxes()[0].phase(), SessionPhase::Cleaned);
        let confirmed = completed
            .confirm_terminal(&ConfirmedLifecycleResult {
                result: result.clone(),
            })
            .unwrap();
        assert_eq!(
            confirmed.begin_close(&result, u64::MAX),
            Err(DecisionError::Unavailable)
        );
        assert!(confirmed.is_valid());
    }
}

#[test]
fn deleted_sdk_keys_cannot_be_accepted_again_without_a_retained_record() {
    use sandproof_protocol::lifecycle::{ClosedLifecycleWindow, ConfirmedLifecycleResult};
    let owner = "1".try_into().unwrap();
    let initial = state();
    let mut input = create("1", 1);
    let LifecycleSubmission::Create { key, .. } = &mut input else {
        unreachable!()
    };
    *key = initial.next_sdk_operation_key(&owner).unwrap();
    let (accepted, original) = initial.accept_lifecycle(&owner, &input, 10).unwrap();
    let (completed, result) = cleaned(&accepted, &original, true, 100);
    let confirmed = completed
        .confirm_terminal(&ConfirmedLifecycleResult {
            result: result.clone(),
        })
        .unwrap();
    let (closing, close) = confirmed.begin_close(&result, 200).unwrap();
    let retired = closing
        .apply_close_ack(
            &ClosedLifecycleWindow {
                terminal: close.terminal.clone(),
                retired_through: close.terminal.request.retention.window,
            },
            200,
        )
        .unwrap();
    assert_eq!(
        retired.accept_lifecycle(&owner, &input, 200),
        Err(DecisionError::Stale)
    );
    assert_eq!(
        retired
            .public_request_window(&owner)
            .unwrap()
            .unwrap()
            .get(),
        9
    );
    let LifecycleSubmission::Create { key, .. } = &mut input else {
        unreachable!()
    };
    *key = retired.next_sdk_operation_key(&owner).unwrap();
    assert_eq!(key.sequence.get(), 3);
    assert!(retired.accept_lifecycle(&owner, &input, 200).is_ok());
}

#[test]
fn public_window_exhaustion_never_wraps_or_consumes_the_internal_key_reserve() {
    use sandproof_protocol::lifecycle::{ClosedLifecycleWindow, ConfirmedLifecycleResult};
    let mut value = serde_json::to_value(state()).unwrap();
    value["lifecycle"]["windows"][0]["registration"]["window"] = u64::MAX.into();
    value["lifecycle"]["windows"][0]["current"] = u64::MAX.into();
    let initial: CenterState = serde_json::from_value(value).unwrap();
    assert!(initial.is_valid());
    let owner = "1".try_into().unwrap();
    let mut input = create("1", 1);
    let LifecycleSubmission::Create { key, .. } = &mut input else {
        unreachable!()
    };
    key.window = u64::MAX.try_into().unwrap();
    let (accepted, original) = initial.accept_lifecycle(&owner, &input, 10).unwrap();
    let (completed, result) = cleaned(&accepted, &original, true, 100);
    let confirmed = completed
        .confirm_terminal(&ConfirmedLifecycleResult {
            result: result.clone(),
        })
        .unwrap();
    let (closing, close) = confirmed.begin_close(&result, 200).unwrap();
    let retired = closing
        .apply_close_ack(
            &ClosedLifecycleWindow {
                terminal: close.terminal.clone(),
                retired_through: close.terminal.request.retention.window,
            },
            200,
        )
        .unwrap();
    assert_eq!(retired.public_request_window(&owner).unwrap(), None);
    assert_eq!(
        retired.accept_lifecycle(&owner, &input, 200),
        Err(DecisionError::Stale)
    );
    let LifecycleSubmission::Create { key, .. } = &mut input else {
        unreachable!()
    };
    *key = retired.next_sdk_operation_key(&owner).unwrap();
    assert!(retired.accept_lifecycle(&owner, &input, 200).is_ok());
    assert!(retired.is_valid());
}

#[test]
fn acceptance_keeps_a_future_destroy_identity_before_exhaustion() {
    let mut value = serde_json::to_value(state()).unwrap();
    value["operation_highwater"] = (u64::MAX - 1).into();
    let initial: CenterState = serde_json::from_value(value).unwrap();
    assert!(initial.is_valid());
    assert_eq!(
        initial.accept_lifecycle(&"1".try_into().unwrap(), &create("1", 1), 10),
        Err(DecisionError::Exhausted)
    );
    assert_eq!(initial.identity_frontiers(), (0, u64::MAX - 1));
}

#[test]
fn temporary_bad_clock_can_establish_retention_once_after_restart() {
    use sandproof_protocol::lifecycle::{ClosedLifecycleWindow, ConfirmedLifecycleResult};
    let owner = "1".try_into().unwrap();
    let (accepted, original) = state()
        .accept_lifecycle(&owner, &create("1", 1), 10)
        .unwrap();
    let (completed, result) = cleaned(&accepted, &original, false, 0);
    assert_eq!(completed.sandboxes()[0].phase(), SessionPhase::Cleaned);
    assert!(!completed.pending_lifecycle().is_empty());
    let reopened: CenterState =
        serde_json::from_slice(&serde_json::to_vec(&completed).unwrap()).unwrap();
    let restored = reopened
        .accept_lifecycle_terminal_observation(&result, 100)
        .unwrap();
    let duplicate = restored
        .accept_lifecycle_terminal_observation(&result, 150)
        .unwrap();
    assert_eq!(restored, duplicate);
    let confirmed = duplicate
        .confirm_terminal(&ConfirmedLifecycleResult {
            result: result.clone(),
        })
        .unwrap();
    assert!(confirmed.pending_lifecycle().is_empty());
    assert_eq!(
        confirmed.begin_close(&result, 199),
        Err(DecisionError::Unavailable)
    );
    let (closing, close) = confirmed.begin_close(&result, 200).unwrap();
    let retired = closing
        .apply_close_ack(
            &ClosedLifecycleWindow {
                terminal: close.terminal.clone(),
                retired_through: close.terminal.request.retention.window,
            },
            200,
        )
        .unwrap();
    assert!(retired.sandboxes().is_empty());
}

#[test]
fn pending_renewal_expiry_uses_its_accepted_deadline_without_releasing_resources() {
    let owner = "1".try_into().unwrap();
    let (created, original) = state()
        .accept_lifecycle(&owner, &create("1", 1), 10)
        .unwrap();
    let running = created
        .accept_lifecycle_create_observation(&observed(&original))
        .unwrap();
    let mut key = original.request.binding.key.clone();
    key.sequence = 2.try_into().unwrap();
    let (pending, _) = running
        .accept_lifecycle(
            &owner,
            &LifecycleSubmission::Renew {
                key,
                session: original.request.binding.session.clone(),
                expected_version: 1,
                timeout_secs: 1,
                extend_only: true,
            },
            150,
        )
        .unwrap();
    assert_eq!(pending.terminate_expired(250).unwrap(), pending);
    let terminating = pending.terminate_expired(600).unwrap();
    assert_eq!(
        terminating.sandboxes()[0].phase(),
        SessionPhase::Terminating
    );
    assert_eq!(terminating.identity_frontiers(), (1, 3));
    assert_eq!(terminating.reserved(None, None), Some(resources(1)));
    assert!(!terminating.sandboxes()[0].released());
}

#[test]
fn startup_expiry_preserves_both_owner_rows_and_their_reserved_destroy_capacity() {
    let (first, _) = state()
        .accept_lifecycle(&"1".try_into().unwrap(), &create("1", 1), 10)
        .unwrap();
    let (both, _) = first
        .accept_lifecycle(&"2".try_into().unwrap(), &create("2", 1), 10)
        .unwrap();
    let terminating = both.terminate_expired(1010).unwrap();
    assert_eq!(terminating.sandboxes().len(), 2);
    assert_eq!(terminating.identity_frontiers(), (2, 4));
    assert!(
        terminating
            .sandboxes()
            .iter()
            .all(|record| record.terminating() && !record.released())
    );
    assert_eq!(terminating.reserved(None, None), Some(resources(2)));
}

#[test]
fn recovery_rejects_malformed_lanes_duplicate_occupancy_and_changed_frontiers() {
    let (first, _) = state()
        .accept_lifecycle(&"1".try_into().unwrap(), &create("1", 1), 10)
        .unwrap();
    let (both, _) = first
        .accept_lifecycle(&"2".try_into().unwrap(), &create("2", 1), 10)
        .unwrap();
    assert!(both.is_valid());
    for case in [
        "zero_slot",
        "duplicate_occupancy",
        "wrong_frontier",
        "empty_operations",
    ] {
        let mut value = serde_json::to_value(&both).unwrap();
        match case {
            "zero_slot" => value["retention_lanes"][0]["retention_slot"] = 0.into(),
            "duplicate_occupancy" => {
                let slot =
                    value["sandboxes"][0]["operations"][0]["intent"]["retention"]["slot"].clone();
                value["sandboxes"][1]["operations"][0]["intent"]["retention"]["slot"] = slot;
            }
            "wrong_frontier" => {
                value["sandboxes"][0]["operations"][0]["intent"]["retention"]["window"] = 2.into();
            }
            "empty_operations" => value["sandboxes"][0]["operations"] = serde_json::json!([]),
            _ => unreachable!(),
        }
        let invalid: CenterState = serde_json::from_value(value).unwrap();
        assert!(!invalid.is_valid(), "invalid lane case accepted: {case}");
    }
}
