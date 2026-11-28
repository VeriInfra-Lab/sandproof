use super::*;
use sandproof_controller_service::{JournalError, template_registry::evidence::encode_bounded};
use sandproof_protocol::lifecycle::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext,
    RecordedLifecycleIntent,
};
use serde_json::{Value, json};

fn envelope(operation: Value, create: bool) -> AcceptedLifecycleIntent {
    let identifier = "x".repeat(128);
    serde_json::from_value(json!({
        "request":{"binding":{"protocol_version":4,
            "node":{"deployment_id":identifier,"node_id":identifier,"node_generation":u64::MAX},
            "session":identifier,"key":{"deployment":identifier,"owner":identifier,
                "window":u64::MAX,"sequence":u64::MAX},
            "expected":if create {json!({"state":"absent"})}
                else {json!({"state":"version","version":u64::MAX-1})}
        },"operation":operation},
        "accepted_version":if create {1} else {u64::MAX},
        "center":{"deployment":identifier,"center":identifier},
        "retention":{"slot":4096,"window":1}
    }))
    .unwrap()
}

fn create(options: usize) -> AcceptedLifecycleIntent {
    envelope(
        json!({"kind":"create","create":{
            "template_revision":"x".repeat(128),"template_digest":vec![255;32],
            "resources":{"instances":1,"cpu":u64::MAX,
                "memory_bytes":u64::MAX,"disk_bytes":u64::MAX},
            "options":vec![255;options],"startup_deadline_ms":1,
            "timeout_ms":u64::MAX-1,"max_lifetime_ms":u64::MAX-1
        },"target_incarnation":u64::MAX}),
        true,
    )
}

fn context(value: &AcceptedLifecycleIntent) -> AuthenticatedCenterContext {
    AuthenticatedCenterContext::from_verified_peer_registration(
        value.center.clone(),
        value.request.binding.node.clone(),
    )
}

#[test]
fn original_constants_and_maximum_create_use_the_actual_writer() {
    assert_eq!(max_options_bytes(), 65_536);
    assert_eq!(request_body_bytes(), 278_528);
    let value = create(max_options_bytes());
    assert_eq!(value.validate(&context(&value)), Ok(()));
    let ordinary = serde_json::to_vec(&value).unwrap();
    let bounded = encode_bounded(&value, request_body_bytes()).unwrap();
    assert_eq!(ordinary, bounded);
    assert!(bounded.len() <= 270_336);
    assert!(bounded.len() > 4 * max_options_bytes());
    assert_eq!(
        serde_json::from_slice::<AcceptedLifecycleIntent>(&bounded).unwrap(),
        value
    );
    // Exact output threshold, with the same actual encoder and complete value.
    assert_eq!(encode_bounded(&value, bounded.len()).unwrap(), bounded);
    assert!(matches!(
        encode_bounded(&value, bounded.len() - 1),
        Err(JournalError::MaintenanceCapacity)
    ));
    assert!(matches!(
        encode_bounded(&value, 4 * max_options_bytes()),
        Err(JournalError::MaintenanceCapacity)
    ));
}

#[test]
fn five_operations_and_recorded_outer_fields_keep_full_bytes() {
    let values = [
        create(max_options_bytes()),
        envelope(
            json!({"kind":"unload","source_incarnation":u64::MAX,"capture":u64::MAX}),
            false,
        ),
        envelope(
            json!({"kind":"resume","source_incarnation":1,"capture":u64::MAX,
            "target_incarnation":u64::MAX,"manifest_digest":vec![255;32]}),
            false,
        ),
        envelope(
            json!({"kind":"renew","incarnation":u64::MAX,
            "end_at_ms":i64::MIN,"extend_only":false}),
            false,
        ),
        envelope(json!({"kind":"destroy"}), false),
    ];
    for value in values {
        assert_eq!(value.validate(&context(&value)), Ok(()));
        let bytes = encode_bounded(&value, request_body_bytes()).unwrap();
        assert_eq!(bytes, serde_json::to_vec(&value).unwrap());
        assert_eq!(
            serde_json::from_slice::<AcceptedLifecycleIntent>(&bytes).unwrap(),
            value
        );
        for terminal in [false, true] {
            let recorded = RecordedLifecycleIntent {
                current_session_version: value.accepted_version,
                intent: value.clone(),
                current: true,
                terminal,
            };
            let bytes = encode_bounded(&recorded, request_body_bytes()).unwrap();
            assert_eq!(bytes, serde_json::to_vec(&recorded).unwrap());
            assert!(bytes.len() <= 270_336);
            let decoded: RecordedLifecycleIntent = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(decoded, recorded);
            assert!(decoded.matches(&value));
        }
    }
}

#[test]
fn fitting_bytes_do_not_replace_the_original_create_guard() {
    let value = create(max_options_bytes() + 1);
    let bytes = encode_bounded(&value, request_body_bytes()).unwrap();
    assert_eq!(
        serde_json::from_slice::<AcceptedLifecycleIntent>(&bytes).unwrap(),
        value
    );
    assert!(matches!(
        value.validate(&context(&value)),
        Err(AcceptedIntentError::Request(_))
    ));
}
