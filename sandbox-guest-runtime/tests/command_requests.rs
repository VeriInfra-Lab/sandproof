use sandproof_guest_runtime::protocol::{
    Input, ProtocolError, Request, Selector, Signal, decode_request,
};
use serde_json::json;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

fn decode(method: &str, payload: &[u8], limit: usize) -> Result<Request, ProtocolError> {
    let mut body = Vec::new();
    if matches!(method, "Start" | "Connect") {
        body.push(0);
        body.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    }
    body.extend(payload);
    decode_request(method, &body, limit)
}

#[test]
fn ignored_fields_enums_null_duplicates_and_map_keys_follow_original_codec() {
    let Request::Start(start) = decode(
        "Start",
        br#"{"process":{"command":"/bin/sh"},"envs":{"UNUSED":"value"},"future":1,"future":null}"#,
        4096,
    )
    .unwrap() else {
        panic!("Start expected")
    };
    assert!(start.process.unwrap().cmd.is_empty()); // never renamed into cmd
    assert_eq!(
        decode("List", br#"{"unexpected":"value"}"#, 4096).unwrap(),
        Request::List
    );
    for unknown in ["SIGKILL", "15", "FUTURE", ""] {
        let payload = serde_json::to_vec(&json!({"signal":unknown})).unwrap();
        assert_eq!(
            decode("SendSignal", &payload, 4096).unwrap(),
            Request::SendSignal {
                process: None,
                signal: Signal::Unspecified
            }
        );
    }
    for (method, payload) in [
        ("Start", r#"{"tag":null,"tag":"x"}"#),
        ("Start", r#"{"stdin":null,"stdin":true}"#),
        ("Start", r#"{"process":null,"process":{}}"#),
        ("Start", r#"{"process":{"cwd":null,"cwd":"x"}}"#),
        ("Start", r#"{"process":{"envs":{"x":"1","\u0078":"2"}}}"#),
        ("Start", r#"{"process":{"envs":{"x":null}}}"#),
        ("Start", r#"{"process":{"envs":[]}}"#),
        ("Connect", r#"{"process":{"pid":null,"pid":1}}"#),
        ("SendInput", r#"{"input":{"stdin":null,"stdin":""}}"#),
        ("SendSignal", r#"{"signal":"FUTURE","signal":9}"#),
        ("SendSignal", r#"{"signal":null,"signal":9}"#),
        ("List", r#"{"future":"\ud800"}"#),
    ] {
        assert_eq!(
            decode(method, payload.as_bytes(), 4096),
            Err(ProtocolError::InvalidArgument),
            "{method} {payload}"
        );
    }
}

#[test]
fn exact_integer_spellings_and_permissive_bytes_preserve_the_decoded_value() {
    for number in ["1", "1.0", "1e0", "10e-1", "\"1.00\"", "\"0.01e2\"", "1e"] {
        let payload = format!(r#"{{"process":{{"pid":{number}}}}}"#);
        assert_eq!(
            decode("Connect", payload.as_bytes(), 4096).unwrap(),
            Request::Connect(Some(Selector::Pid(1)))
        );
    }
    for (number, signal) in [
        ("15.0", Signal::Terminate),
        ("1.5e1", Signal::Terminate),
        ("-2147483648", Signal::Unknown(i32::MIN)),
        ("0e9999999999999", Signal::Unspecified),
        ("-1e", Signal::Unknown(-1)),
    ] {
        assert_eq!(
            decode(
                "SendSignal",
                format!(r#"{{"signal":{number}}}"#).as_bytes(),
                4096
            )
            .unwrap(),
            Request::SendSignal {
                process: None,
                signal
            }
        );
    }
    for number in ["15e+", "15e-", "0.1", "2147483648", "-2147483649"] {
        assert_eq!(
            decode(
                "SendSignal",
                format!(r#"{{"signal":{number}}}"#).as_bytes(),
                4096
            ),
            Err(ProtocolError::InvalidArgument)
        );
    }
    for encoded in ["/x==", "_x==", "/x", "_x"] {
        assert_eq!(
            decode(
                "SendInput",
                &serde_json::to_vec(&json!({"input":{"stdin":encoded}})).unwrap(),
                4096
            )
            .unwrap(),
            Request::SendInput {
                process: None,
                input: Some(Input::Stdin(vec![255]))
            }
        );
    }
    for encoded in ["AA=\n=", "AA=", "AA\n=", "AA AA"] {
        assert_eq!(
            decode(
                "SendInput",
                &serde_json::to_vec(&json!({"input":{"stdin":encoded}})).unwrap(),
                4096
            ),
            Err(ProtocolError::InvalidArgument)
        );
    }
    assert_eq!(
        decode("SendInput", br#"{"input":{"stdin":"AA\nAA"}}"#, 4096).unwrap(),
        Request::SendInput {
            process: None,
            input: Some(Input::Stdin(vec![0; 3]))
        }
    );
}

#[test]
fn unknown_nesting_and_syntax_are_bounded_after_original_message_admission() {
    assert_eq!(
        decode("Unknown", b"invalid JSON", 4096),
        Err(ProtocolError::Unimplemented)
    );
    for depth in [1, 127, 128, 9999] {
        let body = format!(
            r#"{{"future":{}0{}}}"#,
            "[".repeat(depth),
            "]".repeat(depth)
        );
        assert_eq!(
            decode("List", body.as_bytes(), body.len()).unwrap(),
            Request::List
        );
        assert_eq!(
            decode("List", body.as_bytes(), body.len() - 1),
            Err(ProtocolError::ResourceExhausted)
        );
    }
    let body = format!(
        r#"{{"future":{}0{}}}"#,
        "[".repeat(10_000),
        "]".repeat(10_000)
    );
    assert_eq!(
        decode("List", body.as_bytes(), body.len()),
        Err(ProtocolError::InvalidArgument)
    );
    for value in ["1e+", "-1e-", "0e"] {
        assert_eq!(
            decode("List", format!(r#"{{"future":{value}}}"#).as_bytes(), 4096).unwrap(),
            Request::List
        );
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(result, "{byte:02x}").unwrap();
    }
    result
}
fn unhex(text: &str) -> Vec<u8> {
    assert!(text.len().is_multiple_of(2));
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
struct Reference {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    count: usize,
}
impl Reference {
    fn new() -> Self {
        let mut child = Command::new(
            std::env::var_os("SANDPROOF_GUEST_CONNECT_REFERENCE")
                .expect("explicit fixed Go reference required"),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            input,
            output,
            count: 0,
        }
    }
    fn check(&mut self, method: &str, body: &[u8]) {
        let record = serde_json::to_vec(&json!({"Method":method,"Body":hex(body)})).unwrap();
        writeln!(self.input, "Q\t{}", hex(&record)).unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        assert!(self.output.read_line(&mut line).unwrap() > 0);
        let result = decode(method, body, 1 << 20);
        if line.trim_end() == "ERR\tinvalid_argument" {
            assert_eq!(
                result,
                Err(ProtocolError::InvalidArgument),
                "{method} {:?}",
                String::from_utf8_lossy(body)
            );
        } else {
            let canonical = unhex(
                line.trim_end()
                    .strip_prefix("OK\t")
                    .expect("unexpected Go result"),
            );
            let expected =
                decode(method, &canonical, 1 << 20).expect("Go canonical request must decode");
            assert_eq!(
                result,
                Ok(expected),
                "{method} {:?}",
                String::from_utf8_lossy(body)
            );
        }
        self.count += 1;
    }
    fn finish(mut self) {
        drop(self.input);
        assert!(self.child.wait().unwrap().success());
        eprintln!(
            "{} original command request observations matched",
            self.count
        );
    }
}

impl Reference {
    fn check_frame(&mut self, method: &str, body: &[u8]) {
        let record = serde_json::to_vec(&json!({"Method":method,"Body":hex(body)})).unwrap();
        writeln!(self.input, "F\t{}", hex(&record)).unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        assert!(self.output.read_line(&mut line).unwrap() > 0);
        let result = decode_request(method, body, 1 << 20);
        if line.starts_with("ERR\t") {
            // Acceptance only: public framing error code/text is a separate
            // pending adapter, not equivalent to this payload-free category.
            assert!(result.is_err(), "{method} {body:?}: {line}");
        } else {
            let canonical = unhex(line.trim_end().strip_prefix("OK\t").unwrap());
            let expected = decode(method, &canonical, 1 << 20).unwrap();
            assert_eq!(result, Ok(expected), "{method} {body:?}");
        }
        self.count += 1;
    }
}

#[test]
#[ignore = "requires explicit pinned Go server-stream request reference"]
fn fixed_go_stream_requests_match_empty_message_acceptance_and_decoded_fields() {
    fn frame(bytes: &[u8]) -> Vec<u8> {
        let mut body = vec![0];
        body.extend(u32::try_from(bytes.len()).unwrap().to_be_bytes());
        body.extend(bytes);
        body
    }
    let mut reference = Reference::new();
    for method in ["Start", "Connect"] {
        for flag in 0..=u8::MAX {
            let mut zero = frame(b"");
            zero[0] = flag;
            reference.check_frame(method, &zero);
        }
        for payload in [
            b"{}".as_slice(),
            b"null",
            b"[]",
            b" ",
            b"{",
            br#"{"process":{"pid":17,"cmd":"/bin/true"}}"#,
        ] {
            reference.check_frame(method, &frame(payload));
        }
        let valid = frame(b"{}");
        for end in 0..valid.len() {
            reference.check_frame(method, &valid[..end]);
        }
        let mut multiple = frame(b"");
        multiple.extend(frame(b""));
        reference.check_frame(method, &multiple);
        let mut multiple = valid.clone();
        multiple.extend(&valid);
        reference.check_frame(method, &multiple);
        let mut trailing = valid;
        trailing.push(0);
        reference.check_frame(method, &trailing);
    }
    reference.finish();
}

#[test]
#[ignore = "requires explicit pinned Go Connect reference"]
fn fixed_generated_command_requests_match_original_default_http_codec() {
    let mut reference = Reference::new();
    shapes(&mut reference);
    numbers(&mut reference);
    binary(&mut reference);
    reference.finish();
}

fn shapes(reference: &mut Reference) {
    for method in [
        "Start",
        "Connect",
        "List",
        "SendInput",
        "CloseStdin",
        "SendSignal",
        "Update",
    ] {
        for body in [
            "{}",
            "null",
            "[]",
            r#"{"future":1,"future":null}"#,
            r#"{"future":"\ud800"}"#,
            r#"{"future":1e+}"#,
            r#"{"process":null,"process":{}}"#,
            r#"{"tag":null,"tag":"x"}"#,
        ] {
            reference.check(method, body.as_bytes());
        }
    }
    for body in [
        r#"{"stdin":null,"stdin":true}"#,
        r#"{"process":{"cmd":null,"cmd":"x"}}"#,
        r#"{"process":{"args":[null]}}"#,
        r#"{"process":{"envs":{"x":null}}}"#,
        r#"{"process":{"envs":{"x":"1","\u0078":"2"}}}"#,
        r#"{"process":{"cwd":null,"cwd":""}}"#,
        r#"{"pty":{"size":{"rows":null,"rows":1}}}"#,
    ] {
        reference.check("Start", body.as_bytes());
    }
    for depth in [1, 126, 127, 128, 9998, 9999, 10_000] {
        reference.check(
            "List",
            format!(
                r#"{{"future":{}0{}}}"#,
                "[".repeat(depth),
                "]".repeat(depth)
            )
            .as_bytes(),
        );
    }
    for (method, prefix, suffix) in [
        ("Start", b"{\"tag\":\"".as_slice(), b"\"}".as_slice()),
        ("List", b"{\"future\":\"", b"\"}"),
        ("SendInput", b"{\"input\":{\"stdin\":\"", b"\"}}"),
    ] {
        for byte in 0..=255_u8 {
            let mut body = prefix.to_vec();
            body.push(byte);
            body.extend(suffix);
            reference.check(method, &body);
        }
    }
}
fn numbers(reference: &mut Reference) {
    for number in [
        "0",
        "-0",
        "1",
        "-1",
        "9",
        "15",
        "19",
        "2147483647",
        "2147483648",
        "-2147483648",
        "-2147483649",
        "4294967295",
        "4294967296",
        "1.0",
        "1.25",
        "0.001",
        "-0.0",
        "01",
        "+1",
        "1.",
        "true",
        "null",
    ] {
        for exponent in ["", "e", "e+", "e-", "e0", "e1", "e-1", "e2147483648"] {
            let number = format!("{number}{exponent}");
            for spelling in [number.clone(), serde_json::to_string(&number).unwrap()] {
                reference.check(
                    "Connect",
                    format!(r#"{{"process":{{"pid":{spelling}}}}}"#).as_bytes(),
                );
                reference.check(
                    "Update",
                    format!(r#"{{"pty":{{"size":{{"rows":{spelling}}}}}}}"#).as_bytes(),
                );
                reference.check(
                    "SendSignal",
                    format!(r#"{{"signal":{spelling}}}"#).as_bytes(),
                );
                reference.check("List", format!(r#"{{"future":{spelling}}}"#).as_bytes());
            }
        }
    }
    for signal in ["SIGNAL_SIGTERM", "SIGNAL_SIGKILL", "FUTURE", "15", ""] {
        reference.check(
            "SendSignal",
            &serde_json::to_vec(&json!({"signal":signal})).unwrap(),
        );
        reference.check(
            "SendSignal",
            format!(r#"{{"signal":"{signal}","signal":9}}"#).as_bytes(),
        );
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
                for field in ["stdin", "pty"] {
                    reference.check(
                        "SendInput",
                        &serde_json::to_vec(&json!({"input":{field:encoded}})).unwrap(),
                    );
                }
            }
        }
    }
}
