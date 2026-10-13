#![cfg(target_os = "linux")]

use std::future::{Future, poll_fn};
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::Router;
use axum::body::{Body, Bytes, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::get;
use futures_util::Stream;
use sandproof_guest_service::{
    InitLimits, Metadata, MetadataSource, ServiceError, Session, initialization_routes,
};
use sha2::{Digest, Sha512};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tower::ServiceExt;

#[derive(Clone)]
struct Source(Arc<AtomicUsize>);

impl MetadataSource for Source {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Metadata {
            instance_id: "vm-fixture".into(),
            template_id: "template-fixture".into(),
            access_token_sha512: Sha512::digest(b"fixture-token").into(),
        })
    }
}

fn fixture() -> (Session<Source>, Router, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let session = Session::new(
        Source(Arc::clone(&calls)),
        InitLimits {
            body_bytes: 4096,
            token_bytes: 128,
            state_bytes: 1024,
            env_entries: 16,
            in_flight: 1,
            timeout: Duration::from_secs(5),
        },
    )
    .unwrap();
    (session.clone(), initialization_routes(session), calls)
}

fn init(body: impl Into<Body>) -> Request<Body> {
    Request::post("/init")
        .header("content-type", "application/json")
        .body(body.into())
        .unwrap()
}

const VALID: &str =
    r#"{"accessToken":"fixture-token","defaultUser":"root","envVars":{"VALUE":"fixture"}}"#;

#[tokio::test]
async fn health_and_init_have_distinct_authorization_meanings() {
    let (session, router, calls) = fixture();
    let response = router
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(response.headers().contains_key("x-sandproof-guest-version"));
    assert!(
        !response.headers().contains_key("x-envd-version"),
        "do not impersonate the Go envd binary version"
    );
    assert!(matches!(
        session.authorize(Some("fixture-token")).await,
        Err(ServiceError::Unauthenticated)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let response = router.clone().oneshot(init(VALID)).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        session
            .authorize(Some("fixture-token"))
            .await
            .unwrap()
            .defaults()
            .env["VALUE"],
        "fixture"
    );
    assert_eq!(
        router.oneshot(init(VALID)).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn malformed_oversize_wrong_credentials_and_unsupported_fields_are_visible_errors() {
    let (session, router, _) = fixture();
    for (body, status) in [
        ("[]".to_owned(), StatusCode::BAD_REQUEST),
        ("{}".to_owned(), StatusCode::UNAUTHORIZED),
        (
            r#"{"accessToken":"SECRET-FIXTURE-INPUT"}"#.to_owned(),
            StatusCode::UNAUTHORIZED,
        ),
        (
            r#"{"accessToken":"fixture-token","defaultUser":"USER-SECRET\u0000"}"#.to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        (
            r#"{"accessToken":"fixture-token","volumeMounts":[]}"#.to_owned(),
            StatusCode::BAD_REQUEST,
        ),
        (" ".repeat(4097), StatusCode::PAYLOAD_TOO_LARGE),
    ] {
        let response = router.clone().oneshot(init(body)).await.unwrap();
        assert_eq!(response.status(), status);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(!text.contains("SECRET") && !text.contains("fixture-token"));
        assert!(matches!(
            session.authorize(Some("fixture-token")).await,
            Err(ServiceError::Unauthenticated)
        ));
    }
    let request = Request::post("/init")
        .header("content-type", "text/plain")
        .body(Body::from(VALID))
        .unwrap();
    assert_eq!(
        router.clone().oneshot(request).await.unwrap().status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        router.oneshot(init(VALID)).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
}

struct SlowBody(Arc<AtomicUsize>);

impl Stream for SlowBody {
    type Item = Result<Bytes, io::Error>;
    fn poll_next(self: std::pin::Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

impl Drop for SlowBody {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test(start_paused = true)]
async fn slow_body_has_a_deadline_and_does_not_consume_health_or_leak_init_slots() {
    let (session, router, calls) = fixture();
    let another_router = initialization_routes(session);
    let dropped = Arc::new(AtomicUsize::new(0));
    let body = Body::from_stream(SlowBody(Arc::clone(&dropped)));
    let mut pending = Box::pin(router.clone().oneshot(init(body)));
    assert!(
        poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    assert_eq!(
        router.clone().oneshot(init(VALID)).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        router
            .clone()
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        another_router.oneshot(init(VALID)).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS,
        "router reconstruction must not duplicate session request capacity"
    );
    tokio::time::advance(Duration::from_secs(6)).await;
    assert_eq!(pending.await.unwrap().status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_eq!(
        router.oneshot(init(VALID)).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn cancelled_body_and_broken_transport_release_their_slots_without_authorization() {
    let (session, router, calls) = fixture();
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut pending = Box::pin(
        router
            .clone()
            .oneshot(init(Body::from_stream(SlowBody(Arc::clone(&dropped))))),
    );
    assert!(
        poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(pending);
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    let broken = Body::from_stream(futures_util::stream::iter([Err::<Bytes, _>(
        io::Error::other("PRIVATE-TRANSPORT-DETAIL"),
    )]));
    let response = router.clone().oneshot(init(broken)).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        !std::str::from_utf8(&to_bytes(response.into_body(), 1024).await.unwrap())
            .unwrap()
            .contains("PRIVATE")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        session.authorize(Some("fixture-token")).await,
        Err(ServiceError::Unauthenticated)
    ));
    assert_eq!(
        router.oneshot(init(VALID)).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn duplicate_or_nontext_access_headers_are_not_authorization() {
    let (session, router, _) = fixture();
    assert_eq!(
        router.oneshot(init(VALID)).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
    let mut headers = HeaderMap::new();
    assert!(matches!(
        session.authorize_headers(&headers).await,
        Err(ServiceError::Unauthenticated)
    ));
    headers.insert("x-access-token", "fixture-token".parse().unwrap());
    assert!(session.authorize_headers(&headers).await.is_ok());
    headers.append("x-access-token", "wrong".parse().unwrap());
    assert!(matches!(
        session.authorize_headers(&headers).await,
        Err(ServiceError::Unauthenticated)
    ));
    headers.clear();
    headers.insert(
        "x-access-token",
        axum::http::HeaderValue::from_bytes(b"\xff").unwrap(),
    );
    assert!(matches!(
        session.authorize_headers(&headers).await,
        Err(ServiceError::Unauthenticated)
    ));
}

#[tokio::test]
async fn real_http_init_enables_only_matching_instance_credential() {
    let (session, router, _) = fixture();
    let router = router.route(
        "/authorization-probe",
        get(move |headers: HeaderMap| {
            let session = session.clone();
            async move {
                match session.authorize_headers(&headers).await {
                    Ok(_permit) => StatusCode::NO_CONTENT,
                    Err(_) => StatusCode::UNAUTHORIZED,
                }
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = stopped.await;
            })
            .await
    });
    for (method, path, token, body, expected) in [
        ("GET", "/health", "", "", 204),
        ("GET", "/authorization-probe", "fixture-token", "", 401),
        ("POST", "/init", "", VALID, 204),
        ("GET", "/authorization-probe", "fixture-token", "", 204),
        ("GET", "/authorization-probe", "old-token", "", 401),
    ] {
        let mut stream = TcpStream::connect(address).await.unwrap();
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Type: application/json\r\nX-Access-Token: {token}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(
            std::str::from_utf8(&response)
                .unwrap()
                .starts_with(&format!("HTTP/1.1 {expected} "))
        );
    }
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
