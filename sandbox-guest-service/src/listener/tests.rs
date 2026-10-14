use super::*;
use bytes::Bytes;
use http_body_util::BodyExt;
use std::{
    convert::Infallible,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadBuf},
    net::TcpStream,
    sync::{Semaphore, oneshot},
};

fn limits() -> Limits {
    Limits {
        connections: 1,
        header_count: 8,
        buffer_bytes: 8192,
        header_timeout: Duration::from_secs(2),
        connection_timeout: Duration::from_secs(10),
    }
}

#[test]
fn invalid_transport_limits_are_rejected_before_accept() {
    for invalid in [
        Limits {
            connections: 0,
            ..limits()
        },
        Limits {
            header_count: 0,
            ..limits()
        },
        Limits {
            buffer_bytes: 8191,
            ..limits()
        },
        Limits {
            header_timeout: Duration::ZERO,
            ..limits()
        },
        Limits {
            connection_timeout: Duration::ZERO,
            ..limits()
        },
        Limits {
            header_timeout: Duration::from_secs(11),
            ..limits()
        },
        Limits {
            connection_timeout: Duration::MAX,
            ..limits()
        },
    ] {
        assert!(matches!(Listener::new(invalid), Err(InvalidLimits)));
    }
    assert!(Listener::new(limits()).is_ok());
}

async fn read_all(socket: &mut TcpStream) -> Vec<u8> {
    let mut bytes = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(3), socket.read_to_end(&mut bytes))
        .await
        .unwrap();
    if let Err(error) = result {
        assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
    }
    bytes
}

