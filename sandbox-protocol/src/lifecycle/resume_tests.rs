use super::*;
use crate::lifecycle::{
    CenterIdentity, ContentDigest, ExpectedSession, LifecycleRequest, OperationBinding,
    OperationKey, RetentionBinding,
};

fn original() -> AcceptedLifecycleIntent {
    let node = crate::internal::NodeIdentity {
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
        accepted_version: 3,
        request: LifecycleRequest {
            binding: OperationBinding {
                protocol_version: crate::lifecycle::PROTOCOL_VERSION,
                node: node.clone(),
                session: "session".try_into().unwrap(),
                key: OperationKey {
                    deployment: node.deployment_id,
                    owner: "owner".try_into().unwrap(),
                    window: 1.try_into().unwrap(),
                    sequence: 3.try_into().unwrap(),
                },
                expected: ExpectedSession::Version(2),
            },
            operation: LifecycleOperation::Resume {
                source_incarnation: 1.try_into().unwrap(),
                capture: 1.try_into().unwrap(),
                target_incarnation: 2.try_into().unwrap(),
                manifest_digest: ContentDigest::from_sha256([7; 32]),
            },
        },
    }
}

fn ready() -> CurrentReadyFacts {
    CurrentReadyFacts {
        started_at_ms: 10,
        end_at_ms: 1000,
        max_end_at_ms: 2000,
        startup_deadline_ms: 20,
        observed_at_ms: 100,
    }
}

#[test]
fn resume_observation_keeps_the_complete_current_envelope() {
    let original = original();
    let view = ResumeObservation {
        request: original.clone(),
        current_request: original.clone(),
        readiness: CurrentResumeReadiness::Observed { facts: ready() },
    };
    assert!(view.matches_request(&original));
    assert_eq!(
        serde_json::from_slice::<ResumeObservation>(&serde_json::to_vec(&view).unwrap()).unwrap(),
        view
    );
    let mut changed = view.clone();
    changed.request.request.binding.key.owner = "foreign".try_into().unwrap();
    assert!(!changed.matches_request(&original));
    changed = view.clone();
    changed.current_request.request.binding.node.node_generation = 2.try_into().unwrap();
    assert!(!changed.matches_request(&original));
    changed = view;
    changed.current_request.retention.slot = 2;
    assert!(!changed.matches_request(&original));
}

#[test]
fn a_later_operation_can_only_report_the_original_resume_as_not_current() {
    let original = original();
    let mut view = ResumeObservation {
        request: original.clone(),
        current_request: original.clone(),
        readiness: CurrentResumeReadiness::NotCurrent,
    };
    assert!(!view.matches_request(&original));
    view.current_request.request.operation = LifecycleOperation::Destroy {};
    view.current_request.request.binding.expected = ExpectedSession::Version(3);
    view.current_request.accepted_version = 4;
    // A new operation must not reuse the original operation key.
    assert!(!view.matches_request(&original));
    view.current_request.request.binding.key.sequence = 4.try_into().unwrap();
    assert!(view.matches_request(&original));
    view.readiness = CurrentResumeReadiness::Observed { facts: ready() };
    assert!(!view.matches_request(&original));
    view.readiness = CurrentResumeReadiness::StateLost;
    assert!(!view.matches_request(&original));
}

#[test]
fn resumed_facts_preserve_the_original_source_and_all_lifetime_fields() {
    let original = original();
    let facts = ready();
    let source = UnloadedFacts {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
        manifest_digest: ContentDigest::from_sha256([7; 32]),
        material_bytes: 3000,
        started_at_ms: facts.started_at_ms,
        end_at_ms: facts.end_at_ms,
        max_end_at_ms: facts.max_end_at_ms,
        startup_deadline_ms: facts.startup_deadline_ms,
    };
    assert!(facts.matches_resumed(&original, &source));
    for altered in [
        CurrentReadyFacts {
            started_at_ms: 11,
            ..facts.clone()
        },
        CurrentReadyFacts {
            end_at_ms: 1001,
            ..facts.clone()
        },
        CurrentReadyFacts {
            max_end_at_ms: 2001,
            ..facts.clone()
        },
        CurrentReadyFacts {
            startup_deadline_ms: 21,
            ..facts.clone()
        },
        CurrentReadyFacts {
            observed_at_ms: 1000,
            ..facts.clone()
        },
    ] {
        assert!(!altered.matches_resumed(&original, &source));
    }
    let mut wrong = source.clone();
    wrong.capture = 2.try_into().unwrap();
    assert!(!facts.matches_resumed(&original, &wrong));
    wrong = source;
    wrong.manifest_digest = ContentDigest::from_sha256([8; 32]);
    assert!(!facts.matches_resumed(&original, &wrong));
}

#[test]
fn original_resume_guard_rejects_invalid_request_versions_and_incarnations() {
    let original = original();
    let mut view = ResumeObservation {
        request: original.clone(),
        current_request: original.clone(),
        readiness: CurrentResumeReadiness::Pending,
    };
    view.request.accepted_version = 4;
    view.current_request = view.request.clone();
    assert!(!view.matches_request(&view.request));
    view.request = original;
    if let LifecycleOperation::Resume {
        target_incarnation, ..
    } = &mut view.request.request.operation
    {
        *target_incarnation = 1.try_into().unwrap();
    }
    view.current_request = view.request.clone();
    assert!(!view.matches_request(&view.request));
}

