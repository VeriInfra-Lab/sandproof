use super::*;
use crate::command_requests::{Error as RequestError, compression};
use crate::file_download::tests::reference::{Reference, hex};
use crate::file_rpc_requests::tests::gzip::compressed;

fn frame(bytes: &[u8], gzip: bool) -> Vec<u8> {
    let mut wire = vec![u8::from(gzip)];
    wire.extend(u32::try_from(bytes.len()).unwrap().to_be_bytes());
    wire.extend(bytes);
    wire
}
fn wire(rpc: Rpc, body: &[u8], gzip: bool) -> Vec<u8> {
    let bytes = if gzip {
        compressed(body)
    } else {
        body.to_vec()
    };
    if rpc.is_stream() {
        frame(&bytes, gzip)
    } else {
        bytes
    }
}
fn observe(
    rpc: Rpc,
    body: &[u8],
    gzip: bool,
) -> Result<sandproof_guest_runtime::protocol::Request, RequestError> {
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    let result =
        compression::decode_selected(rpc.method(), body, 65536, 13, deadline(), &cancelled, gzip);
    let limited = compression::decode_limited(
        rpc.method(),
        body,
        65536,
        13,
        deadline(),
        &cancelled,
        (
            gzip,
            Some(sandproof_guest_runtime::protocol::StartLimits {
                arguments: 0,
                env_entries: 0,
            }),
        ),
    );
    if result.is_err() {
        assert_eq!(limited, result);
    } else {
        assert!(limited.is_ok());
    }
    result
}

#[test]
fn compact_start_validation_matches_original_json_corpus() {
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    for payload in cases(Rpc::Start) {
        for gzip in [false, true] {
            let body = wire(Rpc::Start, &payload, gzip);
            assert_eq!(
                compression::decode_limited(
                    "Start",
                    &body,
                    65536,
                    13,
                    deadline(),
                    &cancelled,
                    (
                        gzip,
                        Some(sandproof_guest_runtime::protocol::StartLimits {
                            arguments: usize::MAX,
                            env_entries: usize::MAX,
                        }),
                    ),
                ),
                observe(Rpc::Start, &body, gzip),
            );
        }
    }
}

fn format(rpc: Rpc, gzip: bool, utf8: bool) -> ResponseFormat {
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        format!(
            "application/{}json{}",
            if rpc.is_stream() { "connect+" } else { "" },
            if utf8 { "; charset=utf-8" } else { "" }
        )
        .parse()
        .unwrap(),
    );
    if gzip {
        headers.insert(
            if rpc.is_stream() {
                "connect-content-encoding"
            } else {
                "content-encoding"
            },
            "gzip".parse().unwrap(),
        );
    }
    select(
        &headers,
        &Method::POST,
        rpc,
        65536,
        Instant::now(),
        deadline(),
    )
    .unwrap()
    .response()
}
const VALUES: &[&str] = &[
    "null",
    "true",
    "false",
    "0",
    "-0",
    "1.0",
    "1e",
    "1e+",
    "4294967295",
    "4294967296",
    "-2147483648",
    "2147483648",
    "1e999",
    "1.1",
    "\"\"",
    "\"0\"",
    "\"1e\"",
    "\"1e0\"",
    "\"a===\"",
    "\"aA==\"",
    "\"_w\"",
    "\"/w\"",
    "[]",
    "{}",
    "[null]",
    "[\"a\",3]",
    "{\"x\":null}",
    "{\"x\":\"a\",\"\\u0078\":\"b\"}",
];

