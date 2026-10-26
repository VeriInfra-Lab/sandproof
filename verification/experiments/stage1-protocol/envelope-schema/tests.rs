use sandproof_protocol::internal::NodeIdentity;
use sandproof_protocol::internal::lifecycle::{CreateBinding, Resources};
use sandproof_protocol::lifecycle::{
    AcceptedIntentError, AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity,
    ContentDigest, ExpectedSession, LifecycleOperation, LifecycleRequest, OperationBinding,
    OperationKey, RecordedLifecycleIntent, RetentionBinding,
};
use serde_json::{Value, json};

fn node() -> NodeIdentity {
    NodeIdentity {
        deployment_id: "deployment".try_into().unwrap(),
        node_id: "host".try_into().unwrap(),
        node_generation: 1.try_into().unwrap(),
    }
}

fn samples() -> Vec<AcceptedLifecycleIntent> {
    let operations = [
        LifecycleOperation::Create {
            create: CreateBinding {
                template_revision: "base".try_into().unwrap(),
                template_digest: [7; 32],
                resources: Resources {
                    instances: 1,
                    cpu: 2,
                    memory_bytes: 3,
                    disk_bytes: 4,
                },
                options: vec![0, 1, 255],
                startup_deadline_ms: 5,
                timeout_ms: 6,
                max_lifetime_ms: 7,
            },
            target_incarnation: 1.try_into().unwrap(),
        },
        LifecycleOperation::Unload {
            source_incarnation: 1.try_into().unwrap(),
            capture: 2.try_into().unwrap(),
        },
        LifecycleOperation::Resume {
            source_incarnation: 1.try_into().unwrap(),
            capture: 2.try_into().unwrap(),
            target_incarnation: 9.try_into().unwrap(),
            manifest_digest: ContentDigest::from_sha256([8; 32]),
        },
        LifecycleOperation::Renew {
            incarnation: 1.try_into().unwrap(),
            end_at_ms: -1,
            extend_only: false,
        },
        LifecycleOperation::Destroy {},
    ];
    operations
        .into_iter()
        .enumerate()
        .map(|(index, operation)| AcceptedLifecycleIntent {
            request: LifecycleRequest {
                binding: OperationBinding {
                    protocol_version: 4,
                    node: node(),
                    session: "session".try_into().unwrap(),
                    key: OperationKey {
                        deployment: "deployment".try_into().unwrap(),
                        owner: "owner".try_into().unwrap(),
                        window: 3.try_into().unwrap(),
                        sequence: 4.try_into().unwrap(),
                    },
                    expected: if index == 0 {
                        ExpectedSession::Absent
                    } else {
                        ExpectedSession::Version(0)
                    },
                },
                operation,
            },
            accepted_version: 1,
            center: CenterIdentity {
                deployment: "deployment".try_into().unwrap(),
                center: "center".try_into().unwrap(),
            },
            retention: RetentionBinding {
                slot: 1,
                window: 9.try_into().unwrap(),
            },
        })
        .collect()
}

fn expected(index: usize) -> Value {
    let operations = [
        json!({"kind":"create", "create": {
            "template_revision":"base", "template_digest":vec![7; 32],
            "resources":{"instances":1,"cpu":2,"memory_bytes":3,"disk_bytes":4},
            "options":[0,1,255], "startup_deadline_ms":5,"timeout_ms":6,"max_lifetime_ms":7
        },"target_incarnation":1}),
        json!({"kind":"unload","source_incarnation":1,"capture":2}),
        json!({"kind":"resume","source_incarnation":1,"capture":2,"target_incarnation":9,"manifest_digest":vec![8; 32]}),
        json!({"kind":"renew","incarnation":1,"end_at_ms":-1,"extend_only":false}),
        json!({"kind":"destroy"}),
    ];
    json!({
        "request":{"binding":{"protocol_version":4,
            "node":{"deployment_id":"deployment","node_id":"host","node_generation":1},
            "session":"session","key":{"deployment":"deployment","owner":"owner","window":3,"sequence":4},
            "expected":if index == 0 { json!({"state":"absent"}) } else { json!({"state":"version","version":0}) }
        },"operation":operations[index]},
        "accepted_version":1,"center":{"deployment":"deployment","center":"center"},
        "retention":{"slot":1,"window":9}
    })
}

