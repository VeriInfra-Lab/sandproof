use sandproof_internal::TransportError;
use sandproof_internal::status::{
    HostStatus, STATUS_REQUEST_BYTES, STATUS_RESPONSE_BYTES, StatusRequest,
};
use sandproof_protocol::internal::{NodeIdentity, PositiveCounter, lifecycle::Resources};
use sandproof_schema_domains_extract::{
    request_decode, request_encode, response_decode, response_encode,
};
use serde_json::{Value, json};

fn node() -> NodeIdentity {
    NodeIdentity {
        deployment_id: "deployment-a".try_into().unwrap(),
        node_id: "node-a".try_into().unwrap(),
        node_generation: PositiveCounter::try_from(1).unwrap(),
    }
}

fn request() -> StatusRequest {
    StatusRequest { node: node() }
}

fn status() -> HostStatus {
    HostStatus {
        node: node(),
        retention_slots: 1,
        observed_at_ms: 0,
        free: Resources::ZERO,
    }
}

#[test]
fn exact_request_node_binding_includes_deployment_node_and_generation() {
    let request = request();
    assert_eq!(request.validate(&node()), Ok(()));
    let mut foreign = node();
    foreign.deployment_id = "other-deployment".try_into().unwrap();
    assert_eq!(
        request.validate(&foreign),
        Err(TransportError::Rejected),
        "same node text must not bypass the original deployment/generation binding"
    );
    foreign = node();
    foreign.node_id = "other-node".try_into().unwrap();
    assert_eq!(
        request.validate(&foreign),
        Err(TransportError::Rejected),
        "same node text must not bypass the original deployment/generation binding"
    );
    foreign = node();
    foreign.node_generation = PositiveCounter::try_from(2).unwrap();
    assert_eq!(
        request.validate(&foreign),
        Err(TransportError::Rejected),
        "same node text must not bypass the original deployment/generation binding"
    );
}

#[test]
fn every_u16_retention_count_has_exact_guard_outcome() {
    let request = request();
    let mut value = status();
    for slots in 0..=u16::MAX {
        value.retention_slots = slots;
        let expected = if (1..=4096).contains(&slots) {
            Ok(())
        } else {
            Err(TransportError::InvalidResponse)
        };
        assert_eq!(
            value.validate_for(&request),
            expected,
            "original retention range must reject zero and every value above 4096: {slots}"
        );
    }
}

#[test]
fn response_rejects_foreign_node_even_with_valid_retention() {
    let request = request();
    for part in 0..3 {
        let mut value = status();
        match part {
            0 => value.node.deployment_id = "other-deployment".try_into().unwrap(),
            1 => value.node.node_id = "other-node".try_into().unwrap(),
            _ => value.node.node_generation = PositiveCounter::try_from(2).unwrap(),
        }
        assert_eq!(
            value.validate_for(&request),
            Err(TransportError::InvalidResponse)
        );
    }
}

#[test]
fn zero_free_and_arbitrary_representable_observation_are_not_guard_rejections() {
    let request = request();
    for observed_at_ms in [0, 1, u64::MAX] {
        let mut value = status();
        value.observed_at_ms = observed_at_ms;
        assert_eq!(value.validate_for(&request), Ok(()));
        value.free = Resources {
            instances: u64::MAX,
            cpu: u64::MAX,
            memory_bytes: u64::MAX,
            disk_bytes: u64::MAX,
        };
        assert_eq!(value.validate_for(&request), Ok(()));
    }
}

#[test]
fn actual_request_and_response_roundtrip_keep_every_field_and_large_integer() {
    let request = request();
    assert_eq!(
        request_decode(&request_encode(&request).unwrap()).unwrap(),
        request
    );
    let value = HostStatus {
        node: NodeIdentity {
            node_generation: PositiveCounter::try_from(u64::MAX).unwrap(),
            ..node()
        },
        retention_slots: u16::MAX,
        observed_at_ms: u64::MAX,
        free: Resources {
            instances: 1,
            cpu: 2,
            memory_bytes: u64::MAX,
            disk_bytes: 4,
        },
    };
    let bytes = response_encode(&value).unwrap();
    assert_eq!(response_decode(&bytes).unwrap(), value);
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).unwrap()["observed_at_ms"],
        json!(u64::MAX)
    );
    // Schema accepts u16::MAX, the separate project guard rejects it.
    assert_eq!(
        value.validate_for(&request),
        Err(TransportError::InvalidResponse)
    );
}

