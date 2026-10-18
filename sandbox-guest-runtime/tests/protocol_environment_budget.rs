use sandproof_guest_runtime::protocol::{
    ProtocolError, Request, StartLimits, decode_request, decode_request_with_start_limits,
};

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut wire = vec![0];
    wire.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    wire.extend(payload);
    wire
}

fn limits(env_entries: usize) -> StartLimits {
    StartLimits {
        arguments: 2,
        env_entries,
    }
}

#[test]
fn environment_boundary_preserves_full_requests_and_never_returns_a_prefix() {
    for payload in [
        br"{}".as_slice(),
        br#"{"process":null}"#,
        br#"{"process":{"envs":null}}"#,
        br#"{"process":{"cmd":"echo","args":["","a"],"envs":{}}}"#,
        br#"{"process":{"envs":{"a":"","\u0062":"\u503c"},"cwd":""},"stdin":false,"pty":{"size":{"cols":80}},"tag":"x"}"#,
        br#"{"process":{"envs":{"":"v","a=b":"w"},"cwd":"~other"}}"#,
    ] {
        let wire = frame(payload);
        assert_eq!(
            decode_request_with_start_limits("Start", &wire, wire.len(), limits(2)),
            decode_request("Start", &wire, wire.len()),
        );
    }
    for (payload, cwd, invalid_name) in [
        (
            br#"{"process":{"envs":{"a":"","b":""}}}"#.as_slice(),
            None,
            false,
        ),
        (
            br#"{"process":{"envs":{"a":"","b":""},"cwd":"~other"}}"#,
            Some("~other"),
            false,
        ),
        (
            br#"{"process":{"envs":{"a":"","\u003d":""},"cwd":""}}"#,
            Some(""),
            true,
        ),
        (br#"{"process":{"envs":{"a":"","":""}}}"#, None, true),
    ] {
        let wire = frame(payload);
        for count in [0, 1] {
            assert_eq!(
                decode_request_with_start_limits("Start", &wire, wire.len(), limits(count)),
                Ok(Request::StartEnvironmentExceeded {
                    limits: limits(count),
                    cwd: cwd.map(str::to_owned),
                    invalid_name,
                })
            );
        }
    }
    let wire = frame(br#"{"process":{"args":["","",""],"envs":{"a":"","b":""},"cwd":"~other"}}"#);
    assert_eq!(
        decode_request_with_start_limits("Start", &wire, wire.len(), limits(1)),
        Ok(Request::StartArgumentsExceeded { limit: 2 })
    );
}

#[test]
fn compact_environment_validation_preserves_late_schema_and_decoded_duplicate_errors() {
    for payload in [
        br#"{"process":{"envs":{"a":"","b":null}}}"#.as_slice(),
        br#"{"process":{"envs":{"a":"","b":1}}}"#,
        br#"{"process":{"envs":{"a":"","b":{}}}}"#,
        br#"{"process":{"envs":{"a":"","b":"\ud800"}}}"#,
        br#"{"process":{"envs":{"a":"","\u0061":""}}}"#,
        br#"{"process":{"envs":{"\u0061":"","a":""}}}"#,
        br#"{"process":{"envs":{"\u0061":"","\u0061":""}}}"#,
        br#"{"process":{"envs":{"a12345678901234":"","\u006112345678901234":""}}}"#,
        br#"{"process":{"envs":{"a123456789012345":"","\u0061123456789012345":""}}}"#,
        br#"{"process":{"envs":{"\ud83d\ude00":"","\ud83d\ude00":""}}}"#,
        br#"{"process":{"envs":{"a":"","b":""},"envs":null}}"#,
        br#"{"process":{"envs":{"a":"","b":""},"args":[null]}}"#,
        br#"{"process":{"envs":{"a":"","b":""},"cwd":1}}"#,
        br#"{"process":{"envs":{"a":"","b":""}},"stdin":"false"}"#,
        br#"{"process":{"envs":{"a":"","b":""}},"pty":[]}"#,
        br#"{"process":{"envs":{"a":"","b":""}},"unknown":"\ud800"}"#,
        br#"{"process":{"envs":{"a":"","b":""}}} true"#,
        br#"{"process":{"envs":[]}}"#,
    ] {
        let wire = frame(payload);
        assert_eq!(
            decode_request("Start", &wire, wire.len()),
            Err(ProtocolError::InvalidArgument)
        );
        assert_eq!(
            decode_request_with_start_limits("Start", &wire, wire.len(), limits(1)),
            Err(ProtocolError::InvalidArgument)
        );
    }
}

#[test]
fn environment_refusal_cannot_hide_late_frames_or_body_budget_failure() {
    let wire = frame(br#"{"process":{"envs":{"a":"","b":""}}}"#);
    for tail in [frame(b"{}"), vec![2, 0, 0, 0, 1, b'!'], vec![2, 0]] {
        let complete = [wire.clone(), tail].concat();
        assert_eq!(
            decode_request_with_start_limits("Start", &complete, complete.len(), limits(1)),
            Err(ProtocolError::InvalidArgument)
        );
    }
    assert_eq!(
        decode_request_with_start_limits("Start", &wire, 1, limits(1)),
        Err(ProtocolError::ResourceExhausted)
    );
}

#[test]
fn exact_environment_duplicates_across_scratch_chunks_keep_original_results() {
    use std::fmt::Write;

    // Exercise both sides of the internal chunk boundary without making that
    // allocation granularity a public environment limit.
    for count in [4095, 4096, 4097, 8193] {
        let mut payload = String::from(r#"{"process":{"envs":{"#);
        for index in (0..count).rev() {
            if index + 1 != count {
                payload.push(',');
            }
            write!(&mut payload, "\"k{index}\":\"\"").unwrap();
        }
        for suffix in ["}}}", r#", "\u006b0":""}}}"#, r#", "k4094":null}}}"#] {
            let complete = format!("{payload}{suffix}");
            let wire = frame(complete.as_bytes());
            let original = decode_request("Start", &wire, wire.len());
            let actual = decode_request_with_start_limits("Start", &wire, wire.len(), limits(64));
            if suffix == "}}}" {
                assert!(matches!(original, Ok(Request::Start(_))));
                assert!(matches!(
                    actual,
                    Ok(Request::StartEnvironmentExceeded { .. })
                ));
            } else {
                assert_eq!(original, Err(ProtocolError::InvalidArgument));
                assert_eq!(actual, original);
            }
        }
    }
}

#[test]
fn escaped_key_pool_boundaries_do_not_change_acceptance_or_duplicate_identity() {
    for suffix in [
        "x".repeat(14),
        "🙂".repeat(4),
        "中".repeat(21_845),
        "x".repeat(65_536),
    ] {
        let key = format!("k{suffix}");
        let plain = serde_json::to_string(&key).unwrap();
        let escaped = plain.replacen('k', r"\u006b", 1);
        let single = format!(r#"{{"process":{{"envs":{{{escaped}:""}}}}}}"#);
        let wire = frame(single.as_bytes());
        assert_eq!(
            decode_request_with_start_limits("Start", &wire, wire.len(), limits(1)),
            decode_request("Start", &wire, wire.len()),
        );
        let duplicate = format!(r#"{{"process":{{"envs":{{{plain}:"",{escaped}:""}}}}}}"#);
        let wire = frame(duplicate.as_bytes());
        assert_eq!(
            decode_request_with_start_limits("Start", &wire, wire.len(), limits(0)),
            Err(ProtocolError::InvalidArgument),
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn oversized_environment_decodes_in_an_independent_memory_limited_process() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "environment_decode_memory_child",
            "--nocapture",
        ])
        .env("SANDPROOF_ENVIRONMENT_DECODE_CHILD", "1")
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
fn environment_decode_memory_child() {
    use rustix::process::{Resource, Rlimit, setrlimit};
    use std::io::Write;
    assert_eq!(
        std::env::var("SANDPROOF_ENVIRONMENT_DECODE_CHILD").as_deref(),
        Ok("1")
    );
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
    // Existing harness mappings plus 32 MiB is a regression condition, not a
    // daemon budget. Wire input remains below the original 16 MiB body limit.
    let ceiling = baseline.checked_add(32 * 1024 * 1024).unwrap();
    setrlimit(
        Resource::As,
        Rlimit {
            current: Some(ceiling),
            maximum: Some(ceiling),
        },
    )
    .unwrap();
    let mut wire = Vec::with_capacity(16 * 1024 * 1024);
    wire.extend([0; 5]);
    wire.extend(br#"{"process":{"envs":{"#);
    for index in 0..1_200_000 {
        if index != 0 {
            wire.push(b',');
        }
        write!(&mut wire, "\"k{index}\":\"\"").unwrap();
    }
    wire.extend(b"}}}");
    let length = u32::try_from(wire.len() - 5).unwrap();
    wire[1..5].copy_from_slice(&length.to_be_bytes());
    assert!(wire.len() < 16 * 1024 * 1024);
    assert_eq!(
        decode_request_with_start_limits("Start", &wire, wire.len(), limits(64)),
        Ok(Request::StartEnvironmentExceeded {
            limits: limits(64),
            cwd: None,
            invalid_name: false
        })
    );
}
