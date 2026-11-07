//! Actual decoded Create observation assertions, not evidence of journal/VM truth.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};
use sandproof_protocol::lifecycle::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity,
    ContentDigest, CreateObservation, CreateRefusalReason, CurrentCreateReadiness,
    CurrentReadyFacts, ExpectedSession, LifecycleOperation, LifecycleRequest, OperationBinding,
    OperationKey, PROTOCOL_VERSION, RetentionBinding, ValidationError,
};
use sandproof_stage1_create_observation_contract::matches;

fn original() -> AcceptedLifecycleIntent {
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
        retention: RetentionBinding {
            slot: 1,
            window: 1.try_into().unwrap(),
        },
        accepted_version: 1,
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
                expected: ExpectedSession::Absent,
            },
            operation: LifecycleOperation::Create {
                create: CreateBinding {
                    template_revision: "base".try_into().unwrap(),
                    template_digest: [0; 32],
                    resources: Resources {
                        cpu: 1,
                        memory_bytes: 1,
                        disk_bytes: 1,
                        instances: 1,
                    },
                    options: vec![1],
                    startup_deadline_ms: 20,
                    timeout_ms: 40,
                    max_lifetime_ms: 90,
                },
                target_incarnation: 1.try_into().unwrap(),
            },
        },
    }
}
fn context(value: &AcceptedLifecycleIntent) -> AuthenticatedCenterContext {
    AuthenticatedCenterContext::from_verified_peer_registration(
        value.center.clone(),
        value.request.binding.node.clone(),
    )
}
fn newer(operation: LifecycleOperation) -> AcceptedLifecycleIntent {
    let mut value = original();
    value.accepted_version = 2;
    value.request.binding.expected = ExpectedSession::Version(1);
    value.request.binding.key.window = 2.try_into().unwrap();
    value.request.binding.key.sequence = 2.try_into().unwrap();
    value.request.operation = operation;
    value
}
fn renewal(end_at_ms: i64, extend_only: bool) -> AcceptedLifecycleIntent {
    newer(LifecycleOperation::Renew {
        incarnation: 1.try_into().unwrap(),
        end_at_ms,
        extend_only,
    })
}
fn unload() -> AcceptedLifecycleIntent {
    newer(LifecycleOperation::Unload {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    })
}
fn destroy() -> AcceptedLifecycleIntent {
    newer(LifecycleOperation::Destroy {})
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
    readiness: CurrentCreateReadiness,
    historical_create_succeeded: bool,
) -> CreateObservation {
    CreateObservation {
        request: original(),
        current_request: current,
        historical_create_succeeded,
        readiness,
    }
}
fn refused() -> CurrentCreateReadiness {
    CurrentCreateReadiness::Refused {
        reason: CreateRefusalReason::Capacity,
    }
}

#[test]
fn legal_original_and_successors() {
    let original = original();
    for current in [original.clone(), renewal(50, false), unload(), destroy()] {
        assert_eq!(current.validate(&context(&current)), Ok(()));
    }
    for current in [original.clone(), renewal(50, false), unload()] {
        for readiness in [
            CurrentCreateReadiness::Pending,
            CurrentCreateReadiness::NotRunning,
            CurrentCreateReadiness::Expired,
        ] {
            assert!(matches(
                &observation(current.clone(), readiness, false),
                &original
            ));
        }
        assert!(matches(
            &observation(
                current,
                CurrentCreateReadiness::Observed { facts: facts() },
                true
            ),
            &original
        ));
    }
    assert!(matches(
        &observation(destroy(), CurrentCreateReadiness::NotCurrent, false),
        &original
    ));
    let resume = newer(LifecycleOperation::Resume {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
        target_incarnation: 3.try_into().unwrap(),
        manifest_digest: ContentDigest::from_sha256([0; 32]),
    });
    assert_eq!(resume.validate(&context(&resume)), Ok(()));
    assert!(matches(
        &observation(resume, CurrentCreateReadiness::NotCurrent, true),
        &original
    ));
}

