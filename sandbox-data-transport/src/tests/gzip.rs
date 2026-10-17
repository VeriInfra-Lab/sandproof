use super::*;
use sandproof_stream_codec::{EncodeLimits, Encoder};
use std::future::Future;

fn encoded(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = Encoder::new(
        bytes,
        EncodeLimits {
            chunk_bytes: 4096,
            decoded_bytes: bytes.len() as u64,
            encoded_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    let mut output = Vec::new();
    while let Some(chunk) = encoder.next_chunk().unwrap() {
        output.extend(chunk);
    }
    output
}
fn upload(bytes: Vec<u8>) -> Request<Body> {
    let frames: Vec<_> = bytes
        .chunks(4096)
        .map(|bytes| Ok::<_, io::Error>(Bytes::copy_from_slice(bytes)))
        .collect();
    Request::builder()
        .method(Method::POST)
        .uri("/files?path=/tmp/gzip.bin")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_ENCODING, "gzip")
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(Body::from_stream(futures_util::stream::iter(frames)))
        .unwrap()
}
async fn drain(mut body: Body) -> Result<Vec<u8>, axum::Error> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        if let Ok(chunk) = frame?.into_data() {
            bytes.extend_from_slice(&chunk);
        }
    }
    Ok(bytes)
}

#[tokio::test]
async fn gzip_upload_decodes_in_bounded_chunks_and_removes_only_obsolete_framing() {
    for size in [0, 1, 16_383, 16_384, 65_792] {
        let expected: Vec<u8> = (0..=255).cycle().take(size).collect();
        let prepared = prepare_body(
            upload(encoded(&expected)),
            true,
            Some(&TransferLimits::fixture()),
        )
        .await
        .unwrap();
        assert!(!prepared.headers().contains_key(header::CONTENT_ENCODING));
        assert!(!prepared.headers().contains_key(header::CONTENT_LENGTH));
        assert_eq!(
            prepared.headers()[header::CONTENT_TYPE],
            "application/octet-stream"
        );
        let progress = prepared
            .extensions()
            .get::<UploadProgress>()
            .unwrap()
            .clone();
        let mut body = prepared.into_body();
        let mut actual = Vec::new();
        while let Some(frame) = body.frame().await {
            let bytes = frame.unwrap().into_data().unwrap();
            assert!(bytes.len() <= 16 * 1024);
            actual.extend_from_slice(&bytes);
        }
        assert_eq!(actual, expected);
        assert!(progress.complete());
    }
}

