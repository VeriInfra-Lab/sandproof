//! Direct probes for the production byte decoder, encoder and own validator.
//! These are extraction entries, not a replacement server or a copied parser.
use sandproof_protocol::internal::lifecycle::LifecycleRequest;
use sandproof_protocol::internal::{InternalError, NodeIdentity};

pub fn decode(bytes: &[u8]) -> Result<LifecycleRequest, serde_json::Error> {
    serde_json::from_slice::<LifecycleRequest>(bytes)
}

pub fn encode(request: &LifecycleRequest) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(request)
}

pub fn validate(request: &LifecycleRequest, node: &NodeIdentity) -> Result<(), InternalError> {
    request.validate(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandproof_protocol::internal::lifecycle::{
        CreateBinding, Intent, LIFECYCLE_BODY_BYTES, MAX_OPTIONS_BYTES, Resources,
    };
    use sandproof_protocol::internal::{
        ControlIdentity, INTERNAL_PROTOCOL_VERSION, PositiveCounter,
    };
    use sandproof_protocol::{DeploymentId, NodeId, OperationId, ResourceId, SandboxId, TenantId};
    use serde_json::{Value, error::Category, json};

    fn request() -> LifecycleRequest {
        LifecycleRequest {
            identity: ControlIdentity {
                protocol_version: INTERNAL_PROTOCOL_VERSION,
                node: NodeIdentity {
                    deployment_id: DeploymentId::try_from("dep-a").unwrap(),
                    node_id: NodeId::try_from("node-a").unwrap(),
                    node_generation: PositiveCounter::try_from(1).unwrap(),
                },
                tenant_id: TenantId::try_from("tenant-a").unwrap(),
                sandbox_id: SandboxId::try_from("sandbox-a").unwrap(),
                operation_id: OperationId::try_from("operation-a").unwrap(),
                intent_version: PositiveCounter::try_from(1).unwrap(),
                retention_slot: 1,
                request_window: PositiveCounter::try_from(1).unwrap(),
            },
            create: CreateBinding {
                template_revision: ResourceId::try_from("revision-a").unwrap(),
                template_digest: std::array::from_fn(|i| u8::try_from(i).unwrap()),
                resources: Resources {
                    instances: 1,
                    cpu: 2,
                    memory_bytes: 4096,
                    disk_bytes: 8192,
                },
                options: vec![0, 127, 255],
                startup_deadline_ms: 10,
                timeout_ms: 1,
                max_lifetime_ms: 2,
            },
            intent: Intent::Create,
        }
    }

    fn bytes(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap()
    }

    #[test]
    fn full_roundtrip_preserves_every_field_and_all_intents() {
        for intent in [
            Intent::Create,
            Intent::Renew { end_at_ms: 11 },
            Intent::Destroy,
        ] {
            let mut expected = request();
            expected.intent = intent;
            let encoded = encode(&expected).unwrap();
            assert!(encoded.len() <= LIFECYCLE_BODY_BYTES);
            let actual = decode(&encoded).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(validate(&actual, &expected.identity.node), Ok(()));
        }
    }

    #[test]
    fn derived_struct_sequences_preserve_complete_values() {
        let expected = request();
        let object = serde_json::to_value(&expected).unwrap();
        let top_sequence = json!([object["identity"], object["create"], object["intent"]]);
        assert_eq!(decode(&bytes(&top_sequence)).unwrap(), expected);

        let node = &object["identity"]["node"];
        let identity = &object["identity"];
        let create = &object["create"];
        let resources = &create["resources"];
        let nested_sequences = json!([
            [
                identity["protocol_version"],
                [
                    node["deployment_id"],
                    node["node_id"],
                    node["node_generation"]
                ],
                identity["tenant_id"],
                identity["sandbox_id"],
                identity["operation_id"],
                identity["intent_version"],
                identity["retention_slot"],
                identity["request_window"]
            ],
            [
                create["template_revision"],
                create["template_digest"],
                [
                    resources["instances"],
                    resources["cpu"],
                    resources["memory_bytes"],
                    resources["disk_bytes"]
                ],
                create["options"],
                create["startup_deadline_ms"],
                create["timeout_ms"],
                create["max_lifetime_ms"]
            ],
            object["intent"]
        ]);
        assert_eq!(decode(&bytes(&nested_sequences)).unwrap(), expected);
    }

    #[test]
    fn missing_duplicate_unknown_and_trailing_inputs_are_rejected() {
        let expected = request();
        let text = String::from_utf8(encode(&expected).unwrap()).unwrap();
        let duplicate = format!(
            "{{\"identity\":{},{}",
            serde_json::to_string(&expected.identity).unwrap(),
            &text[1..]
        );
        assert!(decode(duplicate.as_bytes()).is_err());
        let unknown = format!("{{\"extra\":0,{}", &text[1..]);
        assert!(decode(unknown.as_bytes()).is_err());
        let mut missing = serde_json::to_value(&expected).unwrap();
        missing.as_object_mut().unwrap().remove("intent");
        assert!(decode(&bytes(&missing)).is_err());
        let mut nested = serde_json::to_value(&expected).unwrap();
        nested["create"]["resources"]["extra"] = json!(1);
        assert!(decode(&bytes(&nested)).is_err());
        assert!(decode(format!("{text} x").as_bytes()).is_err());
    }

    #[test]
    fn numeric_width_and_fixed_digest_are_real_decoder_conditions() {
        let original = serde_json::to_value(request()).unwrap();
        for size in [31, 33] {
            let mut bad = original.clone();
            bad["create"]["template_digest"] = json!(vec![0; size]);
            assert!(decode(&bytes(&bad)).is_err());
        }
        for value in [json!(-1), json!(256), json!(1.0)] {
            let mut bad = original.clone();
            bad["create"]["options"] = json!([value]);
            assert!(decode(&bytes(&bad)).is_err());
        }
        let mut bad = original.clone();
        bad["identity"]["protocol_version"] = json!(65536);
        assert!(decode(&bytes(&bad)).is_err());
        let mut bad = original;
        bad["identity"]["node"]["node_generation"] = json!(0);
        assert!(decode(&bytes(&bad)).is_err());
    }

    #[test]
    fn own_validation_order_and_shape_are_separate_from_authority() {
        let expected = request();
        let mut bad = expected.clone();
        bad.identity.protocol_version = INTERNAL_PROTOCOL_VERSION + 1;
        bad.identity.node.node_id = NodeId::try_from("other-node").unwrap();
        bad.identity.retention_slot = 0;
        bad.create.resources.cpu = 0;
        assert_eq!(
            validate(&bad, &expected.identity.node),
            Err(InternalError::UnsupportedVersion)
        );

        let mut bad = expected.clone();
        bad.identity.retention_slot = 0;
        let decoded = decode(&encode(&bad).unwrap()).unwrap();
        assert_eq!(
            validate(&decoded, &expected.identity.node),
            Err(InternalError::BindingMismatch)
        );

        let mut renew = expected.clone();
        renew.intent = Intent::Renew {
            end_at_ms: u64::MAX,
        };
        assert_eq!(validate(&renew, &expected.identity.node), Ok(()));
        renew.intent = Intent::Renew { end_at_ms: 0 };
        assert_eq!(
            validate(&renew, &expected.identity.node),
            Err(InternalError::BindingMismatch)
        );

        let mut destroy = expected.clone();
        destroy.intent = Intent::Destroy;
        destroy.create.options.clear();
        assert_eq!(
            validate(&destroy, &expected.identity.node),
            Err(InternalError::BindingMismatch)
        );
    }

    #[test]
    fn own_option_limit_and_transport_limit_have_distinct_providers() {
        let mut expected = request();
        expected.create.options = vec![255; MAX_OPTIONS_BYTES];
        let encoded = encode(&expected).unwrap();
        assert!(encoded.len() <= LIFECYCLE_BODY_BYTES);
        let actual = decode(&encoded).unwrap();
        assert_eq!(actual.create.options.len(), MAX_OPTIONS_BYTES);
        assert_eq!(validate(&actual, &expected.identity.node), Ok(()));
        expected.create.options.push(255);
        let actual = decode(&encode(&expected).unwrap()).unwrap();
        assert_eq!(
            validate(&actual, &expected.identity.node),
            Err(InternalError::BindingMismatch)
        );

        // from_slice does not implement the HTTP route's Limited budget.
        let mut padded = encode(&request()).unwrap();
        padded.resize(LIFECYCLE_BODY_BYTES + 1, b' ');
        assert!(decode(&padded).is_ok());
    }

    #[test]
    fn escape_decoding_preserves_values_and_duplicate_key_rejection() {
        let expected = request();
        let text = String::from_utf8(encode(&expected).unwrap()).unwrap();
        let escaped = text.replace("\"tenant-a\"", r#""\u0074enant-a""#);
        assert_ne!(escaped, text);
        assert_eq!(decode(escaped.as_bytes()).unwrap(), expected);
        let duplicate = format!(
            r#"{{"ident\u0069ty":{},{}"#,
            serde_json::to_string(&expected.identity).unwrap(),
            &text[1..]
        );
        assert!(decode(duplicate.as_bytes()).is_err());
        let mut invalid_utf8 = text.into_bytes();
        let pos = invalid_utf8
            .windows(b"tenant-a".len())
            .position(|w| w == b"tenant-a")
            .unwrap();
        invalid_utf8[pos] = 255;
        assert!(decode(&invalid_utf8).is_err());
    }

    #[test]
    fn own_counter_error_precedes_engine_trailing_check() {
        let text = String::from_utf8(encode(&request()).unwrap()).unwrap();
        assert_eq!(text.matches("\"intent_version\":1").count(), 1);
        let zero = text.replace("\"intent_version\":1", "\"intent_version\":0");
        let error = decode(format!("{zero} x").as_bytes()).unwrap_err();
        assert_eq!(error.classify(), Category::Data);
        assert!(
            error
                .to_string()
                .starts_with("invalid or exhausted internal counter")
        );
        assert_eq!(error.line(), 1);
        assert!(error.column() > 0 && error.column() < text.len());

        let error = decode(format!("{text} x").as_bytes()).unwrap_err();
        assert_eq!(error.classify(), Category::Syntax);
        assert_eq!((error.line(), error.column()), (1, text.len() + 2));
    }

    #[test]
    fn every_custom_field_uses_its_original_leaf() {
        let original = serde_json::to_value(request()).unwrap();
        for path in [
            "/identity/node/deployment_id",
            "/identity/node/node_id",
            "/identity/tenant_id",
            "/identity/sandbox_id",
            "/identity/operation_id",
            "/create/template_revision",
        ] {
            for value in [
                String::new(),
                "a".repeat(129),
                "é".to_owned(),
                "a\0b".to_owned(),
            ] {
                let mut changed = original.clone();
                *changed.pointer_mut(path).unwrap() = json!(value);
                let error = decode(&bytes(&changed)).unwrap_err();
                assert_eq!(error.classify(), Category::Data, "{path}");
                assert!(
                    error.to_string().starts_with("invalid opaque identifier"),
                    "{path}: {error}"
                );
            }
            let mut changed = original.clone();
            *changed.pointer_mut(path).unwrap() = json!("a".repeat(128));
            assert!(decode(&bytes(&changed)).is_ok(), "{path}");
        }
        for path in [
            "/identity/node/node_generation",
            "/identity/intent_version",
            "/identity/request_window",
        ] {
            let mut changed = original.clone();
            *changed.pointer_mut(path).unwrap() = json!(0);
            let error = decode(&bytes(&changed)).unwrap_err();
            assert_eq!(error.classify(), Category::Data, "{path}");
            assert!(
                error
                    .to_string()
                    .starts_with("invalid or exhausted internal counter"),
                "{path}: {error}"
            );
            *changed.pointer_mut(path).unwrap() = json!(u64::MAX);
            assert!(decode(&bytes(&changed)).is_ok(), "{path}");
        }
    }

    #[test]
    fn own_leaf_error_precedes_later_incomplete_json() {
        let text = String::from_utf8(encode(&request()).unwrap()).unwrap();
        let marker = ",\"create\":";
        let end = text.find(marker).unwrap() + marker.len();
        let incomplete = &text[..end];
        assert_eq!(
            decode(incomplete.as_bytes()).unwrap_err().classify(),
            Category::Eof
        );
        for (from, to, message) in [
            (
                "\"intent_version\":1",
                "\"intent_version\":0",
                "invalid or exhausted internal counter",
            ),
            (
                "\"tenant_id\":\"tenant-a\"",
                "\"tenant_id\":\"\"",
                "invalid opaque identifier",
            ),
        ] {
            assert_eq!(incomplete.matches(from).count(), 1);
            let bad = incomplete.replace(from, to);
            let error = decode(bad.as_bytes()).unwrap_err();
            assert_eq!(error.classify(), Category::Data);
            assert!(error.to_string().starts_with(message));
            assert!(error.column() < bad.len());
        }
    }

    #[test]
    fn internally_tagged_unit_variants_follow_frozen_serde_policy() {
        for intent in [Intent::Create, Intent::Destroy] {
            let mut expected = request();
            expected.intent = intent;
            let mut value = serde_json::to_value(&expected).unwrap();
            // The frozen unit visitor consumes and ignores these keys. They
            // do not select Renew or call an identifier decoder at this path.
            value["intent"]["end_at_ms"] = json!(0);
            value["intent"]["tenant_id"] = json!("");
            assert_eq!(decode(&bytes(&value)).unwrap(), expected);
        }
        let mut expected = request();
        expected.intent = Intent::Renew { end_at_ms: 11 };
        let mut value = serde_json::to_value(&expected).unwrap();
        value["intent"]["extra"] = json!(0);
        let error = decode(&bytes(&value)).unwrap_err();
        assert_eq!(error.classify(), Category::Data);
        assert!(error.to_string().contains("unknown field `extra`"));
    }
}
