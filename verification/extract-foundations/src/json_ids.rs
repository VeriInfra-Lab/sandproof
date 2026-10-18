#![forbid(unsafe_code)]

//! Direct calls to the production Deserialize implementations; no validator copy.
use serde::{Deserialize, Deserializer};

pub fn tenant_decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<sandproof_protocol::TenantId, D::Error> {
    sandproof_protocol::TenantId::deserialize(input)
}

pub fn sandbox_decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<sandproof_protocol::SandboxId, D::Error> {
    sandproof_protocol::SandboxId::deserialize(input)
}

pub fn operation_decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<sandproof_protocol::OperationId, D::Error> {
    sandproof_protocol::OperationId::deserialize(input)
}

pub fn logical_action_decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<sandproof_protocol::LogicalActionId, D::Error> {
    sandproof_protocol::LogicalActionId::deserialize(input)
}

pub fn resource_decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<sandproof_protocol::ResourceId, D::Error> {
    sandproof_protocol::ResourceId::deserialize(input)
}

pub fn client_request_decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<sandproof_protocol::ClientRequestId, D::Error> {
    sandproof_protocol::ClientRequestId::deserialize(input)
}

pub fn deployment_decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<sandproof_protocol::DeploymentId, D::Error> {
    sandproof_protocol::DeploymentId::deserialize(input)
}

pub fn node_decode<'de, D: Deserializer<'de>>(
    input: D,
) -> Result<sandproof_protocol::NodeId, D::Error> {
    sandproof_protocol::NodeId::deserialize(input)
}

pub fn string_primitive<'de, D: Deserializer<'de>>(input: D) -> Result<String, D::Error> {
    String::deserialize(input)
}

#[cfg(test)]
mod tests {
    macro_rules! identifier_cases {
        ($module:ident, $identity:ident) => {
            mod $module {
                use sandproof_protocol::$identity;

                #[test]
                fn successful_decode_is_valid() {
                    let mut inputs = vec![
                        "".to_owned(), "id.A-1_".to_owned(), "-".to_owned(),
                        "_".to_owned(), ".".to_owned(), "a b".to_owned(),
                        "a/b".to_owned(), "line\nx".to_owned(), "中文".to_owned(),
                        "é".to_owned(), "\0".to_owned(), "a:b".to_owned(),
                        "a@b".to_owned(),
                    ];
                    inputs.push("a".repeat(128));
                    inputs.push("a".repeat(129));
                    for text in inputs {
                        let json = serde_json::to_string(&text).unwrap();
                        let borrowed = serde_json::from_str::<$identity>(&json);
                        let bytes = serde_json::from_slice::<$identity>(json.as_bytes());
                        let owned = serde_json::from_value::<$identity>(
                            serde_json::Value::String(text.clone()));
                        for result in [borrowed, bytes, owned] {
                            if let Ok(value) = result {
                                let actual = value.as_str();
                                assert!(
                                    !actual.is_empty() && actual.len() <= 128 &&
                                        actual.bytes().all(|b| b.is_ascii_alphanumeric() ||
                                            matches!(b, b'-' | b'_' | b'.')),
                                    "successful decode violated opaque identifier contract: {text:?}"
                                );
                                assert_eq!(actual, text,
                                    "successful decode changed the original string");
                            }
                        }
                    }
                }

                #[test]
                fn accepts_exact_values_and_escaped_text() {
                    for text in ["id.A-1_".to_owned(), "a".repeat(128)] {
                        let json = serde_json::to_string(&text).unwrap();
                        assert_eq!(serde_json::from_str::<$identity>(&json).unwrap().as_str(), text);
                        assert_eq!(serde_json::from_slice::<$identity>(json.as_bytes())
                            .unwrap().as_str(), text);
                        assert_eq!(serde_json::from_value::<$identity>(
                            serde_json::Value::String(text.clone())).unwrap().as_str(), text);
                    }
                    assert_eq!(serde_json::from_str::<$identity>(r#""id\u002eA-1_""#)
                        .unwrap().as_str(), "id.A-1_");
                }

                #[test]
                fn original_error_and_first_rejection() {
                    let invalid = serde_json::from_slice::<$identity>(br#""" x"#).unwrap_err();
                    assert_eq!(invalid.to_string(), "invalid opaque identifier");
                    assert_eq!(invalid.classify(), serde_json::error::Category::Data);
                    assert_eq!((invalid.line(), invalid.column()), (0, 0));
                    let trailing = serde_json::from_slice::<$identity>(br#""good" x"#).unwrap_err();
                    assert_eq!(trailing.to_string(), "trailing characters at line 1 column 8");
                    assert_eq!(trailing.classify(), serde_json::error::Category::Syntax);
                    for input in ["42", "null", "true", "[]", "{}"] {
                        let error = serde_json::from_str::<$identity>(input).unwrap_err();
                        assert!(error.to_string().contains("expected a string"), "{error}");
                        assert_eq!(error.classify(), serde_json::error::Category::Data);
                    }
                }
            }
        };
    }
    identifier_cases!(tenant, TenantId);
    identifier_cases!(sandbox, SandboxId);
    identifier_cases!(operation, OperationId);
    identifier_cases!(logical_action, LogicalActionId);
    identifier_cases!(resource, ResourceId);
    identifier_cases!(client_request, ClientRequestId);
    identifier_cases!(deployment, DeploymentId);
    identifier_cases!(node, NodeId);
}