#[test]
fn renewed_resume_uses_the_same_execution_and_original_total_lifetime() {
    let original = original();
    let mut renewed = original.clone();
    renewed.request.operation = LifecycleOperation::Renew {
        incarnation: 2.try_into().unwrap(),
        end_at_ms: 1800,
        extend_only: false,
    };
    renewed.request.binding.expected = ExpectedSession::Version(3);
    renewed.request.binding.key.sequence = 4.try_into().unwrap();
    renewed.accepted_version = 4;
    let original_facts = ready();
    let source = UnloadedFacts {
        source_incarnation: 1.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
        manifest_digest: ContentDigest::from_sha256([7; 32]),
        material_bytes: 3000,
        started_at_ms: original_facts.started_at_ms,
        end_at_ms: original_facts.end_at_ms,
        max_end_at_ms: original_facts.max_end_at_ms,
        startup_deadline_ms: original_facts.startup_deadline_ms,
    };
    // Observation may be after the previous deadline, within the accepted
    // renewed deadline. Renewal admission is the journal's separate obligation.
    let facts = CurrentReadyFacts {
        end_at_ms: 1800,
        observed_at_ms: 1500,
        ..original_facts
    };
    let mut view = ResumeObservation {
        request: original.clone(),
        current_request: renewed.clone(),
        readiness: CurrentResumeReadiness::Observed {
            facts: facts.clone(),
        },
    };
    assert!(view.matches_request(&original));
    assert!(facts.matches_resumed_renewal(&original, &renewed, &source));
    assert!(!facts.matches_resumed(&original, &source));
    if let LifecycleOperation::Renew { incarnation, .. } =
        &mut view.current_request.request.operation
    {
        *incarnation = 1.try_into().unwrap();
    }
    assert!(!view.matches_request(&original));
    assert!(!facts.matches_resumed_renewal(&original, &view.current_request, &source));
    let overflow = CurrentReadyFacts {
        end_at_ms: 2001,
        ..facts.clone()
    };
    assert!(!overflow.matches_resumed_renewal(&original, &renewed, &source));
    let changed_start = CurrentReadyFacts {
        started_at_ms: 11,
        ..facts
    };
    assert!(!changed_start.matches_resumed_renewal(&original, &renewed, &source));
}

#[test]
fn renewal_signed_lifetime_boundaries_preserve_observation_and_source_checks() {
    let original = original();
    for (end_at_ms, expected) in [
        (i64::MIN, false),
        (-1, false),
        (0, false),
        (1, true),
        (i64::MAX, true),
    ] {
        let mut renewed = original.clone();
        renewed.request.operation = LifecycleOperation::Renew {
            incarnation: 2.try_into().unwrap(),
            end_at_ms,
            extend_only: false,
        };
        renewed.request.binding.expected = ExpectedSession::Version(3);
        renewed.request.binding.key.sequence = 4.try_into().unwrap();
        renewed.accepted_version = 4;
        let end = if end_at_ms < 0 {
            1
        } else {
            u64::try_from(end_at_ms).unwrap()
        };
        let facts = CurrentReadyFacts {
            started_at_ms: 0,
            startup_deadline_ms: 0,
            observed_at_ms: 0,
            end_at_ms: end,
            max_end_at_ms: u64::MAX,
        };
        let source = UnloadedFacts {
            source_incarnation: 1.try_into().unwrap(),
            capture: 1.try_into().unwrap(),
            manifest_digest: ContentDigest::from_sha256([7; 32]),
            material_bytes: 3000,
            started_at_ms: 0,
            end_at_ms: 1,
            max_end_at_ms: u64::MAX,
            startup_deadline_ms: 0,
        };
        let view = ResumeObservation {
            request: original.clone(),
            current_request: renewed.clone(),
            readiness: CurrentResumeReadiness::Observed {
                facts: facts.clone(),
            },
        };
        assert_eq!(view.matches_request(&original), expected, "end={end_at_ms}");
        assert_eq!(
            facts.matches_resumed_renewal(&original, &renewed, &source),
            expected,
            "end={end_at_ms}"
        );
        let expired = CurrentReadyFacts {
            observed_at_ms: end,
            ..facts
        };
        assert!(!expired.matches_resumed_renewal(&original, &renewed, &source));
    }
}

#[test]
fn renewal_conversion_stays_inside_the_observed_branch() {
    let original = original();
    let mut current = original.clone();
    current.request.binding.expected = ExpectedSession::Version(3);
    current.request.binding.key.sequence = 4.try_into().unwrap();
    current.accepted_version = 4;
    current.request.operation = LifecycleOperation::Renew {
        incarnation: 2.try_into().unwrap(),
        end_at_ms: -1,
        extend_only: true,
    };
    let mut view = ResumeObservation {
        request: original.clone(),
        current_request: current,
        readiness: CurrentResumeReadiness::Pending,
    };
    // Pending reports no executable lifetime; preserve the original order.
    assert!(view.matches_request(&original));
    view.readiness = CurrentResumeReadiness::Observed { facts: ready() };
    assert!(!view.matches_request(&original));
    if let LifecycleOperation::Renew { end_at_ms, .. } = &mut view.current_request.request.operation {
        *end_at_ms = 0;
    }
    assert!(view.matches_request(&original));
}