#[tokio::test]
async fn connection_capacity_refuses_without_dispatch_and_recovers_after_completion() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let handler = {
        let (entered, release, calls) = (entered.clone(), release.clone(), calls.clone());
        move |_: Request<Body>| {
            let (entered, release, calls) = (entered.clone(), release.clone(), calls.clone());
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                entered.add_permits(1);
                release.acquire().await.unwrap().forget();
                Ok::<_, Infallible>(Response::new(Body::from("ok")))
            }
        }
    };
    let server = tokio::spawn(
        Listener::new(limits())
            .unwrap()
            .serve(listener, handler, async {
                let _ = stopped.await;
            }),
    );
    let mut first = TcpStream::connect(address).await.unwrap();
    first
        .write_all(b"GET / HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    entered.acquire().await.unwrap().forget();
    let mut excess = TcpStream::connect(address).await.unwrap();
    assert!(read_all(&mut excess).await.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    release.add_permits(1);
    assert!(read_all(&mut first).await.ends_with(b"\r\n\r\nok"));
    let mut next = TcpStream::connect(address).await.unwrap();
    next.write_all(b"GET / HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    entered.acquire().await.unwrap().forget();
    release.add_permits(1);
    assert!(read_all(&mut next).await.ends_with(b"\r\n\r\nok"));
    stop.send(()).unwrap();
    let report = server.await.unwrap().unwrap();
    assert_eq!(
        report,
        Report {
            admitted: 2,
            refused: 1,
            completed: 2,
            peak_connections: 1,
            ..Report::default()
        }
    );
}

#[tokio::test]
async fn malformed_and_oversized_headers_never_reach_handler_and_do_not_poison_listener() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let server = tokio::spawn(
        Listener::new(Limits {
            header_count: 2,
            ..limits()
        })
        .unwrap()
        .serve(
            listener,
            move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
                async { Ok::<_, Infallible>(Response::new(Body::from("ok"))) }
            },
            async {
                let _ = stopped.await;
            },
        ),
    );
    for request in [
        b"GET / HTTP/1.1\r\nHost: local\r\nX-A: 1\r\nX-B: 2\r\n\r\n".to_vec(),
        [
            b"GET / HTTP/1.1\r\nHost: local\r\nX-A: ".as_slice(),
            &vec![b'a'; 9000],
            b"\r\n\r\n",
        ]
        .concat(),
        b"GET / HTTP/1.1\r\nHost: local\r\nContent-Length: bad\r\n\r\n".to_vec(),
    ] {
        let mut socket = TcpStream::connect(address).await.unwrap();
        let _ = socket.write_all(&request).await;
        let response = read_all(&mut socket).await;
        assert!(response.starts_with(b"HTTP/1.1 431") || response.starts_with(b"HTTP/1.1 400"));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    let mut socket = TcpStream::connect(address).await.unwrap();
    socket
        .write_all(b"GET / HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    assert!(read_all(&mut socket).await.ends_with(b"\r\n\r\nok"));
    stop.send(()).unwrap();
    let report = server.await.unwrap().unwrap();
    assert_eq!(report.failed, 3);
    assert_eq!(report.completed, 1);
    assert_eq!(report.refused, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn slow_response_consumer_cannot_retain_connection_or_payload_past_deadline() {
    struct Payload {
        bytes: Vec<u8>,
        _owner: Dropped,
    }
    impl AsRef<[u8]> for Payload {
        fn as_ref(&self) -> &[u8] {
            &self.bytes
        }
    }
    let (mut client, socket) = tokio::io::duplex(128);
    let (_stop, stopped) = watch::channel(false);
    let entered = Arc::new(Semaphore::new(0));
    let noticed = entered.clone();
    let dropped = Arc::new(AtomicUsize::new(0));
    let observed = dropped.clone();
    let task = tokio::spawn(connection(
        socket,
        move |_| {
            let body = Body::from(Bytes::from_owner(Payload {
                bytes: vec![b'x'; 32768],
                _owner: Dropped(observed.clone()),
            }));
            noticed.add_permits(1);
            async { Ok::<_, Infallible>(Response::new(body)) }
        },
        limits(),
        Instant::now() + limits().connection_timeout,
        stopped,
    ));
    client
        .write_all(b"GET / HTTP/1.1\r\nHost: local\r\n\r\n")
        .await
        .unwrap();
    entered.acquire().await.unwrap().forget();
    // The transport has room for only 128 bytes; no consumer drains it.
    assert_eq!(dropped.load(Ordering::SeqCst), 0);
    tokio::time::advance(limits().connection_timeout).await;
    assert_eq!(task.await.unwrap(), Outcome::Expired);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(response.len() <= 128);
}

struct Dropped(Arc<AtomicUsize>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn handler_panic_releases_connection_and_does_not_stop_future_admission() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(Listener::new(limits()).unwrap().serve(
        listener,
        |request| async move {
            assert_ne!(request.uri().path(), "/panic", "controlled handler panic");
            Ok::<_, Infallible>(Response::new(Body::from("recovered")))
        },
        async {
            let _ = stopped.await;
        },
    ));
    for path in ["/panic", "/healthy"] {
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let response = read_all(&mut socket).await;
        if path == "/panic" {
            assert!(response.is_empty());
        } else {
            assert!(response.ends_with(b"recovered"));
        }
    }
    stop.send(()).unwrap();
    let report = server.await.unwrap().unwrap();
    assert_eq!(report.failed, 1);
    assert_eq!(report.completed, 1);
    assert_eq!(report.refused, 0);
}

#[tokio::test]
async fn dropping_server_aborts_transport_but_does_not_release_separately_owned_work() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let entered = Arc::new(Semaphore::new(0));
    let noticed = entered.clone();
    let transport_dropped = Arc::new(AtomicUsize::new(0));
    let transport = transport_dropped.clone();
    let work_capacity = Arc::new(Semaphore::new(1));
    let slot = work_capacity.clone().acquire_owned().await.unwrap();
    let (release, gate) = oneshot::channel();
    let worker = tokio::spawn(async move {
        let _slot = slot;
        let _ = gate.await;
    });
    let server = tokio::spawn(Listener::new(limits()).unwrap().serve(
        listener,
        move |_| {
            let transport = Dropped(transport.clone());
            let noticed = noticed.clone();
            async move {
                let _transport = transport;
                noticed.add_permits(1);
                std::future::pending::<Result<Response, Infallible>>().await
            }
        },
        std::future::pending(),
    ));
    let mut socket = TcpStream::connect(address).await.unwrap();
    socket
        .write_all(b"GET / HTTP/1.1\r\nHost: local\r\n\r\n")
        .await
        .unwrap();
    entered.acquire().await.unwrap().forget();
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
    assert!(read_all(&mut socket).await.is_empty());
    assert_eq!(transport_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(work_capacity.available_permits(), 0);
    assert!(TcpStream::connect(address).await.is_err());
    release.send(()).unwrap();
    worker.await.unwrap();
    assert_eq!(work_capacity.available_permits(), 1);
}

#[tokio::test]
async fn typed_handler_failure_and_broken_response_close_without_complete_success() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let dropped = Arc::new(AtomicUsize::new(0));
    let observed = dropped.clone();
    let server = tokio::spawn(Listener::new(limits()).unwrap().serve(
        listener,
        move |request| {
            let observed = observed.clone();
            async move {
                if request.uri().path() == "/failure" {
                    return Err(Dropped(observed));
                }
                let body = Body::from_stream(futures_util::stream::iter([
                    Ok(Bytes::from_static(b"partial")),
                    Err(io::Error::other("private body failure")),
                ]));
                Ok(Response::builder()
                    .header("content-length", "100")
                    .body(body)
                    .unwrap())
            }
        },
        async {
            let _ = stopped.await;
        },
    ));
    for path in ["/failure", "/broken"] {
        let mut socket = TcpStream::connect(address).await.unwrap();
        socket
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let response = read_all(&mut socket).await;
        assert!(!String::from_utf8_lossy(&response).contains("private"));
        if path == "/failure" {
            assert!(response.is_empty());
        }
        // The declared 100-byte response can never have been delivered.
        let body = response
            .windows(4)
            .position(|s| s == b"\r\n\r\n")
            .map_or(0, |i| response.len() - i - 4);
        assert!(body < 100);
    }
    stop.send(()).unwrap();
    let report = server.await.unwrap().unwrap();
    assert_eq!(report.failed, 2);
    assert_eq!(report.completed, 0);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
}

// A deterministic observation of the first actual transport read, without
// sleeps, kernel timing assumptions or exposing a production test hook.
struct ReadObserved {
    socket: DuplexStream,
    read: Option<oneshot::Sender<()>>,
}
impl AsyncRead for ReadObserved {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let result = Pin::new(&mut self.socket).poll_read(cx, buf);
        if buf.filled().len() > before
            && let Some(read) = self.read.take()
        {
            let _ = read.send(());
        }
        result
    }
}
impl AsyncWrite for ReadObserved {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.socket).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(cx)
    }
}

