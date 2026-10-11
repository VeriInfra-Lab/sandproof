//! HTTP framing over bounded in-memory transports; no VM or authorization claim.

use super::*;
use axum::body::Bytes;
use http_body_util::BodyExt;
use hyper::{
    body::{Frame, SizeHint},
    service::service_fn,
};
use std::{
    convert::Infallible,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream},
    sync::oneshot,
    time::timeout,
};

const CHUNK: usize = 64 * 1024;

mod streaming;

struct Chunks {
    remaining: usize,
    known_length: bool,
    polled_bytes: Arc<AtomicUsize>,
}

impl hyper::body::Body for Chunks {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        if self.remaining == 0 {
            return Poll::Ready(None);
        }
        let size = self.remaining.min(CHUNK);
        self.remaining -= size;
        self.polled_bytes.fetch_add(size, Ordering::SeqCst);
        Poll::Ready(Some(Ok(Frame::data(Bytes::from(vec![b'X'; size])))))
    }

    fn size_hint(&self) -> SizeHint {
        if self.known_length {
            SizeHint::with_exact(self.remaining as u64)
        } else {
            SizeHint::default()
        }
    }
}

fn request(size: usize, file: bool, known_length: bool) -> (Request<Body>, Arc<AtomicUsize>) {
    let polled_bytes = Arc::new(AtomicUsize::new(0));
    let body = Body::new(Chunks {
        remaining: size,
        known_length,
        polled_bytes: polled_bytes.clone(),
    });
    let request = Request::builder()
        .method(Method::POST)
        .uri(if file {
            "/files?path=/tmp/probe"
        } else {
            "/process.Process/Start"
        })
        .body(body)
        .unwrap();
    (request, polled_bytes)
}

