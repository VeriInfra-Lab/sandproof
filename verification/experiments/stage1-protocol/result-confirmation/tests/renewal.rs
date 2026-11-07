//! Decoded values only; these tests do not establish durable confirmation.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity,
    ConfirmedRenewal, ExpectedSession, LifecycleOperation, LifecycleRequest, OperationBinding,
    OperationKey, PROTOCOL_VERSION, RenewalConfirmation, RetentionBinding,
};

fn original() -> RenewalConfirmation {
    let node = NodeIdentity {
        deployment_id: "deployment".try_into().unwrap(),
        node_id: "host".try_into().unwrap(),
        node_generation: 1.try_into().unwrap(),
    };
    RenewalConfirmation {
        request: AcceptedLifecycleIntent {
            center: CenterIdentity {
                deployment: node.deployment_id.clone(),
                center: "center".try_into().unwrap(),
            },
            retention: RetentionBinding {
                slot: 1,
                window: 1.try_into().unwrap(),
            },
            accepted_version: 2,
            request: LifecycleRequest {
                binding: OperationBinding {
                    protocol_version: PROTOCOL_VERSION,
                    node: node.clone(),
                    session: "session".try_into().unwrap(),
                    key: OperationKey {
                        deployment: node.deployment_id,
                        owner: "owner".try_into().unwrap(),
                        window: 2.try_into().unwrap(),
                        sequence: 1.try_into().unwrap(),
                    },
                    expected: ExpectedSession::Version(1),
                },
                operation: LifecycleOperation::Renew {
                    incarnation: 1.try_into().unwrap(),
                    end_at_ms: 200,
                    extend_only: false,
                },
            },
        },
        effective_end_at_ms: 200,
    }
}

fn extended(effective_end_at_ms: i64) -> RenewalConfirmation {
    let mut value = original();
    value.request.request.operation = LifecycleOperation::Renew {
        incarnation: 1.try_into().unwrap(),
        end_at_ms: 200,
        extend_only: true,
    };
    value.effective_end_at_ms = effective_end_at_ms;
    value
}

#[test]
fn legal_values_match_exactly() {
    for result in [original(), extended(200), extended(201)] {
        let context = AuthenticatedCenterContext::from_verified_peer_registration(
            result.request.center.clone(),
            result.request.request.binding.node.clone(),
        );
        assert_eq!(result.request.validate(&context), Ok(()));
        assert!(result.is_sdk_renewal());
        assert!(result.matches_request(&result.request));
        assert!(
            ConfirmedRenewal {
                result: result.clone()
            }
            .matches(&result)
        );
        assert_ne!(
            result.request.request.binding.key.window.get(),
            result.request.retention.window.get()
        );
    }
}

#[test]
fn renewal_keeps_complete_intent() {
    let result = original();
    let mut changed = result.request.clone();
    changed.retention.window = 2.try_into().unwrap();
    let context = AuthenticatedCenterContext::from_verified_peer_registration(
        changed.center.clone(),
        changed.request.binding.node.clone(),
    );
    assert_eq!(changed.validate(&context), Ok(()));
    assert_eq!(
        result.request.request.binding.key,
        changed.request.binding.key
    );
    assert!(
        !result.matches_request(&changed),
        "renewal must retain the complete intent"
    );
}

#[test]
fn confirmation_keeps_effective_deadline() {
    let saved = extended(200);
    let changed = extended(201);
    assert!(saved.is_sdk_renewal());
    assert!(changed.is_sdk_renewal());
    assert_eq!(saved.request, changed.request);
    assert!(
        !ConfirmedRenewal { result: saved }.matches(&changed),
        "confirmation must retain the effective deadline"
    );
}

#[test]
fn extension_cannot_shorten() {
    assert!(
        !extended(199).is_sdk_renewal(),
        "extension must not shorten the requested deadline"
    );
}

#[test]
fn first_version_cannot_confirm_renewal() {
    let mut result = original();
    result.request.accepted_version = 1;
    result.request.request.binding.expected = ExpectedSession::Version(0);
    assert!(
        result
            .request
            .request
            .validate(&result.request.request.binding.node)
            .is_ok()
    );
    assert!(
        !result.is_sdk_renewal(),
        "renewal must not accept the first version"
    );
}

#[test]
fn invalid_headers_and_shapes_rejected() {
    for field in 0..11 {
        let mut result = original();
        match field {
            0 => result.request.request.binding.protocol_version = PROTOCOL_VERSION - 1,
            1 => result.request.request.binding.key.window = 3.try_into().unwrap(),
            2 => result.request.accepted_version = 3,
            3 => result.request.center.deployment = "other".try_into().unwrap(),
            4 => result.request.retention.slot = 0,
            5 => result.request.retention.slot = 4097,
            6 => result.request.request.binding.expected = ExpectedSession::Absent,
            7 => result.request.request.binding.key.deployment = "other".try_into().unwrap(),
            8 => result.request.request.operation = LifecycleOperation::Destroy {},
            9 => result.effective_end_at_ms = -1,
            10 => result.effective_end_at_ms = 201,
            _ => unreachable!(),
        }
        assert!(!result.is_sdk_renewal(), "invalid field {field} accepted");
    }
}

#[test]
fn equal_fields_do_not_authenticate_center() {
    let result = original();
    let mut center = result.request.center.clone();
    center.center = "other-center".try_into().unwrap();
    let context = AuthenticatedCenterContext::from_verified_peer_registration(
        center,
        result.request.request.binding.node.clone(),
    );
    assert!(result.matches_request(&result.request));
    assert_eq!(
        result.request.validate(&context),
        Err(AcceptedIntentError::ForeignCenter)
    );
}
