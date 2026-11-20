use sandproof_protocol::internal::{
    NodeIdentity,
    lifecycle::{CreateBinding, Resources},
};
use sandproof_protocol::lifecycle::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity,
    ContentDigest, ExpectedSession, LifecycleOperation, LifecycleRequest, OperationBinding,
    OperationKey, PROTOCOL_VERSION, RetentionBinding, TerminalLifecycleObservation, TerminalStatus,
    ValidationError,
};
use sandproof_stage1_inbox_record_extract::terminal_matches;

fn intent() -> AcceptedLifecycleIntent {
    let node = NodeIdentity {
        deployment_id: "deployment".try_into().unwrap(),
        node_id: "host".try_into().unwrap(),
        node_generation: 1_u64.try_into().unwrap(),
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
                    window: 1_u64.try_into().unwrap(),
                    sequence: 1_u64.try_into().unwrap(),
                },
                expected: ExpectedSession::Version(0),
            },
            operation: LifecycleOperation::Destroy {},
        },
        accepted_version: 1,
        retention: RetentionBinding {
            slot: 1,
            window: 1_u64.try_into().unwrap(),
        },
    }
}
fn observation(
    request: &AcceptedLifecycleIntent,
    status: TerminalStatus,
) -> TerminalLifecycleObservation {
    TerminalLifecycleObservation {
        request: request.clone(),
        status,
    }
}
fn context(request: &AcceptedLifecycleIntent) -> AuthenticatedCenterContext {
    AuthenticatedCenterContext::from_verified_peer_registration(
        request.center.clone(),
        request.request.binding.node.clone(),
    )
}
fn assert_rejected(observed: AcceptedLifecycleIntent, requested: &AcceptedLifecycleIntent) {
    assert!(!terminal_matches(
        &observation(&observed, TerminalStatus::Cleaned),
        requested
    ));
}

