use super::*;
use crate::{
    command_requests::http::{Encoding, Rpc},
    file_download::tests::reference::{Reference, hex},
};
use axum::{body::Body, http::HeaderValue};
use http_body_util::BodyExt;
use sandproof_guest_runtime::{
    gzip::{DecodeLimits, Decoder},
    protocol::JsonContentType,
};
use std::time::Duration;
use tokio::sync::Semaphore;

fn limits() -> reply::Limits {
    reply::Limits {
        wire_bytes: 16384,
        chunk_bytes: 17,
    }
}
fn source(user: &str, codec: JsonContentType, gzip: bool) -> (Source, Arc<Semaphore>) {
    let slots = Arc::new(Semaphore::new(1));
    (
        Source {
            rejection: Some(if user.is_empty() {
                Rejection::Missing
            } else {
                Rejection::Unknown(UnknownUser::new(user))
            }),
            format: Some(ResponseFormat::new(
                Rpc::Start,
                codec,
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
async fn released(slots: Arc<Semaphore>) {
    let _slot = tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
        .await
        .unwrap()
        .unwrap();
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        bytes.extend_from_slice(&frame.unwrap().into_data().unwrap());
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
    let mut output = Vec::new();
    while let Some(bytes) = decoder.next_chunk().unwrap() {
        output.extend(bytes);
    }
    output
}

#[tokio::test]
async fn original_default_failure_retains_last_bytes_and_expiry_with_negotiated_format() {
    for gzip in [false, true] {
        for user in ["", "missing'\"\\<>&\n中"] {
            let (source, slots) = source(user, JsonContentType::Utf8, gzip);
            let deadline = source.deadline;
            let response = send(
                source,
                reply::Limits {
                    chunk_bytes: 16384,
                    ..limits()
                },
            )
            .await
            .unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(
                response.headers()["content-type"],
                "application/connect+json; charset=utf-8"
            );
            assert_eq!(response.headers()["connect-accept-encoding"], "gzip");
            assert_eq!(
                response.headers().contains_key("connect-content-encoding"),
                gzip
            );
            assert!(!response.headers().contains_key("content-length"));
            let mut body = response.into_body();
            let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
            let message: serde_json::Value = serde_json::from_slice(&decoded(&bytes)).unwrap();
            assert_eq!(message["error"]["code"], "unauthenticated");
            assert_eq!(
                message["error"]["message"],
                if user.is_empty() {
                    "no user specified".to_owned()
                } else {
                    format!("invalid default user: '{user}'")
                }
            );
            let copy = bytes.clone();
            drop(bytes);
            tokio::time::pause();
            tokio::time::advance(deadline.saturating_duration_since(Instant::now())).await;
            assert!(body.frame().await.unwrap().is_err());
            tokio::time::resume();
            drop(body);
            assert_eq!(slots.available_permits(), 0);
            drop(copy);
            released(slots).await;
        }
    }
}

#[tokio::test]
async fn raw_and_wire_limits_original_expiry_and_missing_observations_retain_capacity() {
    for gzip in [false, true] {
        let (initial, _) = source("private-secret", JsonContentType::Json, gzip);
        let bytes = drain(send(initial, limits()).await.unwrap().into_body()).await;
        let required = bytes.len().max(decoded(&bytes).len() + 5);
        for wire_bytes in [0, required - 1, required] {
            let (source, slots) = source("private-secret", JsonContentType::Json, gzip);
            let deadline = source.deadline;
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
                assert_eq!(
                    failure.error(),
                    if wire_bytes == 0 {
                        reply::Error::InvalidConfiguration
                    } else {
                        reply::Error::ResponseLimit
                    }
                );
                assert_eq!(failure.deadline(), deadline);
                assert!(!format!("{failure:?}").contains("private-secret"));
                assert_eq!(slots.available_permits(), 0);
                drop(failure);
            }
            released(slots).await;
        }
    }
    for case in 0..4 {
        let (mut source, slots) = source("private-secret", JsonContentType::Json, false);
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
        let deadline = source.deadline;
        let failure = send(source, limits()).await.unwrap_err();
        assert_eq!(
            failure.error(),
            if case == 3 {
                reply::Error::DeadlineExceeded
            } else {
                reply::Error::UnclassifiedFault
            }
        );
        assert_eq!(failure.deadline(), deadline);
        assert_eq!(failure.request_error(), RequestError::Unavailable);
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
    }
}

#[tokio::test]
#[ignore = "explicit fixed Go Start default-account replies; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn default_start_errors_match_original_connect_headers_flags_and_json_bytes() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    for codec in [JsonContentType::Json, JsonContentType::Utf8] {
        for gzip in [false, true] {
            for user in [
                "",
                "missing",
                "'\"\\<>&中\u{2028}",
                "\0\r\n",
                &"x".repeat(2100),
            ] {
                let (source, slots) = source(user, codec, gzip);
                let content_type = source.format.unwrap().content_type();
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
                let input = serde_json::json!({"User":user, "Headers":{"content-type":[content_type], "connect-accept-encoding":[if gzip {"gzip"} else {"identity"}]}});
                oracle.check(
                    &format!("D\t{}", hex(&serde_json::to_vec(&input).unwrap())),
                    &observed.join("\t"),
                );
                released(slots).await;
            }
        }
    }
    oracle.finish();
}
