#![cfg(target_os = "linux")]

use sandproof_guest_core::Channel;
use sandproof_guest_runtime::protocol::{
    End, Event, Input, ProtocolError, Request, Selector, Signal, decode_request, encode_event,
    encode_stream_end,
};
use serde_json::{Value, json};

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut result = vec![0];
    result.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    result.extend(payload);
    result
}

fn decode(method: &str, value: impl serde::Serialize) -> Result<Request, ProtocolError> {
    let payload = serde_json::to_vec(&value).unwrap();
    let body = if matches!(method, "Start" | "Connect") {
        frame(&payload)
    } else {
        payload
    };
    decode_request(method, &body, 16384)
}

#[test]
fn pinned_shared_command_requests_are_consumed() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../tests/compat/guest-service-contract.json"
    ))
    .unwrap();
    let mut count = 0;
    for case in fixtures["rpcs"].as_array().unwrap() {
        if case["service"] != "process" {
            continue;
        }
        let request = decode(case["method"].as_str().unwrap(), case["request"].clone()).unwrap();
        match case["id"].as_str().unwrap() {
            "command-start" => {
                let Request::Start(start) = request else {
                    panic!("wrong method")
                };
                assert!(!start.stdin_enabled());
                assert!(start.pty.is_none());
                let config = start.process.unwrap();
                assert_eq!(config.cmd, "/bin/sh");
                assert_eq!(config.cwd.as_deref(), Some("/tmp"));
                assert_eq!(config.envs["SAMPLE"], "value");
                assert_eq!(config.args.len(), 2);
            }
            "command-connect" => assert_eq!(request, Request::Connect(Some(Selector::Pid(4321)))),
            "command-input" => assert_eq!(
                request,
                Request::SendInput {
                    process: Some(Selector::Pid(4321)),
                    input: Some(Input::Stdin(b"\0\xffA\n".to_vec())),
                }
            ),
            "command-close-input" => {
                assert_eq!(request, Request::CloseStdin(Some(Selector::Pid(4321))));
            }
            "command-signal" => assert_eq!(
                request,
                Request::SendSignal {
                    process: Some(Selector::Pid(4321)),
                    signal: Signal::Kill,
                }
            ),
            "command-list" => assert_eq!(request, Request::List),
            "pty-start" => {
                let Request::Start(start) = request else {
                    panic!("wrong method")
                };
                assert!(start.stdin_enabled());
                let size = start.pty.unwrap().size.unwrap();
                assert_eq!((size.cols, size.rows), (80, 24));
            }
            "pty-resize" => {
                let Request::Update { process, pty } = request else {
                    panic!("wrong method")
                };
                assert_eq!(process, Some(Selector::Pid(4322)));
                let size = pty.unwrap().size.unwrap();
                assert_eq!((size.cols, size.rows), (120, 40));
            }
            other => panic!("unexamined fixture {other}"),
        }
        count += 1;
    }
    assert_eq!(count, 8);
}

