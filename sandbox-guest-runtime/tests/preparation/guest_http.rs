//! One assembly over original admission/worker/file components and HTTP/1.
use super::*;
use sandproof_guest_runtime::file_download::date::Utc;
use sandproof_guest_service::{
    file_http::FileHttp,
    file_requests::FileRequests,
    file_rpc_http::FileRpcHttp,
    file_rpc_requests::FileRpcRequests,
    file_transfer::FileStreams,
    guest_http::{Components, Failure as GuestFailure, GuestHttp, SessionMismatch},
    listener,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::{net::TcpListener, sync::oneshot};
#[path = "guest_http/configuration.rs"]
mod configuration;
#[path = "guest_http/transport.rs"]
mod transport;
#[path = "guest_http/wire.rs"]
mod wire;
use configuration::parts;

fn at(method: Method, path: &str, body: impl Into<Body>) -> HttpRequest<Body> {
    HttpRequest::builder()
        .method(method)
        .uri(path)
        .header("x-access-token", "token")
        .header("content-type", "application/json")
        .body(body.into())
        .unwrap()
}
fn init(token: &str) -> HttpRequest<Body> {
    at(
        Method::POST,
        "/init",
        json!({"accessToken":token,"defaultUser":"fixture","defaultWorkdir":"."}).to_string(),
    )
}
fn probes() -> [HttpRequest<Body>; 3] {
    [
        request("List", &json!({})),
        at(
            Method::POST,
            "/filesystem.Filesystem/Stat",
            r#"{"path":"data"}"#,
        ),
        at(Method::GET, "/files?path=data", Body::empty()),
    ]
}
async fn collected(response: axum::response::Response) -> Vec<u8> {
    let mut stream = response.into_body().into_data_stream();
    let mut result = Vec::new();
    while let Some(chunk) = timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap()
    {
        result.extend_from_slice(&chunk.unwrap()); // Drop before polling again.
        assert!(result.len() <= 8192);
    }
    result
}

#[tokio::test]
async fn guest_rejects_each_distinct_session_even_with_identical_token_and_metadata() {
    for selected in 0..3 {
        let fixture = fixture().await;
        let other = super::fixture().await;
        let registry = registry();
        let mut original = parts(&fixture, &registry, &fixture.session);
        let replacement = parts(&other, &registry, &other.session);
        match selected {
            0 => original.components.commands = replacement.components.commands,
            1 => original.components.files = replacement.components.files,
            _ => original.components.filesystem = replacement.components.filesystem,
        }
        assert!(matches!(
            GuestHttp::new(fixture.session.clone(), original.components),
            Err(SessionMismatch)
        ));
        assert!(original.pools.ordinary.admit().is_ok());
        assert!(original.pools.control.admit().is_ok());
        assert!(original.pools.files.admit().is_ok());
        assert!(original.pools.rpc.admit().is_ok());
        assert_eq!(original.pools.streams.available_slots(), 1);
        assert!(fixture.owners.reserve().is_ok());
        fixture.assert_no_effect();
        assert_eq!(fixture.gate_count(), 0);
    }
}

struct Rotating(Arc<AtomicBool>);
impl MetadataSource for Rotating {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        Ok(Metadata {
            instance_id: "vm".into(),
            template_id: "template".into(),
            access_token_sha512: Sha512::digest(if self.0.load(Ordering::SeqCst) {
                b"new-token".as_slice()
            } else {
                b"token".as_slice()
            })
            .into(),
        })
    }
}
#[tokio::test]
async fn guest_initialization_and_token_rotation_reach_all_three_original_interfaces() {
    let fixture = fixture().await;
    std::fs::write(fixture.directory.path().join("data"), b"content").unwrap();
    let rotated = Arc::new(AtomicBool::new(false));
    let session = Session::new(Rotating(rotated.clone()), fixture.session.limits()).unwrap();
    let components = parts(&fixture, &registry(), &session);
    let guest = GuestHttp::new(session, components.components).unwrap();
    for probe in probes() {
        let reply = guest.handle(probe).await.unwrap();
        assert_eq!(reply.status(), StatusCode::UNAUTHORIZED);
        collected(reply).await;
    }
    for (token, old) in [("token", None), ("new-token", Some("token"))] {
        if old.is_some() {
            rotated.store(true, Ordering::SeqCst);
        }
        assert_eq!(
            guest.clone().handle(init(token)).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            guest.handle(init("wrong")).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        for mut probe in probes() {
            if let Some(old) = old {
                probe
                    .headers_mut()
                    .insert("x-access-token", old.parse().unwrap());
                let reply = guest.handle(probe).await.unwrap();
                assert_eq!(reply.status(), StatusCode::UNAUTHORIZED);
                collected(reply).await;
            }
        }
        for mut probe in probes() {
            probe
                .headers_mut()
                .insert("x-access-token", token.parse().unwrap());
            let reply = guest.clone().handle(probe).await.unwrap();
            assert_eq!(reply.status(), StatusCode::OK);
            collected(reply).await;
        }
    }
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
}

#[tokio::test]
async fn guest_preserves_response_owners_health_availability_and_initialization_order() {
    let fixture = fixture().await;
    std::fs::write(fixture.directory.path().join("data"), b"content").unwrap();
    let components = parts(&fixture, &registry(), &fixture.session);
    let pools = components.pools;
    let guest = GuestHttp::new(fixture.session.clone(), components.components).unwrap();
    let mut held = Vec::new();
    for probe in probes() {
        held.push(bytes(guest.handle(probe).await.unwrap()).await);
    }
    full(&pools.ordinary);
    assert!(pools.files.admit().is_err());
    assert!(pools.rpc.admit().is_err());
    // Download bytes retain their original authorization through the last
    // owner. Initialization has its own admission slot but must still wait for
    // that authorization; health is independent of both slots and auth locks.
    let mut initializing = Box::pin(guest.handle(init("token")));
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(initializing.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    let health = guest
        .handle(at(Method::GET, "/health", Body::empty()))
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::NO_CONTENT);
    assert_eq!(health.headers()["cache-control"], "no-store");
    assert_eq!(
        health.headers()["x-sandproof-guest-version"],
        env!("CARGO_PKG_VERSION")
    );
    let last = held.clone();
    drop(held);
    full(&pools.ordinary);
    assert!(pools.files.admit().is_err());
    assert!(pools.rpc.admit().is_err());
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(initializing.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    drop(last);
    assert_eq!(
        timeout(Duration::from_secs(5), initializing)
            .await
            .unwrap()
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert!(pools.ordinary.admit().is_ok());
    timeout(Duration::from_secs(5), async {
        while pools.files.admit().is_err() || pools.rpc.admit().is_err() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn guest_preserves_typed_unmapped_failures_without_polling_or_replaying_requests() {
    let fixture = fixture().await;
    let components = parts(&fixture, &registry(), &fixture.session);
    let pools = components.pools;
    let guest = GuestHttp::new(fixture.session.clone(), components.components).unwrap();
    for path in [
        "/process.Process/List/",
        "/filesystem.Filesystem/Stat/",
        "/files/",
        "/unknown-secret",
    ] {
        let mut probe = at(Method::POST, path, Body::empty());
        unread(&mut probe);
        let failure = guest.handle(probe).await.unwrap_err();
        assert_eq!(format!("{failure:?}"), "GuestHttpFailure { .. }");
        match &failure {
            GuestFailure::Command(failure) => {
                assert!(matches!(**failure, Failure::Route(_)));
                full(&pools.ordinary);
            }
            GuestFailure::Filesystem(_) => assert!(pools.rpc.admit().is_err()),
            GuestFailure::Route => {
                assert!(pools.ordinary.admit().is_ok());
                assert!(pools.rpc.admit().is_ok());
            }
            GuestFailure::Files(_) => panic!("non-exact files path was routed"),
        }
        assert!(pools.files.admit().is_ok());
        drop(failure);
        assert!(pools.ordinary.admit().is_ok());
        assert!(pools.rpc.admit().is_ok());
    }
    fixture.assert_no_effect();
    assert_eq!(fixture.gate_count(), 0);
}