#[test]
fn actual_five_branch_fields_and_roundtrips() {
    for (index, original) in samples().into_iter().enumerate() {
        assert_eq!(serde_json::to_value(&original).unwrap(), expected(index));
        let bytes = serde_json::to_vec(&original).unwrap();
        let decoded: AcceptedLifecycleIntent = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, original);
        let context = AuthenticatedCenterContext::from_verified_peer_registration(
            original.center.clone(),
            node(),
        );
        assert_eq!(decoded.validate(&context), Ok(()));
        let recorded = RecordedLifecycleIntent {
            intent: original.clone(),
            current_session_version: 1,
            current: true,
            terminal: false,
        };
        assert_eq!(
            serde_json::to_value(&recorded).unwrap(),
            json!({"intent":expected(index),"current_session_version":1,"current":true,"terminal":false})
        );
        let returned: RecordedLifecycleIntent =
            serde_json::from_slice(&serde_json::to_vec(&recorded).unwrap()).unwrap();
        assert_eq!(returned, recorded);
        assert!(returned.matches(&original));
    }
}

// Mutate every primitive field, including each digest/options byte. This is
// regression evidence only; the Lean theorem quantifies over arbitrary values.
fn variants(value: &Value) -> Vec<Value> {
    match value {
        Value::String(text) => vec![json!(format!("{text}x"))],
        Value::Number(number) => vec![json!(number.as_i64().unwrap() + 1)],
        Value::Bool(flag) => vec![json!(!flag)],
        Value::Array(items) => items
            .iter()
            .enumerate()
            .flat_map(|(index, item)| {
                variants(item).into_iter().map(move |changed| {
                    let mut copy = value.clone();
                    copy[index] = changed;
                    copy
                })
            })
            .collect(),
        Value::Object(fields) => fields
            .iter()
            .flat_map(|(key, item)| {
                variants(item).into_iter().map(move |changed| {
                    let mut copy = value.clone();
                    copy[key] = changed;
                    copy
                })
            })
            .collect(),
        Value::Null => panic!("no nullable fields in canonical output"),
    }
}

#[test]
fn field_mutations_cannot_silently_equal_original() {
    let mut checked = 0;
    for (index, original) in samples().into_iter().enumerate() {
        for changed in variants(&expected(index)) {
            checked += 1;
            if let Ok(decoded) = serde_json::from_value::<AcceptedLifecycleIntent>(changed) {
                assert_ne!(decoded, original);
                let recorded = RecordedLifecycleIntent {
                    intent: decoded,
                    current_session_version: 1,
                    current: true,
                    terminal: false,
                };
                assert!(!recorded.matches(&original));
            }
        }
    }
    assert_eq!(checked, 168);
}

#[test]
fn ordinary_zero_signed_extremes_and_boolean_remain_representable() {
    for time in [i64::MIN, -1, 0, i64::MAX] {
        for flag in [false, true] {
            let mut value = expected(3);
            value["request"]["operation"]["end_at_ms"] = json!(time);
            value["request"]["operation"]["extend_only"] = json!(flag);
            value["accepted_version"] = json!(0);
            value["retention"]["slot"] = json!(0);
            value["request"]["binding"]["protocol_version"] = json!(0);
            let decoded: AcceptedLifecycleIntent = serde_json::from_value(value).unwrap();
            assert!(matches!(
                decoded.request.binding.expected,
                ExpectedSession::Version(0)
            ));
            assert_eq!(decoded.accepted_version, 0);
            assert!(
                matches!(decoded.request.operation, LifecycleOperation::Renew { end_at_ms, extend_only, .. } if end_at_ms == time && extend_only == flag)
            );
        }
    }
}

#[test]
fn own_leaves_reject_zero_and_invalid_identifier_before_document_end() {
    for pointer in [
        "/request/binding/node/node_generation",
        "/request/binding/key/window",
        "/request/binding/key/sequence",
        "/retention/window",
        "/request/operation/source_incarnation",
        "/request/operation/capture",
        "/request/operation/target_incarnation",
    ] {
        let mut value = expected(2);
        *value.pointer_mut(pointer).unwrap() = json!(0);
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.extend_from_slice(b" x");
        let error = serde_json::from_slice::<AcceptedLifecycleIntent>(&bytes).unwrap_err();
        assert_eq!(error.classify(), serde_json::error::Category::Data);
        assert!(
            error
                .to_string()
                .contains("invalid or exhausted internal counter")
        );
    }
    for pointer in [
        "/request/binding/node/deployment_id",
        "/request/binding/node/node_id",
        "/request/binding/session",
        "/request/binding/key/deployment",
        "/request/binding/key/owner",
        "/center/deployment",
        "/center/center",
        "/request/operation/create/template_revision",
    ] {
        let mut value = expected(0);
        *value.pointer_mut(pointer).unwrap() = json!("");
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.extend_from_slice(b" x");
        assert!(
            serde_json::from_slice::<AcceptedLifecycleIntent>(&bytes)
                .unwrap_err()
                .to_string()
                .contains("invalid opaque identifier")
        );
    }
}

