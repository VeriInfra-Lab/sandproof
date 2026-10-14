use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use base64::Engine;
use serde_json::{Value, json};

fn data_frame(bytes: &[u8]) -> Vec<u8> {
    let payload = serde_json::to_vec(&json!({"event":{"data":{"stdout":base64::engine::general_purpose::STANDARD.encode(bytes)}}})).unwrap();
    let mut frame = vec![0];
    frame.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    frame.extend(payload);
    frame
}

async fn check(oracle: &mut Reference, input: &mut Value, body: Vec<u8>, stream: bool) {
    for utf8 in [false, true] {
        for gzip in [false, true] {
            input["UTF8"] = utf8.into();
            input["Gzip"] = gzip.into();
            let format = ResponseFormat::new(
                if stream { Rpc::Start } else { Rpc::List },
                if utf8 {
                    JsonContentType::Utf8
                } else {
                    JsonContentType::Json
                },
                if gzip {
                    Encoding::Gzip
                } else {
                    Encoding::Identity
                },
            );
            let body = if stream {
                source(vec![
                    body.clone(),
                    protocol::encode_stream_end(None, 2).unwrap(),
                ])
            } else {
                Body::from(body.clone())
            };
            let response = send(
                body,
                stream,
                Some(format),
                Limits {
                    wire_bytes: 32768,
                    chunk_bytes: 257,
                },
                deadline(),
                Arc::new(()),
            )
            .await
            .unwrap();
            let mut headers = serde_json::Map::new();
            for name in [
                "content-type",
                "content-encoding",
                "connect-content-encoding",
                "accept-encoding",
                "connect-accept-encoding",
                "content-length",
                "vary",
            ] {
                headers.insert(
                    name.into(),
                    response
                        .headers()
                        .get(name)
                        .map_or("", |v| v.to_str().unwrap())
                        .into(),
                );
            }
            let bytes = drain(response.into_body()).await;
            if headers["content-length"] != "" {
                assert_eq!(headers["content-length"], bytes.len().to_string());
                headers.insert("content-length".into(), "valid".into());
            }
            let frames = if stream {
                frames(&bytes, gzip)
            } else {
                let payload = if gzip { expand(&bytes) } else { bytes };
                vec![(0, serde_json::from_slice::<Value>(&payload).unwrap())]
            };
            let observed = json!({"Status":200,"Headers":headers,"Frames":frames.into_iter().map(|(flag, payload)| json!({"Flags":flag,"Payload":payload})).collect::<Vec<_>>()});
            // Go's encoding/json escapes HTML and line separators in this test
            // record. Compare JSON trees through a canonical escaped spelling.
            let observed = serde_json::to_string(&observed)
                .unwrap()
                .replace('<', "\\u003c")
                .replace('>', "\\u003e")
                .replace('&', "\\u0026")
                .replace('\u{2028}', "\\u2028")
                .replace('\u{2029}', "\\u2029");
            oracle.check(
                &format!("N\t{}", hex(&serde_json::to_vec(input).unwrap())),
                &observed,
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires explicit fixed Go successful command HTTP reference"]
async fn original_success_headers_gzip_frames_and_messages_match() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    for method in ["List", "SendInput", "CloseStdin", "SendSignal", "Update"] {
        check(
            &mut oracle,
            &mut json!({"Method":method}),
            b"{}".to_vec(),
            false,
        )
        .await;
    }
    for text in ["", "plain", "<>&\0\n中\u{2028}", &"xyz".repeat(1500)] {
        let config = protocol::ProcessConfig {
            cmd: text.into(),
            ..Default::default()
        };
        let entries = [protocol::ProcessInfo {
            config: Some(&config),
            pid: 17,
            tag: Some(text),
        }];
        let body = protocol::encode_response(protocol::Response::List(&entries), 16384).unwrap();
        check(
            &mut oracle,
            &mut json!({"Method":"List","Processes":[{"Config":{"Cmd":text},"PID":17,"Tag":text}]}),
            body,
            false,
        )
        .await;
        for method in ["Start", "Connect"] {
            for (kind, body, input) in [
                (
                    "Start",
                    protocol::encode_event(Event::Start(17), 16384).unwrap(),
                    json!({"PID":17}),
                ),
                (
                    "Keepalive",
                    protocol::encode_event(Event::KeepAlive, 16384).unwrap(),
                    json!({}),
                ),
                (
                    "Stdout",
                    data_frame(text.as_bytes()),
                    json!({"Data":hex(text.as_bytes())}),
                ),
                (
                    "End",
                    protocol::encode_event(
                        Event::End(protocol::End {
                            exit_code: 7,
                            exited: true,
                            status: text,
                            error: Some(text),
                        }),
                        16384,
                    )
                    .unwrap(),
                    json!({"ExitCode":7,"Exited":true,"Status":text,"Error":text}),
                ),
            ] {
                let mut input = json!({"Method":method,"Event":input});
                input["Event"]["Kind"] = kind.into();
                check(&mut oracle, &mut input, body, true).await;
            }
        }
    }
    oracle.finish();
    eprintln!("164 original command success observations matched");
}
