//! Frozen derive shapes compare the public JSON contract before and after a
//! translation-only refactor. They are test oracles, not production adapters.
use sandproof_protocol::internal::{
    ControlIdentity as ActualControlIdentity, NodeIdentity as ActualNodeIdentity, PositiveCounter,
};
use sandproof_protocol::{ContractError, DeploymentId, NodeId, OperationId, SandboxId, TenantId};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

macro_rules! derived_id {
    ($reference:ident, $actual:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        struct $reference($actual);

        impl TryFrom<String> for $reference {
            type Error = ContractError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                $actual::try_from(value).map(Self)
            }
        }
        impl From<$reference> for String {
            fn from(value: $reference) -> Self {
                value.0.into()
            }
        }
    };
}

derived_id!(DerivedDeployment, DeploymentId);
derived_id!(DerivedNode, NodeId);
derived_id!(DerivedTenant, TenantId);
derived_id!(DerivedSandbox, SandboxId);
derived_id!(DerivedOperation, OperationId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
struct DerivedCounter(PositiveCounter);
impl TryFrom<u64> for DerivedCounter {
    type Error = sandproof_protocol::internal::InternalError;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        PositiveCounter::try_from(value).map(Self)
    }
}
impl From<DerivedCounter> for u64 {
    fn from(value: DerivedCounter) -> Self {
        value.0.get()
    }
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeIdentity {
    deployment_id: DerivedDeployment,
    node_id: DerivedNode,
    node_generation: DerivedCounter,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlIdentity {
    protocol_version: u16,
    node: NodeIdentity,
    tenant_id: DerivedTenant,
    sandbox_id: DerivedSandbox,
    operation_id: DerivedOperation,
    intent_version: DerivedCounter,
    retention_slot: u16,
    request_window: DerivedCounter,
}

fn check_result<A: Serialize, B: Serialize>(
    actual: Result<A, serde_json::Error>,
    reference: Result<B, serde_json::Error>,
    input: &[u8],
) {
    match (actual, reference) {
        (Ok(actual), Ok(reference)) => assert_eq!(
            serde_json::to_vec(&actual).unwrap(),
            serde_json::to_vec(&reference).unwrap(),
            "accepted fields differ for {input:?}"
        ),
        (Err(actual), Err(reference)) => {
            assert_eq!(actual.to_string(), reference.to_string(), "{input:?}");
            assert_eq!(actual.classify(), reference.classify(), "{input:?}");
            assert_eq!(actual.line(), reference.line(), "{input:?}");
            assert_eq!(actual.column(), reference.column(), "{input:?}");
        }
        (actual, reference) => panic!(
            "acceptance differs for {input:?}: actual={}, reference={}",
            actual.is_ok(),
            reference.is_ok()
        ),
    }
}

fn check<A: Serialize + DeserializeOwned, B: Serialize + DeserializeOwned>(input: &[u8]) {
    check_result(
        serde_json::from_slice::<A>(input),
        serde_json::from_slice::<B>(input),
        input,
    );
    if let Ok(text) = std::str::from_utf8(input) {
        check_result(
            serde_json::from_str::<A>(text),
            serde_json::from_str::<B>(text),
            input,
        );
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            check_result(
                serde_json::from_value::<A>(value.clone()),
                serde_json::from_value::<B>(value),
                input,
            );
        }
    }
}

const NODE: &str = r#"{"deployment_id":"deployment-a","node_id":"node-a","node_generation":1}"#;
const CONTROL: &str = r#"{"protocol_version":2,"node":{"deployment_id":"deployment-a","node_id":"node-a","node_generation":1},"tenant_id":"tenant-a","sandbox_id":"sandbox-a","operation_id":"operation-a","intent_version":1,"retention_slot":1,"request_window":1}"#;

#[test]
fn leaf_conversion_keeps_primitive_errors_and_all_identifier_bytes() {
    let identifiers = [
        r#""a-_.09AZaz""#,
        r#""""#,
        r#""../../x""#,
        r#""\u0061""#,
        r#""\u00e9""#,
        r#""\ud800""#,
        "null",
        "1",
        "true",
        "[]",
        "{}",
        r#""valid" false"#,
    ];
    for input in identifiers {
        check::<TenantId, DerivedTenant>(input.as_bytes());
    }
    for length in [127, 128, 129] {
        check::<TenantId, DerivedTenant>(format!("\"{}\"", "a".repeat(length)).as_bytes());
    }
    check::<TenantId, DerivedTenant>(&[b'"', 0xff, b'"']);
    for input in [
        "1",
        "18446744073709551615",
        "0",
        "-0",
        "-1",
        "1.0",
        "1e0",
        "18446744073709551616",
        "null",
        r#""1""#,
        "true",
        "[]",
        "0 x",
        "1 false",
    ] {
        check::<PositiveCounter, DerivedCounter>(input.as_bytes());
    }
}

#[test]
fn counter_validation_precedes_the_original_trailing_input_check() {
    // Parsing a whole u64 first and checking nonzero afterwards is NOT an
    // equivalent adapter: it would report trailing input before our zero error.
    let zero = serde_json::from_slice::<PositiveCounter>(b"0 x").unwrap_err();
    assert_eq!(zero.to_string(), "invalid or exhausted internal counter");
    assert_eq!(zero.classify(), serde_json::error::Category::Data);
    assert_eq!((zero.line(), zero.column()), (0, 0));
    let nonzero = serde_json::from_slice::<PositiveCounter>(b"1 x").unwrap_err();
    assert_eq!(
        nonzero.to_string(),
        "trailing characters at line 1 column 3"
    );
    assert_eq!(nonzero.classify(), serde_json::error::Category::Syntax);
    assert_eq!((nonzero.line(), nonzero.column()), (1, 3));
    check::<PositiveCounter, DerivedCounter>(b"0 x");
    check::<PositiveCounter, DerivedCounter>(b"1 x");
}

#[test]
fn complete_identity_keeps_object_and_sequence_shapes_and_wire_bytes() {
    for input in [
        NODE.to_owned(),
        r#"["deployment-a","node-a",1]"#.to_owned(),
        r#"{"node_generation":18446744073709551615,"node_id":"node-a","deployment_id":"deployment-a"}"#.to_owned(),
    ] {
        check::<ActualNodeIdentity, NodeIdentity>(input.as_bytes());
    }
    for input in [
        CONTROL.to_owned(),
        r#"[2,["deployment-a","node-a",1],"tenant-a","sandbox-a","operation-a",1,1,1]"#.to_owned(),
        CONTROL.replace("\"protocol_version\":2", "\"protocol_version\":65535"),
        CONTROL.replace("\"retention_slot\":1", "\"retention_slot\":0"),
        CONTROL.replace(
            "\"intent_version\":1",
            "\"intent_version\":18446744073709551615",
        ),
    ] {
        check::<ActualControlIdentity, ControlIdentity>(input.as_bytes());
    }
}

#[test]
fn identity_decode_preserves_first_error_and_does_not_preparse_later_fields() {
    let cases = [
        r#"{"unknown":?,"node_id":"node-a"}"#,
        r#"{"deployment_id":"ok","deployment_id":?}"#,
        r#"{"deployment_id":"ok","\u0064eployment_id":?}"#,
        r#"{"deployment_id":"../x","unknown":?}"#,
        r#"{"node_generation":0,"unknown":?}"#,
        r#"{"node_generation":1}"#,
        r#"{"deployment_id":"ok","node_id":"ok"}"#,
        r#"{"deployment_id":"ok","node_id":"ok","node_generation":1,"未知":0}"#,
        r#"{"deployment_id":"ok","node_id":"ok","node_generation":1} false"#,
        "[]",
        r#"["ok"]"#,
        r#"["ok","ok"]"#,
        r#"["ok","ok",1,0]"#,
    ];
    for input in cases {
        check::<ActualNodeIdentity, NodeIdentity>(input.as_bytes());
    }
    for input in [
        "{}".to_owned(),
        r#"{"protocol_version":65536,"unknown":?}"#.to_owned(),
        r#"{"protocol_version":2,"protocol_version":?}"#.to_owned(),
        CONTROL.replace("\"tenant-a\"", "\"../tenant\""),
        CONTROL.replace("\"request_window\":1", "\"request_window\":0"),
        CONTROL.replace("\"retention_slot\":1", "\"retention_slot\":65536"),
        CONTROL.replace("\"node_generation\":1", "\"node_generation\":1.0"),
    ] {
        check::<ActualControlIdentity, ControlIdentity>(input.as_bytes());
    }
    let node_array = r#"["deployment-a","node-a",1]"#;
    let fields = ["2", node_array, "\"t\"", "\"s\"", "\"o\"", "1", "1", "1"];
    for count in 0..=fields.len() {
        check::<ActualControlIdentity, ControlIdentity>(
            format!("[{}]", fields[..count].join(",")).as_bytes(),
        );
    }
    check::<ActualControlIdentity, ControlIdentity>(format!("[{},0]", fields.join(",")).as_bytes());
}
