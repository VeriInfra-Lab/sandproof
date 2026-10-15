use super::*;
use crate::command_requests::http::Rpc;
use sandproof_guest_runtime::{
    gzip::{DecodeLimits, Decoder},
    protocol::{self, Event, JsonContentType},
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Waker},
};

mod reference;

fn assert_pending<F: Future>(future: Pin<&mut F>) {
    assert!(
        future
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
}

fn limits() -> Limits {
    Limits {
        wire_bytes: 8192,
        chunk_bytes: 8192,
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn format(stream: bool, gzip: bool) -> ResponseFormat {
    ResponseFormat::new(
        if stream { Rpc::Start } else { Rpc::List },
        JsonContentType::Utf8,
        if gzip {
            Encoding::Gzip
        } else {
            Encoding::Identity
        },
    )
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut result = Vec::new();
    while let Some(frame) = body.frame().await {
        let bytes = frame.unwrap().into_data().unwrap();
        result.extend_from_slice(&bytes);
    }
    result
}
fn expand(bytes: &[u8]) -> Vec<u8> {
    let mut decoder = Decoder::new(
        bytes,
        DecodeLimits {
            encoded_bytes: 1 << 20,
            decoded_bytes: 1 << 20,
            chunk_bytes: 128,
            header_bytes: 4096,
            members: 8,
        },
    )
    .unwrap();
    let mut result = Vec::new();
    while let Some(chunk) = decoder.next_chunk().unwrap() {
        result.extend(chunk);
    }
    result
}
fn source(frames: Vec<Vec<u8>>) -> Body {
    Body::from_stream(futures_util::stream::iter(
        frames
            .into_iter()
            .map(|bytes| Ok::<_, Error>(Bytes::from(bytes))),
    ))
}
fn frames(body: &[u8], gzip: bool) -> Vec<(u8, serde_json::Value)> {
    let mut bytes = body;
    let mut result = Vec::new();
    while !bytes.is_empty() {
        let size = u32::from_be_bytes(bytes[1..5].try_into().unwrap()) as usize;
        let payload = &bytes[5..5 + size];
        let value = if gzip {
            expand(payload)
        } else {
            payload.to_vec()
        };
        result.push((bytes[0], serde_json::from_slice(&value).unwrap()));
        bytes = &bytes[5 + size..];
    }
    result
}

#[tokio::test]
async fn unary_and_stream_headers_framing_and_small_gzip_messages_preserve_format() {
    for gzip in [false, true] {
        for size in [2, 2048, 2049, 4096] {
            let payload = format!("\"{}\"", "a".repeat(size - 2)).into_bytes();
            let response = send(
                Body::from(payload.clone()),
                false,
                Some(format(false, gzip)),
                limits(),
                deadline(),
                Arc::new(()),
            )
            .await
            .unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(
                response.headers()["content-type"],
                "application/json; charset=utf-8"
            );
            assert_eq!(response.headers()["accept-encoding"], "gzip");
            assert_eq!(response.headers().contains_key("content-encoding"), gzip);
            assert!(!response.headers().contains_key("connect-content-encoding"));
            let length = response
                .headers()
                .get("content-length")
                .map(|v| v.to_str().unwrap().parse::<usize>().unwrap());
            let encoded = drain(response.into_body()).await;
            assert_eq!(length, (encoded.len() <= 2048).then_some(encoded.len()));
            assert_eq!(if gzip { expand(&encoded) } else { encoded }, payload);
        }
        let input = vec![
            protocol::encode_event(Event::Start(17), 1024).unwrap(),
            protocol::encode_event(Event::KeepAlive, 1024).unwrap(),
            protocol::encode_stream_end(None, 1024).unwrap(),
        ];
        let expected = frames(&input.concat(), false);
        let response = send(
            source(input),
            true,
            Some(format(true, gzip)),
            Limits {
                chunk_bytes: 7,
                ..limits()
            },
            deadline(),
            Arc::new(()),
        )
        .await
        .unwrap();
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
        let observed = frames(&drain(response.into_body()).await, gzip);
        assert_eq!(
            observed,
            expected
                .into_iter()
                .map(|(flag, value)| (flag | u8::from(gzip), value))
                .collect::<Vec<_>>()
        );
    }
}

struct Count(Arc<AtomicUsize>);
impl Drop for Count {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct OriginalFrame {
    bytes: Vec<u8>,
    _count: Count,
}
impl AsRef<[u8]> for OriginalFrame {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

#[tokio::test]
async fn original_frame_and_request_survive_compressed_byte_clones_and_no_read_ahead() {
    for gzip in [false, true] {
        let request_drops = Arc::new(AtomicUsize::new(0));
        let frame_drops = Arc::new(AtomicUsize::new(0));
        let polls = Arc::new(AtomicUsize::new(0));
        let poll_count = polls.clone();
        let first = Bytes::from_owner(OriginalFrame {
            bytes: protocol::encode_event(Event::Start(17), 1024).unwrap(),
            _count: Count(frame_drops.clone()),
        });
        let body = Body::from_stream(futures_util::stream::unfold(Some(first), move |value| {
            let polls = poll_count.clone();
            async move {
                polls.fetch_add(1, Ordering::SeqCst);
                Some((
                    Ok::<_, Error>(value.unwrap_or_else(|| {
                        Bytes::from(protocol::encode_stream_end(None, 2).unwrap())
                    })),
                    None,
                ))
            }
        }));
        let response = send(
            body,
            true,
            Some(format(true, gzip)),
            limits(),
            deadline(),
            Arc::new(Count(request_drops.clone())),
        )
        .await
        .unwrap();
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        let mut body = response.into_body();
        let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
        let copy = bytes.clone();
        drop(bytes);
        let mut wait = Box::pin(body.frame());
        assert_pending(wait.as_mut());
        drop(wait);
        assert_eq!(polls.load(Ordering::SeqCst), 1);
        assert_eq!(frame_drops.load(Ordering::SeqCst), 0);
        drop(body);
        assert_eq!(request_drops.load(Ordering::SeqCst), 0);
        drop(copy);
        assert_eq!(request_drops.load(Ordering::SeqCst), 1);
        assert_eq!(frame_drops.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn source_truncation_invalid_framing_expiry_and_independent_budgets_never_fake_eof() {
    for input in [
        vec![],
        vec![vec![0]],
        vec![vec![1, 0, 0, 0, 2, b'{', b'}']],
        vec![vec![0, 0, 0, 0, 4, b'{', b'}']],
    ] {
        let mut body = send(
            source(input),
            true,
            Some(format(true, true)),
            limits(),
            deadline(),
            Arc::new(()),
        )
        .await
        .unwrap()
        .into_body();
        assert!(body.frame().await.unwrap().is_err());
    }
    for gzip in [false, true] {
        for limit in [1, 2] {
            let result = send(
                Body::from("{}"),
                false,
                Some(format(false, gzip)),
                Limits {
                    wire_bytes: limit,
                    ..limits()
                },
                deadline(),
                Arc::new(()),
            )
            .await;
            if limit == 2 && !gzip {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.unwrap_err().error(), Error::ResponseLimit);
            }
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let failure = send(
        Body::from("{}"),
        false,
        None,
        limits(),
        deadline(),
        Arc::new(Count(drops.clone())),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.error(), Error::UnclassifiedFault);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(failure);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    tokio::time::pause();
    let expires = deadline();
    let mut body = send(
        source(vec![protocol::encode_stream_end(None, 2).unwrap()]),
        true,
        Some(format(true, true)),
        limits(),
        expires,
        Arc::new(()),
    )
    .await
    .unwrap()
    .into_body();
    tokio::time::advance(Duration::from_secs(31)).await;
    let error = body.frame().await.unwrap().unwrap_err();
    assert!(error.to_string().contains("DeadlineExceeded"));
    let failure = send(
        Body::from("{}"),
        false,
        Some(format(false, false)),
        limits(),
        expires,
        Arc::new(()),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.error(), Error::DeadlineExceeded);
    assert_eq!(failure.deadline(), expires);
}

#[test]
fn cancelled_queued_encoder_keeps_original_bytes_and_request_until_actual_worker_exit() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, hold) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            hold.recv().unwrap();
        });
        ready.await.unwrap();
        let drops = Arc::new(AtomicUsize::new(0));
        let frame_drops = Arc::new(AtomicUsize::new(0));
        let mut pending = Box::pin(send(
            Body::from(Bytes::from_owner(OriginalFrame {
                bytes: b"{}".to_vec(),
                _count: Count(frame_drops.clone()),
            })),
            false,
            Some(format(false, true)),
            limits(),
            deadline(),
            Arc::new(Count(drops.clone())),
        ));
        assert_pending(pending.as_mut());
        drop(pending);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(frame_drops.load(Ordering::SeqCst), 0);
        release.send(()).unwrap();
        blocker.await.unwrap();
        // A second blocking job is behind the cancelled encoder in this one-slot
        // pool. Its completion observes actual queued-work destruction.
        tokio::task::spawn_blocking(|| ()).await.unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(frame_drops.load(Ordering::SeqCst), 1);
    });
}

#[tokio::test]
async fn each_frame_has_its_own_limit_without_capping_total_stream_output() {
    for gzip in [false, true] {
        let input_frames = (0..128)
            .map(|pid| protocol::encode_event(Event::Start(pid), 128).unwrap())
            .chain(std::iter::once(
                protocol::encode_stream_end(None, 2).unwrap(),
            ))
            .map(|bytes| Ok::<_, Error>(Bytes::from(bytes)));
        let response = send(
            Body::from_stream(futures_util::stream::iter(input_frames)),
            true,
            Some(format(true, gzip)),
            Limits {
                wire_bytes: 128,
                chunk_bytes: 31,
            },
            deadline(),
            Arc::new(()),
        )
        .await
        .unwrap();
        let bytes = drain(response.into_body()).await;
        assert!(bytes.len() > 128);
        assert_eq!(frames(&bytes, gzip).len(), 129);
    }
}