#[test]
fn unknown_missing_duplicate_fields_use_current_derive() {
    for index in 0..5 {
        let original = expected(index);
        let mut pointers = vec![
            "",
            "/request",
            "/request/binding",
            "/request/binding/node",
            "/request/binding/key",
            "/request/binding/expected",
            "/request/operation",
            "/center",
            "/retention",
        ];
        if index == 0 {
            pointers.extend([
                "/request/operation/create",
                "/request/operation/create/resources",
            ]);
        }
        for pointer in pointers {
            let mut value = original.clone();
            value
                .pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("extra".into(), json!(1));
            assert!(
                serde_json::from_value::<AcceptedLifecycleIntent>(value).is_err(),
                "extra field {index}:{pointer}"
            );
            for key in original
                .pointer(pointer)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
            {
                let mut value = original.clone();
                value
                    .pointer_mut(pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap()
                    .remove(key);
                assert!(
                    serde_json::from_value::<AcceptedLifecycleIntent>(value).is_err(),
                    "missing {index}:{pointer}/{key}"
                );
                let object = original.pointer(pointer).unwrap();
                let object_bytes = serde_json::to_string(object).unwrap();
                let repeated = format!(
                    "{{{}:{},{}",
                    serde_json::to_string(key).unwrap(),
                    serde_json::to_string(&object[key]).unwrap(),
                    &object_bytes[1..]
                );
                let bytes =
                    serde_json::to_string(&original)
                        .unwrap()
                        .replacen(&object_bytes, &repeated, 1);
                assert!(
                    serde_json::from_slice::<AcceptedLifecycleIntent>(bytes.as_bytes()).is_err(),
                    "duplicate {index}:{pointer}/{key}"
                );
            }
        }
        // Value maps erase duplicates, so exercise bytes directly.
        let bytes = serde_json::to_string(&original).unwrap();
        let duplicate = format!("{{\"accepted_version\":1,{}", &bytes[1..]);
        assert!(
            serde_json::from_slice::<AcceptedLifecycleIntent>(duplicate.as_bytes())
                .unwrap_err()
                .to_string()
                .contains("duplicate field `accepted_version`")
        );
    }
}

#[test]
fn declaration_order_struct_sequences_and_adjacent_enum_input() {
    for original in samples() {
        let value = serde_json::to_value(&original).unwrap();
        let sequence = json!([
            value["request"],
            value["accepted_version"],
            value["center"],
            value["retention"]
        ]);
        assert_eq!(
            serde_json::from_value::<AcceptedLifecycleIntent>(sequence).unwrap(),
            original
        );
    }
    assert_eq!(
        serde_json::from_str::<ExpectedSession>(r#"{"state":"absent","version":null}"#).unwrap(),
        ExpectedSession::Absent
    );
    assert_eq!(
        serde_json::from_str::<ExpectedSession>(r#"["version",0]"#).unwrap(),
        ExpectedSession::Version(0)
    );
    assert!(serde_json::from_str::<ExpectedSession>(r#"{"state":"absent","version":1}"#).is_err());
    assert_eq!(
        serde_json::from_str::<LifecycleOperation>(r#"["destroy"]"#).unwrap(),
        LifecycleOperation::Destroy {}
    );
    for (index, original) in samples().into_iter().enumerate() {
        let value = expected(index)["request"]["operation"].clone();
        let ordered = match index {
            0 => json!(["create", value["create"], value["target_incarnation"]]),
            1 => json!(["unload", value["source_incarnation"], value["capture"]]),
            2 => json!([
                "resume",
                value["source_incarnation"],
                value["capture"],
                value["target_incarnation"],
                value["manifest_digest"]
            ]),
            3 => json!([
                "renew",
                value["incarnation"],
                value["end_at_ms"],
                value["extend_only"]
            ]),
            _ => json!(["destroy"]),
        };
        assert_eq!(
            serde_json::from_value::<LifecycleOperation>(ordered.clone()).unwrap(),
            original.request.operation
        );
        let mut long = ordered.clone();
        long.as_array_mut().unwrap().push(json!(0));
        assert!(serde_json::from_value::<LifecycleOperation>(long).is_err());
        let mut short = ordered;
        short.as_array_mut().unwrap().pop();
        assert!(serde_json::from_value::<LifecycleOperation>(short).is_err());
    }
}

#[test]
fn accepted_guard_remains_separate_and_ordered() {
    let mut value = samples().remove(1);
    let context =
        AuthenticatedCenterContext::from_verified_peer_registration(value.center.clone(), node());
    value.accepted_version = 0;
    value.retention.slot = 0;
    value.center.center = "foreign".try_into().unwrap();
    let decoded: AcceptedLifecycleIntent =
        serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        decoded.validate(&context),
        Err(AcceptedIntentError::ForeignCenter)
    );
    value.center = context.center().clone();
    assert_eq!(
        value.validate(&context),
        Err(AcceptedIntentError::InvalidRetention)
    );
    value.retention.slot = 1;
    assert_eq!(
        value.validate(&context),
        Err(AcceptedIntentError::InvalidAcceptedVersion)
    );
}
