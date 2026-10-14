use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use axum::body::Body;
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn rejection(default: bool, name: &str) -> Rejection {
    let user = UnknownUser::new(name);
    if default {
        Rejection::DefaultUser(user)
    } else {
        Rejection::ExplicitUser(user)
    }
}
fn retain() -> (Owner, Arc<Semaphore>) {
    let slots = Arc::new(Semaphore::new(1));
    (Arc::new(slots.clone().try_acquire_owned().unwrap()), slots)
}
fn limits() -> Limits {
    Limits {
        wire_bytes: 65536,
        chunk_bytes: 17,
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
async fn released(slots: Arc<Semaphore>) {
    drop(
        tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
            .await
            .unwrap()
            .unwrap(),
    );
}
async fn drain(mut body: Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let chunk = frame.unwrap().into_data().unwrap();
        bytes.extend_from_slice(&chunk);
    }
    bytes
}

#[tokio::test]
async fn account_origin_selects_fixed_message_and_handler_headers() {
    for default in [false, true] {
        let (owner, slots) = retain();
        let response = rejected(rejection(default, "unknown"), limits(), deadline(), owner)
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(
            response
                .headers()
                .get("accept-encoding")
                .map(|v| v.to_str().unwrap()),
            default.then_some("gzip")
        );
        for absent in ["content-encoding", "x-content-type-options", "vary"] {
            assert!(!response.headers().contains_key(absent));
        }
        let prefix = if default {
            "invalid default user"
        } else {
            "invalid username"
        };
        let expected = format!(r#"{{"code":"unauthenticated","message":"{prefix}: 'unknown'"}}"#);
        assert_eq!(
            response.headers()["content-length"],
            expected.len().to_string()
        );
        assert_eq!(drain(response.into_body()).await, expected.as_bytes());
        released(slots).await;
    }
}

#[tokio::test]
async fn escaped_payload_budget_and_configuration_failures_keep_capacity() {
    for default in [false, true] {
        let diagnostic = rejection(default, "用户'<>&\"\\\n\0\u{2028}\u{2029}");
        let bytes = payload(&diagnostic, 65536).unwrap();
        assert!(!bytes.ends_with(b"\n"));
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["code"], "unauthenticated");
        assert!(
            value["message"]
                .as_str()
                .unwrap()
                .ends_with("用户'<>&\"\\\n\0\u{2028}\u{2029}'")
        );
        for (config, expected) in [
            (
                Limits {
                    wire_bytes: bytes.len() - 1,
                    ..limits()
                },
                Error::ResponseLimit,
            ),
            (
                Limits {
                    wire_bytes: 0,
                    ..limits()
                },
                Error::InvalidConfiguration,
            ),
            (
                Limits {
                    chunk_bytes: 0,
                    ..limits()
                },
                Error::InvalidConfiguration,
            ),
            (
                Limits {
                    wire_bytes: usize::MAX,
                    ..limits()
                },
                Error::InvalidConfiguration,
            ),
        ] {
            let (owner, slots) = retain();
            let end = deadline();
            let failure = rejected(diagnostic.clone(), config, end, owner)
                .await
                .unwrap_err();
            assert_eq!(failure.error(), expected);
            assert_eq!(failure.deadline(), end);
            assert_eq!(slots.available_permits(), 0);
            drop(failure);
            released(slots).await;
        }
        let (owner, slots) = retain();
        let response = rejected(
            diagnostic,
            Limits {
                wire_bytes: bytes.len(),
                ..limits()
            },
            deadline(),
            owner,
        )
        .await
        .unwrap();
        assert_eq!(drain(response.into_body()).await, bytes);
        released(slots).await;
    }
}

#[tokio::test]
async fn rejection_bytes_and_clones_keep_original_capacity_after_expiry() {
    for default in [false, true] {
        let (owner, slots) = retain();
        let response = rejected(rejection(default, "unknown"), limits(), deadline(), owner)
            .await
            .unwrap();
        let mut body = response.into_body();
        let first = body.frame().await.unwrap().unwrap().into_data().unwrap();
        let copy = first.clone();
        drop(first);
        let mut next = Box::pin(body.frame());
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(next.as_mut().poll(cx).is_pending()))
                .await
        );
        drop(next);
        tokio::time::pause();
        tokio::time::advance(Duration::from_secs(31)).await;
        assert!(body.frame().await.unwrap().is_err());
        tokio::time::resume();
        drop(body);
        assert_eq!(slots.available_permits(), 0);
        drop(copy);
        released(slots).await;
    }
}

#[tokio::test]
async fn expired_rejection_cannot_start_a_fresh_response_deadline() {
    let (owner, slots) = retain();
    let end = Instant::now();
    let failure = rejected(rejection(false, "unknown"), limits(), end, owner)
        .await
        .unwrap_err();
    assert_eq!(failure.error(), Error::DeadlineExceeded);
    assert_eq!(failure.deadline(), end);
    assert_eq!(slots.available_permits(), 0);
    drop(failure);
    released(slots).await;
}

#[test]
fn cancelled_queued_rejection_encoder_holds_capacity_until_real_exit() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let (entered, started) = tokio::sync::oneshot::channel();
        let (resume, resumed) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            resumed.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        started.await.unwrap();
        let (owner, slots) = retain();
        let mut response = Box::pin(rejected(
            rejection(false, "unknown"),
            limits(),
            deadline(),
            owner,
        ));
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                response.as_mut().poll(cx).is_pending()
            ))
            .await
        );
        drop(response);
        assert_eq!(slots.available_permits(), 0);
        resume.send(()).unwrap();
        blocker.await.unwrap();
        released(slots).await;
    });
}

#[tokio::test]
#[ignore = "explicit fixed Go authn/Connect HTTP reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_account_rejection_status_headers_and_json_reference() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for default in [false, true] {
        let mut names = vec![
            String::new(),
            "unknown".into(),
            "用户'<>&\"\\\n\0\u{2028}\u{2029}".into(),
            "a\tb\r\u{7f}".into(),
        ];
        let base = payload(&rejection(default, ""), 65536).unwrap().len();
        for length in [2047, 2048, 2049, 8192] {
            names.push("a".repeat(length - base));
        }
        for name in names {
            let (owner, slots) = retain();
            let response = rejected(rejection(default, &name), limits(), deadline(), owner)
                .await
                .unwrap();
            let mut observed = vec![response.status().as_u16().to_string()];
            for field in [
                "content-type",
                "content-length",
                "accept-encoding",
                "content-encoding",
                "x-content-type-options",
                "vary",
            ] {
                observed.push(hex(response
                    .headers()
                    .get(field)
                    .map_or(b"", |v| v.as_bytes())));
            }
            observed.push(hex(&drain(response.into_body()).await));
            oracle.check(
                &format!(
                    "{}\t{}",
                    if default { "U" } else { "A" },
                    hex(name.as_bytes())
                ),
                &observed.join("\t"),
            );
            released(slots).await;
            count += 1;
        }
    }
    oracle.finish();
    eprintln!(
        "{count} fixed authn/Connect account-error HTTP cases; controlled missing account, no real account database."
    );
}
