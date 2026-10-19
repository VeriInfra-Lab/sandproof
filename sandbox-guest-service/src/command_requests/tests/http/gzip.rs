use super::*;
use crate::command_requests::http::Encoding;
use crate::file_rpc_requests::tests::gzip::compressed;

fn wire(flag: u8, payload: &[u8]) -> Vec<u8> {
    let mut result = vec![flag];
    result.extend(u32::try_from(payload.len()).unwrap().to_be_bytes());
    result.extend(payload);
    result
}
fn chunks(bytes: &[u8]) -> Body {
    Body::from_stream(stream::iter(
        bytes
            .chunks(17)
            .map(|b| Ok::<_, std::io::Error>(Bytes::copy_from_slice(b)))
            .collect::<Vec<_>>(),
    ))
}

#[tokio::test]
async fn selected_gzip_preserves_request_format_deadline_and_original_capacity() {
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    for rpc in [
        Rpc::Start,
        Rpc::Connect,
        Rpc::List,
        Rpc::SendInput,
        Rpc::CloseStdin,
        Rpc::SendSignal,
        Rpc::Update,
    ] {
        let mut h = headers();
        h.insert(
            "content-type",
            if rpc.is_stream() {
                "application/connect+json"
            } else {
                "application/json"
            }
            .parse()
            .unwrap(),
        );
        h.insert(
            if rpc.is_stream() {
                "connect-content-encoding"
            } else {
                "content-encoding"
            },
            "gzip".parse().unwrap(),
        );
        let selected = prepare_headers(&pool, &session, h)
            .await
            .select_json(&Method::POST, rpc)
            .unwrap();
        assert_eq!(selected.request_encoding(), Encoding::Gzip);
        assert_eq!(selected.response_encoding(), Encoding::Gzip);
        let deadline = selected.deadline();
        let member = [compressed(b"{"), compressed(b"}")].concat();
        let payload = if rpc.is_stream() {
            [wire(1, &member), wire(3, &compressed(b"{}"))].concat()
        } else {
            member
        };
        let ready = selected.decode(chunks(&payload)).await.unwrap();
        let identity = if rpc.is_stream() {
            wire(0, b"{}")
        } else {
            b"{}".to_vec()
        };
        assert_eq!(
            ready.request(),
            &protocol::decode_request(rpc.method(), &identity, 128).unwrap()
        );
        assert_eq!(ready.deadline(), deadline);
        full(&pool);
        drop(ready);
        released(&pool).await;
    }
}

fn decode_reply(bytes: &[u8]) -> Vec<u8> {
    use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};
    let mut decoder = Decoder::new(
        &bytes[5..],
        DecodeLimits {
            encoded_bytes: 4096,
            decoded_bytes: 4096,
            chunk_bytes: 4096,
            header_bytes: 128,
            members: 1,
        },
    )
    .unwrap();
    let json = decoder.next_chunk().unwrap().unwrap();
    assert!(decoder.next_chunk().unwrap().is_none());
    json
}

#[tokio::test]
async fn selected_gzip_rejections_preserve_original_response_and_release_completed_auth() {
    use crate::command_requests::http::reply;
    let session = session().await;
    let pool = CommandRequests::new(limits()).unwrap();
    for rpc in [Rpc::List, Rpc::Start] {
        for tail in [false, true] {
            let mut h = headers();
            h.insert(
                "content-type",
                if rpc.is_stream() {
                    "application/connect+json; charset=utf-8"
                } else {
                    "application/json; charset=utf-8"
                }
                .parse()
                .unwrap(),
            );
            h.insert(
                if rpc.is_stream() {
                    "connect-content-encoding"
                } else {
                    "content-encoding"
                },
                "gzip".parse().unwrap(),
            );
            let selected = prepare_headers(&pool, &session, h)
                .await
                .select_json(&Method::POST, rpc)
                .unwrap();
            let deadline = selected.deadline();
            let bytes = if rpc.is_stream() {
                if tail {
                    [wire(0, b"{}"), wire(3, b"invalid gzip")].concat()
                } else {
                    wire(1, b"invalid gzip")
                }
            } else if tail {
                [compressed(b"{}"), b"invalid gzip".to_vec()].concat()
            } else {
                b"invalid gzip".to_vec()
            };
            let failure = selected.decode(chunks(&bytes)).await.err().unwrap();
            assert_eq!(failure.deadline(), deadline);
            assert!(matches!(
                failure.error(),
                SelectionError::Request(Error::Compression { .. })
            ));
            session
                .initialize(br#"{"accessToken":"token"}"#)
                .await
                .unwrap();
            full(&pool);
            let response = failure
                .into_response(reply::Limits {
                    wire_bytes: 4096,
                    chunk_bytes: 4096,
                })
                .await
                .unwrap();
            assert_eq!(response.status(), if rpc.is_stream() { 200 } else { 400 });
            assert_eq!(
                response.headers()["content-type"],
                if rpc.is_stream() {
                    "application/connect+json; charset=utf-8"
                } else {
                    "application/json"
                }
            );
            let mut body = response.into_body();
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let json = if rpc.is_stream() {
                assert_eq!(bytes[0], 3);
                decode_reply(&bytes)
            } else {
                bytes.to_vec()
            };
            let json: serde_json::Value = serde_json::from_slice(&json).unwrap();
            let error = if rpc.is_stream() {
                &json["error"]
            } else {
                &json
            };
            assert_eq!(error["code"], "invalid_argument");
            assert!(
                error["message"]
                    .as_str()
                    .unwrap()
                    .ends_with("gzip: invalid header")
            );
            let copy = bytes.clone();
            drop(body);
            drop(bytes);
            full(&pool);
            drop(copy);
            released(&pool).await;
        }
    }
}
