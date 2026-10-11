//! Wire binding only; these tests do not attest host persistence or VM cleanup.
use sandproof_protocol::internal::{
    INTERNAL_PROTOCOL_VERSION, InternalError,
    lifecycle::{Intent, LifecycleFact, LifecycleResponse, RESULT_CONFIRMATION_BODY_BYTES},
    window::{WINDOW_CLOSE_BODY_BYTES, WindowCloseAck, WindowCloseRequest},
};

#[path = "support/lifecycle.rs"]
mod fixture;

fn request() -> WindowCloseRequest {
    WindowCloseRequest::new(fixture::node("host-a"), 1.try_into().unwrap())
}

#[test]
fn round_trip_preserves_the_complete_window_identity_without_cleanup_claims() {
    let request = request();
    let ack = WindowCloseAck {
        identity: request.identity.clone(),
    };
    assert_eq!(request.validate(&fixture::node("host-a")), Ok(()));
    assert_eq!(ack.validate_for(&request), Ok(()));
    let encoded = serde_json::to_vec(&request).unwrap();
    assert_eq!(
        serde_json::from_slice::<WindowCloseRequest>(&encoded).unwrap(),
        request
    );
    let encoded = serde_json::to_vec(&ack).unwrap();
    assert_eq!(
        serde_json::from_slice::<WindowCloseAck>(&encoded).unwrap(),
        ack
    );
    let object = serde_json::to_value(ack).unwrap();
    assert_eq!(object.as_object().unwrap().len(), 1);
    assert!(object.get("fact").is_none());
    assert!(object.get("sandboxes").is_none());
}

#[test]
fn acknowledgement_rejects_every_changed_binding_field_and_unknown_version() {
    let request = request();
    let answer = WindowCloseAck {
        identity: request.identity.clone(),
    };
    let mut variants = vec![answer; 5];
    variants[0].identity.protocol_version = INTERNAL_PROTOCOL_VERSION + 1;
    variants[1].identity.node.deployment_id = "other-deployment".try_into().unwrap();
    variants[2].identity.node.node_id = "other-node".try_into().unwrap();
    variants[3].identity.node.node_generation = 2.try_into().unwrap();
    variants[4].identity.request_window = 2.try_into().unwrap();
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
    let mut changed_request = request.clone();
    changed_request.identity.protocol_version += 1;
    assert_eq!(
        changed_request.validate(&request.identity.node),
        Err(InternalError::UnsupportedVersion)
    );
}

#[test]
fn arbitrary_owner_deletion_list_ttl_and_malformed_counters_fail_closed() {
    let original = serde_json::to_value(request()).unwrap();
    for field in ["owner", "sandboxes", "ttl", "path", "cleaned"] {
        let mut object = original.clone();
        object[field] = "untrusted".into();
        assert!(serde_json::from_value::<WindowCloseRequest>(object.clone()).is_err());
        assert!(serde_json::from_value::<WindowCloseAck>(object).is_err());
    }
    for field in ["owner", "ttl", "untrusted_url"] {
        let mut object = original.clone();
        object["identity"][field] = "untrusted".into();
        assert!(serde_json::from_value::<WindowCloseRequest>(object).is_err());
    }
    for counter in ["request_window", "node_generation"] {
        let mut object = original.clone();
        if counter == "request_window" {
            object["identity"][counter] = 0.into();
        } else {
            object["identity"]["node"][counter] = 0.into();
        }
        assert!(serde_json::from_value::<WindowCloseAck>(object).is_err());
    }
    let malformed = r#"{"identity":{"protocol_version":1,"node":{"deployment_id":"d","node_id":"n","node_generation":1},"request_window":18446744073709551616}}"#;
    assert!(serde_json::from_str::<WindowCloseRequest>(malformed).is_err());
}

#[test]
fn maximum_legal_fixed_results_and_window_messages_fit_the_reserved_envelopes() {
    let maximum_id = "z".repeat(128);
    let mut lifecycle = fixture::request(1, 1, Intent::Create);
    lifecycle.identity.node.deployment_id = maximum_id.clone().try_into().unwrap();
    lifecycle.identity.node.node_id = maximum_id.clone().try_into().unwrap();
    lifecycle.identity.node.node_generation = u64::MAX.try_into().unwrap();
    lifecycle.identity.tenant_id = maximum_id.clone().try_into().unwrap();
    lifecycle.identity.sandbox_id = maximum_id.clone().try_into().unwrap();
    lifecycle.identity.operation_id = maximum_id.try_into().unwrap();
    lifecycle.identity.intent_version = u64::MAX.try_into().unwrap();
    lifecycle.identity.request_window = u64::MAX.try_into().unwrap();
    // No CreateOptions are copied into a result confirmation. Every opaque ID
    // is maximal, every integer uses the longest legal decimal representation,
    // and all fixed result shapes are checked rather than guessing old slack.
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
    let close = WindowCloseRequest::new(lifecycle.identity.node, lifecycle.identity.request_window);
    let answer = WindowCloseAck {
        identity: close.identity.clone(),
    };
    assert_eq!(answer.validate_for(&close), Ok(()));
    assert!(serde_json::to_vec(&close).unwrap().len() <= WINDOW_CLOSE_BODY_BYTES);
    assert!(serde_json::to_vec(&answer).unwrap().len() <= WINDOW_CLOSE_BODY_BYTES);
}
