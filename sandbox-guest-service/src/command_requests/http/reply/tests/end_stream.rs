use super::framing::{format, frame};
use super::*;
use crate::command_requests::{Error as RequestError, compression};
use crate::file_download::tests::reference::{Reference, hex};
use crate::file_rpc_requests::tests::gzip::compressed;

fn observed(
    rpc: Rpc,
    wire: &[u8],
    gzip: bool,
) -> Result<sandproof_guest_runtime::protocol::Request, RequestError> {
    let (_cancel, cancelled) = tokio::sync::watch::channel(false);
    compression::decode_selected(rpc.method(), wire, 65536, 13, deadline(), &cancelled, gzip)
}
fn wire(body: &[u8], gzip: bool) -> Vec<u8> {
    let bytes = if gzip {
        compressed(body)
    } else {
        body.to_vec()
    };
    [frame(0, b"{}"), frame(if gzip { 3 } else { 2 }, &bytes)].concat()
}

#[test]
fn eos_observation_keeps_framing_gzip_message_and_metadata_failure_order() {
    use sandproof_guest_runtime::protocol::{EndStreamReason as E, FramingError as F};
    for rpc in [Rpc::Start, Rpc::Connect] {
        assert!(matches!(
            observed(rpc, &[frame(0, b"bad"), frame(2, b"bad")].concat(), false),
            Err(RequestError::Json(_))
        ));
        assert_eq!(
            observed(rpc, &[wire(b"bad", false), vec![0]].concat(), false),
            Err(RequestError::Framing(F::ExtraBytes(1)))
        );
        assert!(matches!(
            observed(rpc, &wire(b"bad", true), false),
            Err(RequestError::Framing(F::CompressedWithoutSupport))
        ));
        for (body, expected) in [
            (b"".as_slice(), E::Eof),
            (
                br#"{"metadata":false,"error":{"code":false}}"#,
                E::Type {
                    value: "bool",
                    field: "error.code",
                    target: "string",
                },
            ),
            (
                br#"{"metadata":false,"error":{"details":[{"value":"?"}]}}"#,
                E::Base64 { offset: 0 },
            ),
        ] {
            for gzip in [false, true] {
                let Err(RequestError::EndStream(error)) = observed(rpc, &wire(body, gzip), gzip)
                else {
                    panic!("EOS diagnostic required")
                };
                assert_eq!(error.reason(), expected);
            }
        }
        // Whole JSON syntax validation precedes even an earlier custom failure.
        let Err(RequestError::EndStream(error)) =
            observed(rpc, &wire(br#"{"error":false,}"#, false), false)
        else {
            panic!("syntax error required")
        };
        assert_eq!(
            error.reason(),
            E::Syntax {
                byte: b'}',
                context: "looking for beginning of object key string"
            }
        );
    }
}

#[tokio::test]
async fn eos_reply_keeps_exact_budget_original_expiry_and_last_byte_responsibility() {
    for gzip in [false, true] {
        let error = SelectionError::Request(
            observed(Rpc::Start, &wire(br#"{"error":false}"#, gzip), gzip).unwrap_err(),
        );
        let selected = Some(format(Rpc::Start, gzip, true));
        let response = send(error.clone(), selected, limits(), deadline(), Arc::new(()))
            .await
            .unwrap();
        let output = drain(response.into_body()).await;
        let raw = if gzip {
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
                selected,
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
                assert_eq!(failure.selection_error(), &error);
                assert_eq!(failure.deadline(), end);
                assert_eq!(slots.available_permits(), 0);
                drop(failure);
            }
            released(slots).await;
        }
        let (owner, slots) = retain();
        let failure = send(error.clone(), selected, limits(), Instant::now(), owner)
            .await
            .unwrap_err();
        assert_eq!(failure.error(), Error::DeadlineExceeded);
        assert_eq!(failure.selection_error(), &error);
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
        let (_, unary) = rejected(Rpc::List, false);
        for format in [None, unary] {
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
fn cases() -> Vec<Vec<u8>> {
    let values = [
        "null", "true", "0", "1e99999", "\"x\"", "[]", "{}", "[null]", "[false]", "[{}]", "[\"x\"]",
    ];
    let mut cases: Vec<Vec<u8>> = ["", " ", "{}", "{}x", "1e", "-", "tr", "1.", "1e+"]
        .into_iter()
        .map(|s| s.as_bytes().to_vec())
        .collect();
    for value in values {
        for (prefix, suffix) in [
            ("", ""),
            ("{\"error\":", "}"),
            ("{\"metadata\":", "}"),
            ("{\"metadata\":{\"x\":", "}}"),
            ("{\"metadata\":{\"x\":[", "]}}"),
            ("{\"error\":{\"code\":", "}}"),
            ("{\"error\":{\"message\":", "}}"),
            ("{\"error\":{\"details\":", "}}"),
            ("{\"error\":{\"details\":[", "]}}"),
            ("{\"error\":{\"details\":[{\"type\":", "}]}}"),
            ("{\"error\":{\"details\":[{\"value\":", "}]}}"),
        ] {
            cases.push(format!("{prefix}{value}{suffix}").into_bytes());
        }
    }
    let fields = [
        r#""metadata":false"#,
        r#""metadata":{"x":[false]}"#,
        r#""error":{"code":false}"#,
        r#""error":{"message":false,"details":[{"value":"?"}]}"#,
        r#""error":{"details":[{"value":"?","type":false}]}"#,
        r#""error":{"details":[{"type":false}]}"#,
        r#""error":{"details":[{"value":"?"}]}"#,
    ];
    for first in fields {
        for second in fields {
            cases.push(format!("{{{first},{second}}}").into_bytes());
        }
    }
    for value in [
        "", "?", "a", "ab", "abc", "abcd", "ab=", "ab==", "abc=", "abcd=", "=abc", "a===", "ab=?",
        "ab==?", "ab\r\n==", "ab\r\n=", "ab=\n=", "abcd\n", "ab-_/", "雪",
    ] {
        cases.push(
            serde_json::to_vec(&serde_json::json!({"error":{"details":[{"value":value}]}}))
                .unwrap(),
        );
        for at in 0..value.len() {
            let mut input = value.as_bytes().to_vec();
            input[at] = b'?';
            let input = String::from_utf8_lossy(&input);
            cases.push(
                serde_json::to_vec(&serde_json::json!({"error":{"details":[{"value":input}]}}))
                    .unwrap(),
            );
        }
    }
    for seed in [
        br#"{"error":{"details":[{"value":"aA=="}]},"metadata":{"x":["a"]}}"#.as_slice(),
        br#"{"future":"\ud800\udc00","error":null}"#,
        br"[true,false,null,1.2e-3]",
    ] {
        for at in 0..seed.len() {
            for byte in [
                0, 9, 10, 13, b' ', b'0', b'e', b'+', b'-', b':', b',', b'"', b'\'', b'\\', b'[',
                b']', b'{', b'}', 127, 128, 255,
            ] {
                let mut body = seed.to_vec();
                body[at] = byte;
                cases.push(body);
            }
            cases.push(seed[..at].to_vec());
        }
    }
    for byte in 0..=255 {
        cases.push(vec![byte]);
        cases.push([b"{\"x\":\"\\".as_slice(), &[byte], b"\"}"].concat());
    }
    for depth in [9998, 9999, 10000] {
        cases.push(
            format!("{{\"future\":{}0{}}}", "[".repeat(depth), "]".repeat(depth)).into_bytes(),
        );
        cases.push(
            format!(
                "{{\"future\":{}0{},\"metadata\":false}}",
                "[".repeat(depth),
                "]".repeat(depth)
            )
            .into_bytes(),
        );
    }
    cases.extend(binary_cases());
    cases
}

fn binary_cases() -> Vec<Vec<u8>> {
    let mut cases = Vec::new();
    for seed in [
        b"abcd".as_slice(),
        b"aA==",
        b"aBc=",
        b"ab",
        b"a\r\nb==",
        b"aA\n",
    ] {
        for offset in 0..=seed.len() {
            for byte in 0..=255 {
                let mut value = seed.to_vec();
                value.insert(offset, byte);
                cases.push(serde_json::to_vec(&serde_json::json!({"error":{"details":[{"value":String::from_utf8_lossy(&value)}]}})).unwrap());
            }
        }
    }
    cases
}

#[tokio::test]
#[ignore = "explicit fixed Go EOS JSON error HTTP bytes; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_end_stream_diagnostics_match_complete_http_replies() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut counts = [0, 0];
    for rpc in [Rpc::Start, Rpc::Connect] {
        for (index, body) in cases().into_iter().enumerate() {
            let combinations = if index < 200 {
                vec![(false, false), (false, true), (true, false), (true, true)]
            } else {
                vec![(index % 2 == 0, index % 3 == 0)]
            };
            for (gzip, utf8) in combinations {
                let body = wire(&body, gzip);
                let result = observed(rpc, &body, gzip);
                let input = serde_json::json!({"Method":rpc.method(),"Body":hex(&body),"Gzip":gzip,"Reply":result.is_err(),"UTF8":utf8});
                let expected =
                    oracle.observe(&format!("E\t{}", hex(&serde_json::to_vec(&input).unwrap())));
                let error = match result {
                    Ok(_) => {
                        assert!(expected.starts_with("OK\t"), "{input}: {expected}");
                        counts[0] += 1;
                        continue;
                    }
                    Err(error @ RequestError::EndStream(_)) => error,
                    Err(error) => panic!("unclassified EOS {error:?}: {input}"),
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
                assert_eq!(
                    bytes.len(),
                    u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize + 5
                );
                output.push(bytes[0].to_string());
                output.push(hex(&if gzip {
                    decoded(&bytes[5..])
                } else {
                    bytes[5..].to_vec()
                }));
                assert_eq!(output.join("\t"), expected, "{input}");
                counts[1] += 1;
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{} fixed Go valid EOS cases and {} complete HTTP EOS error cases.",
        counts[0], counts[1]
    );
}