#[test]
fn response_keeps_complete_original() {
    let original = original();
    let mut value = observation(original.clone(), CurrentCreateReadiness::Pending, false);
    let LifecycleOperation::Create { create, .. } = &mut value.request.request.operation else {
        unreachable!()
    };
    create.timeout_ms = 30;
    assert_eq!(value.request.validate(&context(&original)), Ok(()));
    assert_eq!(
        value.request.request.binding.key,
        original.request.binding.key
    );
    assert!(
        !matches(&value, &original),
        "create response must retain complete original"
    );
}

#[test]
fn current_retention_is_preserved() {
    let original = original();
    let mut current = renewal(50, false);
    current.retention.window = 2.try_into().unwrap();
    assert_eq!(current.validate(&context(&original)), Ok(()));
    assert!(
        !matches(
            &observation(current, CurrentCreateReadiness::Pending, false),
            &original
        ),
        "create observation must preserve current retention"
    );
}

#[test]
fn legal_refusal_and_refused_destroy() {
    let original = original();
    for current in [original.clone(), destroy()] {
        assert!(matches(&observation(current, refused(), false), &original));
    }
    assert!(!matches(
        &observation(renewal(50, false), refused(), false),
        &original
    ));
    let mut reused_key = destroy();
    reused_key.request.binding.key = original.request.binding.key.clone();
    assert!(!matches(
        &observation(reused_key, refused(), false),
        &original
    ));
}

#[test]
fn refused_cannot_follow_historical_success() {
    let original = original();
    assert_eq!(original.validate(&context(&original)), Ok(()));
    assert!(
        !matches(&observation(original.clone(), refused(), true), &original),
        "refused must reject historical success"
    );
}

#[test]
fn observed_requires_historical_success() {
    let original = original();
    for current in [original.clone(), renewal(50, false), unload()] {
        assert!(!matches(
            &observation(
                current,
                CurrentCreateReadiness::Observed { facts: facts() },
                false
            ),
            &original
        ));
    }
}

#[test]
fn negative_renewal_pending_is_allowed() {
    let current = renewal(-2, false);
    assert_eq!(current.validate(&context(&original())), Ok(()));
    assert!(matches(
        &observation(current, CurrentCreateReadiness::Pending, false),
        &original()
    ));
}

#[test]
fn negative_renewal_cannot_wrap_into_observation() {
    let mut original = original();
    let LifecycleOperation::Create { create, .. } = &mut original.request.operation else {
        unreachable!()
    };
    create.startup_deadline_ms = 1;
    create.max_lifetime_ms = u64::MAX - 1;
    assert_eq!(original.validate(&context(&original)), Ok(()));
    let current = renewal(-2, false);
    assert_eq!(current.validate(&context(&original)), Ok(()));
    let value = CreateObservation {
        request: original.clone(),
        current_request: current,
        historical_create_succeeded: true,
        readiness: CurrentCreateReadiness::Observed {
            facts: CurrentReadyFacts {
                started_at_ms: 0,
                startup_deadline_ms: 1,
                observed_at_ms: 0,
                end_at_ms: u64::MAX - 1,
                max_end_at_ms: u64::MAX - 1,
            },
        },
    };
    assert!(
        !matches(&value, &original),
        "negative renewal must not wrap into an observed deadline"
    );
}

#[test]
fn renewal_keeps_incarnation_and_lifetime() {
    let original = original();
    let mut wrong_incarnation = renewal(50, false);
    wrong_incarnation.request.operation = LifecycleOperation::Renew {
        incarnation: 2.try_into().unwrap(),
        end_at_ms: 50,
        extend_only: false,
    };
    assert!(!matches(
        &observation(
            wrong_incarnation,
            CurrentCreateReadiness::Observed { facts: facts() },
            true
        ),
        &original
    ));
    for field in 0..3 {
        let mut changed = facts();
        match field {
            0 => changed.startup_deadline_ms += 1,
            1 => changed.max_end_at_ms += 1,
            2 => changed.end_at_ms += 1,
            _ => unreachable!(),
        }
        assert!(!matches(
            &observation(
                renewal(50, false),
                CurrentCreateReadiness::Observed { facts: changed },
                true
            ),
            &original
        ));
    }
    let mut later = facts();
    later.end_at_ms = 60;
    assert!(matches(
        &observation(
            renewal(50, true),
            CurrentCreateReadiness::Observed { facts: later },
            true
        ),
        &original
    ));
}