#[tokio::test(start_paused = true)]
async fn slow_headers_expire_without_refreshing_on_partial_input() {
    let (mut client, socket) = tokio::io::duplex(1024);
    let (read, observed) = oneshot::channel();
    let (_stop, stopped) = watch::channel(false);
    let task = tokio::spawn(connection(
        ReadObserved {
            socket,
            read: Some(read),
        },
        |_| async {
            panic!("partial headers reached dispatch");
            #[allow(unreachable_code)]
            Ok::<_, Infallible>(Response::new(Body::empty()))
        },
        limits(),
        Instant::now() + limits().connection_timeout,
        stopped,
    ));
    client.write_all(b"GET / HTTP/1.1\r\nHost: ").await.unwrap();
    observed.await.unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    client.write_all(b"local").await.unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(task.await.unwrap(), Outcome::Failed);
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(!response.starts_with(b"HTTP/1.1 200"));
}

#[tokio::test(start_paused = true)]
async fn original_connection_deadline_covers_body_handler_response_and_keepalive() {
    for phase in ["body", "handler", "response", "keepalive"] {
        let configuration = Limits {
            header_timeout: limits().connection_timeout,
            ..limits()
        };
        let (mut client, socket) = tokio::io::duplex(4096);
        let (_stop, stopped) = watch::channel(false);
        let entered = Arc::new(Semaphore::new(0));
        let noticed = entered.clone();
        let dropped = Arc::new(AtomicUsize::new(0));
        let owned = dropped.clone();
        let task = tokio::spawn(connection(
            socket,
            move |mut request| {
                let noticed = noticed.clone();
                let owned = Dropped(owned.clone());
                async move {
                    noticed.add_permits(1);
                    if phase == "body" {
                        while let Some(frame) = request.body_mut().frame().await {
                            frame.unwrap();
                        }
                        panic!("incomplete body was accepted");
                    }
                    if phase == "handler" {
                        std::future::pending::<()>().await;
                    }
                    let body = if phase == "response" {
                        Body::from_stream(futures_util::stream::poll_fn(move |_| {
                            let _ = &owned;
                            Poll::Pending::<Option<Result<Bytes, io::Error>>>
                        }))
                    } else {
                        Body::from("ok")
                    };
                    Ok::<_, Infallible>(Response::new(body))
                }
            },
            configuration,
            Instant::now() + limits().connection_timeout,
            stopped,
        ));
        client
            .write_all(if phase == "body" {
                b"POST / HTTP/1.1\r\nHost: local\r\nContent-Length: 100\r\n\r\none"
            } else {
                b"GET / HTTP/1.1\r\nHost: local\r\n\r\n"
            })
            .await
            .unwrap();
        entered.acquire().await.unwrap().forget();
        tokio::time::advance(Duration::from_secs(9)).await;
        if phase == "body" {
            client.write_all(b"two").await.unwrap();
        }
        if phase == "keepalive" {
            client
                .write_all(b"GET /again HTTP/1.1\r\nHost: local\r\n\r\n")
                .await
                .unwrap();
            entered.acquire().await.unwrap().forget();
        }
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(task.await.unwrap(), Outcome::Expired, "{phase}");
        assert_eq!(
            dropped.load(Ordering::SeqCst),
            if phase == "keepalive" { 2 } else { 1 },
            "{phase}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn shutdown_drains_current_response_without_admitting_keepalive_request_or_resetting_deadline()
 {
    for complete in [true, false] {
        let (mut client, socket) = tokio::io::duplex(4096);
        let (stop, stopped) = watch::channel(false);
        let entered = Arc::new(Semaphore::new(0));
        let release = Arc::new(Semaphore::new(0));
        let (noticed, gate) = (entered.clone(), release.clone());
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let task = tokio::spawn(connection(
            socket,
            move |_| {
                let (noticed, gate) = (noticed.clone(), gate.clone());
                observed.fetch_add(1, Ordering::SeqCst);
                async move {
                    noticed.add_permits(1);
                    gate.acquire().await.unwrap().forget();
                    Ok::<_, Infallible>(Response::new(Body::from("finished")))
                }
            },
            limits(),
            Instant::now() + limits().connection_timeout,
            stopped,
        ));
        client
            .write_all(
                b"GET / HTTP/1.1\r\nHost: local\r\n\r\nGET /again HTTP/1.1\r\nHost: local\r\n\r\n",
            )
            .await
            .unwrap();
        entered.acquire().await.unwrap().forget();
        tokio::time::advance(Duration::from_secs(9)).await;
        stop.send_replace(true);
        if complete {
            release.add_permits(1);
        } else {
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        assert_eq!(
            task.await.unwrap(),
            if complete {
                Outcome::Completed
            } else {
                Outcome::Expired
            }
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        if complete {
            assert!(response.ends_with(b"finished"));
        }
    }
}