#[tokio::test]
async fn unsupported_http_versions_are_rejected_before_body_or_forwarding() {
    for version in [
        Version::HTTP_09,
        Version::HTTP_10,
        Version::HTTP_2,
        Version::HTTP_3,
    ] {
        for file in [false, true] {
            let (mut request, polled) = request(1, file, false);
            *request.version_mut() = version;
            let rejection = prepare_body(request, file, None).await.unwrap_err();
            assert_eq!(rejection.status(), StatusCode::HTTP_VERSION_NOT_SUPPORTED);
            assert_eq!(polled.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn file_upload_limit_accepts_exact_boundary_for_known_and_streamed_bodies() {
    for known in [false, true] {
        for size in [
            FILE_REQUEST_LIMIT - 1,
            FILE_REQUEST_LIMIT,
            FILE_REQUEST_LIMIT + 1,
        ] {
            let (request, _) = request(size, true, known);
            let prepared = prepare_body(request, true, None).await;
            if size > FILE_REQUEST_LIMIT {
                assert_eq!(
                    prepared.unwrap_err().status(),
                    StatusCode::PAYLOAD_TOO_LARGE
                );
            } else {
                let bytes = prepared
                    .unwrap()
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes();
                assert_eq!(bytes.len(), size);
                assert!(bytes.iter().all(|byte| *byte == b'X'));
            }
        }
    }
}

#[tokio::test]
async fn process_input_is_lazy_and_enforces_total_across_chunks() {
    for known in [false, true] {
        for size in [
            PROCESS_REQUEST_LIMIT - 1,
            PROCESS_REQUEST_LIMIT,
            PROCESS_REQUEST_LIMIT + 1,
        ] {
            let (request, polled) = request(size, false, known);
            let request = prepare_body(request, false, None).await.unwrap();
            assert_eq!(
                polled.load(Ordering::SeqCst),
                0,
                "Start must remain streaming"
            );
            let received = request.into_body().collect().await;
            if size > PROCESS_REQUEST_LIMIT {
                assert!(received.is_err(), "oversized process input was accepted");
            } else {
                assert_eq!(received.unwrap().to_bytes().len(), size);
            }
        }
    }
}

#[tokio::test]
async fn process_input_overflow_is_not_forwarded_as_a_short_successful_request() {
    timeout(Duration::from_secs(10), async {
        for known in [false, true] {
            for size in [PROCESS_REQUEST_LIMIT, PROCESS_REQUEST_LIMIT + 1] {
                let (client_io, server_io) = tokio::io::duplex(CHUNK);
                let seen = Arc::new(Mutex::new(None));
                let starts = Arc::new(AtomicUsize::new(0));
                let observation = seen.clone();
                let calls = starts.clone();
                let upstream = tokio::spawn(async move {
                    hyper::server::conn::http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(server_io),
                            service_fn(move |request: Request<hyper::body::Incoming>| {
                                let seen = observation.clone();
                                calls.fetch_add(1, Ordering::SeqCst);
                                async move {
                                    let result = request.into_body().collect().await;
                                    let complete = result.is_ok();
                                    *seen.lock().unwrap() =
                                        Some(result.map(|body| body.to_bytes().len()));
                                    Ok::<_, Infallible>(
                                        Response::builder()
                                            .status(if complete {
                                                StatusCode::OK
                                            } else {
                                                StatusCode::BAD_REQUEST
                                            })
                                            .header(header::CONNECTION, "close")
                                            .body(Body::empty())
                                            .unwrap(),
                                    )
                                }
                            }),
                        )
                        .await
                });
                let (mut sender, connection) =
                    http1::handshake(TokioIo::new(client_io)).await.unwrap();
                let driver = tokio::spawn(connection);
                let (request, _) = request(size, false, known);
                let prepared = prepare_body(request, false, None).await.unwrap();
                let response = forward_request(&mut sender, prepared, None).await;
                let status = response.status();
                let _ = response.into_body().collect().await;
                drop(sender);
                let _ = driver.await.unwrap();
                let _ = upstream.await.unwrap();
                let received = seen
                    .lock()
                    .unwrap()
                    .take()
                    .expect("upstream observed the request");
                assert_eq!(starts.load(Ordering::SeqCst), 1);
                if size <= PROCESS_REQUEST_LIMIT {
                    assert_eq!(status, StatusCode::OK);
                    assert_eq!(received.unwrap(), size);
                } else {
                    assert_ne!(
                        status,
                        StatusCode::OK,
                        "input was silently truncated to the limit"
                    );
                    assert!(received.is_err());
                }
            }
        }
    })
    .await
    .expect("request transport did not finish");
}

#[derive(Clone, Copy)]
enum Framing {
    Fixed,
    Chunked,
    CloseDelimited,
}

#[derive(Clone, Copy)]
struct Reply {
    size: usize,
    framing: Framing,
    truncated: bool,
    gated: bool,
}

// Record the request before producing a response, like a command whose side
// effect has occurred but whose final result can still be lost in transport.
async fn synthetic_envd(
    stream: DuplexStream,
    reply: Reply,
    starts: Arc<AtomicUsize>,
    release: oneshot::Receiver<()>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut headers = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        headers.push_str(&line);
        if line == "\r\n" {
            break;
        }
        assert!(headers.len() < 8192);
    }
    assert!(
        [
            "/process.Process/Start",
            "/process.Process/Connect",
            "/process.Process/List"
        ]
        .iter()
        .any(|path| headers.starts_with(&format!("POST {path} HTTP/1.1\r\n")))
    );
    assert!(!headers.to_lowercase().contains("x-api-key"));
    starts.fetch_add(1, Ordering::SeqCst);
    let stream = reader.get_mut();
    let framing = match reply.framing {
        Framing::Chunked => "Transfer-Encoding: chunked\r\n".to_owned(),
        Framing::Fixed => format!(
            "Content-Length: {}\r\n",
            reply.size + usize::from(reply.truncated)
        ),
        Framing::CloseDelimited => String::new(),
    };
    let version = match reply.framing {
        Framing::CloseDelimited => "HTTP/1.0",
        Framing::Fixed | Framing::Chunked => "HTTP/1.1",
    };
    stream
        .write_all(format!("{version} 200 OK\r\n{framing}Connection: close\r\n\r\n").as_bytes())
        .await?;
    let bytes = vec![b'X'; CHUNK];
    let mut remaining = reply.size;
    let mut release = Some(release);
    while remaining > 0 {
        let size = remaining.min(CHUNK);
        if matches!(reply.framing, Framing::Chunked) {
            stream.write_all(format!("{size:x}\r\n").as_bytes()).await?;
        }
        stream.write_all(&bytes[..size]).await?;
        if matches!(reply.framing, Framing::Chunked) {
            stream.write_all(b"\r\n").await?;
        }
        remaining -= size;
        if reply.gated
            && let Some(release) = release.take()
        {
            release
                .await
                .expect("client must observe the first chunk before the rest is sent");
        }
    }
    if matches!(reply.framing, Framing::Chunked) && !reply.truncated {
        stream.write_all(b"0\r\n\r\n").await?;
    }
    stream.shutdown().await
}

struct Observed {
    status: StatusCode,
    bytes: usize,
    complete: bool,
}

async fn exchange(reply: Reply, starts: Arc<AtomicUsize>) -> Observed {
    exchange_at(reply, starts, "/process.Process/Start").await
}

async fn exchange_at(reply: Reply, starts: Arc<AtomicUsize>, path: &str) -> Observed {
    timeout(Duration::from_secs(10), async {
        let (gateway_io, envd_io) = tokio::io::duplex(CHUNK);
        let (release, gate) = oneshot::channel();
        let upstream = tokio::spawn(synthetic_envd(envd_io, reply, starts, gate));
        let (mut sender, connection) = http1::handshake(TokioIo::new(gateway_io)).await.unwrap();
        let upstream_driver = tokio::spawn(connection);
        let request = Request::builder()
            .method(Method::POST)
            .uri(path)
            .header("x-api-key", "must-not-reach-guest")
            .body(Body::empty())
            .unwrap();
        let limits = TransferLimits {
            rpc_response_bytes: RESPONSE_LIMIT,
            ..TransferLimits::fixture()
        };
        let response = forward_request(&mut sender, request, Some(&limits)).await;
        drop(sender);

        // Serialize the actual bounded gateway body through HTTP again. Merely
        // collecting Limited directly would miss Content-Length/framing bugs.
        let (client_io, server_io) = tokio::io::duplex(CHUNK);
        let response = Arc::new(Mutex::new(Some(response)));
        let gateway = tokio::spawn(async move {
            hyper::server::conn::http1::Builder::new()
                .serve_connection(
                    TokioIo::new(server_io),
                    service_fn(move |_| {
                        let response = response.lock().unwrap().take().expect("one client request");
                        std::future::ready(Ok::<_, Infallible>(response))
                    }),
                )
                .await
        });
        let (mut client, connection) = http1::handshake(TokioIo::new(client_io)).await.unwrap();
        let client_driver = tokio::spawn(connection);
        let response = client
            .send_request(Request::new(Body::empty()))
            .await
            .unwrap();
        let status = response.status();
        let mut body = response.into_body();
        let mut bytes = 0;
        let mut complete = true;
        let mut release = Some(release);
        while let Some(frame) = body.frame().await {
            let Ok(frame) = frame else {
                complete = false;
                break;
            };
            if let Some(data) = frame.data_ref() {
                assert!(data.iter().all(|byte| *byte == b'X'));
                bytes += data.len();
                if let Some(release) = release.take() {
                    let _ = release.send(());
                }
            }
        }
        drop(body);
        drop(client);
        // Transport failures are expected in negative cases; task panics or
        // drivers that remain alive are failures. No sleeps or abandoned tasks.
        let (a, b, c, d) = tokio::join!(upstream, upstream_driver, gateway, client_driver);
        let _ = a.unwrap();
        let _ = b.unwrap();
        let _ = c.unwrap();
        let _ = d.unwrap();
        Observed {
            status,
            bytes,
            complete,
        }
    })
    .await
    .expect("HTTP transport did not finish")
}

#[tokio::test]
async fn response_limit_is_exact_for_fixed_chunked_and_close_delimited_upstreams() {
    let starts = Arc::new(AtomicUsize::new(0));
    for framing in [Framing::Fixed, Framing::Chunked, Framing::CloseDelimited] {
        for size in [RESPONSE_LIMIT - 1, RESPONSE_LIMIT, RESPONSE_LIMIT + 1] {
            let before = starts.load(Ordering::SeqCst);
            let observed = exchange_at(
                Reply {
                    size,
                    framing,
                    truncated: false,
                    gated: false,
                },
                starts.clone(),
                "/process.Process/List",
            )
            .await;
            assert_eq!(observed.status, StatusCode::OK);
            assert_eq!(
                starts.load(Ordering::SeqCst),
                before + 1,
                "request was replayed"
            );
            assert!(observed.bytes <= RESPONSE_LIMIT);
            assert_eq!(observed.complete, size <= RESPONSE_LIMIT);
            if observed.complete {
                assert_eq!(observed.bytes, size);
            }
        }
    }
}

#[tokio::test]
async fn upstream_disconnect_is_not_a_success_or_replay_and_next_request_works() {
    let starts = Arc::new(AtomicUsize::new(0));
    for framing in [Framing::Fixed, Framing::Chunked] {
        let before = starts.load(Ordering::SeqCst);
        let broken = exchange(
            Reply {
                size: CHUNK * 2,
                framing,
                truncated: true,
                gated: true,
            },
            starts.clone(),
        )
        .await;
        assert_eq!(broken.status, StatusCode::OK);
        assert!(!broken.complete);
        assert!(broken.bytes > 0);
        assert_eq!(starts.load(Ordering::SeqCst), before + 1);
        let healthy = exchange(
            Reply {
                size: 17,
                framing,
                truncated: false,
                gated: false,
            },
            starts.clone(),
        )
        .await;
        assert!(healthy.complete);
        assert_eq!(healthy.bytes, 17);
        assert_eq!(starts.load(Ordering::SeqCst), before + 2);
    }
}

#[tokio::test]
async fn command_output_exceeds_former_cap_and_flows_before_upstream_finishes() {
    let starts = Arc::new(AtomicUsize::new(0));
    let reply = Reply {
        size: RESPONSE_LIMIT + 1,
        framing: Framing::Chunked,
        truncated: false,
        gated: true,
    };
    let large = exchange(reply, starts.clone()).await;
    assert!(large.complete);
    assert_eq!(large.bytes, reply.size);
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    let healthy = exchange(
        Reply {
            size: CHUNK * 2,
            ..reply
        },
        starts.clone(),
    )
    .await;
    assert!(healthy.complete);
    assert_eq!(healthy.bytes, CHUNK * 2);
    assert_eq!(starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn start_and_connect_output_remain_uncapped_for_all_supported_framing() {
    let calls = Arc::new(AtomicUsize::new(0));
    for path in ["/process.Process/Start", "/process.Process/Connect"] {
        for framing in [Framing::Fixed, Framing::Chunked, Framing::CloseDelimited] {
            let reply = Reply {
                size: RESPONSE_LIMIT + CHUNK,
                framing,
                truncated: false,
                gated: true,
            };
            let before = calls.load(Ordering::SeqCst);
            let observed = exchange_at(reply, calls.clone(), path).await;
            assert!(observed.complete);
            assert_eq!(observed.bytes, reply.size);
            assert_eq!(calls.load(Ordering::SeqCst), before + 1);
        }
    }
}
