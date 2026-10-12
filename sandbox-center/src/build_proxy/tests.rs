use super::*;
use sandproof_api::admission::RequestAdmissionLimits;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

const USER_KEY: &str = "build-proxy-test-user-key-0000000001";
const PRIVATE_KEY: &str = "build-proxy-test-private-key-000001";
fn proxy(address: SocketAddr) -> Proxy {
    Proxy {
        backend: Backend {
            address,
            key: PRIVATE_KEY.into(),
            timeout: Duration::from_secs(2),
            max_request_bytes: 64,
            max_response_bytes: 64,
        },
        credentials: vec![ApiCredential {
            tenant: "1".into(),
            secret: USER_KEY.into(),
        }]
        .into(),
        admission: RequestAdmission::new(RequestAdmissionLimits {
            max_pending_requests: 2,
            control_reserve: 1,
        })
        .unwrap(),
    }
}
fn request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/sandproof/template-builds")
        .header("x-api-key", USER_KEY)
        .header("content-type", "application/json")
        .header("content-length", "2")
        .body(Body::from("{}"))
        .unwrap()
}

#[tokio::test]
async fn protected_projection_discards_all_caller_authorities() {
    let proxy = proxy("127.0.0.1:1".parse().unwrap());
    let mut input = request();
    for (name, value) in [
        ("authorization", "Bearer public-key"),
        ("host", "evil.invalid"),
        ("x-forwarded-host", "evil.invalid"),
        (PRIVATE_BUILD_CREDENTIAL_HEADER, "forged"),
        (PRIVATE_BUILD_TENANT_HEADER, "99"),
        ("idempotency-key", "original-job"),
    ] {
        input
            .headers_mut()
            .insert(name, HeaderValue::from_static(value));
    }
    let output = proxy.request(input, "1").unwrap();
    assert_eq!(
        output.headers()[PRIVATE_BUILD_CREDENTIAL_HEADER],
        PRIVATE_KEY
    );
    assert!(output.headers()[PRIVATE_BUILD_CREDENTIAL_HEADER].is_sensitive());
    assert_eq!(output.headers()[PRIVATE_BUILD_TENANT_HEADER], "1");
    assert_eq!(output.headers()[header::HOST], "localhost");
    assert_eq!(output.headers()["idempotency-key"], "original-job");
    for name in ["authorization", "x-api-key", "x-forwarded-host"] {
        assert!(!output.headers().contains_key(name));
    }
    assert_eq!(to_bytes(output.into_body(), 64).await.unwrap(), "{}");
    let oversized = Request::builder().body(Body::from(vec![0; 65])).unwrap();
    assert!(
        to_bytes(proxy.request(oversized, "1").unwrap().into_body(), 128)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn authentication_and_full_budget_reject_before_backend_io() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = proxy(listener.local_addr().unwrap());
    let ordinary = proxy.admission.try_acquire(RequestClass::Ordinary).unwrap();
    assert_eq!(
        proxy.handle(request()).await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    drop(ordinary);
    let mut unauthorized = request();
    unauthorized.headers_mut().remove("x-api-key");
    assert_eq!(
        proxy.handle(unauthorized).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    let ordinary = proxy.admission.try_acquire(RequestClass::Ordinary).unwrap();
    let mut close = request();
    *close.uri_mut() = "/sandproof/template-build-windows/close".parse().unwrap();
    close.headers_mut().remove("x-api-key");
    assert_eq!(proxy.handle(close).await.status(), StatusCode::UNAUTHORIZED);
    drop(ordinary);
}

async fn read_headers(tcp: &mut tokio::net::TcpStream) -> Vec<u8> {
    let mut headers = vec![];
    while !headers.windows(4).any(|value| value == b"\r\n\r\n") {
        let mut byte = [0];
        tcp.read_exact(&mut byte).await.unwrap();
        headers.push(byte[0]);
        assert!(headers.len() <= 8192);
    }
    headers
}

#[tokio::test]
async fn original_reply_and_private_owner_cross_only_the_fixed_backend() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = proxy(listener.local_addr().unwrap());
    let backend = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        let headers = String::from_utf8(read_headers(&mut tcp).await).unwrap();
        assert!(headers.contains(&format!(
            "{PRIVATE_BUILD_CREDENTIAL_HEADER}: {PRIVATE_KEY}\r\n"
        )));
        assert!(headers.contains(&format!("{PRIVATE_BUILD_TENANT_HEADER}: 1\r\n")));
        assert!(!headers.contains(USER_KEY));
        let mut body = [0; 2];
        tcp.read_exact(&mut body).await.unwrap();
        assert_eq!(body, *b"{}");
        tcp.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 12\r\nConnection: close\r\nX-Secret: forbidden\r\n\r\n{\"job\":\"j1\"}").await.unwrap();
        tcp.shutdown().await.unwrap();
    });
    let reply = proxy.handle(request()).await;
    assert_eq!(reply.status(), StatusCode::ACCEPTED);
    assert!(!reply.headers().contains_key("x-secret"));
    assert_eq!(
        to_bytes(reply.into_body(), 64).await.unwrap(),
        "{\"job\":\"j1\"}"
    );
    backend.await.unwrap();
}

#[tokio::test]
async fn total_deadline_holds_permit_until_cancelled_connection_is_dropped() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut proxy = proxy(listener.local_addr().unwrap());
    proxy.backend.timeout = Duration::from_millis(100);
    let proxy = Arc::new(proxy);
    let (observed, ready) = oneshot::channel();
    let backend = tokio::spawn(async move {
        let (mut tcp, _) = listener.accept().await.unwrap();
        read_headers(&mut tcp).await;
        let mut body = [0; 2];
        tcp.read_exact(&mut body).await.unwrap();
        observed.send(()).unwrap();
        // No sleep: the complete response is deliberately withheld. Deadline
        // must drop the owned connection instead of detaching its driver.
        let mut byte = [0];
        assert_eq!(tcp.read(&mut byte).await.unwrap(), 0);
    });
    let active = Arc::clone(&proxy);
    let request = tokio::spawn(async move { active.handle(request()).await });
    ready.await.unwrap();
    assert!(
        proxy
            .admission
            .try_acquire(RequestClass::Ordinary)
            .is_none()
    );
    assert!(proxy.admission.try_acquire(RequestClass::Control).is_some());
    assert_eq!(
        request.await.unwrap().status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    backend.await.unwrap();
    assert!(
        proxy
            .admission
            .try_acquire(RequestClass::Ordinary)
            .is_some()
    );
}