#[tokio::test]
async fn gzip_upload_enforces_expanded_budget_and_validated_eof_not_compressed_length() {
    let limits = TransferLimits {
        file_upload_bytes: 32 * 1024,
        ..TransferLimits::fixture()
    };
    for size in [
        limits.file_upload_bytes - 1,
        limits.file_upload_bytes,
        limits.file_upload_bytes + 1,
    ] {
        let bytes = encoded(&vec![b'A'; size]);
        assert!(bytes.len() < limits.file_upload_bytes);
        let prepared = prepare_body(upload(bytes), true, Some(&limits))
            .await
            .unwrap();
        let progress = prepared
            .extensions()
            .get::<UploadProgress>()
            .unwrap()
            .clone();
        let gzip = prepared
            .extensions()
            .get::<gzip_upload::Progress>()
            .unwrap()
            .clone();
        let result = drain(prepared.into_body()).await;
        if size > limits.file_upload_bytes {
            assert!(result.is_err());
            assert!(!progress.complete());
            assert_eq!(gzip.status(), Some(StatusCode::PAYLOAD_TOO_LARGE));
        } else {
            assert_eq!(result.unwrap(), vec![b'A'; size]);
            assert!(progress.complete());
            assert_eq!(gzip.status(), None);
        }
    }
    let valid = encoded(&vec![b'A'; 20_000]);
    for mut bad in [valid[..valid.len() - 1].to_vec(), valid.clone()] {
        let last = bad.len() - 1;
        bad[last] ^= 1;
        let prepared = prepare_body(upload(bad), true, Some(&limits))
            .await
            .unwrap();
        let progress = prepared
            .extensions()
            .get::<UploadProgress>()
            .unwrap()
            .clone();
        assert!(drain(prepared.into_body()).await.is_err());
        assert!(!progress.complete());
    }
    for bytes in [Vec::new(), b"not gzip".to_vec()] {
        assert_eq!(
            prepare_body(upload(bytes), true, Some(&limits))
                .await
                .unwrap_err()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[tokio::test]
async fn gzip_encoded_limit_is_independent_even_when_decoded_file_is_empty() {
    let encoded = encoded(b"");
    for limit in [encoded.len() - 1, encoded.len()] {
        let limits = TransferLimits {
            file_upload_bytes: limit,
            ..TransferLimits::fixture()
        };
        let prepared = prepare_body(upload(encoded.clone()), true, Some(&limits))
            .await
            .unwrap();
        let gzip = prepared
            .extensions()
            .get::<gzip_upload::Progress>()
            .unwrap()
            .clone();
        let result = drain(prepared.into_body()).await;
        if limit < encoded.len() {
            assert!(result.is_err());
            assert_eq!(gzip.status(), Some(StatusCode::PAYLOAD_TOO_LARGE));
        } else {
            assert!(result.unwrap().is_empty());
            assert_eq!(gzip.status(), None);
        }
    }
}

#[tokio::test]
async fn gzip_output_clones_keep_both_admission_and_decoder_backpressure() {
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let mut request = upload(encoded(&vec![b'Z'; 40_000]));
    request.extensions_mut().insert(BodyWorkOwner::new(
        slots.clone().try_acquire_owned().unwrap(),
    ));
    let prepared = prepare_body(request, true, Some(&TransferLimits::fixture()))
        .await
        .unwrap();
    let mut body = prepared.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    let mut next = Box::pin(body.frame());
    std::future::poll_fn(|cx| {
        assert!(next.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(slots.available_permits(), 0);
    drop(copy);
    let bytes = timeout(Duration::from_secs(5), next)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap();
    drop(body);
    assert_eq!(slots.available_permits(), 0);
    drop(bytes);
    // Cancellation wakes the original blocking reader/consumer; wait for its
    // real guard drop, not an assumed completion after the HTTP body vanished.
    let permit = timeout(Duration::from_secs(5), slots.acquire())
        .await
        .unwrap()
        .unwrap();
    drop(permit);
}

struct PendingAfterHeader {
    header: Option<Bytes>,
    blocked: Option<oneshot::Sender<()>>,
}

#[tokio::test]
async fn decoded_gzip_progress_refreshes_original_upload_idle_clock_without_new_source_reads() {
    let prepared = prepare_body(
        upload(encoded(&vec![b'Z'; 40_000])),
        true,
        Some(&TransferLimits::fixture()),
    )
    .await
    .unwrap();
    let activity = prepared.extensions().get::<Activity>().unwrap().clone();
    let mut body = prepared.into_body();
    let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(first.len(), 16 * 1024);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6)).await;
    drop(first);
    let second = body.frame().await.unwrap().unwrap().into_data().unwrap();
    assert_eq!(second.len(), 16 * 1024);
    tokio::time::advance(Duration::from_secs(6)).await;
    let mut stalled = Box::pin(activity.stalled(Duration::from_secs(10)));
    std::future::poll_fn(|cx| {
        assert!(stalled.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    drop(stalled);
    tokio::time::resume();
    drop(second);
    assert_eq!(drain(body).await.unwrap(), vec![b'Z'; 40_000 - 32_768]);
}
impl hyper::body::Body for PendingAfterHeader {
    type Data = Bytes;
    type Error = io::Error;
    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        if let Some(header) = self.header.take() {
            return Poll::Ready(Some(Ok(Frame::data(header))));
        }
        if let Some(blocked) = self.blocked.take() {
            let _ = blocked.send(());
        }
        Poll::Pending
    }
}

#[tokio::test]
async fn cancelling_gzip_during_a_pending_source_read_wakes_the_original_worker_and_releases_its_slot()
 {
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let (blocked, observed) = oneshot::channel();
    let mut request = upload(Vec::new());
    *request.body_mut() = Body::new(PendingAfterHeader {
        header: Some(Bytes::copy_from_slice(&encoded(b"data")[..10])),
        blocked: Some(blocked),
    });
    request.headers_mut().remove(header::CONTENT_LENGTH);
    request.extensions_mut().insert(BodyWorkOwner::new(
        slots.clone().try_acquire_owned().unwrap(),
    ));
    let mut body = prepare_body(request, true, Some(&TransferLimits::fixture()))
        .await
        .unwrap()
        .into_body();
    let mut next = Box::pin(body.frame());
    std::future::poll_fn(|cx| {
        assert!(next.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    timeout(Duration::from_secs(5), observed)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(slots.available_permits(), 0);
    drop(next);
    drop(body);
    drop(
        timeout(Duration::from_secs(5), slots.acquire())
            .await
            .unwrap()
            .unwrap(),
    );
}

#[tokio::test]
async fn gzip_upload_crosses_real_http_before_success_and_never_forwards_encoding_twice() {
    let limits = TransferLimits::fixture();
    let expected = vec![b'G'; 65_792];
    let prepared = prepare_body(upload(encoded(&expected)), true, Some(&limits))
        .await
        .unwrap();
    let (client_io, server_io) = tokio::io::duplex(CHUNK);
    let upstream = tokio::spawn(async move {
        hyper::server::conn::http1::Builder::new()
            .serve_connection(
                TokioIo::new(server_io),
                service_fn(move |request: Request<hyper::body::Incoming>| {
                    let expected = expected.clone();
                    async move {
                        assert!(!request.headers().contains_key(header::CONTENT_ENCODING));
                        assert!(!request.headers().contains_key(header::CONTENT_LENGTH));
                        assert_eq!(
                            request.into_body().collect().await.unwrap().to_bytes(),
                            expected
                        );
                        Ok::<_, Infallible>(Response::new(Body::from("uploaded")))
                    }
                }),
            )
            .await
    });
    let (mut sender, connection) = http1::handshake(TokioIo::new(client_io)).await.unwrap();
    let driver = tokio::spawn(connection);
    let response = timeout(
        Duration::from_secs(5),
        forward_request(&mut sender, prepared, Some(&limits)),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "uploaded"
    );
    drop(sender);
    driver.await.unwrap().unwrap();
    upstream.await.unwrap().unwrap();
}
