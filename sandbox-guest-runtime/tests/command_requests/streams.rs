use super::*;

fn frame(flag: u8, payload: &[u8]) -> Vec<u8> {
    let mut body = vec![flag];
    body.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    body.extend(payload);
    body
}
fn check(reference: &mut Reference, payload: &[u8]) {
    for method in ["Start", "Connect"] {
        let mut body = frame(0, br#"{"process":{"pid":17,"cmd":"/bin/true"}}"#);
        body.extend(frame(2, payload));
        reference.check_frame(method, &body);
    }
}

#[test]
#[ignore = "requires explicit pinned Go server-stream request reference"]
fn fixed_go_final_envelopes_preserve_request_fields_and_original_acceptance() {
    let mut reference = Reference::new();
    for method in ["Start", "Connect"] {
        for prefix in [
            b"".as_slice(),
            br#"{"process":{"pid":17,"cmd":"/bin/true"}}"#,
        ] {
            for flag in 0..=u8::MAX {
                let mut body = frame(0, prefix);
                body.extend(frame(flag, b"{}"));
                reference.check_frame(method, &body);
            }
        }
        let prefix = frame(0, b"{}");
        let eos = frame(2, br#"{"metadata":{"a":["x"]}}"#);
        for end in 1..eos.len() {
            reference.check_frame(method, &[prefix.as_slice(), &eos[..end]].concat());
        }
        for tail in [vec![0], frame(0, b"{}"), frame(2, b"{}"), frame(2, b"")] {
            reference.check_frame(method, &[prefix.as_slice(), &eos, &tail].concat());
        }
    }
    for payload in [
        "",
        " ",
        "{}",
        " {}\n",
        "null",
        "[]",
        "0",
        "true",
        "\"x\"",
        "{}{}",
        r#"{"future":1e999999999999999999999999}"#,
        r#"{"future":1e+}"#,
        r#"{"future":01}"#,
        r#"{"error":null,"metadata":null}"#,
        r#"{"error":{}}"#,
        r#"{"error":{"code":"not_found","message":"ignored"}}"#,
        r#"{"error":{"code":"future","message":null,"details":null}}"#,
        r#"{"Error":{"Meſſage":"x","Detailſ":[{"Value":"!","Value":""}]}}"#,
        r#"{"error":{"details":[{"value":"AA","value":null}]}}"#,
        r#"{"error":{"details":[{"value":"!","value":null}]}}"#,
        r#"{"error":{"details":[{"value":1,"value":""}]}}"#,
        r#"{"metadata":{"a":["x",null],"b":null},"metadata":null}"#,
        r#"{"metadata":{"a":[1]},"metadata":{"a":[]}}"#,
        r#"{"error":{"code":1},"error":{"code":"unknown"}}"#,
        r#"{"error":{"details":[{"value":"!"}]},"error":null}"#,
        r#"{"metadata":{},"metadata":{"a":["x"]}}"#,
        r#"{"metadata":{"a":["x"],"a":null,"a":["y"]}}"#,
        r#"{"error":{"details":[{},null,{"type":"unknown","value":"/x=="}]}}"#,
    ] {
        check(&mut reference, payload.as_bytes());
    }
    for value in [
        "null",
        "{}",
        "[]",
        "0",
        "true",
        "\"x\"",
        "[null]",
        "[\"x\",null]",
    ] {
        for payload in [
            format!(r#"{{"error":{value}}}"#),
            format!(r#"{{"metadata":{value}}}"#),
            format!(r#"{{"metadata":{{"a":{value}}}}}"#),
            format!(r#"{{"error":{{"code":{value},"message":{value}}}}}"#),
            format!(r#"{{"error":{{"details":{value}}}}}"#),
            format!(r#"{{"error":{{"details":[{value}]}}}}"#),
            format!(r#"{{"error":{{"details":[{{"type":{value},"value":{value}}}]}}}}"#),
        ] {
            check(&mut reference, payload.as_bytes());
        }
    }
    unicode(&mut reference);
    binary(&mut reference);
    for depth in [1, 127, 128, 9998, 9999, 10000] {
        check(
            &mut reference,
            format!(
                r#"{{"future":{}0{}}}"#,
                "[".repeat(depth),
                "]".repeat(depth)
            )
            .as_bytes(),
        );
    }
    reference.finish();
}

fn unicode(reference: &mut Reference) {
    for (prefix, suffix) in [
        (b"{\"metadata\":{\"".as_slice(), b"\":[\"x\"]}}".as_slice()),
        (b"{\"metadata\":{\"a\":[\"", b"\"]}}"),
        (b"{\"error\":{\"message\":\"", b"\"}}"),
        (b"{\"future\":\"", b"\"}"),
    ] {
        for byte in 0..=u8::MAX {
            check(reference, &[prefix, &[byte], suffix].concat());
        }
        for value in [
            br"\ud800".as_slice(),
            br"\udc00",
            br"\ud800\udc00",
            br"\ud800\u0061",
            b"\xc0\x80",
            b"\xe0\x80\x80",
            b"\xf4\x90\x80\x80",
            "𝄞中".as_bytes(),
        ] {
            check(reference, &[prefix, value, suffix].concat());
        }
    }
}

fn binary(reference: &mut Reference) {
    for bytes in [
        "", "/w", "/x", "_x", "/w==", "/x==", "_x==", "AA", "AAA", "AAAA", "AA=", "AA==", "AAA=",
        "====", "AA==AA==", "AA AA",
    ] {
        for separator in ["", "\r", "\n", "\r\n"] {
            for offset in 0..=bytes.len() {
                let encoded = format!("{}{separator}{}", &bytes[..offset], &bytes[offset..]);
                check(
                    reference,
                    &serde_json::to_vec(&json!({"error":{"details":[{"value":encoded}]}})).unwrap(),
                );
            }
        }
    }
}
