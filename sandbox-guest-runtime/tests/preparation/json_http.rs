use super::framing::{expanded, frame};
use super::*;
use sandproof_guest_runtime::gzip::{EncodeLimits, Encoder};

fn compressed(input: &[u8]) -> Vec<u8> {
    let mut encoder = Encoder::new(
        input,
        EncodeLimits {
            chunk_bytes: 17,
            decoded_bytes: 8192,
            encoded_bytes: 8192,
        },
    )
    .unwrap();
    let mut output = Vec::new();
    while let Some(bytes) = encoder.next_chunk().unwrap() {
        output.extend(bytes);
    }
    output
}

#[tokio::test]
async fn shared_http_json_errors_precede_dispatch_and_retain_last_response_bytes() {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    for method in [
        "Start",
        "Connect",
        "List",
        "SendInput",
        "CloseStdin",
        "SendSignal",
        "Update",
    ] {
        let stream = matches!(method, "Start" | "Connect");
        let pool = if matches!(method, "CloseStdin" | "SendSignal" | "Update") {
            &control
        } else {
            &ordinary
        };
        for gzip in [false, true] {
            for utf8 in [false, true] {
                // JSON must fail before default-account and PID/command work.
                fixture
                    .session
                    .initialize(br#"{"accessToken":"token","defaultUser":"missing"}"#)
                    .await
                    .unwrap();
                let raw = b"{\"future\":\"secret\",}";
                let payload = if gzip { compressed(raw) } else { raw.to_vec() };
                let mut input = request(method, &json!({}));
                *input.body_mut() = Body::from(if stream {
                    frame(u8::from(gzip), &payload)
                } else {
                    payload
                });
                let content_type = format!(
                    "application/{}json{}",
                    if stream { "connect+" } else { "" },
                    if utf8 { "; charset=utf-8" } else { "" }
                );
                input
                    .headers_mut()
                    .insert("content-type", content_type.parse().unwrap());
                if gzip {
                    input.headers_mut().insert(
                        if stream {
                            "connect-content-encoding"
                        } else {
                            "content-encoding"
                        },
                        "gzip".parse().unwrap(),
                    );
                }
                let reply = http.handle(input).await.unwrap();
                assert_eq!(
                    reply.status(),
                    if stream {
                        StatusCode::OK
                    } else {
                        StatusCode::BAD_REQUEST
                    }
                );
                assert_eq!(
                    reply.headers()["content-type"],
                    if stream {
                        content_type.as_str()
                    } else {
                        "application/json"
                    }
                );
                let payload = bytes(reply).await;
                let raw = decoded_response(&payload, stream, gzip);
                let error = json!({"code":"invalid_argument", "message":format!("unmarshal message: unmarshal into *process.{method}Request: proto: syntax error (line 1:20): unexpected token }}")});
                assert_eq!(
                    serde_json::from_slice::<Value>(&raw).unwrap(),
                    if stream {
                        json!({"error":error})
                    } else {
                        error
                    }
                );
                let last = payload.clone();
                drop(payload);
                full(pool);
                rotates(&fixture).await;
                drop(last);
                assert!(pool.admit().is_ok());
            }
        }
    }
    assert_eq!(
        bytes(http.handle(request("List", &json!({}))).await.unwrap()).await,
        "{}"
    );
    assert_eq!(registry.list_json(2).unwrap(), b"{}");
    assert!(ordinary.admit().is_ok());
    assert!(control.admit().is_ok());
    assert!(fixture.owners.reserve().is_ok());
    assert_eq!(fixture.gate_count(), 0);
    fixture.assert_no_effect();
}

fn decoded_response(payload: &[u8], stream: bool, gzip: bool) -> Vec<u8> {
    if stream {
        assert_eq!(payload[0], if gzip { 3 } else { 2 });
        assert_eq!(
            payload.len(),
            5 + u32::from_be_bytes(payload[1..5].try_into().unwrap()) as usize
        );
        if gzip {
            expanded(&payload[5..])
        } else {
            payload[5..].to_vec()
        }
    } else {
        payload.to_vec()
    }
}
