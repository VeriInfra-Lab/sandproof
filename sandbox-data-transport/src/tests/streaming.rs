use super::*;
use std::future::Future;

struct GatedChunks {
    body: Chunks,
    first: bool,
    release: oneshot::Receiver<()>,
}

impl hyper::body::Body for GatedChunks {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        if !self.first && self.body.remaining > 0 {
            match Pin::new(&mut self.release).poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(result) => result.unwrap(),
            }
            // The gate is used once; later frames can proceed immediately.
            self.first = true;
        }
        let was_first = self.body.polled_bytes.load(Ordering::SeqCst) == 0;
        let frame = Pin::new(&mut self.body).poll_frame(cx);
        if was_first {
            self.first = false;
        }
        frame
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

#[tokio::test]
async fn large_upload_streams_to_upstream_before_client_finishes() {
    timeout(Duration::from_secs(10), async {
        let limits = TransferLimits::fixture();
        for known_length in [false, true] {
            let (release, gate) = oneshot::channel();
            let polled = Arc::new(AtomicUsize::new(0));
            let request = Request::builder()
                .method(Method::POST)
                .uri("/files?path=/tmp/large.bin")
                .body(Body::new(GatedChunks {
                    body: Chunks {
                        remaining: limits.file_upload_bytes,
                        known_length,
                        polled_bytes: polled.clone(),
                    },
                    first: true,
                    release: gate,
                }))
                .unwrap();
            let request = prepare_body(request, true, Some(&limits)).await.unwrap();
            assert_eq!(polled.load(Ordering::SeqCst), 0, "upload was buffered");
            let (client_io, server_io) = tokio::io::duplex(CHUNK);
            let release = Arc::new(Mutex::new(Some(release)));
            let expected = limits.file_upload_bytes;
            let upstream = tokio::spawn(async move {
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(server_io),
                        service_fn(move |request: Request<hyper::body::Incoming>| {
                            let release = release.lock().unwrap().take().unwrap();
                            async move {
                                assert_eq!(
                                    request.headers().contains_key(header::CONTENT_LENGTH),
                                    known_length
                                );
                                assert_eq!(request.headers()[header::ACCEPT_ENCODING], "identity");
                                let mut body = request.into_body();
                                let first =
                                    body.frame().await.unwrap().unwrap().into_data().unwrap();
                                let mut received = first.len();
                                // A whole-file buffering gateway would deadlock here.
                                release.send(()).unwrap();
                                while let Some(frame) = body.frame().await {
                                    let bytes = frame.unwrap().into_data().unwrap();
                                    assert!(bytes.iter().all(|byte| *byte == b'X'));
                                    received += bytes.len();
                                }
                                assert_eq!(received, expected);
                                Ok::<_, Infallible>(Response::new(Body::empty()))
                            }
                        }),
                    )
                    .await
            });
            let (mut sender, connection) = http1::handshake(TokioIo::new(client_io)).await.unwrap();
            let driver = tokio::spawn(connection);
            let response = forward_request(&mut sender, request, Some(&limits)).await;
            assert_eq!(response.status(), StatusCode::OK);
            response.into_body().collect().await.unwrap();
            drop(sender);
            driver.await.unwrap().unwrap();
            upstream.await.unwrap().unwrap();
            assert_eq!(polled.load(Ordering::SeqCst), expected);
        }
    })
    .await
    .expect("streaming upload stalled at the producer/consumer barrier");
}

#[tokio::test]
async fn streaming_upload_limits_cover_empty_exact_and_excessive_known_and_unknown_lengths() {
    let limits = TransferLimits {
        file_upload_bytes: CHUNK * 2,
        ..TransferLimits::fixture()
    };
    for known in [false, true] {
        for size in [
            0,
            limits.file_upload_bytes - 1,
            limits.file_upload_bytes,
            limits.file_upload_bytes + 1,
        ] {
            let (request, polled) = request(size, true, known);
            let prepared = prepare_body(request, true, Some(&limits)).await;
            if known && size > limits.file_upload_bytes {
                assert_eq!(
                    prepared.unwrap_err().status(),
                    StatusCode::PAYLOAD_TOO_LARGE
                );
                assert_eq!(polled.load(Ordering::SeqCst), 0);
                continue;
            }
            let prepared = prepared.unwrap();
            let progress = prepared
                .extensions()
                .get::<UploadProgress>()
                .unwrap()
                .clone();
            assert_eq!(polled.load(Ordering::SeqCst), 0);
            let result = prepared.into_body().collect().await;
            if size > limits.file_upload_bytes {
                assert!(result.is_err());
                assert!(!progress.complete());
            } else {
                assert_eq!(result.unwrap().to_bytes().len(), size);
                assert!(progress.complete());
            }
        }
    }
}

#[tokio::test]
async fn early_upload_success_is_not_accepted_as_complete() {
    timeout(Duration::from_secs(5), async {
        let limits = TransferLimits::fixture();
        for known in [false, true] {
            let (request, _) = request(limits.file_upload_bytes, true, known);
            let request = prepare_body(request, true, Some(&limits)).await.unwrap();
            let (client_io, server_io) = tokio::io::duplex(CHUNK);
            let upstream = tokio::spawn(async move {
                hyper::server::conn::http1::Builder::new()
                    .serve_connection(
                        TokioIo::new(server_io),
                        service_fn(|_request: Request<hyper::body::Incoming>| async {
                            // A final 200 before reading the file cannot validate it.
                            Ok::<_, Infallible>(Response::new(Body::empty()))
                        }),
                    )
                    .await
            });
            let (mut sender, connection) = http1::handshake(TokioIo::new(client_io)).await.unwrap();
            let driver = tokio::spawn(connection);
            let response = forward_request(&mut sender, request, Some(&limits)).await;
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
            drop(response);
            drop(sender);
            let _ = driver.await.unwrap();
            let _ = upstream.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn compressed_rpc_and_unsupported_file_encodings_are_rejected_before_reading() {
    let limits = TransferLimits::fixture();
    for header in [
        "content-encoding",
        "connect-content-encoding",
        "grpc-encoding",
    ] {
        let (mut request, polled) = request(10, true, false);
        if header == "content-encoding" {
            *request.uri_mut() = "/filesystem.Filesystem/Stat".parse().unwrap();
        }
        request
            .headers_mut()
            .insert(header, "gzip".parse().unwrap());
        assert_eq!(
            prepare_body(request, true, Some(&limits))
                .await
                .unwrap_err()
                .status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(polled.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn transfer_configuration_keeps_buffered_rpc_budget_separate() {
    let mut limits = TransferLimits::fixture();
    assert!(limits.validate().is_ok());
    limits.file_upload_bytes = 0;
    assert!(limits.validate().is_err());
    limits.file_upload_bytes = usize::MAX;
    assert!(limits.validate().is_ok());
    limits.rpc_request_bytes = FILE_REQUEST_LIMIT + 1;
    assert!(limits.validate().is_err());
}
