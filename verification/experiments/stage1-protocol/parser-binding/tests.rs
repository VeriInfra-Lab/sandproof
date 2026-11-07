use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, ExpectedSession, LifecycleOperation, LifecycleRequest,
    RecordedLifecycleIntent,
};
use serde::de::DeserializeOwned;

const BINDING: &str = r#"{"protocol_version":4,"node":{"deployment_id":"d","node_id":"n","node_generation":1},"session":"s","key":{"deployment":"d","owner":"o","window":1,"sequence":1},"expected":{"state":"version","version":0}}"#;
const ZERO: &str = "invalid or exhausted internal counter";

fn decode_error<T: DeserializeOwned + std::fmt::Debug>(bytes: &str) -> serde_json::Error {
    serde_json::from_slice::<T>(bytes.as_bytes()).unwrap_err()
}

fn request(operation: &str) -> String {
    format!(r#"{{"binding":{BINDING},"operation":{operation}}}"#)
}

fn accepted(request: &str) -> String {
    format!(
        r#"{{"request":{request},"accepted_version":1,"center":{{"deployment":"d","center":"c"}},"retention":{{"slot":1,"window":9}}}}"#
    )
}

fn recorded(accepted: &str) -> String {
    format!(
        r#"{{"intent":{accepted},"current_session_version":1,"current":true,"terminal":false}}"#
    )
}

fn data_at(error: &serde_json::Error, message: &str, line: usize, column: usize) {
    assert!(error.is_data(), "{error}");
    assert_eq!((error.line(), error.column()), (line, column), "{error}");
    let expected = if line == 0 {
        message.to_owned()
    } else {
        format!("{message} at line {line} column {column}")
    };
    assert_eq!(error.to_string(), expected);
}

#[test]
fn buffered_operation_leaf_stays_unpositioned_without_an_outer_json_container() {
    let operation = r#"{"kind":"unload","source_incarnation":0,"capture":1}"#;
    data_at(&decode_error::<LifecycleOperation>(operation), ZERO, 0, 0);
    // The document end check must not replace this error with trailing input.
    data_at(
        &decode_error::<LifecycleOperation>(&format!("{operation} trailing")),
        ZERO,
        0,
        0,
    );
}

#[test]
fn nearest_real_container_positions_once_through_both_envelopes() {
    let operation = r#"{"kind":"unload","source_incarnation":0,"capture":1}"#;
    let inner = request(operation);
    assert_eq!(inner.len(), 280);
    data_at(&decode_error::<LifecycleRequest>(&inner), ZERO, 1, 280);
    data_at(
        &decode_error::<AcceptedLifecycleIntent>(&accepted(&inner)),
        ZERO,
        1,
        291,
    );
    data_at(
        &decode_error::<RecordedLifecycleIntent>(&recorded(&accepted(&inner))),
        ZERO,
        1,
        301,
    );
    let multiline = inner.strip_suffix('}').unwrap().to_owned() + "\n} trailing";
    data_at(&decode_error::<LifecycleRequest>(&multiline), ZERO, 2, 1);
}

#[test]
fn operation_buffering_can_reject_syntax_before_a_custom_leaf_runs() {
    let operation = r#"{"kind":"unload","source_incarnation":0,"capture":1,"extra":]}"#;
    let error = decode_error::<LifecycleOperation>(operation);
    assert!(error.is_syntax());
    assert_eq!(error.to_string(), "expected value at line 1 column 61");
    let error = decode_error::<RecordedLifecycleIntent>(&recorded(&accepted(&request(operation))));
    assert!(error.is_syntax());
    assert_eq!(error.to_string(), "expected value at line 1 column 309");
}

#[test]
fn expected_session_tag_content_order_selects_the_actual_reader() {
    let message = "invalid type: string \"x\", expected u64";
    // Tag first: the ordinary u64 reader fixes its error at the original token.
    data_at(
        &decode_error::<ExpectedSession>("{\"state\":\"version\",\"version\":\"x\"\n}"),
        message,
        1,
        32,
    );
    // Content first: buffered u64 has no position; end_map consumes the newline/brace.
    data_at(
        &decode_error::<ExpectedSession>("{\"version\":\"x\",\"state\":\"version\"\n}"),
        message,
        2,
        1,
    );
    data_at(
        &decode_error::<ExpectedSession>("[\"version\",\"x\"\n]"),
        message,
        1,
        14,
    );
}

#[test]
fn content_error_beats_later_container_and_document_errors() {
    let message = "invalid type: string \"x\", expected u64";
    for bytes in [
        r#"{"version":"x","state":"version","extra":]} trailing"#,
        r#"{"state":"version","version":"x","extra":]} trailing"#,
    ] {
        data_at(&decode_error::<ExpectedSession>(bytes), message, 1, 32);
    }
}

#[test]
fn same_original_schema_rejects_unknown_duplicate_and_missing_members() {
    let unknown = r#"{"kind":"unload","source_incarnation":1,"capture":1,"extra":0}"#;
    data_at(
        &decode_error::<LifecycleOperation>(unknown),
        "unknown field `extra`, expected `source_incarnation` or `capture`",
        0,
        0,
    );
    let duplicate = r#"{"kind":"unload","source_incarnation":1,"capture":1,"capture":2}"#;
    data_at(
        &decode_error::<LifecycleOperation>(duplicate),
        "duplicate field `capture`",
        0,
        0,
    );
    let missing = r#"{"kind":"unload","source_incarnation":1}"#;
    data_at(
        &decode_error::<LifecycleOperation>(missing),
        "missing field `capture`",
        0,
        0,
    );
}

#[test]
fn successful_values_accept_both_orders_and_sequences_before_document_end() {
    for bytes in [
        r#"{"state":"version","version":0}"#,
        r#"{"version":0,"state":"version"}"#,
        r#"["version",0]"#,
    ] {
        assert_eq!(
            serde_json::from_slice::<ExpectedSession>(bytes.as_bytes()).unwrap(),
            ExpectedSession::Version(0)
        );
        let error = decode_error::<ExpectedSession>(&format!("{bytes} trailing"));
        assert!(error.is_syntax());
        assert!(error.to_string().starts_with("trailing characters"));
    }
    let bytes = recorded(&accepted(&request(
        r#"{"kind":"unload","source_incarnation":1,"capture":1}"#,
    )));
    let value: RecordedLifecycleIntent = serde_json::from_slice(bytes.as_bytes()).unwrap();
    assert_eq!(
        serde_json::from_slice::<RecordedLifecycleIntent>(&serde_json::to_vec(&value).unwrap())
            .unwrap(),
        value
    );
}
