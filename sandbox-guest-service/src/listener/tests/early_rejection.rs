use super::*;

// The client waits for the response's write-half EOF before finishing its
// upload. The production connection must keep its read half alive meanwhile.
// A duplex stream makes the old BrokenPipe deterministic rather than depending
// on TCP buffering or scheduler delays.
#[tokio::test]
async fn rejected_request_can_finish_upload_after_response_eof() {
    let (mut client, server) = tokio::io::duplex(4096);
    let (_stop, stopped) = watch::channel(false);
    let task = tokio::spawn(connection(
        server,
        |_: Request<Body>| async {
            Ok::<_, Infallible>(Response::new(Body::from("original rejection")))
        },
        limits(),
        Instant::now() + Duration::from_secs(10),
        stopped,
    ));
    client
        .write_all(
            b"POST / HTTP/1.1\r\nHost: local\r\nConnection: close\r\nContent-Length: 4\r\n\r\n",
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.ends_with(b"original rejection"));
    // Do not accept a correct response object as evidence that the sender can
    // complete its upload: a real SDK may still be writing before decoding it.
    client.write_all(b"body").await.unwrap();
    client.shutdown().await.unwrap();
    assert_eq!(task.await.unwrap(), Outcome::Completed);
}

async fn late_upload(headers: &[u8], initial: &[u8], remaining: &[u8]) {
    let (mut client, server) = tokio::io::duplex(4096);
    let (_stop, stopped) = watch::channel(false);
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let task = tokio::spawn(connection(
        server,
        move |_: Request<Body>| {
            observed.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, Infallible>(Response::new(Body::from("original rejection"))) }
        },
        limits(),
        Instant::now() + Duration::from_secs(10),
        stopped,
    ));
    client.write_all(headers).await.unwrap();
    client.write_all(initial).await.unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(2), client.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.ends_with(b"original rejection"));
    assert!(String::from_utf8_lossy(&response).contains("connection: close"));
    assert!(!String::from_utf8_lossy(&response).contains("100 Continue"));
    client.write_all(remaining).await.unwrap();
    client.shutdown().await.unwrap();
    assert_eq!(task.await.unwrap(), Outcome::Completed);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn partial_fixed_body_and_pipelined_bytes_are_discarded_without_dispatch() {
    late_upload(
        b"POST / HTTP/1.1\r\nHost: local\r\nContent-Length: 5\r\n\r\n",
        b"he",
        b"lloGET /must-not-run HTTP/1.1\r\nHost: local\r\n\r\n",
    )
    .await;
}

#[tokio::test]
async fn partial_chunked_body_and_late_terminator_are_discarded() {
    late_upload(
        b"POST / HTTP/1.1\r\nHost: local\r\nTransfer-Encoding: chunked\r\n\r\n",
        b"5\r\nhe",
        b"llo\r\n0\r\n\r\n",
    )
    .await;
}

#[tokio::test]
async fn rejected_expect_continue_does_not_ask_for_a_body() {
    late_upload(
        b"POST / HTTP/1.1\r\nHost: local\r\nContent-Length: 4\r\nExpect: 100-continue\r\n\r\n",
        b"",
        b"",
    )
    .await;
}

async fn stalled_upload(deadline: Duration, expected: Outcome) {
    let (mut client, server) = tokio::io::duplex(4096);
    let (_stop, stopped) = watch::channel(false);
    let task = tokio::spawn(connection(
        server,
        |_: Request<Body>| async { Ok::<_, Infallible>(Response::new(Body::from("refused"))) },
        limits(),
        Instant::now() + deadline,
        stopped,
    ));
    client
        .write_all(b"POST / HTTP/1.1\r\nHost: local\r\nContent-Length: 4\r\n\r\n")
        .await
        .unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(response.ends_with(b"refused"));
    assert!(!task.is_finished());
    tokio::time::advance(deadline.min(limits().header_timeout)).await;
    assert_eq!(task.await.unwrap(), expected);
}

#[tokio::test(start_paused = true)]
async fn cleanup_stall_is_bounded_by_header_budget() {
    stalled_upload(Duration::from_secs(10), Outcome::Failed).await;
}

#[tokio::test(start_paused = true)]
async fn cleanup_never_extends_original_connection_deadline() {
    stalled_upload(Duration::from_secs(1), Outcome::Expired).await;
}

struct EndlessInput(Arc<AtomicUsize>);
impl AsyncRead for EndlessInput {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let count = buffer.remaining().min(64);
        buffer.put_slice(&[b'x'; 64][..count]);
        self.0.fetch_add(count, Ordering::SeqCst);
        Poll::Ready(Ok(()))
    }
}
impl AsyncWrite for EndlessInput {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn cleanup_never_reads_beyond_transport_byte_budget() {
    let read = Arc::new(AtomicUsize::new(0));
    let mut socket = rejection::ClosingIo::new(
        EndlessInput(Arc::clone(&read)),
        Arc::new(std::sync::atomic::AtomicBool::new(true)),
        limits(),
        Instant::now() + Duration::from_secs(10),
    );
    let error = socket.shutdown().await.unwrap_err();
    assert_eq!(error.to_string(), "rejected request cleanup byte limit");
    assert_eq!(read.load(Ordering::SeqCst), limits().buffer_bytes);
}

#[tokio::test]
async fn completed_request_does_not_enter_cleanup() {
    let read = Arc::new(AtomicUsize::new(0));
    let mut socket = rejection::ClosingIo::new(
        EndlessInput(Arc::clone(&read)),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        limits(),
        Instant::now() + Duration::from_secs(10),
    );
    socket.shutdown().await.unwrap();
    assert_eq!(read.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn closing_rejected_connection_keeps_its_original_admission_slot() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    let server = tokio::spawn(Listener::new(limits()).unwrap().serve(
        listener,
        move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, Infallible>(Response::new(Body::from("refused"))) }
        },
        async {
            let _ = stopped.await;
        },
    ));
    let mut first = TcpStream::connect(address).await.unwrap();
    first
        .write_all(b"POST / HTTP/1.1\r\nHost: local\r\nContent-Length: 4\r\n\r\n")
        .await
        .unwrap();
    let mut reply = Vec::new();
    first.read_to_end(&mut reply).await.unwrap();
    assert!(reply.ends_with(b"refused"));
    let mut excess = TcpStream::connect(address).await.unwrap();
    assert!(read_all(&mut excess).await.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    first.write_all(b"body").await.unwrap();
    first.shutdown().await.unwrap();
    stop.send(()).unwrap();
    let report = server.await.unwrap().unwrap();
    assert_eq!(report.admitted, 1);
    assert_eq!(report.refused, 1);
    assert_eq!(report.completed, 1);
    assert_eq!(report.failed + report.expired, 0);
}
