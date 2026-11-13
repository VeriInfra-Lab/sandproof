//! Actual Unload guard and projections: decoded values, no physical custody/exit claim.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ContentDigest,
    CurrentReadyFacts, ExpectedSession, LifecycleOperation, LifecycleRequest, OperationBinding,
    OperationKey, PROTOCOL_VERSION, RetentionBinding, UnloadObservation, UnloadOutcome,
    UnloadedFacts,
};
use sandproof_stage1_unload_observation_extract::{
    continued_facts, unload_matches, unloaded_facts,
};

fn intent(
    operation: LifecycleOperation,
    version: u64,
    previous: ExpectedSession,
) -> AcceptedLifecycleIntent {
    let node = NodeIdentity {
        deployment_id: "deployment".try_into().unwrap(),
        node_id: "host".try_into().unwrap(),
        node_generation: 1.try_into().unwrap(),
    };
    AcceptedLifecycleIntent {
        center: CenterIdentity {
            deployment: node.deployment_id.clone(),
            center: "center".try_into().unwrap(),
        },
        request: LifecycleRequest {
            binding: OperationBinding {
                protocol_version: PROTOCOL_VERSION,
                node: node.clone(),
                session: "session".try_into().unwrap(),
                key: OperationKey {
                    deployment: node.deployment_id,
                    owner: "owner".try_into().unwrap(),
                    window: 1.try_into().unwrap(),
                    sequence: 1.try_into().unwrap(),
                },
                expected: previous,
            },
            operation,
        },
        accepted_version: version,
        retention: RetentionBinding {
            slot: 1,
            window: 1.try_into().unwrap(),
        },
    }
}
fn context(intent: &AcceptedLifecycleIntent) -> AuthenticatedCenterContext {
    AuthenticatedCenterContext::from_verified_peer_registration(
        intent.center.clone(),
        intent.request.binding.node.clone(),
    )
}
fn facts() -> CurrentReadyFacts {
    CurrentReadyFacts {
        started_at_ms: 10,
        end_at_ms: 50,
        max_end_at_ms: 100,
        startup_deadline_ms: 20,
        observed_at_ms: 30,
    }
}

fn original() -> AcceptedLifecycleIntent {
    intent(
        LifecycleOperation::Unload {
            source_incarnation: 3.try_into().unwrap(),
            capture: 2.try_into().unwrap(),
        },
        3,
        ExpectedSession::Version(2),
    )
}
fn payload() -> UnloadedFacts {
    UnloadedFacts {
        source_incarnation: 3.try_into().unwrap(),
        capture: 2.try_into().unwrap(),
        manifest_digest: ContentDigest::from_sha256([0; 32]),
        material_bytes: 64,
        started_at_ms: 10,
        end_at_ms: 50,
        max_end_at_ms: 100,
        startup_deadline_ms: 20,
    }
}
fn observation(outcome: UnloadOutcome) -> UnloadObservation {
    UnloadObservation {
        request: original(),
        outcome,
    }
}
#[test]
fn legal_outcomes_match() {
    let original = original();
    assert_eq!(original.validate(&context(&original)), Ok(()));
    for outcome in [
        UnloadOutcome::Pending,
        UnloadOutcome::Unloaded { facts: payload() },
        UnloadOutcome::OriginalContinued { facts: facts() },
    ] {
        assert!(unload_matches(&observation(outcome), &original));
    }
}
#[test]
fn full_original_is_not_key_only() {
    let mut value = observation(UnloadOutcome::Pending);
    value.request.retention.window = 2.try_into().unwrap();
    assert_eq!(value.request.validate(&context(&value.request)), Ok(()));
    assert_eq!(
        value.request.request.binding.key,
        original().request.binding.key
    );
    assert!(
        !unload_matches(&value, &original()),
        "unload must bind complete original intent"
    );
}
#[test]
fn capture_is_not_omitted() {
    let mut facts = payload();
    facts.capture = 1.try_into().unwrap();
    assert!(
        !unload_matches(&observation(UnloadOutcome::Unloaded { facts }), &original()),
        "unload must preserve original capture"
    );
}
#[test]
fn zero_material_is_rejected() {
    let facts = UnloadedFacts {
        material_bytes: 0,
        ..payload()
    };
    assert!(
        !unload_matches(&observation(UnloadOutcome::Unloaded { facts }), &original()),
        "unloaded value must contain positive material bytes"
    );
}
#[test]
fn expired_continuation_is_rejected() {
    let facts = CurrentReadyFacts {
        observed_at_ms: 50,
        ..facts()
    };
    assert!(
        !unload_matches(
            &observation(UnloadOutcome::OriginalContinued { facts }),
            &original()
        ),
        "continued observation at deadline must be rejected"
    );
}
#[test]
fn matching_does_not_establish_validation() {
    let mut original = original();
    original.request.binding.protocol_version = 3;
    assert!(original.validate(&context(&original)).is_err());
    let value = UnloadObservation {
        request: original.clone(),
        outcome: UnloadOutcome::Pending,
    };
    assert!(unload_matches(&value, &original));
}
#[test]
fn matching_does_not_establish_digest_truth() {
    let mut facts = payload();
    facts.manifest_digest = ContentDigest::from_sha256([255; 32]);
    assert!(unload_matches(
        &observation(UnloadOutcome::Unloaded { facts }),
        &original()
    ));
}
#[test]
fn projections_preserve_complete_payloads() {
    let payload = payload();
    let unloaded = UnloadOutcome::Unloaded {
        facts: payload.clone(),
    };
    assert_eq!(unloaded_facts(&unloaded), Some(&payload));
    assert_eq!(continued_facts(&unloaded), None);
    let facts = facts();
    let continued = UnloadOutcome::OriginalContinued {
        facts: facts.clone(),
    };
    assert_eq!(continued_facts(&continued), Some(&facts));
    assert_eq!(unloaded_facts(&continued), None);
    assert_eq!(unloaded_facts(&UnloadOutcome::Pending), None);
    assert_eq!(continued_facts(&UnloadOutcome::Pending), None);
}
