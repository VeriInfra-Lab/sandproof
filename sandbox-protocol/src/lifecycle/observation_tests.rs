use super::*;
use crate::lifecycle::{
    CenterIdentity, ExpectedSession, OperationBinding, OperationKey, RetentionBinding,
};
#[path = "../../tests/support/lifecycle.rs"]
mod fixture;

fn create() -> AcceptedLifecycleIntent {
    let node = fixture::node("host-a");
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
        request: crate::lifecycle::LifecycleRequest {
            binding: OperationBinding {
                protocol_version: crate::lifecycle::PROTOCOL_VERSION,
                node: node.clone(),
                session: "sandbox-1".try_into().unwrap(),
                key: OperationKey {
                    deployment: node.deployment_id,
                    owner: "1".try_into().unwrap(),
                    window: 1.try_into().unwrap(),
                    sequence: 1.try_into().unwrap(),
                },
                expected: ExpectedSession::Absent,
            },
            operation: LifecycleOperation::Create {
                create: fixture::binding(),
                target_incarnation: 1.try_into().unwrap(),
            },
        },
    }
}

fn refused(original: &AcceptedLifecycleIntent) -> CreateObservation {
    CreateObservation {
        request: original.clone(),
        current_request: original.clone(),
        historical_create_succeeded: false,
        readiness: CurrentCreateReadiness::Refused {
            reason: CreateRefusalReason::Capacity,
        },
    }
}

#[test]
fn refusal_is_bound_to_complete_original_create_without_historical_success() {
    let original = create();
    let view = refused(&original);
    assert!(view.matches_request(&original));
    let bytes = serde_json::to_vec(&view).unwrap();
    assert_eq!(
        serde_json::from_slice::<CreateObservation>(&bytes).unwrap(),
        view
    );
    let mut changed = view.clone();
    let LifecycleOperation::Create { create, .. } = &mut changed.request.request.operation else {
        unreachable!()
    };
    create.options.push(b' ');
    assert!(!changed.matches_request(&original));
    changed = view.clone();
    changed.historical_create_succeeded = true;
    assert!(!changed.matches_request(&original));
    changed = view;
    changed.current_request.request.binding.key.owner = "2".try_into().unwrap();
    assert!(!changed.matches_request(&original));
}

#[test]
fn original_refusal_survives_only_its_later_complete_destroy() {
    let original = create();
    let mut view = refused(&original);
    view.current_request.request.operation = LifecycleOperation::Destroy {};
    view.current_request.request.binding.expected = ExpectedSession::Version(1);
    view.current_request.request.binding.key.sequence = 2.try_into().unwrap();
    view.current_request.accepted_version = 2;
    assert!(view.matches_request(&original));
    let destroy = view.current_request.clone();
    view.current_request.request.binding.key = original.request.binding.key.clone();
    assert!(!view.matches_request(&original));
    view.current_request = destroy.clone();
    view.current_request.accepted_version = 3;
    assert!(!view.matches_request(&original));
    view.current_request = destroy;
    view.current_request.request.operation = LifecycleOperation::Renew {
        incarnation: 1.try_into().unwrap(),
        end_at_ms: 1100,
        extend_only: false,
    };
    assert!(!view.matches_request(&original));
}