#[test]
fn fixed_standard_derive_accepts_original_declared_sequence_order() {
    let req = request_decode(br#"[["deployment-a","node-a",1]]"#).unwrap();
    assert_eq!(req, request());
    let answer = response_decode(br#"[["deployment-a","node-a",1],1,0,[0,0,0,0]]"#).unwrap();
    assert_eq!(answer, status());
    assert!(
        response_decode(br#"[["deployment-a","node-a",1],0,1,[0,0,0,0]]"#)
            .unwrap()
            .validate_for(&req)
            .is_err()
    );
    for bytes in [
        b"[]".as_slice(),
        br#"[[]]"#,
        br#"[["deployment-a","node-a",1],1]"#,
    ] {
        assert!(response_decode(bytes).is_err());
    }
}

#[test]
fn all_response_root_fields_are_required_and_unknown_fields_rejected() {
    let encoded = serde_json::to_value(status()).unwrap();
    for field in ["node", "retention_slots", "observed_at_ms", "free"] {
        let mut value = encoded.clone();
        value.as_object_mut().unwrap().remove(field);
        let error = response_decode(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(&format!("missing field `{field}`"))
        );
    }
    let mut value = encoded;
    value["unknown"] = json!(true);
    assert!(
        response_decode(&serde_json::to_vec(&value).unwrap())
            .unwrap_err()
            .to_string()
            .contains("unknown field `unknown`")
    );
}

#[test]
fn nested_node_and_resource_fields_are_required_and_unknown_fields_rejected() {
    let encoded = serde_json::to_value(status()).unwrap();
    for (parent, fields) in [
        ("node", vec!["deployment_id", "node_id", "node_generation"]),
        (
            "free",
            vec!["instances", "cpu", "memory_bytes", "disk_bytes"],
        ),
    ] {
        for field in fields {
            let mut value = encoded.clone();
            value[parent].as_object_mut().unwrap().remove(field);
            assert!(
                response_decode(&serde_json::to_vec(&value).unwrap())
                    .unwrap_err()
                    .to_string()
                    .contains(&format!("missing field `{field}`"))
            );
        }
        let mut value = encoded.clone();
        value[parent]["unknown"] = json!(true);
        assert!(
            response_decode(&serde_json::to_vec(&value).unwrap())
                .unwrap_err()
                .to_string()
                .contains("unknown field `unknown`")
        );
    }
}

#[test]
fn duplicate_root_and_nested_members_are_not_silently_overwritten() {
    for text in [
        r#"{"node":{"deployment_id":"deployment-a","node_id":"node-a","node_generation":1},"node":{}}"#,
        r#"{"node":{"deployment_id":"deployment-a","node_id":"node-a","node_id":"node-b","node_generation":1}}"#,
    ] {
        assert!(
            request_decode(text.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("duplicate field")
        );
    }
    let base = String::from_utf8(response_encode(&status()).unwrap()).unwrap();
    let duplicate = base.replace(
        "\"retention_slots\":1",
        "\"retention_slots\":1,\"retention_slots\":2",
    );
    assert!(
        response_decode(duplicate.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("duplicate field `retention_slots`")
    );
    let duplicate = base.replace("\"cpu\":0", "\"cpu\":0,\"cpu\":1");
    assert!(
        response_decode(duplicate.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("duplicate field `cpu`")
    );
}

#[test]
fn null_missing_and_unknown_request_shape_is_rejected() {
    for text in [
        "null",
        "{}",
        "{\"node\":null}",
        "{\"node\":[]}",
        "{\"node\":{\"deployment_id\":\"deployment-a\",\"node_id\":\"node-a\",\"node_generation\":1},\"unknown\":true}",
    ] {
        assert!(request_decode(text.as_bytes()).is_err(), "accepted {text}");
    }
}

#[test]
fn unsigned_machine_domains_are_not_silently_narrowed() {
    let base = serde_json::to_value(status()).unwrap();
    for value in [
        json!(-1),
        json!(65536_u64),
        json!(1.0),
        json!(null),
        json!("1"),
    ] {
        let mut answer = base.clone();
        answer["retention_slots"] = value;
        assert!(response_decode(&serde_json::to_vec(&answer).unwrap()).is_err());
    }
    for path in [
        "observed_at_ms",
        "free.instances",
        "free.cpu",
        "free.memory_bytes",
        "free.disk_bytes",
    ] {
        for value in [json!(-1), json!(1.0), json!(null), json!("1")] {
            let mut answer = base.clone();
            if let Some(field) = path.strip_prefix("free.") {
                answer["free"][field] = value;
            } else {
                answer[path] = value;
            }
            assert!(response_decode(&serde_json::to_vec(&answer).unwrap()).is_err());
        }
    }
    let text = String::from_utf8(response_encode(&status()).unwrap())
        .unwrap()
        .replace(
            "\"observed_at_ms\":0",
            "\"observed_at_ms\":18446744073709551616",
        );
    assert!(response_decode(text.as_bytes()).is_err());
}

#[test]
fn original_identifier_and_counter_leaf_errors_precede_later_truncation() {
    let invalid_id =
        br#"{"node":{"deployment_id":"bad!","node_id":"node-a","node_generation":1},"later":["#;
    assert!(
        request_decode(invalid_id)
            .unwrap_err()
            .to_string()
            .contains("invalid opaque identifier")
    );
    let zero_counter = br#"{"node":{"deployment_id":"deployment-a","node_id":"node-a","node_generation":0},"later":["#;
    assert!(
        request_decode(zero_counter)
            .unwrap_err()
            .to_string()
            .contains("invalid or exhausted internal counter")
    );
}

#[test]
fn parsed_escape_keeps_original_valid_text_and_non_ascii_is_not_normalized() {
    let escaped =
        br#"{"node":{"deployment_id":"deployment-a","node_id":"\u006eode-a","node_generation":1}}"#;
    assert_eq!(request_decode(escaped).unwrap(), request());
    let non_ascii =
        br#"{"node":{"deployment_id":"deployment-a","node_id":"\u00e9","node_generation":1}}"#;
    assert!(
        request_decode(non_ascii)
            .unwrap_err()
            .to_string()
            .contains("invalid opaque identifier")
    );
    let mut invalid_utf8 = request_encode(&request()).unwrap();
    let position = invalid_utf8.iter().position(|b| *b == b'n').unwrap();
    invalid_utf8[position] = 0xff;
    assert!(request_decode(&invalid_utf8).is_err());
}

#[test]
fn document_end_is_checked_after_original_fields() {
    let original = request_encode(&request()).unwrap();
    let mut whitespace = original.clone();
    whitespace.extend_from_slice(b" \r\n\t");
    assert_eq!(request_decode(&whitespace).unwrap(), request());
    let mut trailing = original;
    trailing.extend_from_slice(b" x");
    assert!(
        request_decode(&trailing)
            .unwrap_err()
            .to_string()
            .contains("trailing characters")
    );
}

#[test]
fn legal_maximum_identifiers_and_integers_fit_original_wire_budgets() {
    let request = StatusRequest {
        node: NodeIdentity {
            deployment_id: "a".repeat(128).try_into().unwrap(),
            node_id: "b".repeat(128).try_into().unwrap(),
            node_generation: PositiveCounter::try_from(u64::MAX).unwrap(),
        },
    };
    assert!(request_encode(&request).unwrap().len() <= STATUS_REQUEST_BYTES);
    let value = HostStatus {
        node: request.node.clone(),
        retention_slots: 4096,
        observed_at_ms: u64::MAX,
        free: Resources {
            instances: u64::MAX,
            cpu: u64::MAX,
            memory_bytes: u64::MAX,
            disk_bytes: u64::MAX,
        },
    };
    assert!(response_encode(&value).unwrap().len() <= STATUS_RESPONSE_BYTES);
    assert_eq!(value.validate_for(&request), Ok(()));
    // Byte limits belong to original transport callers, not from_slice itself.
    let mut padded = vec![b' '; STATUS_REQUEST_BYTES + 1];
    padded.extend_from_slice(&request_encode(&request).unwrap());
    assert!(request_decode(&padded).is_ok());
}

#[test]
fn ordinary_compact_ascii_fields_fit_the_reviewed_schema_allowance() {
    for text in ["a".to_owned(), "Z9-_.".to_owned(), "a".repeat(128)] {
        let request = StatusRequest {
            node: NodeIdentity {
                deployment_id: text.clone().try_into().unwrap(),
                node_id: text.clone().try_into().unwrap(),
                node_generation: PositiveCounter::try_from(u64::MAX).unwrap(),
            },
        };
        let ids = 2 * text.len();
        let request_bytes = request_encode(&request).unwrap();
        assert!(request_bytes.len() <= 128 + ids);
        assert!(request_bytes.len() <= STATUS_REQUEST_BYTES);
        let value = HostStatus {
            node: request.node,
            retention_slots: u16::MAX,
            observed_at_ms: u64::MAX,
            free: Resources {
                instances: u64::MAX,
                cpu: u64::MAX,
                memory_bytes: u64::MAX,
                disk_bytes: u64::MAX,
            },
        };
        let response_bytes = response_encode(&value).unwrap();
        assert!(response_bytes.len() <= 512 + ids);
        assert!(response_bytes.len() <= STATUS_RESPONSE_BYTES);
        assert_eq!(response_decode(&response_bytes).unwrap(), value);
    }
}

#[test]
fn original_server_bounded_writer_preserves_status_bytes_and_exact_limit() {
    use sandproof_controller_service::{JournalError, template_registry::evidence::encode_bounded};
    let value = HostStatus {
        observed_at_ms: u64::MAX,
        free: Resources {
            instances: 1,
            cpu: 2,
            memory_bytes: u64::MAX,
            disk_bytes: 4,
        },
        ..status()
    };
    let reference = response_encode(&value).unwrap();
    let actual = encode_bounded(&value, STATUS_RESPONSE_BYTES).unwrap();
    assert_eq!(actual, reference);
    assert_eq!(response_decode(&actual).unwrap(), value);
    assert_eq!(encode_bounded(&value, reference.len()).unwrap(), reference);
    assert!(matches!(
        encode_bounded(&value, reference.len() - 1),
        Err(JournalError::MaintenanceCapacity)
    ));
    assert!(matches!(
        encode_bounded(&value, 0),
        Err(JournalError::MaintenanceCapacity)
    ));
    // Engineering evidence for the actual server helper, not a general
    // refinement or an allocation-failure proof for its private Writer.
}
