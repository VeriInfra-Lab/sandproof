//! Current production envelope guards; no network, journal or host fixture.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext, CaptureId,
    CenterIdentity, ExpectedSession, IncarnationId, LifecycleOperation, LifecycleRequest,
    OperationBinding, OperationKey, PROTOCOL_VERSION, RecordedLifecycleIntent, RetentionBinding,
    ValidationError,
};

fn original() -> (AcceptedLifecycleIntent, AuthenticatedCenterContext) {
    let node = NodeIdentity {
        deployment_id: "deployment".try_into().unwrap(),
        node_id: "host".try_into().unwrap(),
        node_generation: 1.try_into().unwrap(),
    };
    let center = CenterIdentity {
        deployment: node.deployment_id.clone(),
        center: "center".try_into().unwrap(),
    };
    let intent = AcceptedLifecycleIntent {
        request: LifecycleRequest {
            binding: OperationBinding {
                protocol_version: PROTOCOL_VERSION,
                node: node.clone(),
                session: "session".try_into().unwrap(),
                key: OperationKey {
                    deployment: node.deployment_id.clone(),
                    owner: "owner".try_into().unwrap(),
                    window: 1.try_into().unwrap(),
                    sequence: 1.try_into().unwrap(),
                },
                expected: ExpectedSession::Version(0),
            },
            operation: LifecycleOperation::Unload {
                source_incarnation: IncarnationId::try_from(1).unwrap(),
                capture: CaptureId::try_from(1).unwrap(),
            },
        },
        accepted_version: 1,
        center: center.clone(),
        retention: RetentionBinding {
            slot: 1,
            window: 1.try_into().unwrap(),
        },
    };
    (
        intent,
        AuthenticatedCenterContext::from_verified_peer_registration(center, node),
    )
}

fn receipt(intent: &AcceptedLifecycleIntent) -> RecordedLifecycleIntent {
    RecordedLifecycleIntent {
        intent: intent.clone(),
        current_session_version: intent.accepted_version,
        current: true,
        terminal: false,
    }
}

#[test]
fn independent_center_and_node_are_checked_before_the_remaining_shape() {
    let (intent, context) = original();
    assert_eq!(intent.validate(&context), Ok(()));
    let mut foreign = intent.clone();
    foreign.center.center = "another-center".try_into().unwrap();
    foreign.request.binding.node.node_generation = 2.try_into().unwrap();
    assert_eq!(
        foreign.validate(&context),
        Err(AcceptedIntentError::ForeignCenter)
    );
    foreign.center = intent.center.clone();
    assert_eq!(
        foreign.validate(&context),
        Err(AcceptedIntentError::Request(ValidationError::ForeignNode))
    );
    let mut wrong_deployment = intent;
    wrong_deployment.center.deployment = "foreign".try_into().unwrap();
    let independent = AuthenticatedCenterContext::from_verified_peer_registration(
        wrong_deployment.center.clone(),
        context.expected_node().clone(),
    );
    assert_eq!(
        wrong_deployment.validate(&independent),
        Err(AcceptedIntentError::ForeignDeployment)
    );
}

#[test]
fn request_retention_and_version_keep_original_rejection_order() {
    let (intent, context) = original();
    for slot in [0, 4097] {
        let mut changed = intent.clone();
        changed.retention.slot = slot;
        assert_eq!(
            changed.validate(&context),
            Err(AcceptedIntentError::InvalidRetention)
        );
    }
    let mut changed = intent.clone();
    changed.accepted_version = 0;
    assert_eq!(
        changed.validate(&context),
        Err(AcceptedIntentError::InvalidAcceptedVersion)
    );
    changed.request.binding.expected = ExpectedSession::Version(u64::MAX);
    assert_eq!(changed.accepted_version_for_request(), None);
    assert_eq!(
        changed.validate(&context),
        Err(AcceptedIntentError::InvalidAcceptedVersion)
    );
    changed.request.binding.protocol_version = 0;
    changed.retention.slot = 0;
    assert_eq!(
        changed.validate(&context),
        Err(AcceptedIntentError::Request(
            ValidationError::UnsupportedVersion
        ))
    );
    let mut upper = intent;
    upper.retention.slot = 4096;
    upper.retention.window = u64::MAX.try_into().unwrap();
    assert_eq!(upper.validate(&context), Ok(()));
}

#[test]
fn same_session_changed_full_intent_is_rejected() {
    let (intent, context) = original();
    assert_eq!(intent.validate(&context), Ok(()));
    assert!(receipt(&intent).matches(&intent));
    let mut changed = receipt(&intent);
    changed.intent.request.operation = LifecycleOperation::Unload {
        source_incarnation: IncarnationId::try_from(1).unwrap(),
        capture: CaptureId::try_from(2).unwrap(),
    };
    assert_eq!(changed.intent.validate(&context), Ok(()));
    assert_eq!(
        changed.intent.request.binding.session,
        intent.request.binding.session
    );
    assert_ne!(changed.intent, intent);
    assert!(
        !changed.matches(&intent),
        "same session cannot replace the complete original accepted intent"
    );
}

