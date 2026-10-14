use super::*;

#[test]
fn metadata_validation_handles_null_duplicates_known_shapes_and_binary_details() {
    for input in [
        "null",
        "{}",
        r#"{"error":null,"metadata":null}"#,
        r#"{"error":{"code":"future","message":null,"details":[null,{}, {"type":"unknown","value":"/x=="}]}}"#,
        r#"{"metadata":{"a":["x",null],"b":null},"metadata":null}"#,
        r#"{"Error":{"Meſſage":"x","Detailſ":[{"Value":"!","Value":""}]}}"#,
        r#"{"error":{"details":[{"value":"AA","value":null}]}}"#,
        r#"{"future":1e999999999999999999999999,"metadata":{"a":["\ud800"]}}"#,
    ] {
        assert_eq!(validate(input.as_bytes()), Ok(()), "{input}");
    }
    for input in [
        "",
        "[]",
        "true",
        r#"{"error":[]}"#,
        r#"{"metadata":{"a":"x"}}"#,
        r#"{"error":{"code":1,"code":"unknown"}}"#,
        r#"{"error":{"details":[{"value":"!","value":null}]}}"#,
        r#"{"error":{"Detailſ":false}}"#,
        r#"{"metadata":{"a":[1]}}"#,
        r#"{"error":{"details":[{"type":false}]}}"#,
        r#"{"error":{"details":[{"value":"_w"}]}}"#,
    ] {
        assert_eq!(
            validate(input.as_bytes()),
            Err(ProtocolError::InvalidArgument),
            "{input}"
        );
    }
}

#[test]
fn eos_only_unicode_repairs_are_bounded_and_unknown_nesting_is_iterative() {
    for byte in 0x80..=0xff {
        let body = [b"{\"metadata\":{\"a\":[\"".as_slice(), &[byte], b"\"]}}"].concat();
        let normalized = normalize::json(&body).unwrap();
        assert!(normalized.len() <= 3 * body.len());
        assert_eq!(validate(&body), Ok(()));
    }
    for value in [
        r"\ud800",
        r"\udc00",
        r"\ud800\udc00",
        r"\ud800\u0061",
        "中",
        "𝄞",
    ] {
        let body = format!(r#"{{"metadata":{{"a":["{value}"]}}}}"#);
        assert_eq!(validate(body.as_bytes()), Ok(()));
    }
    for depth in [1, 100, 9999, 10000] {
        let body = format!(
            r#"{{"future":{}0{}}}"#,
            "[".repeat(depth),
            "]".repeat(depth)
        );
        assert_eq!(validate(body.as_bytes()).is_ok(), depth < 10000);
    }
    // A malformed Unicode sequence outside a string is not repaired to JSON.
    assert!(validate(b"{\"future\":\xff}").is_err());
}
