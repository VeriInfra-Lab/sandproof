//! Wire binding only; these tests do not attest host persistence or VM cleanup.
use sandproof_protocol::internal::{
    INTERNAL_PROTOCOL_VERSION, InternalError,
    lifecycle::{Intent, LifecycleFact, LifecycleResponse, RESULT_CONFIRMATION_BODY_BYTES},
    window::{RETENTION_LANE_BYTES, WINDOW_CLOSE_BODY_BYTES, WindowCloseAck, WindowCloseRequest},
};
#[path = "support/lifecycle.rs"]
mod fixture;
fn request() -> WindowCloseRequest {
    WindowCloseRequest::new(fixture::response(
        &fixture::request(2, 2, Intent::Destroy),
        3,
        LifecycleFact::Cleaned,
    ))
}
#[test]
fn round_trip_preserves_complete_terminal_stamp_without_deletion_lists() {
    let request = request();
    let ack = WindowCloseAck {
        terminal: request.terminal.clone(),
    };
    assert_eq!(request.validate(&fixture::node("host-a")), Ok(()));
    assert_eq!(ack.validate_for(&request), Ok(()));
    assert_eq!(
        serde_json::from_slice::<WindowCloseRequest>(&serde_json::to_vec(&request).unwrap())
            .unwrap(),
        request
    );
    assert_eq!(
        serde_json::from_slice::<WindowCloseAck>(&serde_json::to_vec(&ack).unwrap()).unwrap(),
        ack
    );
    assert_eq!(
        serde_json::to_value(&ack)
            .unwrap()
            .as_object()
            .unwrap()
            .len(),
        1
    );
    const {
        assert!(RETENTION_LANE_BYTES > WINDOW_CLOSE_BODY_BYTES);
    }
}
#[test]
fn acknowledgement_rejects_every_changed_binding_observation_and_fact() {
    let request = request();
    let answer = WindowCloseAck {
        terminal: request.terminal.clone(),
    };
    let mut variants = vec![answer; 12];
    variants[0].terminal.identity.protocol_version = INTERNAL_PROTOCOL_VERSION + 1;
    variants[1].terminal.identity.node.deployment_id = "other-deployment".try_into().unwrap();
    variants[2].terminal.identity.node.node_id = "other-node".try_into().unwrap();
    variants[3].terminal.identity.node.node_generation = 2.try_into().unwrap();
    variants[4].terminal.identity.request_window = 2.try_into().unwrap();
    variants[5].terminal.identity.retention_slot = 2;
    variants[6].terminal.identity.tenant_id = "2".try_into().unwrap();
    variants[7].terminal.identity.sandbox_id = "other-sandbox".try_into().unwrap();
    variants[8].terminal.identity.operation_id = "other-operation".try_into().unwrap();
    variants[9].terminal.identity.intent_version = 3.try_into().unwrap();
    variants[10].terminal.observation_version = 4.try_into().unwrap();
    variants[11].terminal.fact = LifecycleFact::NotAccepted;
    for (index, altered) in variants.into_iter().enumerate() {
        assert_eq!(
            altered.validate_for(&request),
            Err(if index == 0 {
                InternalError::UnsupportedVersion
            } else {
                InternalError::BindingMismatch
            })
        );
    }
    for fact in [
        LifecycleFact::Recorded,
        LifecycleFact::Stopped,
        LifecycleFact::Renewed { end_at_ms: 100 },
    ] {
        let mut changed = request.clone();
        changed.terminal.fact = fact;
        assert_eq!(
            changed.validate(&fixture::node("host-a")),
            Err(InternalError::BindingMismatch)
        );
    }
    let mut zero = request;
    zero.terminal.identity.retention_slot = 0;
    assert_eq!(
        zero.validate(&fixture::node("host-a")),
        Err(InternalError::BindingMismatch)
    );
}
#[test]
fn old_whole_window_shapes_unknown_fields_and_missing_slots_fail_closed() {
    assert!(serde_json::from_str::<WindowCloseRequest>(r#"{"identity":{"protocol_version":1,"node":{"deployment_id":"d","node_id":"n","node_generation":1},"request_window":1}}"#).is_err());
    let original = serde_json::to_value(request()).unwrap();
    for field in ["owner", "sandboxes", "ttl", "path", "cleaned"] {
        let mut object = original.clone();
        object[field] = "untrusted".into();
        assert!(serde_json::from_value::<WindowCloseRequest>(object.clone()).is_err());
        assert!(serde_json::from_value::<WindowCloseAck>(object).is_err());
    }
    let mut absent = original.clone();
    absent["terminal"]["identity"]
        .as_object_mut()
        .unwrap()
        .remove("retention_slot");
    assert!(serde_json::from_value::<WindowCloseRequest>(absent).is_err());
    for counter in ["request_window", "node_generation"] {
        let mut object = original.clone();
        if counter == "request_window" {
            object["terminal"]["identity"][counter] = 0.into();
        } else {
            object["terminal"]["identity"]["node"][counter] = 0.into();
        }
        assert!(serde_json::from_value::<WindowCloseAck>(object).is_err());
    }
}
#[test]
fn maximum_legal_fixed_results_and_close_stamps_fit_reserved_envelopes() {
    let maximum = "z".repeat(128);
    let mut lifecycle = fixture::request(1, 1, Intent::Create);
    lifecycle.identity.node.deployment_id = maximum.clone().try_into().unwrap();
    lifecycle.identity.node.node_id = maximum.clone().try_into().unwrap();
    lifecycle.identity.node.node_generation = u64::MAX.try_into().unwrap();
    lifecycle.identity.tenant_id = maximum.clone().try_into().unwrap();
    lifecycle.identity.sandbox_id = maximum.clone().try_into().unwrap();
    lifecycle.identity.operation_id = maximum.try_into().unwrap();
    lifecycle.identity.intent_version = u64::MAX.try_into().unwrap();
    lifecycle.identity.retention_slot = u16::MAX;
    lifecycle.identity.request_window = u64::MAX.try_into().unwrap();
    for fact in [
        LifecycleFact::Recorded,
        LifecycleFact::Ready {
            started_at_ms: u64::MAX,
            end_at_ms: u64::MAX,
            max_end_at_ms: u64::MAX,
        },
        LifecycleFact::Renewed {
            end_at_ms: u64::MAX,
        },
        LifecycleFact::Stopped,
        LifecycleFact::Cleaned,
        LifecycleFact::NotAccepted,
    ] {
        let result = fixture::response(&lifecycle, u64::MAX, fact);
        let encoded = serde_json::to_vec(&result).unwrap();
        assert!(encoded.len() <= RESULT_CONFIRMATION_BODY_BYTES);
        assert_eq!(
            serde_json::from_slice::<LifecycleResponse>(&encoded).unwrap(),
            result
        );
    }
    let close = WindowCloseRequest::new(fixture::response(
        &lifecycle,
        u64::MAX,
        LifecycleFact::Cleaned,
    ));
    let ack = WindowCloseAck {
        terminal: close.terminal.clone(),
    };
    assert_eq!(ack.validate_for(&close), Ok(()));
    assert!(serde_json::to_vec(&close).unwrap().len() <= WINDOW_CLOSE_BODY_BYTES);
    assert!(serde_json::to_vec(&ack).unwrap().len() <= WINDOW_CLOSE_BODY_BYTES);
}
