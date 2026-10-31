//! Decoded value contracts only; no journal, authorization or physical facts.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ContentDigest,
    CurrentReadyFacts, ExpectedSession, LifecycleOperation, LifecycleRequest, OperationBinding,
    OperationKey, PROTOCOL_VERSION, RetentionBinding, UnloadedFacts,
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
fn create(timeout_ms: u64) -> AcceptedLifecycleIntent {
    intent(
        LifecycleOperation::Create {
            create: CreateBinding {
                template_revision: "base".try_into().unwrap(),
                template_digest: [0; 32],
                resources: Resources {
                    instances: 1,
                    cpu: 1,
                    memory_bytes: 1,
                    disk_bytes: 1,
                },
                options: vec![1],
                startup_deadline_ms: 20,
                timeout_ms,
                max_lifetime_ms: 90,
            },
            target_incarnation: 1.try_into().unwrap(),
        },
        1,
        ExpectedSession::Absent,
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
fn source() -> UnloadedFacts {
    UnloadedFacts {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
        manifest_digest: ContentDigest::from_sha256([0; 32]),
        material_bytes: 64,
        started_at_ms: 10,
        end_at_ms: 50,
        max_end_at_ms: 100,
        startup_deadline_ms: 20,
    }
}
#[test]
fn legal_values_match_all_three_guards() {
    let create = create(40);
    let resume = resume();
    let renewal = renewal(60, false);
    for intent in [&create, &resume, &renewal] {
        assert_eq!(intent.validate(&context(intent)), Ok(()));
    }
    assert!(facts().matches_create(&create));
    assert!(facts().matches_resumed(&resume, &source()));
    assert!(
        CurrentReadyFacts {
            end_at_ms: 60,
            ..facts()
        }
        .matches_resumed_renewal(&resume, &renewal, &source())
    );
}
#[test]
fn create_timeout_is_not_omitted() {
    let changed = create(30);
    assert_eq!(changed.validate(&context(&changed)), Ok(()));
    assert!(
        !facts().matches_create(&changed),
        "create must preserve exact requested timeout"
    );
}
#[test]
fn source_digest_is_not_omitted() {
    let mut changed = source();
    let mut bytes = [0; 32];
    bytes[0] = 1;
    changed.manifest_digest = ContentDigest::from_sha256(bytes);
    assert!(
        !facts().matches_resumed(&resume(), &changed),
        "resume must preserve complete source digest"
    );
}
#[test]
fn expired_observation_is_rejected() {
    let expired = CurrentReadyFacts {
        observed_at_ms: 50,
        ..facts()
    };
    assert!(
        !expired.matches_resumed(&resume(), &source()),
        "observation at the exact deadline is expired"
    );
}
#[test]
fn negative_deadline_is_rejected_before_cast() {
    let current = renewal(-1, false);
    assert_eq!(current.validate(&context(&current)), Ok(()));
    let facts = CurrentReadyFacts {
        end_at_ms: u64::MAX,
        max_end_at_ms: u64::MAX,
        ..facts()
    };
    let source = UnloadedFacts {
        max_end_at_ms: u64::MAX,
        ..source()
    };
    assert!(
        !facts.matches_resumed_renewal(&resume(), &current, &source),
        "negative deadline must not become u64 max"
    );
}
#[test]
fn value_matching_does_not_certify_material_bytes() {
    let empty = UnloadedFacts {
        material_bytes: 0,
        ..source()
    };
    assert!(facts().matches_resumed(&resume(), &empty));
}
#[test]
fn extend_only_leaf_does_not_compare_previous_end() {
    let shorter = CurrentReadyFacts {
        end_at_ms: 40,
        ..facts()
    };
    assert!(shorter.end_at_ms < source().end_at_ms);
    assert!(shorter.matches_resumed_renewal(&resume(), &renewal(30, true), &source()));
}
