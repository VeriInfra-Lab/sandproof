use super::*;

struct BufferedWire {
    input: std::io::Cursor<Vec<u8>>,
}

impl AsyncRead for BufferedWire {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // A real HTTP caller waits for the response, rather than closing its
        // upload socket as soon as the declared request bytes are delivered.
        // EOF here would exercise Hyper's default half-close rejection instead.
        if self.input.position() == self.input.get_ref().len() as u64 {
            return Poll::Pending;
        }
        Pin::new(&mut self.input).poll_read(cx, buf)
    }
}

impl AsyncWrite for BufferedWire {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn coalesced_http_body_frames_do_not_exceed_the_original_transport_read_budget() {
    const PAYLOAD: usize = 131_072;
    for padding in [0, 31, 511, 4095] {
        let budget = Limits {
            buffer_bytes: 16384,
            ..limits()
        };
        let mut wire = format!(
            "POST /files HTTP/1.1\r\nHost: local\r\nConnection: close\r\nContent-Length: {PAYLOAD}\r\nX-Padding: {}\r\n\r\n",
            "p".repeat(padding),
        )
        .into_bytes();
        wire.extend(std::iter::repeat_n(b'x', PAYLOAD));
        let peak = Arc::new(AtomicUsize::new(0));
        let observed = peak.clone();
        let (_stop, stopped) = watch::channel(false);
        let result = connection(
            BufferedWire {
                input: std::io::Cursor::new(wire),
            },
            move |request| {
                let observed = observed.clone();
                async move {
                    let mut body = request.into_body();
                    let mut total = 0;
                    while let Some(frame) = body.frame().await {
                        let bytes = frame.unwrap().into_data().unwrap();
                        observed.fetch_max(bytes.len(), Ordering::SeqCst);
                        assert!(bytes.iter().all(|byte| *byte == b'x'));
                        total += bytes.len();
                        // Release the previous frame normally. Retaining every
                        // frame would mask BytesMut's unique-buffer reclaim.
                    }
                    assert_eq!(total, PAYLOAD);
                    Ok::<_, Infallible>(Response::new(Body::empty()))
                }
            },
            budget,
            Instant::now() + budget.connection_timeout,
            stopped,
        )
        .await;
        assert_eq!(result, Outcome::Completed);
        assert!(
            peak.load(Ordering::SeqCst) <= budget.buffer_bytes,
            "body frame exceeded original read budget: {}",
            peak.load(Ordering::SeqCst)
        );
    }
}

#[tokio::test]
async fn original_upload_guard_accepts_a_coalesced_body_without_larger_limits() {
    const PAYLOAD: usize = 131_072;
    let budget = Limits {
        buffer_bytes: 16384,
        ..limits()
    };
    let mut wire = format!(
        "POST /files HTTP/1.1\r\nHost: local\r\nConnection: close\r\nContent-Length: {PAYLOAD}\r\n\r\n"
    )
    .into_bytes();
    wire.extend(std::iter::repeat_n(b'x', PAYLOAD));
    let received = Arc::new(AtomicUsize::new(0));
    let observed = received.clone();
    let (_stop, stopped) = watch::channel(false);
    let result = connection(
        BufferedWire {
            input: std::io::Cursor::new(wire),
        },
        move |request| {
            let observed = observed.clone();
            async move {
                let mut body = crate::upload_body::Input::new(
                    request.into_body(),
                    Some(PAYLOAD as u64),
                    crate::upload_body::HttpBodyLimits {
                        total_bytes: PAYLOAD as u64,
                        frame_bytes: 16384,
                        frames: 4096,
                        trailer_bytes: 4096,
                        trailer_fields: 16,
                    },
                    Instant::now() + budget.connection_timeout,
                )?;
                while let Some(bytes) = body.next().await? {
                    assert!(bytes.iter().all(|byte| *byte == b'x'));
                    observed.fetch_add(bytes.len(), Ordering::SeqCst);
                }
                Ok::<_, crate::upload_body::GzipError>(Response::new(Body::empty()))
            }
        },
        budget,
        Instant::now() + budget.connection_timeout,
        stopped,
    )
    .await;
    // The original handler error closes the connection, which the gateway
    // maps to 502; it is not a genuine SDK/client execution deadline.
    assert_eq!(result, Outcome::Completed);
    assert_eq!(received.load(Ordering::SeqCst), PAYLOAD);
}