#[test]
fn defaults_presence_and_oneofs_do_not_change_execution_intent() {
    for value in [json!({}), json!({"stdin": null})] {
        let Request::Start(start) = decode("Start", value).unwrap() else {
            panic!("wrong method")
        };
        assert!(start.stdin_enabled());
        assert!(start.process.is_none());
    }
    let Request::Start(start) = decode(
        "Start",
        json!({"process": {"cmd": null, "args": null, "envs": null, "cwd": ""}, "tag": ""}),
    )
    .unwrap() else {
        panic!("wrong method")
    };
    let config = start.process.unwrap();
    assert!(config.cmd.is_empty() && config.args.is_empty() && config.envs.is_empty());
    assert_eq!(config.cwd.as_deref(), Some(""));
    assert_eq!(start.tag.as_deref(), Some(""));
    for (value, expected) in [
        (json!({}), Selector::Unset),
        (json!({"pid": null, "tag": null}), Selector::Unset),
        (json!({"pid": "4294967295"}), Selector::Pid(u32::MAX)),
        (json!({"pid": 0}), Selector::Pid(0)),
        (json!({"tag": ""}), Selector::Tag(String::new())),
    ] {
        assert_eq!(
            decode("Connect", json!({"process": value})).unwrap(),
            Request::Connect(Some(expected))
        );
    }
    for (value, expected) in [
        (json!({}), Input::Unset),
        (json!({"stdin": null}), Input::Unset),
        (json!({"stdin": ""}), Input::Stdin(vec![])),
        (json!({"pty": "AP9B"}), Input::Pty(b"\0\xffA".to_vec())),
        (json!({"stdin": "__8="}), Input::Stdin(vec![255, 255])),
        (json!({"stdin": "//8"}), Input::Stdin(vec![255, 255])),
    ] {
        assert_eq!(
            decode("SendInput", json!({"input": value})).unwrap(),
            Request::SendInput {
                process: None,
                input: Some(expected)
            }
        );
    }
}

