//! Full decoded Resume observation values; no durable or physical claim.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ContentDigest,
    CurrentReadyFacts, CurrentResumeReadiness, ExpectedSession, LifecycleOperation,
    LifecycleRequest, OperationBinding, OperationKey, PROTOCOL_VERSION, ResumeObservation,
    RetentionBinding,
};
use sandproof_stage1_resume_observation_contract::matches;

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
fn resume() -> AcceptedLifecycleIntent {
    intent(
        LifecycleOperation::Resume {
            source_incarnation: 1.try_into().unwrap(),
            capture: 1.try_into().unwrap(),
            target_incarnation: 3.try_into().unwrap(),
            manifest_digest: ContentDigest::from_sha256([0; 32]),
        },
        2,
        ExpectedSession::Version(1),
    )
}
fn renewal(end_at_ms: i64, extend_only: bool) -> AcceptedLifecycleIntent {
    let mut current = intent(
        LifecycleOperation::Renew {
            incarnation: 3.try_into().unwrap(),
            end_at_ms,
            extend_only,
        },
        3,
        ExpectedSession::Version(2),
    );
    current.request.binding.key.window = 2.try_into().unwrap();
    current.request.binding.key.sequence = 2.try_into().unwrap();
    current
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

fn observation(
    current: AcceptedLifecycleIntent,
    readiness: CurrentResumeReadiness,
) -> ResumeObservation {
    ResumeObservation {
        request: resume(),
        current_request: current,
        readiness,
    }
}
fn unload() -> AcceptedLifecycleIntent {
    let mut result = renewal(50, false);
    result.request.operation = LifecycleOperation::Unload {
        source_incarnation: 3.try_into().unwrap(),
        capture: 2.try_into().unwrap(),
    };
    result
}
fn destroy() -> AcceptedLifecycleIntent {
    let mut result = renewal(50, false);
    result.request.operation = LifecycleOperation::Destroy {};
    result
}
#[test]
fn legal_original_and_successors() {
    let original = resume();
    for current in [original.clone(), renewal(50, false), unload(), destroy()] {
        assert_eq!(current.validate(&context(&current)), Ok(()));
    }
    for current in [original.clone(), renewal(50, false), unload()] {
        assert!(matches(
            &observation(current, CurrentResumeReadiness::Observed { facts: facts() }),
            &original
        ));
    }
    assert!(matches(
        &observation(destroy(), CurrentResumeReadiness::NotCurrent),
        &original
    ));
}
#[test]
fn full_original_is_not_session_only() {
    let mut value = observation(resume(), CurrentResumeReadiness::Pending);
    value.request.retention.window = 2.try_into().unwrap();
    assert_eq!(value.request.validate(&context(&value.request)), Ok(()));
    assert_eq!(
        value.request.request.binding.session,
        resume().request.binding.session
    );
    assert!(
        !matches(&value, &resume()),
        "same session does not identify the complete original"
    );
}
#[test]
fn current_retention_is_not_omitted() {
    let mut current = renewal(50, false);
    current.retention.window = 2.try_into().unwrap();
    assert_eq!(current.validate(&context(&current)), Ok(()));
    assert!(
        !matches(
            &observation(current, CurrentResumeReadiness::Pending),
            &resume()
        ),
        "resume must preserve current retention"
    );
}
#[test]
fn lost_unload_keeps_continued_source() {
    let mut current = unload();
    current.request.operation = LifecycleOperation::Unload {
        source_incarnation: 1.try_into().unwrap(),
        capture: 2.try_into().unwrap(),
    };
    assert_eq!(current.validate(&context(&current)), Ok(()));
    assert!(
        !matches(
            &observation(current, CurrentResumeReadiness::StateLost),
            &resume()
        ),
        "StateLost must preserve continued source incarnation"
    );
}
#[test]
fn renewal_incarnation_is_not_omitted() {
    let mut current = renewal(50, false);
    current.request.operation = LifecycleOperation::Renew {
        incarnation: 1.try_into().unwrap(),
        end_at_ms: 50,
        extend_only: false,
    };
    assert_eq!(current.validate(&context(&current)), Ok(()));
    assert!(
        !matches(
            &observation(current, CurrentResumeReadiness::Pending),
            &resume()
        ),
        "renew must target original resumed incarnation"
    );
}
#[test]
fn state_lost_value_is_allowed_for_original_and_successors() {
    for current in [resume(), renewal(50, false), unload()] {
        assert_eq!(current.validate(&context(&current)), Ok(()));
        assert!(matches(
            &observation(current, CurrentResumeReadiness::StateLost),
            &resume()
        ));
    }
}
#[test]
fn negative_renewal_pending_is_allowed() {
    let current = renewal(-1, false);
    assert_eq!(current.validate(&context(&current)), Ok(()));
    assert!(matches(
        &observation(current, CurrentResumeReadiness::Pending),
        &resume()
    ));
}
#[test]
fn negative_renewal_observed_is_rejected() {
    let maximum = CurrentReadyFacts {
        end_at_ms: u64::MAX,
        max_end_at_ms: u64::MAX,
        ..facts()
    };
    assert!(!matches(
        &observation(
            renewal(-1, false),
            CurrentResumeReadiness::Observed { facts: maximum }
        ),
        &resume()
    ));
}
#[test]
fn not_current_needs_a_successor() {
    assert!(!matches(
        &observation(resume(), CurrentResumeReadiness::NotCurrent),
        &resume()
    ));
    assert!(matches(
        &observation(destroy(), CurrentResumeReadiness::NotCurrent),
        &resume()
    ));
}
#[test]
fn expired_observation_is_rejected() {
    let expired = CurrentReadyFacts {
        observed_at_ms: 50,
        ..facts()
    };
    assert!(!matches(
        &observation(
            resume(),
            CurrentResumeReadiness::Observed { facts: expired }
        ),
        &resume()
    ));
}

#[test]
fn lost_unload_keeps_full_binding() {
    let original = resume();
    let mut reused_key = unload();
    reused_key.request.binding.key = original.request.binding.key.clone();
    let mut old_version = unload();
    old_version.accepted_version = 2;
    old_version.request.binding.expected = ExpectedSession::Version(1);
    let mut wrong_center = unload();
    wrong_center.center.center = "other-center".try_into().unwrap();
    let mut wrong_retention = unload();
    wrong_retention.retention.window = 2.try_into().unwrap();
    let mut wrong_session = unload();
    wrong_session.request.binding.session = "other-session".try_into().unwrap();
    for current in [
        reused_key,
        old_version,
        wrong_center,
        wrong_retention,
        wrong_session,
    ] {
        assert_eq!(current.validate(&context(&current)), Ok(()));
        assert!(!matches(
            &observation(current, CurrentResumeReadiness::StateLost),
            &original
        ));
    }
    let mut changed_original = observation(unload(), CurrentResumeReadiness::StateLost);
    changed_original.request.retention.window = 2.try_into().unwrap();
    assert!(!matches(&changed_original, &original));
}
