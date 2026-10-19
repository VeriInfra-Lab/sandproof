use sandproof_guest_core::Channel;
use sandproof_guest_runtime::protocol::{
    End, Event, ProcessConfig, ProcessInfo, ProtocolError, Response, encode_event, encode_response,
    encode_stream_end,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

fn payload(frame: &[u8]) -> Value {
    assert_eq!(frame[0], 0);
    assert_eq!(
        u32::from_be_bytes(frame[1..5].try_into().unwrap()) as usize,
        frame.len() - 5
    );
    serde_json::from_slice(&frame[5..]).unwrap()
}

#[test]
fn list_defaults_optional_presence_and_empty_unary_acknowledgements() {
    for response in [Response::Empty, Response::List(&[])] {
        assert_eq!(encode_response(response, 2).unwrap(), b"{}");
    }
    let config = ProcessConfig::default();
    let optional = ProcessConfig {
        cwd: Some(String::new()),
        ..config.clone()
    };
    let processes = [
        ProcessInfo {
            config: None,
            pid: 0,
            tag: None,
        },
        ProcessInfo {
            config: Some(&config),
            pid: 1,
            tag: None,
        },
        ProcessInfo {
            config: Some(&optional),
            pid: u32::MAX,
            tag: Some(""),
        },
    ];
    assert_eq!(
        serde_json::from_slice::<Value>(
            &encode_response(Response::List(&processes), 4096).unwrap()
        )
        .unwrap(),
        json!({"processes":[{}, {"config":{}, "pid":1}, {"config":{"cwd":""}, "pid":u32::MAX, "tag":""}]})
    );
    for (event, expected) in [
        (Event::Start(0), json!({"start":{}})),
        (
            Event::End(End {
                exit_code: 0,
                exited: false,
                status: "",
                error: None,
            }),
            json!({"end":{}}),
        ),
        (
            Event::End(End {
                exit_code: 0,
                exited: false,
                status: "",
                error: Some(""),
            }),
            json!({"end":{"error":""}}),
        ),
        (
            Event::Output(Channel::Stdout, b""),
            json!({"data":{"stdout":""}}),
        ),
        (
            Event::Output(Channel::Stderr, b""),
            json!({"data":{"stderr":""}}),
        ),
        (Event::Output(Channel::Pty, b""), json!({"data":{"pty":""}})),
    ] {
        assert_eq!(
            payload(&encode_event(event, 1024).unwrap()),
            json!({"event":expected})
        );
    }
}

#[test]
fn complete_response_and_escaped_metadata_must_fit_without_cloning_the_snapshot() {
    assert_eq!(
        encode_response(Response::Empty, 0),
        Err(ProtocolError::ResourceExhausted)
    );
    assert_eq!(
        encode_response(Response::List(&[]), 1),
        Err(ProtocolError::ResourceExhausted)
    );
    for count in [0, 1, 2, 7, 32, 1024] {
        let text = "\"\\\n\0中💡".repeat(count);
        let config = ProcessConfig {
            cmd: text.clone(),
            args: vec![text.clone(), String::new()],
            envs: BTreeMap::from([(String::new(), text.clone()), (text.clone(), String::new())]),
            cwd: Some(text.clone()),
        };
        let entries = [ProcessInfo {
            config: Some(&config),
            pid: u32::MAX,
            tag: Some(&text),
        }];
        let expected = encode_response(Response::List(&entries), 1 << 20).unwrap();
        for limit in [expected.len(), expected.len() + 1, expected.len() + 256] {
            let encoded = encode_response(Response::List(&entries), limit).unwrap();
            assert_eq!(encoded, expected);
            assert!(encoded.capacity() <= limit);
        }
        assert_eq!(
            encode_response(Response::List(&entries), expected.len() - 1),
            Err(ProtocolError::ResourceExhausted)
        );
    }
}

struct Reference {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    observed: usize,
}
impl Reference {
    fn new() -> Self {
        let executable = std::env::var_os("SANDPROOF_GUEST_CONNECT_REFERENCE")
            .expect("explicit fixed Go oracle required");
        let mut child = Command::new(executable)
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
            observed: 0,
        }
    }
    fn check(&mut self, input: &Value, encoded: &[u8], stream: bool) {
        let bytes = serde_json::to_vec(input).unwrap();
        write!(self.input, "O\t").unwrap();
        for b in bytes {
            write!(self.input, "{b:02x}").unwrap();
        }
        writeln!(self.input).unwrap();
        self.input.flush().unwrap();
        let mut line = String::new();
        assert!(
            self.output.read_line(&mut line).unwrap() > 0,
            "oracle closed at {input}"
        );
        let original: Value = serde_json::from_str(&line).expect("original HTTP observation");
        let expected = if stream {
            let end = encode_stream_end(None, 2).unwrap();
            assert_eq!(end, b"\x02\0\0\0\x02{}");
            json!({"Status":200,"ContentType":"application/connect+json","Frames":[
                {"Flags":0,"Payload":payload(encoded)}, {"Flags":2,"Payload":{}}
            ]})
        } else {
            json!({"Status":200,"ContentType":"application/json","Frames":[
                {"Flags":0,"Payload":serde_json::from_slice::<Value>(encoded).unwrap()}
            ]})
        };
        assert_eq!(original, expected, "controlled response input: {input}");
        self.observed += 1;
    }
}
impl Drop for Reference {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
#[ignore = "explicit original generated process messages/Connect HTTP reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn command_responses_match_original_go_presence_bytes_and_stream_framing() {
    let mut oracle = Reference::new();
    for method in ["SendInput", "CloseStdin", "SendSignal", "Update"] {
        oracle.check(
            &json!({"Method":method}),
            &encode_response(Response::Empty, 2).unwrap(),
            false,
        );
    }
    oracle.check(
        &json!({"Method":"List"}),
        &encode_response(Response::List(&[]), 2).unwrap(),
        false,
    );
    for text in ["", "plain", "\"\\\r\n\t\0", "中💡", "<>&\u{2028}\u{2029}"] {
        for pid in [0, 1, u32::MAX] {
            for present in [false, true] {
                let config = ProcessConfig {
                    cmd: text.into(),
                    args: vec![String::new(), text.into()],
                    envs: BTreeMap::from([
                        (String::new(), text.into()),
                        (text.into(), String::new()),
                    ]),
                    cwd: present.then(|| text.into()),
                };
                let default = ProcessConfig::default();
                let entries = [
                    ProcessInfo {
                        config: Some(&config),
                        pid,
                        tag: present.then_some(text),
                    },
                    ProcessInfo {
                        config: present.then_some(&default),
                        pid: 0,
                        tag: None,
                    },
                ];
                oracle.check(&json!({"Method":"List","Processes":[
                    {"Config":{"Cmd":config.cmd,"Args":config.args,"Envs":config.envs,"Cwd":config.cwd},"PID":pid,"Tag":present.then_some(text)},
                    {"Config":present.then(|| json!({}))}
                ]}), &encode_response(Response::List(&entries), 65536).unwrap(), false);
            }
        }
    }
    for method in ["Start", "Connect"] {
        for pid in [0, 1, u32::MAX] {
            oracle.check(
                &json!({"Method":method,"Event":{"Kind":"Start","PID":pid}}),
                &encode_event(Event::Start(pid), 1024).unwrap(),
                true,
            );
        }
        oracle.check(
            &json!({"Method":method,"Event":{"Kind":"Keepalive"}}),
            &encode_event(Event::KeepAlive, 1024).unwrap(),
            true,
        );
        for size in [0, 1, 2, 3, 255, 4096] {
            let data: Vec<u8> = (0..size).map(|i| u8::try_from(i % 256).unwrap()).collect();
            let mut hex = String::new();
            for byte in &data {
                write!(&mut hex, "{byte:02x}").unwrap();
            }
            for (channel, kind) in [
                (Channel::Stdout, "Stdout"),
                (Channel::Stderr, "Stderr"),
                (Channel::Pty, "Pty"),
            ] {
                oracle.check(
                    &json!({"Method":method,"Event":{"Kind":kind,"Data":hex}}),
                    &encode_event(Event::Output(channel, &data), 65536).unwrap(),
                    true,
                );
            }
        }
        for code in [i32::MIN, -1, 0, 17, i32::MAX] {
            for exited in [false, true] {
                for status in ["", "exit status 17", "\"\\\n\0中💡"] {
                    for error in [None, Some(""), Some("signal: killed")] {
                        oracle.check(&json!({"Method":method,"Event":{"Kind":"End","ExitCode":code,"Exited":exited,"Status":status,"Error":error}}), &encode_event(Event::End(End { exit_code: code, exited, status, error }), 1024).unwrap(), true);
                    }
                }
            }
        }
    }
    assert_eq!(oracle.observed, 259);
    eprintln!(
        "{} original Go unary/stream command response observations passed",
        oracle.observed
    );
}