#[test]
fn malformed_fields_never_fall_back_to_another_operation() {
    for (method, value) in [
        ("Start", json!({"process": {"command": "/bin/sh"}})),
        ("Start", json!({"stdin": "false"})),
        ("Start", json!({"pty": {"size": {"rows": -1}}})),
        ("Start", json!({"envs": {"SECRET": "do not echo"}})),
        ("Connect", json!({"process": {"pid": 1, "tag": "other"}})),
        ("Connect", json!({"process": {"pid": 4_294_967_296_u64}})),
        ("Connect", json!({"process": {"pid": -1}})),
        ("SendInput", json!({"input": {"stdin": "", "pty": ""}})),
        ("SendInput", json!({"input": {"stdin": "not base64"}})),
        ("SendSignal", json!({"signal": "SIGKILL"})),
        ("List", json!({"unexpected": "value"})),
        ("List", json!(null)),
        ("List", json!([])),
        ("Start", json!([null, null, null, false])),
        ("Start", json!({"process": ["/bin/sh", [], {}, null]})),
        ("Start", json!({"pty": {"size": [80, 24]}})),
        ("Connect", json!({"process": [123, null]})),
        ("SendInput", json!({"input": ["", null]})),
    ] {
        assert_eq!(
            decode(method, value),
            Err(ProtocolError::InvalidArgument),
            "{method}"
        );
    }
    assert_eq!(
        decode_request("List", b"{}{}", 64),
        Err(ProtocolError::InvalidArgument)
    );
    assert_eq!(
        decode_request("Start", &frame(br#"{"stdin":true,"stdin":false}"#), 64),
        Err(ProtocolError::InvalidArgument)
    );
    assert_eq!(
        decode("StreamInput", json!({})),
        Err(ProtocolError::Unimplemented)
    );
    assert_eq!(
        decode("Unknown", json!({})),
        Err(ProtocolError::Unimplemented)
    );
}

#[test]
fn signals_keep_unknown_values_for_explicit_executor_rejection() {
    for (value, expected) in [
        (json!(null), Signal::Unspecified),
        (json!(0), Signal::Unspecified),
        (json!(15), Signal::Terminate),
        (json!(9), Signal::Kill),
        (json!(19), Signal::Unknown(19)),
        (json!("SIGNAL_SIGTERM"), Signal::Terminate),
    ] {
        assert_eq!(
            decode("SendSignal", json!({"signal": value})).unwrap(),
            Request::SendSignal {
                process: None,
                signal: expected
            }
        );
    }
}

#[test]
fn framed_requests_require_exact_length_flags_and_single_message() {
    let valid = frame(b"{}");
    assert!(decode_request("Start", &valid, 2).is_ok());
    assert_eq!(
        decode_request("Start", &valid, 1),
        Err(ProtocolError::ResourceExhausted)
    );
    assert_eq!(
        decode_request("List", b"{}", 0),
        Err(ProtocolError::ResourceExhausted)
    );
    assert_eq!(
        decode_request("List", b"{}", 1),
        Err(ProtocolError::ResourceExhausted)
    );
    for end in 0..valid.len() {
        assert_eq!(
            decode_request("Start", &valid[..end], 64),
            Err(ProtocolError::InvalidArgument)
        );
    }
    for flag in [1, 2, 3, 128] {
        let mut invalid = valid.clone();
        invalid[0] = flag;
        assert_eq!(
            decode_request("Start", &invalid, 64),
            Err(ProtocolError::InvalidArgument)
        );
    }
    let mut multiple = valid.clone();
    multiple.extend(&valid);
    assert_eq!(
        decode_request("Connect", &multiple, 64),
        Err(ProtocolError::InvalidArgument)
    );
    assert_eq!(
        decode_request("Start", &[0, 255, 255, 255, 255], 64),
        Err(ProtocolError::ResourceExhausted)
    );
}

fn unpack(bytes: &[u8], flag: u8) -> Value {
    assert_eq!(bytes[0], flag);
    assert_eq!(
        u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize,
        bytes.len() - 5
    );
    serde_json::from_slice(&bytes[5..]).unwrap()
}

#[test]
fn binary_output_and_end_framing_preserve_observations() {
    assert_eq!(
        unpack(&encode_event(Event::Start(123), 128).unwrap(), 0),
        json!({"event": {"start": {"pid": 123}}})
    );
    for (channel, name) in [
        (Channel::Stdout, "stdout"),
        (Channel::Stderr, "stderr"),
        (Channel::Pty, "pty"),
    ] {
        let encoded = encode_event(Event::Output(channel, b"\0\xffA\n"), 128).unwrap();
        assert_eq!(
            unpack(&encoded, 0),
            json!({"event": {"data": {name: "AP9BCg=="}}})
        );
        assert_eq!(
            encode_event(Event::Output(channel, b"\0\xffA\n"), encoded.len() - 5).unwrap(),
            encoded
        );
        assert_eq!(
            encode_event(Event::Output(channel, b"\0\xffA\n"), encoded.len() - 6),
            Err(ProtocolError::ResourceExhausted)
        );
    }
    let end = encode_event(
        Event::End(End {
            exit_code: -1,
            exited: false,
            status: "signal: killed",
            error: Some("signal: killed"),
        }),
        256,
    )
    .unwrap();
    assert_eq!(
        unpack(&end, 0),
        json!({"event": {"end": {"exitCode": -1, "exited": false, "status": "signal: killed", "error": "signal: killed"}}})
    );
    assert_eq!(unpack(&encode_stream_end(None, 2).unwrap(), 2), json!({}));
    assert_eq!(
        encode_stream_end(None, 1),
        Err(ProtocolError::ResourceExhausted)
    );
    assert_eq!(
        unpack(
            &encode_stream_end(Some(ProtocolError::InvalidArgument), 128).unwrap(),
            2
        ),
        json!({"error": {"code": "invalid_argument"}})
    );
    assert_eq!(
        unpack(&encode_event(Event::KeepAlive, 64).unwrap(), 0),
        json!({"event": {"keepalive": {}}})
    );
}

#[test]
fn message_budget_is_not_a_total_stream_budget() {
    for _ in 0..10000 {
        assert!(encode_event(Event::Output(Channel::Stdout, &[0; 32]), 128).is_ok());
    }
    assert_eq!(
        encode_event(Event::Output(Channel::Stdout, &[0; 1024]), 128),
        Err(ProtocolError::ResourceExhausted)
    );
    assert_eq!(
        encode_event(
            Event::End(End {
                exit_code: 1,
                exited: true,
                status: &"x".repeat(4096),
                error: None
            }),
            128
        ),
        Err(ProtocolError::ResourceExhausted)
    );
}
