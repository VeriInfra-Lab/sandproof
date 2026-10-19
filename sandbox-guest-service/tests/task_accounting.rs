//! A separate test process gives the production process-wide meter one scope.
//! The real listener and finite response encoder use the production wrappers;
//! harness tasks deliberately do not, so thread/task counts cannot masquerade
//! as the measured service-owned work.
use axum::{body::Body, http::Request};
use sandproof_guest_service::{
    listener::{Limits, Listener},
    task_accounting, upload_response,
};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Semaphore, oneshot},
    time::{Instant, timeout},
};

#[test]
fn listener_and_queued_encoder_share_process_peak_and_settle_after_shutdown() {
    assert_eq!(task_accounting::snapshot().total.live, 0);
    let rt = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        timeout(Duration::from_secs(10), exercise()).await.unwrap();
    });
    rt.shutdown_timeout(Duration::from_secs(5));
    let snapshot = task_accounting::snapshot();
    assert!(snapshot.exact);
    assert_eq!((snapshot.total.live, snapshot.total.peak), (0, 2));
    assert_eq!(snapshot.asynchronous.count.peak, 1);
    assert_eq!(snapshot.blocking.count.peak, 1);
    for role in snapshot
        .asynchronous
        .roles
        .into_iter()
        .chain(snapshot.blocking.roles)
    {
        assert_eq!(role.count.live, 0);
        assert_eq!(
            role.count.peak,
            usize::from(matches!(role.name, "connection" | "body_encode"))
        );
    }
}

async fn exercise() {
    let (entered, began) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let blocker = tokio::task::spawn_blocking(move || {
        entered.send(()).unwrap();
        gate.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    began.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(task_accounting::snapshot().total.live, 0);
    let socket = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let started = Arc::new(Semaphore::new(0));
    let called = started.clone();
    let server = tokio::spawn(
        Listener::new(Limits {
            connections: 1,
            header_count: 8,
            buffer_bytes: 8192,
            header_timeout: Duration::from_secs(2),
            connection_timeout: Duration::from_secs(10),
        })
        .unwrap()
        .serve(
            socket,
            move |_: Request<Body>| {
                let called = called.clone();
                async move {
                    // On this current-thread runtime, the handler proceeds to the
                    // encoder's pending await before the observer can wake and sample.
                    called.add_permits(1);
                    upload_response::success(
                        Vec::new(),
                        upload_response::Limits {
                            wire_bytes: 64,
                            chunk_bytes: 64,
                        },
                        Instant::now() + Duration::from_secs(10),
                        Arc::new(()),
                    )
                    .await
                }
            },
            async {
                stopped.await.unwrap();
            },
        ),
    );
    let mut client = TcpStream::connect(address).await.unwrap();
    client
        .write_all(b"GET /fixture HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    started.acquire().await.unwrap().forget();
    let snapshot = task_accounting::snapshot();
    assert!(snapshot.exact);
    assert_eq!(
        (
            snapshot.asynchronous.count.live,
            snapshot.blocking.count.live,
            snapshot.total.peak
        ),
        (1, 1, 2)
    );
    // A refused second connection never acquires a task or increases the peak.
    let mut refused = TcpStream::connect(address).await.unwrap();
    let mut reply = Vec::new();
    refused.read_to_end(&mut reply).await.unwrap();
    assert!(reply.is_empty());
    assert_eq!(task_accounting::snapshot().total.peak, 2);
    stop.send(()).unwrap();
    release.send(()).unwrap();
    blocker.await.unwrap();
    client.read_to_end(&mut reply).await.unwrap();
    assert!(reply.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(reply.ends_with(b"[]"));
    let report = server.await.unwrap().unwrap();
    assert_eq!(
        (report.admitted, report.refused, report.completed),
        (1, 1, 1)
    );
}
