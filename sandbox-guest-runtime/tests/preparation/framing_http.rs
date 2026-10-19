use super::*;
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};

pub(super) fn frame(flag: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = vec![flag];
    bytes.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    bytes.extend(payload);
    bytes
}
pub(super) fn expanded(payload: &[u8]) -> Vec<u8> {
    let mut decoder = Decoder::new(
        payload,
        DecodeLimits {
            chunk_bytes: 13,
            encoded_bytes: 8192,
            decoded_bytes: 8192,
            header_bytes: 64,
            members: 1,
        },
    )
    .unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = decoder.next_chunk().unwrap() {
        bytes.extend(chunk);
    }
    bytes
}

#[tokio::test]
async fn shared_http_framing_rejections_precede_default_user_and_pid_lookup_without_effects() {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let cases = [
        (vec![], "unimplemented", "unary request has zero messages"),
        (
            vec![0],
            "invalid_argument",
            "protocol error: incomplete envelope: unexpected EOF",
        ),
        (
            frame(0, b"{} ")[..7].to_vec(),
            "invalid_argument",
            "protocol error: promised 3 bytes in enveloped message, got 2 bytes",
        ),
        (
            frame(1, b""),
            "internal",
            "protocol error: sent compressed message without compression support",
        ),
        (
            frame(2, b"{}"),
            "unimplemented",
            "unary request has zero messages",
        ),
        (
            frame(4, b"bad"),
            "internal",
            "protocol error: invalid envelope flags 4",
        ),
        (
            [frame(2, b"bad"), vec![1, 2, 3]].concat(),
            "internal",
            "corrupt response: 3 extra bytes after end of stream",
        ),
        (
            [frame(0, b"{}"), frame(0, b"{}")].concat(),
            "unimplemented",
            "unary request has multiple messages",
        ),
    ];
    for method in ["Start", "Connect"] {
        for gzip in [false, true] {
            for (wire, code, message) in &cases {
                // If framing accidentally reaches Start, this missing default
                // would produce another error. Connect would instead lack a PID.
                fixture
                    .session
                    .initialize(br#"{"accessToken":"token","defaultUser":"missing"}"#)
                    .await
                    .unwrap();
                let mut input = request(method, &json!({}));
                *input.body_mut() = Body::from(wire.clone());
                if gzip {
                    input
                        .headers_mut()
                        .insert("connect-accept-encoding", "gzip".parse().unwrap());
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
                let json = if gzip {
                    expanded(&payload[5..])
                } else {
                    payload[5..].to_vec()
                };
                assert_eq!(
                    serde_json::from_slice::<Value>(&json).unwrap(),
                    json!({"error":{"code":code,"message":message}})
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
