use sandproof_guest_runtime::protocol::{
    ProtocolError, Request, decode_request, decode_request_with_argument_limit,
};

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut wire = vec![0];
    wire.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    wire.extend(payload);
    wire
}

#[test]
fn argument_boundary_preserves_complete_requests_and_never_returns_a_prefix() {
    for payload in [
        br"{}".as_slice(),
        br#"{"process":null}"#,
        br#"{"process":{"args":null}}"#,
        br#"{"process":{"cmd":"echo","args":[],"envs":{"A":"b"}}}"#,
        br#"{"process":{"args":["","\u0061"]},"pty":{"size":{"cols":80}},"stdin":false,"tag":"x"}"#,
    ] {
        let wire = frame(payload);
        assert_eq!(
            decode_request_with_argument_limit("Start", &wire, wire.len(), 2),
            decode_request("Start", &wire, wire.len()),
        );
    }
    for limit in [0, 1, 2] {
        let wire = frame(br#"{"process":{"args":["a","b","c"]}}"#);
        assert_eq!(
            decode_request_with_argument_limit("Start", &wire, wire.len(), limit),
            Ok(Request::StartArgumentsExceeded { limit })
        );
    }
}

#[test]
fn oversized_arguments_do_not_hide_late_json_or_framing_errors() {
    for payload in [
        br#"{"process":{"args":["a","b",null]}}"#.as_slice(),
        br#"{"process":{"args":["a","b",1]}}"#,
        br#"{"process":{"args":["a","b","\ud800"]}}"#,
        br#"{"process":{"args":["a","b"],"args":null}}"#,
        br#"{"process":{"args":["a","b"],"envs":{"A":"b","A":"c"}}}"#,
        br#"{"process":{"args":["a","b"]},"stdin":"false"}"#,
        br#"{"process":{"args":["a","b"]},"pty":[]}"#,
        br#"{"process":{"args":["a","b"]},"unknown":"\ud800"}"#,
        br#"{"process":{"args":["a","b"]}} true"#,
        br#"{"process":{"args":[],"envs":{"A":"b","A":"c"}}}"#,
        br#"{"process":{"args":[],"envs":[]}}"#,
        br#"{"process":{"args":[],"cmd":42}}"#,
        br#"{"process":{"args":[]},"pty":[]}"#,
    ] {
        let wire = frame(payload);
        assert_eq!(
            decode_request("Start", &wire, wire.len()),
            Err(ProtocolError::InvalidArgument)
        );
        assert_eq!(
            decode_request_with_argument_limit("Start", &wire, wire.len(), 1),
            Err(ProtocolError::InvalidArgument)
        );
    }
    let wire = frame(br#"{"process":{"args":["a","b"]}}"#);
    for tail in [frame(b"{}"), vec![2, 0, 0, 0, 1, b'!'], vec![2, 0]] {
        let complete = [wire.clone(), tail].concat();
        assert_eq!(
            decode_request_with_argument_limit("Start", &complete, complete.len(), 1),
            Err(ProtocolError::InvalidArgument)
        );
    }
    assert_eq!(
        decode_request_with_argument_limit("Start", &wire, wire.len() - 1, 1),
        Ok(Request::StartArgumentsExceeded { limit: 1 })
    ); // Limit covers payload, not envelope.
    assert_eq!(
        decode_request_with_argument_limit("Start", &wire, 1, 1),
        Err(ProtocolError::ResourceExhausted)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn oversized_arguments_decode_in_an_independent_memory_limited_process() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "argument_memory_child",
            "--nocapture",
        ])
        .env("SANDPROOF_ARGUMENT_MEMORY_CHILD", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "isolated child invoked by the ordinary parent test"]
fn argument_memory_child() {
    use rustix::process::{Resource, Rlimit, setrlimit};
    assert_eq!(
        std::env::var("SANDPROOF_ARGUMENT_MEMORY_CHILD").as_deref(),
        Ok("1")
    );
    // Allow 64 MiB beyond the isolated harness's existing address space. This
    // avoids depending on libc's initial arena mapping. It is a test limit,
    // not a daemon/VM budget. Five million Strings alone need >= 120 MB.
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let baseline = status
        .lines()
        .find_map(|line| line.strip_prefix("VmSize:"))
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .parse::<u64>()
        .unwrap()
        * 1024;
    let ceiling = baseline.checked_add(64 * 1024 * 1024).unwrap();
    setrlimit(
        Resource::As,
        Rlimit {
            current: Some(ceiling),
            maximum: Some(ceiling),
        },
    )
    .unwrap();
    let count = 5_000_000;
    let prefix = b"{\"process\":{\"args\":[";
    let suffix = b"]}}";
    let length = prefix.len() + 3 * count - 1 + suffix.len();
    let mut wire = Vec::with_capacity(length + 5);
    wire.push(0);
    wire.extend(u32::try_from(length).unwrap().to_be_bytes());
    wire.extend(prefix);
    for _ in 1..count {
        wire.extend(b"\"\",");
    }
    wire.extend(b"\"\"");
    wire.extend(suffix);
    assert!(wire.len() < 16 * 1024 * 1024);
    assert_eq!(
        decode_request_with_argument_limit("Start", &wire, wire.len(), 64),
        Ok(Request::StartArgumentsExceeded { limit: 64 })
    );
}
