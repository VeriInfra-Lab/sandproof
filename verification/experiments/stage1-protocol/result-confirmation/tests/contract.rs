//! Development and ning handoff assertions. These values attest no durable or physical state.
use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ClosedLifecycleWindow,
    ConfirmedLifecycleResult, ExpectedSession, LifecycleOperation, LifecycleRequest,
    LifecycleWindowClose, OperationBinding, OperationKey, PROTOCOL_VERSION, RetentionBinding,
    TerminalLifecycleObservation, TerminalStatus, ValidationError,
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
            operation: LifecycleOperation::Destroy {},
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

fn terminal(status: TerminalStatus) -> TerminalLifecycleObservation {
    TerminalLifecycleObservation {
        request: original().0,
        status,
    }
}

#[test]
fn legal_values_match_and_close_exactly() {
    let (intent, context) = original();
    assert_eq!(intent.validate(&context), Ok(()));
    assert!(terminal(TerminalStatus::Pending).matches_request(&intent));
    let result = terminal(TerminalStatus::Cleaned);
    assert!(
        ConfirmedLifecycleResult {
            result: result.clone()
        }
        .matches(&result)
    );
    let request = LifecycleWindowClose {
        terminal: result.clone(),
    };
    assert!(request.is_terminal_result());
    assert!(
        ClosedLifecycleWindow {
            terminal: result,
            retired_through: 1.try_into().unwrap(),
        }
        .matches(&request)
    );
}

#[test]
fn terminal_keeps_complete_intent() {
    let (intent, context) = original();
    let mut changed = terminal(TerminalStatus::Cleaned);
    changed.request.retention.window = 2.try_into().unwrap();
    assert_eq!(changed.request.validate(&context), Ok(()));
    assert_eq!(
        intent.request.binding.key,
        changed.request.request.binding.key
    );
    assert!(
        !changed.matches_request(&intent),
        "terminal must retain the complete intent"
    );
}

#[test]
fn confirmation_keeps_complete_result() {
    let result = terminal(TerminalStatus::Cleaned);
    let mut changed = result.clone();
    changed.request.retention.window = 2.try_into().unwrap();
    assert_eq!(changed.request.validate(&original().1), Ok(()));
    assert!(
        !ConfirmedLifecycleResult { result: changed }.matches(&result),
        "confirmation must retain the complete result"
    );
}

#[test]
fn pending_cannot_close() {
    let result = terminal(TerminalStatus::Pending);
    assert!(
        !ConfirmedLifecycleResult {
            result: result.clone()
        }
        .matches(&result)
    );
    assert!(
        !LifecycleWindowClose { terminal: result }.is_terminal_result(),
        "pending must not close the retained window"
    );
}

#[test]
fn newer_frontier_cannot_acknowledge() {
    let result = terminal(TerminalStatus::Cleaned);
    let request = LifecycleWindowClose {
        terminal: result.clone(),
    };
    assert!(request.is_terminal_result());
    assert!(
        !ClosedLifecycleWindow {
            terminal: result,
            retired_through: 2.try_into().unwrap(),
        }
        .matches(&request),
        "newer frontier must not replace exact retirement"
    );
}

#[test]
fn matching_does_not_validate_the_protocol() {
    let mut result = terminal(TerminalStatus::Cleaned);
    result.request.request.binding.protocol_version = 3;
    assert!(result.matches_request(&result.request));
    assert_eq!(
        result
            .request
            .request
            .validate(&result.request.request.binding.node),
        Err(ValidationError::UnsupportedVersion)
    );
}