#[test]
fn all_registered_envelope_fields_remain_correlated() {
    let (intent, _) = original();
    let mutations: [fn(&mut AcceptedLifecycleIntent); 10] = [
        |i| i.center.center = "another-center".try_into().unwrap(),
        |i| i.center.deployment = "foreign".try_into().unwrap(),
        |i| i.retention.slot = 2,
        |i| i.retention.window = 2.try_into().unwrap(),
        |i| i.accepted_version = 2,
        |i| i.request.binding.key.owner = "other-owner".try_into().unwrap(),
        |i| i.request.binding.key.sequence = 2.try_into().unwrap(),
        |i| i.request.binding.key.window = 2.try_into().unwrap(),
        |i| i.request.binding.expected = ExpectedSession::Version(7),
        |i| i.request.binding.node.node_generation = 2.try_into().unwrap(),
    ];
    for (index, apply) in mutations.into_iter().enumerate() {
        let mut changed = receipt(&intent);
        apply(&mut changed.intent);
        assert_eq!(
            changed.intent.request.binding.session,
            intent.request.binding.session
        );
        assert_ne!(changed.intent, intent);
        assert!(
            !changed.matches(&intent),
            "changed complete envelope field {index}"
        );
    }
}

#[test]
fn current_version_flags_do_not_certify_terminal_truth() {
    let (intent, _) = original();
    let mut view = receipt(&intent);
    assert!(view.matches(&intent));
    view.current = false;
    assert!(!view.matches(&intent));
    view.current_session_version = 2;
    assert!(view.matches(&intent));
    view.current = true;
    assert!(!view.matches(&intent));
    view.current_session_version = 0;
    view.current = false;
    assert!(!view.matches(&intent));
    let mut original_view = receipt(&intent);
    original_view.terminal = true;
    assert!(original_view.matches(&intent));
}

#[test]
fn matching_a_receipt_does_not_establish_input_validation() {
    let (mut intent, context) = original();
    intent.accepted_version = 0;
    intent.retention.slot = 0;
    assert!(receipt(&intent).matches(&intent));
    assert_eq!(
        intent.validate(&context),
        Err(AcceptedIntentError::InvalidRetention)
    );
}

#[test]
fn renewal_shape_does_not_check_the_business_deadline() {
    let (mut intent, context) = original();
    intent.request.operation = LifecycleOperation::Renew {
        incarnation: IncarnationId::try_from(1).unwrap(),
        end_at_ms: -1,
        extend_only: false,
    };
    assert_eq!(intent.validate(&context), Ok(()));
}

#[test]
fn unload_outcomes_are_exclusive_and_old_optional_shape_is_rejected() {
    use sandproof_protocol::lifecycle::{CurrentReadyFacts, UnloadObservation, UnloadOutcome, UnloadedFacts, ContentDigest};
    let (request, _) = original();
    let running = CurrentReadyFacts { started_at_ms: 1, startup_deadline_ms: 2, observed_at_ms: 3, end_at_ms: 4, max_end_at_ms: 5 };
    let unloaded = UnloadedFacts { source_incarnation: 1.try_into().unwrap(), capture: 1.try_into().unwrap(), manifest_digest: ContentDigest::from_sha256([1;32]), material_bytes: 1, started_at_ms: 1, startup_deadline_ms: 2, end_at_ms: 4, max_end_at_ms: 5 };
    for outcome in [UnloadOutcome::Pending, UnloadOutcome::Unloaded { facts: unloaded }, UnloadOutcome::OriginalContinued { facts: running }] {
        let observation = UnloadObservation { request: request.clone(), outcome };
        assert!(observation.matches_request(&request));
        let encoded = serde_json::to_vec(&observation).unwrap();
        assert_eq!(serde_json::from_slice::<UnloadObservation>(&encoded).unwrap(), observation);
        let mut changed = observation.clone();
        changed.request.accepted_version += 1;
        assert!(!changed.matches_request(&request));
        let mut value = serde_json::to_value(&observation).unwrap();
        value["unloaded"] = serde_json::Value::Null;
        assert!(serde_json::from_value::<UnloadObservation>(value).is_err());
    }
    let old = serde_json::json!({"request": request, "unloaded": null});
    assert!(serde_json::from_value::<UnloadObservation>(old).is_err());
}

#[test]
fn original_continued_does_not_accept_expired_or_unstarted_running_facts() {
    use sandproof_protocol::lifecycle::{CurrentReadyFacts, UnloadObservation, UnloadOutcome};
    let (request, _) = original();
    for observed_at_ms in [0, 4, 5] {
        let observation = UnloadObservation { request: request.clone(), outcome: UnloadOutcome::OriginalContinued { facts: CurrentReadyFacts { started_at_ms: 1, startup_deadline_ms: 2, observed_at_ms, end_at_ms: 4, max_end_at_ms: 5 } } };
        assert!(!observation.matches_request(&request));
    }
}
