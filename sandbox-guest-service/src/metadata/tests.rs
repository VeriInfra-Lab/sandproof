use super::*;
use sha2::{Digest, Sha512};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tower::ServiceExt;

fn limits() -> MetadataLimits {
    MetadataLimits {
        body_bytes: 1024,
        receive_bytes: 8192,
        timeout: Duration::from_secs(3),
    }
}

async fn request_headers(stream: &mut TcpStream) -> String {
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        assert!(headers.len() < 8192);
        headers.push(stream.read_u8().await.unwrap());
    }
    String::from_utf8(headers).unwrap()
}

async fn respond(stream: &mut TcpStream, status: u16, body: &[u8]) {
    stream
        .write_all(
            format!(
                "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stream.write_all(body).await.unwrap();
    stream.shutdown().await.unwrap();
}

#[tokio::test]
async fn real_mmds_exchange_initializes_and_authorizes_actual_http_routes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = MmdsClient {
        address: listener.local_addr().unwrap(),
        limits: limits(),
    };
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let header = request_headers(&mut stream).await;
        assert!(header.starts_with("PUT /latest/api/token HTTP/1.1\r\n"));
        assert!(header.contains("x-metadata-token-ttl-seconds: 60\r\n"));
        assert!(!header.contains("x-metadata-token:"));
        respond(&mut stream, 200, b"fixture-mmds-token").await;
        drop(stream);
        let (mut stream, _) = listener.accept().await.unwrap();
        let header = request_headers(&mut stream).await;
        assert!(header.starts_with("GET / HTTP/1.1\r\n"));
        assert!(header.contains("x-metadata-token: fixture-mmds-token\r\n"));
        assert!(header.contains("accept: application/json\r\n"));
        let body = serde_json::json!({"instanceID":"vm-fixture", "envID":"template-fixture", "accessTokenHash":format!("{:x}", Sha512::digest(b"guest-token")), "unused":"ignored metadata field"});
        respond(&mut stream, 200, &serde_json::to_vec(&body).unwrap()).await;
    });
    let session = crate::Session::new(
        client,
        crate::InitLimits {
            body_bytes: 1024,
            token_bytes: 128,
            state_bytes: 1024,
            env_entries: 16,
            in_flight: 1,
            timeout: Duration::from_secs(5),
        },
    )
    .unwrap();
    let response = crate::initialization_routes(session.clone()).oneshot(
        Request::post("/init").header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"accessToken":"guest-token","defaultUser":"root","defaultWorkdir":"/workspace","envVars":{"VALUE":"init"}}"#)).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(matches!(
        session.authorize(Some("wrong")).await,
        Err(ServiceError::Unauthenticated)
    ));
    let permit = session.authorize(Some("guest-token")).await.unwrap();
    assert_eq!(permit.defaults().workdir.as_deref(), Some("/workspace"));
    assert_eq!(permit.defaults().env["VALUE"], "init");
    assert_eq!(permit.defaults().env["E2B_SANDBOX_ID"], "vm-fixture");
    assert_eq!(permit.defaults().env["E2B_TEMPLATE_ID"], "template-fixture");
    server.await.unwrap();
}

#[tokio::test]
async fn token_errors_redirects_invalid_headers_and_truncation_do_not_fetch_metadata() {
    for mode in [
        "status",
        "redirect",
        "empty",
        "oversize",
        "injection",
        "truncated",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = MmdsClient {
            address: listener.local_addr().unwrap(),
            limits: limits(),
        };
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            request_headers(&mut stream).await;
            match mode {
                "status" => respond(&mut stream, 403, b"forbidden").await,
                "redirect" => {
                    stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                }
                "empty" => respond(&mut stream, 200, b"").await,
                "oversize" => respond(&mut stream, 200, &vec![b'x'; 1025]).await,
                "injection" => respond(&mut stream, 200, b"bad\r\nx-injected: yes").await,
                "truncated" => {
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\nx",
                        )
                        .await
                        .unwrap();
                }
                _ => unreachable!(),
            }
            drop(stream);
            listener
        });
        assert!(
            matches!(client.fetch().await, Err(ServiceError::Unavailable)),
            "{mode}"
        );
        let listener = server.await.unwrap();
        // A rejected token must never start the second MMDS request. Probe
        // accept without blocking rather than waiting for an arbitrary delay.
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(listener.poll_accept(cx)))
                .await
                .is_pending(),
            "unexpected second request: {mode}"
        );
    }
}

#[tokio::test]
async fn missing_or_malformed_metadata_never_becomes_anonymous_initialization() {
    let valid_hash = "ab".repeat(64);
    for body in [
        b"[]".to_vec(), b"{}".to_vec(),
        serde_json::to_vec(&serde_json::json!({"instanceID":"vm", "envID":"tpl", "accessTokenHash":""})).unwrap(),
        serde_json::to_vec(&serde_json::json!({"instanceID":"vm", "envID":"tpl", "accessTokenHash":"g".repeat(128)})).unwrap(),
        serde_json::to_vec(&serde_json::json!({"instanceID":"", "envID":"tpl", "accessTokenHash":valid_hash})).unwrap(),
        br#"{"instanceID":"vm","instanceID":"other","envID":"tpl","accessTokenHash":"bad"}"#.to_vec(),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = MmdsClient { address: listener.local_addr().unwrap(), limits: limits() };
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            request_headers(&mut stream).await;
            respond(&mut stream, 200, b"metadata-token").await;
            drop(stream);
            let (mut stream, _) = listener.accept().await.unwrap();
            request_headers(&mut stream).await;
            respond(&mut stream, 200, &body).await;
        });
        assert!(matches!(client.fetch().await, Err(ServiceError::Unavailable)));
        server.await.unwrap();
    }
}

#[tokio::test]
async fn cancelling_or_expiring_fetch_closes_original_socket_without_detached_driver() {
    for cancel in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = MmdsClient {
            address: listener.local_addr().unwrap(),
            limits: limits(),
        };
        let (seen, received) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            request_headers(&mut stream).await;
            seen.send(()).unwrap();
            assert_eq!(
                stream.read(&mut [0]).await.unwrap(),
                0,
                "client socket not closed"
            );
        });
        let task = tokio::spawn(async move { client.fetch().await });
        received.await.unwrap();
        if cancel {
            task.abort();
            match task.await {
                Err(error) => assert!(error.is_cancelled()),
                Ok(_) => panic!("cancelled metadata fetch returned a result"),
            }
        } else {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(4)).await;
            assert!(matches!(
                task.await.unwrap(),
                Err(ServiceError::Unavailable)
            ));
            tokio::time::resume();
        }
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}

#[test]
fn endpoint_and_budgets_are_fixed_before_network_use() {
    let client = MmdsClient::firecracker(limits()).unwrap();
    assert_eq!(client.address.to_string(), "169.254.169.254:80");
    for change in 0..4 {
        let mut limits = limits();
        match change {
            0 => limits.body_bytes = 0,
            1 => limits.receive_bytes = 8191,
            2 => limits.timeout = Duration::ZERO,
            _ => limits.receive_bytes = usize::MAX,
        }
        assert!(matches!(
            MmdsClient::firecracker(limits),
            Err(ServiceError::InvalidArgument)
        ));
    }
    assert_eq!(decode_hash("ff".repeat(64).as_bytes()).unwrap(), [255; 64]);
    assert!(decode_hash("é".repeat(64).as_bytes()).is_err());
}