fn cases(rpc: Rpc) -> Vec<Vec<u8>> {
    let mut cases: Vec<Vec<u8>> = [
        "",
        " ",
        "{}",
        "null",
        "[]",
        "trueX",
        "{} false",
        "{",
        "{\"x\":}",
        "{\"x\":1,}",
        "{\"x\":{\"x\":null}}",
    ]
    .into_iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    let templates: &[(&str, &str)] = match rpc {
        Rpc::Start => &[
            ("{\"process\":", "}"),
            ("{\"tag\":", "}"),
            ("{\"stdin\":", "}"),
            ("{\"process\":{\"cmd\":", "}}"),
            ("{\"process\":{\"args\":", "}}"),
            ("{\"process\":{\"envs\":", "}}"),
            ("{\"process\":{\"cwd\":", "}}"),
            ("{\"pty\":", "}"),
            ("{\"pty\":{\"size\":", "}}"),
            ("{\"pty\":{\"size\":{\"cols\":", "}}}"),
            ("{\"pty\":{\"size\":{\"rows\":", "}}}"),
        ],
        Rpc::List => &[("{\"unknown\":", "}")],
        Rpc::SendInput => &[
            ("{\"input\":", "}"),
            ("{\"input\":{\"stdin\":", "}}"),
            ("{\"input\":{\"pty\":", "}}"),
        ],
        Rpc::SendSignal => &[("{\"signal\":", "}")],
        Rpc::Update => &[
            ("{\"pty\":{\"size\":{\"cols\":", "}}}"),
            ("{\"pty\":{\"size\":{\"rows\":", "}}}"),
        ],
        _ => &[],
    };
    for &(prefix, suffix) in templates {
        for value in VALUES {
            cases.push(format!("{prefix}{value}{suffix}").into_bytes());
        }
    }
    if !matches!(rpc, Rpc::Start | Rpc::List) {
        for value in VALUES {
            for field in ["pid", "tag"] {
                cases.push(format!(r#"{{"process":{{"{field}":{value}}}}}"#).into_bytes());
            }
        }
    }
    for body in [
        r#"{"process":null,"process":false}"#,
        r#"{"process":{"cmd":null,"cmd":true}}"#,
        r#"{"process":{"pid":1,"tag":"secret"}}"#,
        r#"{"process":{"pid":1,"tag":!}}"#,
        r#"{"process":{"pid":1,"tag":null}}"#,
        r#"{"process":{"pid":null,"tag":"ok"}}"#,
        r#"{"process":{"pid":null,"pid":1}}"#,
        r#"{"process":{"args":[null]}}"#,
        r#"{"process":{"envs":{"x":"ok","x":!}}}"#,
        r#"{"input":{"stdin":"","pty":""}}"#,
        r#"{"input":{"stdin":"","pty":!}}"#,
        r#"{"input":{"stdin":null,"pty":""}}"#,
        r#"{"input":{"stdin":"","stdin":null}}"#,
        "\n{\"future\":\"雪😀\",\"process\":false}",
        "{\"process\":{\"pid\":\"proto:\u{a0}literal\"}}",
    ] {
        cases.push(body.as_bytes().to_vec());
    }
    for seed in [
        br#"{"process":{"pid":1,"cmd":"x","args":["a"]}}"#.as_slice(),
        br#"{"input":{"stdin":"aA=="}}"#,
        br#"{"future":"\ud800\udc00"}"#,
    ] {
        for at in 0..seed.len() {
            for byte in [
                0, 9, 10, b' ', b'0', b'e', b'"', b'\\', b':', b',', b']', b'}', 255,
            ] {
                let mut body = seed.to_vec();
                body[at] = byte;
                cases.push(body);
            }
        }
    }
    for depth in [9998, 9999, 10000] {
        cases.push(
            format!("{{\"future\":{}0{}}}", "[".repeat(depth), "]".repeat(depth)).into_bytes(),
        );
    }
    if matches!(rpc, Rpc::Start) {
        cases.extend(environment_histories());
    }
    cases
}

fn environment_histories() -> Vec<Vec<u8>> {
    use std::fmt::Write;

    let mut cases: Vec<Vec<u8>> = [
        r#"{"process":{"envs":{"z":"","z":"","a":null}}}"#,
        r#"{"process":{"envs":{"z":"","z":"","a":!}}}"#,
        r#"{"process":{"envs":{"z":"","z":""},"cwd":[]}}"#,
        r#"{"process":{"envs":{"z":"","a":"","\u007a":"","a":""}}}"#,
        r#"{"process":{"envs":{"z":"","a":null,"z":""}}}"#,
        r#"{"process":{"envs":{"\ud83d\ude00":"","a":"","\ud83d\ude00":null}}}"#,
        r#"{"process":{"envs":{"a123456789012345":"","\u0061123456789012345":"","b":false}}}"#,
        r#"{"process":{"envs":{"a":"","b":""},"envs":null}}"#,
        r#"{"future":1e+,"process":{"envs":{"a":"","b":""},"cwd":"~other"}}"#,
        r#"{"future":1e+,"process":{"envs":{"a":"","b":"","\u0061":""},"cwd":"~other"}}"#,
    ]
    .into_iter()
    .map(|value| value.as_bytes().to_vec())
    .collect();
    for early in [false, true] {
        let mut body = String::from(r#"{"process":{"envs":{"z":"""#);
        if early {
            body.push_str(r#", "\u007a":"""#);
        }
        for index in 0..4097 {
            write!(body, ",\"k{index}\":\"\"").unwrap();
        }
        body.push_str(r#", "z":"", "k0":null}}}"#);
        assert!(body.len() < 65536);
        cases.push(body.into_bytes());
    }
    cases
}

fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
// Protobuf deliberately chooses ASCII or NBSP after its fixed prefix per build.
// Preserve every byte of the actual diagnostic, including user-supplied NBSP.
fn normalize(method: &str, line: &str) -> String {
    let mut columns: Vec<String> = line.split('\t').map(str::to_owned).collect();
    assert_eq!(columns.len(), 10);
    let bytes = unhex(&columns[9]);
    if !columns[6].is_empty() {
        assert_eq!(unhex(&columns[6]), bytes.len().to_string().as_bytes());
        columns[6] = "valid".into();
    }
    let body = String::from_utf8(bytes).unwrap();
    let prefix =
        format!("\"message\":\"unmarshal message: unmarshal into *process.{method}Request: proto:");
    columns[9] = hex(body
        .replacen(&(prefix.clone() + "\u{a0}"), &(prefix + " "), 1)
        .as_bytes());
    columns.join("\t")
}

#[tokio::test]
#[ignore = "explicit fixed Go command protobuf JSON HTTP bytes; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_json_failures_match_ordered_diagnostics_and_complete_http_bytes() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut counts = [0, 0];
    for rpc in [
        Rpc::Start,
        Rpc::Connect,
        Rpc::List,
        Rpc::SendInput,
        Rpc::CloseStdin,
        Rpc::SendSignal,
        Rpc::Update,
    ] {
        let bodies = cases(rpc);
        let original_end = bodies.len()
            - if matches!(rpc, Rpc::Start) {
                environment_histories().len()
            } else {
                0
            };
        for (index, body) in bodies.into_iter().enumerate() {
            // All schemas get both encodings/charsets. Compact schema cases also
            // run the cross product; lexer mutations share the common tokenizer.
            let combinations = if index < 380 || index >= original_end {
                vec![(false, false), (false, true), (true, false), (true, true)]
            } else {
                vec![(index % 2 == 0, index % 3 == 0)]
            };
            for (gzip, utf8) in combinations {
                let body = wire(rpc, &body, gzip);
                let result = observe(rpc, &body, gzip);
                let input = serde_json::json!({"Method":rpc.method(),"Body":hex(&body),"Gzip":gzip,"Reply":result.is_err(),"UTF8":utf8});
                let expected =
                    oracle.observe(&format!("E\t{}", hex(&serde_json::to_vec(&input).unwrap())));
                let error = match result {
                    Ok(_) => {
                        assert!(expected.starts_with("OK\t"), "{input}: {expected}");
                        counts[0] += 1;
                        continue;
                    }
                    Err(error @ RequestError::Json(_)) => error,
                    Err(error) => panic!("unclassified JSON {error:?}: {input}"),
                };
                let reply = send(
                    SelectionError::Request(error),
                    Some(format(rpc, gzip, utf8)),
                    limits(),
                    deadline(),
                    Arc::new(()),
                )
                .await
                .unwrap();
                let mut output = vec![reply.status().as_u16().to_string()];
                for name in [
                    "content-type",
                    "content-encoding",
                    "connect-content-encoding",
                    "accept-encoding",
                    "connect-accept-encoding",
                    "content-length",
                    "allow",
                ] {
                    output.push(hex(reply.headers().get(name).map_or(b"", |v| v.as_bytes())));
                }
                let bytes = drain(reply.into_body()).await;
                let payload = if rpc.is_stream() {
                    assert_eq!(
                        bytes.len(),
                        u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize + 5
                    );
                    output.push(bytes[0].to_string());
                    if gzip {
                        decoded(&bytes[5..])
                    } else {
                        bytes[5..].to_vec()
                    }
                } else {
                    output.push(String::new());
                    bytes
                };
                output.push(hex(&payload));
                assert_eq!(
                    normalize(rpc.method(), &output.join("\t")),
                    normalize(rpc.method(), &expected),
                    "{input}"
                );
                counts[1] += 1;
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{} fixed Go JSON acceptance cases; {} complete HTTP JSON failure cases; only fixed proto-prefix space normalized.",
        counts[0], counts[1]
    );
}

#[tokio::test]
async fn json_replies_keep_original_budget_deadline_private_diagnostic_and_last_bytes() {
    for rpc in [Rpc::Start, Rpc::SendInput] {
        for gzip in [false, true] {
            let input = wire(
                rpc,
                br#"{"process":{"pid":"private-value","cmd":false}}"#,
                gzip,
            );
            let error = SelectionError::Request(observe(rpc, &input, gzip).unwrap_err());
            assert!(!format!("{error:?}").contains("private-value"));
            let format = Some(format(rpc, gzip, true));
            let response = send(error.clone(), format, limits(), deadline(), Arc::new(()))
                .await
                .unwrap();
            let output = drain(response.into_body()).await;
            let raw = if rpc.is_stream() && gzip {
                decoded(&output[5..]).len() + 5
            } else {
                output.len()
            };
            let needed = raw.max(output.len());
            for budget in [needed, needed - 1] {
                let (owner, slots) = retain();
                let end = deadline();
                let result = send(
                    error.clone(),
                    format,
                    Limits {
                        wire_bytes: budget,
                        chunk_bytes: 16384,
                    },
                    end,
                    owner,
                )
                .await;
                if budget == needed {
                    let mut body = result.unwrap().into_body();
                    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
                    let last = bytes.clone();
                    drop(bytes);
                    drop(body);
                    assert_eq!(slots.available_permits(), 0);
                    drop(last);
                } else {
                    let failure = result.unwrap_err();
                    assert_eq!(failure.error(), Error::ResponseLimit);
                    assert_eq!(failure.deadline(), end);
                    assert_eq!(failure.selection_error(), &error);
                    assert_eq!(slots.available_permits(), 0);
                    drop(failure);
                }
                released(slots).await;
            }
            let (owner, slots) = retain();
            let failure = send(error.clone(), format, limits(), Instant::now(), owner)
                .await
                .unwrap_err();
            assert_eq!(failure.error(), Error::DeadlineExceeded);
            assert_eq!(failure.selection_error(), &error);
            assert_eq!(slots.available_permits(), 0);
            drop(failure);
            released(slots).await;
            let other = Some(self::format(
                if rpc.is_stream() {
                    Rpc::List
                } else {
                    Rpc::Start
                },
                gzip,
                false,
            ));
            for format in [None, other] {
                assert_eq!(
                    send(error.clone(), format, limits(), deadline(), Arc::new(()))
                        .await
                        .unwrap_err()
                        .error(),
                    Error::UnclassifiedFault
                );
            }
        }
    }
}

#[tokio::test]
async fn file_diagnostics_cannot_be_relabelled_as_command_errors() {
    use sandproof_guest_runtime::file_protocol::{self, DecodeFailure};
    let Err(DecodeFailure::Json(error)) =
        file_protocol::decode_request_observed("Stat", b"bad", 64)
    else {
        panic!("file diagnostic required");
    };
    assert_eq!(
        send(
            SelectionError::Request(RequestError::Json(error)),
            Some(format(Rpc::List, false, false)),
            limits(),
            deadline(),
            Arc::new(())
        )
        .await
        .unwrap_err()
        .error(),
        Error::UnclassifiedFault
    );
}