#[test]
fn continued_unload_keeps_lineage_and_timing() {
    let original = original();
    let mut bad = unload();
    bad.request.operation = LifecycleOperation::Unload {
        source_incarnation: 2.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    };
    assert!(!matches(
        &observation(bad, CurrentCreateReadiness::Pending, false),
        &original
    ));
    let mut reused_key = unload();
    reused_key.request.binding.key = original.request.binding.key.clone();
    assert!(!matches(
        &observation(reused_key, CurrentCreateReadiness::Pending, false),
        &original
    ));
    let mut expired = facts();
    expired.observed_at_ms = expired.end_at_ms;
    assert!(!matches(
        &observation(
            unload(),
            CurrentCreateReadiness::Observed { facts: expired },
            true
        ),
        &original
    ));
}

#[test]
fn not_current_needs_newer_value() {
    let original = original();
    assert!(!matches(
        &observation(original.clone(), CurrentCreateReadiness::NotCurrent, false),
        &original
    ));
    let mut same_version = destroy();
    same_version.accepted_version = 1;
    same_version.request.binding.expected = ExpectedSession::Version(0);
    assert_eq!(same_version.validate(&context(&original)), Ok(()));
    assert!(!matches(
        &observation(same_version, CurrentCreateReadiness::NotCurrent, false),
        &original
    ));
}

#[test]
fn renewal_and_not_current_do_not_require_a_new_key() {
    let original = original();
    let mut renewed = renewal(50, false);
    renewed.request.binding.key = original.request.binding.key.clone();
    assert_eq!(renewed.validate(&context(&original)), Ok(()));
    assert!(matches(
        &observation(
            renewed,
            CurrentCreateReadiness::Observed { facts: facts() },
            true
        ),
        &original
    ));
    let mut destroyed = destroy();
    destroyed.request.binding.key = original.request.binding.key.clone();
    assert!(matches(
        &observation(destroyed, CurrentCreateReadiness::NotCurrent, false),
        &original
    ));
}

#[test]
fn changed_non_refused_does_not_revalidate_original() {
    let mut original = original();
    let mut value = observation(destroy(), CurrentCreateReadiness::NotCurrent, false);
    original.request.binding.protocol_version = PROTOCOL_VERSION - 1;
    value.request = original.clone();
    assert_eq!(
        original.validate(&context(&original)),
        Err(AcceptedIntentError::Request(
            ValidationError::UnsupportedVersion
        ))
    );
    assert!(matches(&value, &original));
}

#[test]
fn complete_current_identity_is_checked() {
    for field in 0..8 {
        let original = original();
        let mut current = renewal(50, false);
        match field {
            0 => current.center.center = "other".try_into().unwrap(),
            1 => current.center.deployment = "other".try_into().unwrap(),
            2 => current.request.binding.node.node_generation = 2.try_into().unwrap(),
            3 => current.request.binding.session = "other".try_into().unwrap(),
            4 => current.request.binding.key.owner = "other".try_into().unwrap(),
            5 => current.request.binding.key.deployment = "other".try_into().unwrap(),
            6 => current.request.binding.protocol_version = PROTOCOL_VERSION - 1,
            7 => current.accepted_version = 3,
            _ => unreachable!(),
        }
        assert!(
            !matches(
                &observation(current, CurrentCreateReadiness::Pending, false),
                &original
            ),
            "bad current identity {field} accepted"
        );
    }
}
