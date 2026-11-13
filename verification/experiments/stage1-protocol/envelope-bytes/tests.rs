use super::*;
use sandproof_protocol::lifecycle::{AcceptedIntentError, AuthenticatedCenterContext};
use serde_json::{Value, json};

fn samples() -> Vec<Value> {
    [
        json!({"kind":"create","create":{
            "template_revision":"base","template_digest":vec![7;32],
            "resources":{"instances":1,"cpu":2,"memory_bytes":3,"disk_bytes":4},
            "options":[0,1,255],"startup_deadline_ms":5,"timeout_ms":6,"max_lifetime_ms":7
        },"target_incarnation":1}),
        json!({"kind":"unload","source_incarnation":1,"capture":2}),
        json!({"kind":"resume","source_incarnation":1,"capture":2,"target_incarnation":9,
            "manifest_digest":vec![8;32]}),
        json!({"kind":"renew","incarnation":1,"end_at_ms":-1,"extend_only":false}),
        json!({"kind":"destroy"}),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, operation)| json!({
        "request":{"binding":{"protocol_version":4,
            "node":{"deployment_id":"deployment","node_id":"host","node_generation":1},
            "session":"session","key":{"deployment":"deployment","owner":"owner","window":3,"sequence":4},
            "expected":if index==0 {json!({"state":"absent"})} else {json!({"state":"version","version":0})}
        },"operation":operation},
        "accepted_version":1,"center":{"deployment":"deployment","center":"center"},
        "retention":{"slot":1,"window":9}
    }))
    .collect()
}

#[test]
fn original_slice_calls_keep_five_full_envelopes_and_records() {
    for sample in samples() {
        let original = accepted_decode(&serde_json::to_vec(&sample).unwrap()).unwrap();
        let encoded = accepted_encode(&original).unwrap();
        assert_eq!(accepted_decode(&encoded).unwrap(), original);
        let context = AuthenticatedCenterContext::from_verified_peer_registration(
            original.center.clone(),
            original.request.binding.node.clone(),
        );
        assert_eq!(original.validate(&context), Ok(()));
        for terminal in [false, true] {
            let recorded = RecordedLifecycleIntent {
                intent: original.clone(),
                current_session_version: 1,
                current: true,
                terminal,
            };
            let returned = recorded_decode(&recorded_encode(&recorded).unwrap()).unwrap();
            assert_eq!(returned, recorded);
            assert!(returned.matches(&original));
        }
    }
}

#[test]
fn every_own_field_uses_original_rejection_also_under_intent() {
    let ids = [
        "/request/binding/node/deployment_id",
        "/request/binding/node/node_id",
        "/request/binding/session",
        "/request/binding/key/deployment",
        "/request/binding/key/owner",
        "/request/operation/create/template_revision",
        "/center/deployment",
        "/center/center",
    ];
    let counters = [
        "/request/binding/node/node_generation",
        "/request/binding/key/window",
        "/request/binding/key/sequence",
        "/request/operation/target_incarnation",
        "/request/operation/source_incarnation",
        "/request/operation/incarnation",
        "/request/operation/capture",
        "/retention/window",
    ];
    let mut checked = 0;
    for sample in samples() {
        for (paths, invalid, message) in [
            (&ids, json!(""), "invalid opaque identifier"),
            (&counters, json!(0), "invalid or exhausted internal counter"),
        ] {
            for path in paths {
                let mut changed = sample.clone();
                let Some(field) = changed.pointer_mut(path) else {
                    continue;
                };
                *field = invalid.clone();
                let error = accepted_decode(&serde_json::to_vec(&changed).unwrap()).unwrap_err();
                assert!(error.is_data());
                assert!(error.to_string().starts_with(message), "{path}: {error}");
                let outer = json!({"intent":changed,"current_session_version":1,"current":true,"terminal":false});
                let error = recorded_decode(&serde_json::to_vec(&outer).unwrap()).unwrap_err();
                assert!(error.is_data());
                assert!(
                    error.to_string().starts_with(message),
                    "intent{path}: {error}"
                );
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 63);
}

#[test]
fn original_custom_error_precedes_document_end_but_position_is_container_fixed() {
    let mut invalid = samples().remove(1);
    invalid["retention"]["window"] = json!(0);
    let mut bytes = serde_json::to_vec(&invalid).unwrap();
    bytes.extend_from_slice(b" trailing");
    let error = accepted_decode(&bytes).unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("invalid or exhausted internal counter")
    );
    assert!(error.line() > 0 && error.column() > 0);
    invalid["retention"]["window"] = json!(9);
    let mut bytes = serde_json::to_vec(&invalid).unwrap();
    bytes.extend_from_slice(b" trailing");
    assert!(
        accepted_decode(&bytes)
            .unwrap_err()
            .to_string()
            .starts_with("trailing characters")
    );
}

#[test]
fn decoding_ordinary_zero_fields_does_not_certify_the_guard() {
    let mut sample = samples().remove(1);
    sample["accepted_version"] = json!(0);
    sample["retention"]["slot"] = json!(0);
    let value = accepted_decode(&serde_json::to_vec(&sample).unwrap()).unwrap();
    let context = AuthenticatedCenterContext::from_verified_peer_registration(
        value.center.clone(),
        value.request.binding.node.clone(),
    );
    assert_eq!(
        value.validate(&context),
        Err(AcceptedIntentError::InvalidRetention)
    );
    assert_eq!(
        accepted_decode(&accepted_encode(&value).unwrap()).unwrap(),
        value
    );
}