#[test]
fn normal_matches_and_valid() {
    let request = intent();
    assert_eq!(request.validate(&context(&request)), Ok(()));
    for status in [TerminalStatus::Pending, TerminalStatus::Cleaned] {
        assert!(terminal_matches(&observation(&request, status), &request));
    }
}
#[test]
fn highest_version_and_slot_match() {
    let mut request = intent();
    request.request.binding.expected = ExpectedSession::Version(u64::MAX - 1);
    request.accepted_version = u64::MAX;
    request.retention.slot = 4096;
    request.retention.window = u64::MAX.try_into().unwrap();
    assert_eq!(request.validate(&context(&request)), Ok(()));
    assert!(terminal_matches(
        &observation(&request, TerminalStatus::Cleaned),
        &request
    ));
}
#[test]
fn distinct_retention_window_matches() {
    let mut request = intent();
    request.retention.window = 9_u64.try_into().unwrap();
    assert_ne!(
        request.request.binding.key.window.get(),
        request.retention.window.get()
    );
    assert_eq!(request.validate(&context(&request)), Ok(()));
    assert!(terminal_matches(
        &observation(&request, TerminalStatus::Cleaned),
        &request
    ));
}
#[test]
fn key_fields_are_not_omitted() {
    let request = intent();
    let mut changed = request.clone();
    changed.request.binding.key.deployment = "other".try_into().unwrap();
    assert_rejected(changed, &request);
    let mut changed = request.clone();
    changed.request.binding.key.owner = "other".try_into().unwrap();
    assert_rejected(changed, &request);
    let mut changed = request.clone();
    changed.request.binding.key.window = 2_u64.try_into().unwrap();
    assert_rejected(changed, &request);
    let mut changed = request.clone();
    changed.request.binding.key.sequence = 2_u64.try_into().unwrap();
    assert_rejected(changed, &request);
}
#[test]
fn header_fields_are_not_omitted() {
    let request = intent();
    let mut changed = request.clone();
    changed.request.binding.protocol_version = 3;
    assert_rejected(changed, &request);
    let mut changed = request.clone();
    changed.request.binding.node.deployment_id = "other".try_into().unwrap();
    assert_rejected(changed, &request);
    let mut changed = request.clone();
    changed.request.binding.node.node_id = "other".try_into().unwrap();
    assert_rejected(changed, &request);
    let mut changed = request.clone();
    changed.request.binding.node.node_generation = 2_u64.try_into().unwrap();
    assert_rejected(changed, &request);
    let mut changed = request.clone();
    changed.request.binding.session = "other".try_into().unwrap();
    assert_rejected(changed, &request);
    let mut changed = request.clone();
    changed.request.binding.expected = ExpectedSession::Version(1);
    assert_rejected(changed, &request);
}
#[test]
fn accepted_center_is_not_omitted() {
    let request = intent();
    let mut changed = request.clone();
    changed.center.center = "other-center".try_into().unwrap();
    assert_rejected(changed, &request);
    let mut changed = request.clone();
    changed.center.deployment = "other".try_into().unwrap();
    assert_rejected(changed, &request);
}
#[test]
fn accepted_version_is_not_omitted() {
    let request = intent();
    let mut changed = request.clone();
    changed.accepted_version = 2;
    assert_rejected(changed, &request);
}
#[test]
fn retention_fields_are_not_omitted() {
    let request = intent();
    let mut changed = request.clone();
    changed.retention.slot = 2;
    assert_rejected(changed, &request);
    let mut changed = request.clone();
    changed.retention.window = 9_u64.try_into().unwrap();
    assert_rejected(changed, &request);
}
#[test]
fn observation_operation_is_not_omitted() {
    let request = intent();
    let mut changed = request.clone();
    changed.request.operation = LifecycleOperation::Unload {
        source_incarnation: 1_u64.try_into().unwrap(),
        capture: 1_u64.try_into().unwrap(),
    };
    assert_rejected(changed, &request);
}
#[test]
fn non_destroy_operations_rejected() {
    let operations = [
        LifecycleOperation::Unload {
            source_incarnation: 1_u64.try_into().unwrap(),
            capture: 1_u64.try_into().unwrap(),
        },
        LifecycleOperation::Create {
            create: CreateBinding {
                template_revision: "template".try_into().unwrap(),
                template_digest: [0; 32],
                resources: Resources {
                    instances: 1,
                    cpu: 1,
                    memory_bytes: 1,
                    disk_bytes: 1,
                },
                options: b"{}".to_vec(),
                startup_deadline_ms: 1,
                timeout_ms: 1,
                max_lifetime_ms: 1,
            },
            target_incarnation: 1_u64.try_into().unwrap(),
        },
        LifecycleOperation::Resume {
            source_incarnation: 1_u64.try_into().unwrap(),
            capture: 1_u64.try_into().unwrap(),
            target_incarnation: 2_u64.try_into().unwrap(),
            manifest_digest: ContentDigest::from_sha256([0; 32]),
        },
        LifecycleOperation::Renew {
            incarnation: 1_u64.try_into().unwrap(),
            end_at_ms: 1,
            extend_only: false,
        },
    ];
    for operation in operations {
        let mut request = intent();
        request.request.operation = operation;
        assert_rejected(request.clone(), &request);
    }
}
#[test]
fn zero_slot_rejected() {
    let mut request = intent();
    request.retention.slot = 0;
    assert_rejected(request.clone(), &request);
}
#[test]
fn upper_slot_rejected() {
    let mut request = intent();
    request.retention.slot = 4097;
    assert_rejected(request.clone(), &request);
}
#[test]
fn wrong_version_rejected() {
    let mut request = intent();
    request.accepted_version = 2;
    assert_rejected(request.clone(), &request);
}
#[test]
fn zero_version_rejected() {
    let mut request = intent();
    request.accepted_version = 0;
    assert_rejected(request.clone(), &request);
}
#[test]
fn expected_version_overflow_rejected() {
    let mut request = intent();
    request.request.binding.expected = ExpectedSession::Version(u64::MAX);
    request.accepted_version = u64::MAX;
    assert_rejected(request.clone(), &request);
}
#[test]
fn absent_destroy_matches_but_validate_rejects() {
    let mut request = intent();
    request.request.binding.expected = ExpectedSession::Absent;
    assert!(terminal_matches(
        &observation(&request, TerminalStatus::Pending),
        &request
    ));
    assert_eq!(
        request.validate(&context(&request)),
        Err(AcceptedIntentError::Request(
            ValidationError::InvalidExpectation
        ))
    );
}
#[test]
fn foreign_context_not_certified() {
    let request = intent();
    let mut center = request.center.clone();
    center.center = "other".try_into().unwrap();
    let foreign = AuthenticatedCenterContext::from_verified_peer_registration(
        center,
        request.request.binding.node.clone(),
    );
    assert!(terminal_matches(
        &observation(&request, TerminalStatus::Pending),
        &request
    ));
    assert_eq!(
        request.validate(&foreign),
        Err(AcceptedIntentError::ForeignCenter)
    );
}
#[test]
fn protocol_version_not_certified() {
    let mut request = intent();
    request.request.binding.protocol_version = 3;
    assert!(terminal_matches(
        &observation(&request, TerminalStatus::Pending),
        &request
    ));
    assert_eq!(
        request.validate(&context(&request)),
        Err(AcceptedIntentError::Request(
            ValidationError::UnsupportedVersion
        ))
    );
}
