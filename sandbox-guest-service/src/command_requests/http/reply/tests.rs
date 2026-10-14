use super::super::{RejectedValue, Rpc, select};
use super::*;
use axum::{
    body::Body,
    http::{HeaderMap, Method},
};
use http_body_util::BodyExt;
use sandproof_guest_runtime::gzip::{DecodeLimits, Decoder};
use std::time::Duration;
use tokio::sync::Semaphore;

mod framing;
mod gzip;
mod json;
mod reference;

fn limits() -> Limits {
    Limits {
        wire_bytes: 16384,
        chunk_bytes: 17,
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn retain() -> (Owner, Arc<Semaphore>) {
    let slots = Arc::new(Semaphore::new(1));
    (Arc::new(slots.clone().try_acquire_owned().unwrap()), slots)
}
async fn released(slots: Arc<Semaphore>) {
    let _slot = tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
        .await
        .unwrap()
        .unwrap();
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut output = Vec::new();
    while let Some(frame) = body.frame().await {
        let bytes = frame.unwrap().into_data().unwrap();
        output.extend_from_slice(&bytes);
    }
    output
}
fn decoded(bytes: &[u8]) -> Vec<u8> {
    let mut decoder = Decoder::new(
        bytes,
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
    while let Some(chunk) = decoder.next_chunk().unwrap() {
        output.extend(chunk);
    }
    output
}
fn rejected(rpc: Rpc, gzip: bool) -> (SelectionError, Option<ResponseFormat>) {
    let mut headers = HeaderMap::new();
    headers.insert(
        "content-type",
        if rpc.is_stream() {
            "application/connect+json; charset=utf-8"
        } else {
            "application/json; charset=utf-8"
        }
        .parse()
        .unwrap(),
    );
    headers.insert("connect-protocol-version", "bad".parse().unwrap());
    if gzip {
        headers.insert(
            if rpc.is_stream() {
                "connect-accept-encoding"
            } else {
                "accept-encoding"
            },
            "gzip".parse().unwrap(),
        );
    }
    let (error, _, format) = select(
        &headers,
        &Method::POST,
        rpc,
        16384,
        Instant::now(),
        deadline(),
    )
    .unwrap_err();
    (error, format)
}

#[tokio::test]
async fn header_replies_keep_original_slot_through_last_bytes_clones_and_cancellation() {
    for rpc in [Rpc::Start, Rpc::List] {
        for gzip in [false, true] {
            for expire in [false, true] {
                let (error, format) = rejected(rpc, gzip);
                let (owner, slots) = retain();
                let mut body = send(
                    error,
                    format,
                    Limits {
                        chunk_bytes: 16384,
                        ..limits()
                    },
                    deadline(),
                    owner,
                )
                .await
                .unwrap()
                .into_body();
                let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
                let copy = bytes.clone();
                if expire {
                    tokio::time::pause();
                    tokio::time::advance(Duration::from_secs(31)).await;
                    assert!(body.frame().await.unwrap().is_err());
                    tokio::time::resume();
                }
                drop(body);
                assert_eq!(slots.available_permits(), 0);
                drop(bytes);
                assert_eq!(slots.available_permits(), 0);
                drop(copy);
                released(slots).await;
            }
        }
    }
}

#[tokio::test]
async fn payload_and_wire_limits_fail_closed_without_refreshing_original_failure() {
    for rpc in [Rpc::Start, Rpc::List] {
        for gzip in [false, true] {
            let (error, format) = rejected(rpc, gzip);
            let output = drain(
                send(error.clone(), format, limits(), deadline(), Arc::new(()))
                    .await
                    .unwrap()
                    .into_body(),
            )
            .await;
            let raw_length = if rpc.is_stream() && gzip {
                decoded(&output[5..]).len() + 5
            } else {
                output.len()
            };
            let required = raw_length.max(output.len());
            for limit in [required, required - 1, 4] {
                let (owner, slots) = retain();
                let end = deadline();
                let result = send(
                    error.clone(),
                    format,
                    Limits {
                        wire_bytes: limit,
                        ..limits()
                    },
                    end,
                    owner,
                )
                .await;
                if limit == required {
                    assert_eq!(drain(result.unwrap().into_body()).await, output);
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
        }
    }
}

#[tokio::test]
async fn unclassified_invalid_and_expired_replies_never_invent_a_public_error() {
    for case in 0..5 {
        let (mut error, mut format) = rejected(Rpc::Start, true);
        let mut budget = limits();
        let mut end = deadline();
        let expected = match case {
            0 => {
                error = SelectionError::ContentType;
                Error::UnclassifiedFault
            }
            1 => {
                format = None;
                Error::UnclassifiedFault
            }
            2 => {
                budget.chunk_bytes = 0;
                Error::InvalidConfiguration
            }
            3 => {
                budget.wire_bytes = 0;
                Error::InvalidConfiguration
            }
            _ => {
                end = Instant::now();
                Error::DeadlineExceeded
            }
        };
        let (owner, slots) = retain();
        let failure = send(error.clone(), format, budget, end, owner)
            .await
            .unwrap_err();
        assert_eq!(failure.error(), expected);
        assert_eq!(failure.selection_error(), &error);
        assert_eq!(failure.deadline(), end);
        assert_eq!(slots.available_permits(), 0);
        drop(failure);
        released(slots).await;
    }
    let private = SelectionError::Version(RejectedValue(b"private-value".to_vec()));
    let failure = send(private, None, limits(), deadline(), Arc::new(()))
        .await
        .unwrap_err();
    assert!(!format!("{failure:?}").contains("private-value"));
}

#[tokio::test]
async fn multichunk_rejection_requires_consumption_before_next_data() {
    use futures_util::FutureExt;
    let (error, format) = rejected(Rpc::Start, true);
    let (owner, slots) = retain();
    let mut body = send(
        error,
        format,
        Limits {
            chunk_bytes: 1,
            ..limits()
        },
        deadline(),
        owner,
    )
    .await
    .unwrap()
    .into_body();
    let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert!(body.frame().now_or_never().is_none());
    drop(first);
    assert!(body.frame().await.unwrap().is_ok());
    drop(body);
    released(slots).await;
}

#[tokio::test]
async fn empty_method_replies_keep_capacity_until_eof_drop_or_original_expiry() {
    for head in [false, true] {
        for finish in 0..3 {
            let (owner, slots) = retain();
            let response = send(
                SelectionError::Method { head },
                None,
                Limits {
                    wire_bytes: 1,
                    chunk_bytes: 1,
                },
                deadline(),
                owner,
            )
            .await
            .unwrap();
            assert_eq!(response.status(), 405);
            assert_eq!(response.headers()["allow"], "POST");
            assert_eq!(
                response
                    .headers()
                    .get("content-length")
                    .map(axum::http::HeaderValue::as_bytes),
                if head { None } else { Some(b"0".as_slice()) }
            );
            for absent in [
                "content-type",
                "content-encoding",
                "connect-content-encoding",
                "accept-encoding",
                "connect-accept-encoding",
            ] {
                assert!(!response.headers().contains_key(absent));
            }
            let mut body = response.into_body();
            assert_eq!(slots.available_permits(), 0);
            if finish == 0 {
                assert!(body.frame().await.is_none());
                assert_eq!(slots.available_permits(), 1);
            } else if finish == 1 {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(31)).await;
                assert!(body.frame().await.unwrap().is_err());
                tokio::time::resume();
            }
            drop(body);
            released(slots).await;
        }
    }
}
