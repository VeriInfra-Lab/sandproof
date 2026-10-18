//! Codec shape checks remain distinct from protocol/foreign-node validation.
use sandproof_protocol::internal::{
    ControlIdentity, INTERNAL_PROTOCOL_VERSION, InternalError, NodeIdentity, PositiveCounter,
};
use serde_json::{Value, json};

fn identity() -> ControlIdentity {
    ControlIdentity {
        protocol_version: INTERNAL_PROTOCOL_VERSION,
        node: NodeIdentity {
            deployment_id: "deployment-a".try_into().unwrap(),
            node_id: "node-a".try_into().unwrap(),
            node_generation: PositiveCounter::try_from(1).unwrap(),
        },
        tenant_id: "tenant-a".try_into().unwrap(),
        sandbox_id: "sandbox-1".try_into().unwrap(),
        operation_id: "operation-1".try_into().unwrap(),
        intent_version: PositiveCounter::try_from(1).unwrap(),
        retention_slot: 1,
        request_window: PositiveCounter::try_from(1).unwrap(),
    }
}

#[test]
fn round_trip_keeps_every_field_and_full_width_without_float_conversion() {
    for number in [1, (1 << 53) + 1, u64::MAX] {
        for protocol in [0, INTERNAL_PROTOCOL_VERSION, u16::MAX] {
            for slot in [0, 1, u16::MAX] {
                let mut value = identity();
                value.protocol_version = protocol;
                value.retention_slot = slot;
                value.node.node_generation = number.try_into().unwrap();
                value.intent_version = number.try_into().unwrap();
                value.request_window = number.try_into().unwrap();
                value.node.deployment_id = "D".repeat(128).try_into().unwrap();
                value.node.node_id = "N".repeat(128).try_into().unwrap();
                value.tenant_id = "T".repeat(128).try_into().unwrap();
                value.sandbox_id = "S".repeat(128).try_into().unwrap();
                value.operation_id = "O".repeat(128).try_into().unwrap();
                let encoded = serde_json::to_vec(&value).unwrap();
                assert_eq!(
                    serde_json::from_slice::<ControlIdentity>(&encoded).unwrap(),
                    value
                );
                assert!(encoded.len() <= sandproof_protocol::internal::DATA_IDENTITY_BYTES);
            }
        }
    }
}

#[test]
fn every_required_or_unknown_field_and_duplicate_object_key_fail_closed() {
    let value = serde_json::to_value(identity()).unwrap();
    let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys.len(), 8);
    for key in keys {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(&key);
        assert!(
            serde_json::from_value::<ControlIdentity>(missing).is_err(),
            "{key}"
        );
        let mut unknown = value.clone();
        unknown["extra"] = json!(true);
        assert!(serde_json::from_value::<ControlIdentity>(unknown).is_err());
        let duplicate = format!(
            "{{{}:{},{}",
            serde_json::to_string(&key).unwrap(),
            serde_json::to_string(&value[&key]).unwrap(),
            &serde_json::to_string(&value).unwrap()[1..]
        );
        assert!(
            serde_json::from_str::<ControlIdentity>(&duplicate).is_err(),
            "{key}"
        );
    }
    let node = value["node"].clone();
    for key in ["deployment_id", "node_id", "node_generation"] {
        let mut missing = value.clone();
        missing["node"].as_object_mut().unwrap().remove(key);
        assert!(
            serde_json::from_value::<ControlIdentity>(missing).is_err(),
            "node.{key}"
        );
        let duplicate = format!(
            "{{{}:{},{}",
            serde_json::to_string(key).unwrap(),
            serde_json::to_string(&node[key]).unwrap(),
            &serde_json::to_string(&node).unwrap()[1..]
        );
        assert!(serde_json::from_str::<NodeIdentity>(&duplicate).is_err());
    }
    let mut unknown = value;
    unknown["node"]["extra"] = json!(true);
    assert!(serde_json::from_value::<ControlIdentity>(unknown).is_err());
}

#[test]
fn restoration_uses_original_identifier_and_numeric_constructor_domains() {
    let original = serde_json::to_value(identity()).unwrap();
    for invalid in [
        json!(""),
        json!("a b"),
        json!("é"),
        json!("../x"),
        json!("x".repeat(129)),
        Value::Null,
    ] {
        for field in ["tenant_id", "sandbox_id", "operation_id"] {
            let mut value = original.clone();
            value[field] = invalid.clone();
            assert!(
                serde_json::from_value::<ControlIdentity>(value).is_err(),
                "{field}"
            );
        }
        for field in ["deployment_id", "node_id"] {
            let mut value = original.clone();
            value["node"][field] = invalid.clone();
            assert!(
                serde_json::from_value::<ControlIdentity>(value).is_err(),
                "node.{field}"
            );
        }
    }
    for invalid in [json!(0), json!(-1), json!(1.0), json!("1"), Value::Null] {
        for field in ["intent_version", "request_window"] {
            let mut value = original.clone();
            value[field] = invalid.clone();
            assert!(
                serde_json::from_value::<ControlIdentity>(value).is_err(),
                "{field}"
            );
        }
        let mut value = original.clone();
        value["node"]["node_generation"] = invalid;
        assert!(serde_json::from_value::<ControlIdentity>(value).is_err());
    }
    for field in ["protocol_version", "retention_slot"] {
        let mut value = original.clone();
        value[field] = json!(u32::from(u16::MAX) + 1);
        assert!(serde_json::from_value::<ControlIdentity>(value).is_err());
    }
    for (field, path) in [
        ("intent_version", ""),
        ("request_window", ""),
        ("node_generation", "node"),
    ] {
        let encoded = serde_json::to_string(&original).unwrap();
        let needle = format!("\"{field}\":1");
        let malformed = encoded.replace(&needle, &format!("\"{field}\":18446744073709551616"));
        assert_ne!(malformed, encoded, "{path}.{field}");
        assert!(serde_json::from_str::<ControlIdentity>(&malformed).is_err());
    }
}

#[test]
fn typed_decode_does_not_move_semantic_rejections_or_grant_authority() {
    let expected = identity();
    let mut encoded = serde_json::to_value(&expected).unwrap();
    encoded["protocol_version"] = json!(INTERNAL_PROTOCOL_VERSION + 1);
    encoded["retention_slot"] = json!(0);
    encoded["node"]["node_id"] = json!("foreign-node");
    let mut decoded: ControlIdentity = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        decoded.validate_node(&expected.node),
        Err(InternalError::UnsupportedVersion)
    );
    decoded.protocol_version = INTERNAL_PROTOCOL_VERSION;
    assert_eq!(
        decoded.validate_node(&expected.node),
        Err(InternalError::BindingMismatch)
    );
    decoded.node = expected.node.clone();
    assert_eq!(
        decoded.validate_node(&expected.node),
        Err(InternalError::BindingMismatch)
    );
    // JSON spelling/order may differ intentionally; the decoded identity does not.
    let alternate = serde_json::to_string(&expected)
        .unwrap()
        .replace("tenant-a", "tenant-\\u0061");
    assert_eq!(
        serde_json::from_str::<ControlIdentity>(&alternate).unwrap(),
        expected
    );
}
