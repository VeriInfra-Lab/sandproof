use super::*;
use super::{
    framing::{expanded, frame},
    json_errors::compressed,
};

#[tokio::test]
async fn shared_http_end_stream_errors_preserve_order_and_original_request_ownership() {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let cases = [
        (b"".as_slice(), "unexpected end of JSON input"),
        (
            br#"{"metadata":false}"#,
            "json: cannot unmarshal bool into Go struct field connectEndStreamMessage.metadata of type http.Header",
        ),
        (
            br#"{"error":{"details":[{"value":"?"}]}}"#,
            "decode base64: illegal base64 data at input byte 0",
        ),
    ];
    for method in ["Start", "Connect"] {
        for gzip in [false, true] {
            for after_message in [false, true] {
                for (raw, message) in cases {
                    fixture
                        .session
                        .initialize(br#"{"accessToken":"token","defaultUser":"missing"}"#)
                        .await
                        .unwrap();
                    let mut input = request(method, &json!({}));
                    let payload = if gzip { compressed(raw) } else { raw.to_vec() };
                    let prefix = if after_message {
                        frame(0, b"{}")
                    } else {
                        vec![]
                    };
                    *input.body_mut() =
                        Body::from([prefix, frame(if gzip { 3 } else { 2 }, &payload)].concat());
                    if gzip {
                        input
                            .headers_mut()
                            .insert("connect-content-encoding", "gzip".parse().unwrap());
                    }
                    let reply = http.handle(input).await.unwrap();
                    assert_eq!(reply.status(), StatusCode::OK);
                    assert_eq!(reply.headers()["content-type"], "application/connect+json");
                    let payload = bytes(reply).await;
                    assert_eq!(payload[0], if gzip { 3 } else { 2 });
                    assert_eq!(
                        payload.len(),
                        5 + u32::from_be_bytes(payload[1..5].try_into().unwrap()) as usize
                    );
                    let raw = if gzip {
                        expanded(&payload[5..])
                    } else {
                        payload[5..].to_vec()
                    };
                    assert_eq!(
                        serde_json::from_slice::<Value>(&raw).unwrap(),
                        json!({"error":{"code":"internal","message":format!("unmarshal end stream message: {message}")}})
                    );
                    let last = payload.clone();
                    drop(payload);
                    full(&ordinary);
                    rotates(&fixture).await;
                    drop(last);
                    assert!(ordinary.admit().is_ok());
                }
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
