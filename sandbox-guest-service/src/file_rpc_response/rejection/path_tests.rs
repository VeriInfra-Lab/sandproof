use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

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
async fn drain(mut body: axum::body::Body) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        let data = frame.unwrap().into_data().unwrap();
        bytes.extend_from_slice(&data);
    }
    bytes
}

fn expansion(name: &str) -> Rejection {
    Rejection::PathExpansion(PathExpansion::new(name))
}

#[tokio::test]
async fn path_rejection_preserves_fixed_envelope_exact_budget_and_byte_owner() {
    let name = "user'<>&\"\\\n\u{2028}";
    let diagnostic = expansion(name);
    let bytes = payload(&diagnostic, 65536).unwrap();
    assert_eq!(payload(&diagnostic, bytes.len()).unwrap(), bytes);
    assert_eq!(
        payload(&diagnostic, bytes.len() - 1),
        Err(Error::ResponseLimit)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
        serde_json::json!({
            "code":"invalid_argument", "message":format!("failed to expand path '' for user '{name}': cannot expand user-specific home dir")
        })
    );
    let (owner, slots) = retain();
    let end = deadline();
    let failure = rejected(
        diagnostic.clone(),
        Limits {
            wire_bytes: bytes.len() - 1,
            ..limits()
        },
        end,
        owner,
    )
    .await
    .unwrap_err();
    assert_eq!(failure.error(), Error::ResponseLimit);
    assert_eq!(failure.deadline(), end);
    assert_eq!(slots.available_permits(), 0);
    drop(failure);
    released(slots).await;
    let (owner, slots) = retain();
    let response = rejected(diagnostic, limits(), deadline(), owner)
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(response.headers()["content-type"], "application/json");
    assert_eq!(response.headers()["accept-encoding"], "gzip");
    let mut body = response.into_body();
    let copy = body
        .frame()
        .await
        .unwrap()
        .unwrap()
        .into_data()
        .unwrap()
        .clone();
    drop(body);
    assert_eq!(slots.available_permits(), 0);
    drop(copy);
    released(slots).await;
}

#[tokio::test]
#[ignore = "explicit fixed Go permissions/Connect HTTP reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn fixed_go_rpc_expansion_rejections_match_http() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    for name in [
        "user".to_owned(),
        "user'<>&\"\\\n\0\u{2028}".into(),
        String::new(),
        "n".repeat(2100),
    ] {
        for path in ["~other", "", "~'\"<>&\n\0"] {
            let response = rejected(expansion(&name), limits(), deadline(), Arc::new(()))
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
            let input = serde_json::json!({"Name":name,"Path":path,"Default":"~fallback"});
            oracle.check(
                &format!("X\t{}", hex(input.to_string().as_bytes())),
                &observed.join("\t"),
            );
            count += 1;
        }
    }
    oracle.finish();
    eprintln!(
        "{count} fixed permissions/Connect path expansion HTTP cases; synthetic account, no file I/O."
    );
}
