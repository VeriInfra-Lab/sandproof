use super::*;
use crate::command_requests::http::{Encoding, Rpc};
use axum::body::Body;
use http_body_util::BodyExt;
use sandproof_guest_runtime::{
    gzip::{DecodeLimits, Decoder},
    protocol::JsonContentType,
};
use std::time::Duration;
use tokio::sync::Semaphore;
mod expansion;
mod native;
mod nul;
mod pipes;

#[tokio::test]
#[ignore = "explicit fixed Go missing-cwd error rendering; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_cwd_error_matches_headers_flags_and_complete_json_bytes() {
    use crate::file_download::tests::reference::{Reference, hex};
    use axum::http::HeaderValue;
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for codec in [JsonContentType::Json, JsonContentType::Utf8] {
        for gzip in [false, true] {
            for path in [
                "",
                "/missing",
                "/'\"\\<>&中\u{2028}",
                "/\0\r\n",
                &format!("/{}", "x".repeat(2100)),
            ] {
                let (mut source, slots) = source(path, gzip);
                source.format = Some(ResponseFormat::new(
                    Rpc::Start,
                    codec,
                    if gzip {
                        Encoding::Gzip
                    } else {
                        Encoding::Identity
                    },
                ));
                let response = send(source, limits()).await.unwrap();
                let mut observed = vec![response.status().as_u16().to_string()];
                for name in [
                    "content-type",
                    "content-encoding",
                    "connect-content-encoding",
                    "accept-encoding",
                    "connect-accept-encoding",
                    "content-length",
                    "allow",
                ] {
                    observed.push(hex(response
                        .headers()
                        .get(name)
                        .map_or(b"", HeaderValue::as_bytes)));
                }
                let bytes = drain(response.into_body()).await;
                observed.push(bytes[0].to_string());
                observed.push(hex(&decoded(&bytes)));
                let input = serde_json::json!({"Method":"Start","Kind":"cwd","Cwd":path,"Gzip":gzip,"UTF8":codec==JsonContentType::Utf8});
                oracle.check(
                    &format!("R\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                    &observed.join("\t"),
                );
                released(slots).await;
                count += 1;
            }
        }
    }
    oracle.finish();
    eprintln!("{count} fixed Go cwd error renderings; no original Stat or handler execution.");
}

fn source(path: &str, gzip: bool) -> (Source, Arc<Semaphore>) {
    let slots = Arc::new(Semaphore::new(1));
    (
        Source {
            rejection: Some(Rejection::MissingCwd(path.into())),
            format: Some(ResponseFormat::new(
                Rpc::Start,
                JsonContentType::Json,
                if gzip {
                    Encoding::Gzip
                } else {
                    Encoding::Identity
                },
            )),
            request_error: RequestError::Unavailable,
            deadline: Instant::now() + Duration::from_secs(30),
            _owner: Arc::new(slots.clone().try_acquire_owned().unwrap()),
        },
        slots,
    )
}
fn limits() -> reply::Limits {
    reply::Limits {
        wire_bytes: 16384,
        chunk_bytes: 16384,
    }
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        bytes.extend(frame.unwrap().into_data().unwrap());
    }
    bytes
}
fn decoded(bytes: &[u8]) -> Vec<u8> {
    assert_eq!(
        u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize,
        bytes.len() - 5
    );
    if bytes[0] == 2 {
        return bytes[5..].to_vec();
    }
    assert_eq!(bytes[0], 3);
    let mut decoder = Decoder::new(
        &bytes[5..],
        DecodeLimits {
            chunk_bytes: 13,
            encoded_bytes: 16384,
            decoded_bytes: 16384,
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
async fn released(slots: Arc<Semaphore>) {
    let _permit = tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cwd_reply_keeps_exact_path_negotiation_expiry_and_last_byte_owner() {
    for gzip in [false, true] {
        let path = "/private/'\"\\<>&\n中";
        let (source, slots) = source(path, gzip);
        let deadline = source.deadline;
        let response = send(source, limits()).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["content-type"],
            "application/connect+json"
        );
        assert_eq!(
            response.headers().contains_key("connect-content-encoding"),
            gzip
        );
        let mut body = response.into_body();
        let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&decoded(&first)).unwrap(),
            serde_json::json!({"error":{
                "code":"invalid_argument","message":format!("cwd '{path}' does not exist")
            }})
        );
        let last = first.clone();
        drop(first);
        tokio::time::pause();
        tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
        assert!(body.frame().await.unwrap().is_err());
        tokio::time::resume();
        drop(body);
        assert_eq!(slots.available_permits(), 0);
        drop(last);
        released(slots).await;
    }
}

#[tokio::test]
async fn cwd_reply_exact_budget_and_unclassified_or_expired_failures_keep_original_owner() {
    for gzip in [false, true] {
        let (initial, _) = source("/private-secret", gzip);
        let bytes = drain(send(initial, limits()).await.unwrap().into_body()).await;
        let required = bytes.len().max(decoded(&bytes).len() + 5);
        for wire_bytes in [required - 1, required] {
            let (source, slots) = source("/private-secret", gzip);
            let result = send(
                source,
                reply::Limits {
                    wire_bytes,
                    ..limits()
                },
            )
            .await;
            if wire_bytes == required {
                assert_eq!(
                    decoded(&drain(result.unwrap().into_body()).await),
                    decoded(&bytes)
                );
            } else {
                let failure = result.unwrap_err();
                assert_eq!(failure.error, reply::Error::ResponseLimit);
                assert!(!format!("{failure:?}").contains("private-secret"));
                assert_eq!(slots.available_permits(), 0);
                drop(failure);
            }
            released(slots).await;
        }
    }
    for case in 0..4 {
        let (mut source, slots) = source("/private-secret", false);
        match case {
            0 => source.rejection = None,
            1 => source.format = None,
            2 => {
                source.format = Some(ResponseFormat::new(
                    Rpc::List,
                    JsonContentType::Json,
                    Encoding::Identity,
                ));
            }
            _ => source.deadline = Instant::now(),
        }
        let failure = send(source, limits()).await.unwrap_err();
        assert_eq!(
            failure.error,
            if case == 3 {
                reply::Error::DeadlineExceeded
            } else {
                reply::Error::UnclassifiedFault
            }
        );
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
    }
}
