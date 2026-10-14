#![cfg(target_os = "linux")]

use std::future::{Future, poll_fn};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use sandproof_guest_service::{InitLimits, Metadata, MetadataSource, ServiceError, Session};
use serde_json::json;
use sha2::{Digest, Sha512};
use tokio::sync::Semaphore;

#[derive(Clone)]
struct Source {
    reply: Arc<Mutex<Reply>>,
    calls: Arc<AtomicUsize>,
    gate: Arc<Semaphore>,
}

type Reply = Result<(String, String, String), ServiceError>;

impl Source {
    fn new(token: &str) -> Self {
        Self {
            reply: Arc::new(Mutex::new(Ok((
                "vm-fixture".into(),
                "template-fixture".into(),
                token.into(),
            )))),
            calls: Arc::new(AtomicUsize::new(0)),
            gate: Arc::new(Semaphore::new(0)),
        }
    }
    fn allow(&self) {
        self.gate.add_permits(1);
    }
    fn token(&self, token: &str) {
        *self.reply.lock().unwrap() =
            Ok(("vm-fixture".into(), "template-fixture".into(), token.into()));
    }
}

impl MetadataSource for Source {
    async fn fetch(&self) -> Result<Metadata, ServiceError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.gate.acquire().await.unwrap().forget();
        let (instance_id, template_id, token) = self.reply.lock().unwrap().clone()?;
        Ok(Metadata {
            instance_id,
            template_id,
            access_token_sha512: Sha512::digest(token.as_bytes()).into(),
        })
    }
}

fn limits() -> InitLimits {
    InitLimits {
        body_bytes: 4096,
        token_bytes: 128,
        state_bytes: 1024,
        env_entries: 16,
        in_flight: 2,
        timeout: Duration::from_secs(5),
    }
}

fn body(token: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"accessToken": token})).unwrap()
}

async fn assert_pending(future: std::pin::Pin<&mut impl Future>) {
    let mut future = future;
    assert!(
        poll_fn(|cx| Poll::Ready(future.as_mut().poll(cx)))
            .await
            .is_pending()
    );
}

#[tokio::test]
async fn uninitialized_and_unavailable_metadata_never_grant_access() {
    let source = Source::new("token");
    let session = Session::new(source.clone(), limits()).unwrap();
    for token in [None, Some(""), Some("token"), Some("wrong")] {
        assert!(matches!(
            session.authorize(token).await,
            Err(ServiceError::Unauthenticated)
        ));
    }
    assert_eq!(
        session.initialize(b"{}").await,
        Err(ServiceError::Unauthenticated)
    );
    assert_eq!(
        session.initialize(&body("")).await,
        Err(ServiceError::Unauthenticated)
    );
    source.allow();
    assert_eq!(
        session.initialize(&body("wrong")).await,
        Err(ServiceError::Unauthenticated)
    );
    *source.reply.lock().unwrap() = Err(ServiceError::Unavailable);
    source.allow();
    assert_eq!(
        session.initialize(&body("token")).await,
        Err(ServiceError::Unavailable)
    );
    assert!(matches!(
        session.authorize(Some("token")).await,
        Err(ServiceError::Unauthenticated)
    ));
}

#[tokio::test]
async fn shared_host_init_fixture_and_convergent_replay_preserve_defaults() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/compat/guest-service-contract.json"
    ))
    .unwrap();
    let request = cases["http"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == "initialize")
        .unwrap()["body"]
        .clone();
    let source = Source::new("fixture-only-token");
    let session = Session::new(source.clone(), limits()).unwrap();
    source.allow();
    let bytes = serde_json::to_vec(&request).unwrap();
    session.initialize(&bytes).await.unwrap();
    {
        let permit = session.authorize(Some("fixture-only-token")).await.unwrap();
        assert_eq!(permit.defaults().user, "root");
        assert_eq!(permit.defaults().workdir.as_deref(), Some("/tmp"));
        assert_eq!(permit.defaults().env["SAMPLE"], "value");
        assert_eq!(permit.defaults().env["E2B_SANDBOX_ID"], "vm-fixture");
        assert_eq!(permit.defaults().env["E2B_TEMPLATE_ID"], "template-fixture");
        assert!(
            !permit
                .defaults()
                .env
                .values()
                .any(|value| value.contains("fixture-only-token"))
        );
    }
    *source.reply.lock().unwrap() = Err(ServiceError::Unavailable);
    session.initialize(&bytes).await.unwrap();
    assert_eq!(
        source.calls.load(Ordering::SeqCst),
        1,
        "same credential replay must not depend on MMDS availability"
    );
}

#[tokio::test]
async fn replacement_empty_and_omitted_fields_follow_restricted_host_contract() {
    let source = Source::new("token");
    let session = Session::new(source.clone(), limits()).unwrap();
    source.allow();
    session.initialize(br#"{"accessToken":"token","defaultUser":"app","defaultWorkdir":"/workspace","envVars":{"VALUE":"first","E2B_SANDBOX_ID":"forged"}}"#).await.unwrap();
    session
        .initialize(br#"{"accessToken":"token","defaultUser":"","defaultWorkdir":""}"#)
        .await
        .unwrap();
    {
        let permit = session.authorize(Some("token")).await.unwrap();
        assert_eq!(permit.defaults().user, "app");
        assert_eq!(permit.defaults().workdir.as_deref(), Some("/workspace"));
        assert_eq!(permit.defaults().env["VALUE"], "first");
        assert_eq!(permit.defaults().env["E2B_SANDBOX_ID"], "vm-fixture");
    }
    session
        .initialize(br#"{"accessToken":"token","envVars":{"OTHER":"next"}}"#)
        .await
        .unwrap();
    {
        let permit = session.authorize(Some("token")).await.unwrap();
        assert!(!permit.defaults().env.contains_key("VALUE"));
        assert_eq!(permit.defaults().env["OTHER"], "next");
    }
    session
        .initialize(br#"{"accessToken":"token","envVars":{}}"#)
        .await
        .unwrap();
    assert_eq!(
        session
            .authorize(Some("token"))
            .await
            .unwrap()
            .defaults()
            .env
            .len(),
        2
    );
}

#[tokio::test]
async fn invalid_or_oversize_update_cannot_partially_rotate_token_or_defaults() {
    let source = Source::new("old");
    let session = Session::new(source.clone(), limits()).unwrap();
    source.allow();
    session.initialize(&body("old")).await.unwrap();
    source.token("new");
    for (request, expected) in [
        (
            json!({"accessToken":"new","defaultUser":"changed","envVars":{"BAD=KEY":"bad"}}),
            ServiceError::InvalidArgument,
        ),
        (
            json!({"accessToken":"new","defaultWorkdir":"bad\0path"}),
            ServiceError::InvalidArgument,
        ),
        (
            json!({"accessToken":"new","envVars":{"VALUE":"x".repeat(1024)}}),
            ServiceError::ResourceExhausted,
        ),
    ] {
        source.allow();
        assert_eq!(
            session
                .initialize(&serde_json::to_vec(&request).unwrap())
                .await,
            Err(expected)
        );
        assert_eq!(
            session
                .authorize(Some("old"))
                .await
                .unwrap()
                .defaults()
                .user,
            "root"
        );
        assert!(matches!(
            session.authorize(Some("new")).await,
            Err(ServiceError::Unauthenticated)
        ));
    }
    // Authentication precedes default validation, so wrong credentials cannot
    // use budget errors as an oracle for retained environment sizes.
    source.allow();
    assert_eq!(
        session
            .initialize(br#"{"accessToken":"wrong","envVars":{"BAD=KEY":"x"}}"#)
            .await,
        Err(ServiceError::Unauthenticated)
    );
}

#[tokio::test]
async fn rotation_waits_for_original_dispatch_and_rejects_old_credentials_after_commit() {
    let source = Source::new("old");
    let session = Session::new(source.clone(), limits()).unwrap();
    source.allow();
    session.initialize(&body("old")).await.unwrap();
    let permit = session.authorize(Some("old")).await.unwrap();
    source.token("new");
    source.allow();
    let bytes = br#"{"accessToken":"new","defaultUser":"app","envVars":{"VALUE":"new"}}"#;
    let mut rotation = Box::pin(session.initialize(bytes));
    assert_pending(rotation.as_mut()).await;
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    assert_eq!(permit.defaults().user, "root");
    assert!(!permit.defaults().env.contains_key("VALUE"));
    drop(permit);
    rotation.await.unwrap();
    assert!(matches!(
        session.authorize(Some("old")).await,
        Err(ServiceError::Unauthenticated)
    ));
    let permit = session.authorize(Some("new")).await.unwrap();
    assert_eq!(permit.defaults().user, "app");
    assert_eq!(permit.defaults().env["VALUE"], "new");
}

#[tokio::test]
async fn cancellation_at_metadata_or_write_lock_preserves_old_session_and_releases_init_slot() {
    let source = Source::new("old");
    let session = Session::new(source.clone(), limits()).unwrap();
    source.allow();
    session.initialize(&body("old")).await.unwrap();
    source.token("new");
    let bytes = body("new");
    let mut pending = Box::pin(session.initialize(&bytes));
    assert_pending(pending.as_mut()).await;
    assert_eq!(
        session.initialize(&bytes).await,
        Err(ServiceError::ResourceExhausted)
    );
    assert!(session.authorize(Some("old")).await.is_ok());
    drop(pending);
    source.allow();
    let permit = session.authorize(Some("old")).await.unwrap();
    let mut pending = Box::pin(session.initialize(&bytes));
    assert_pending(pending.as_mut()).await;
    drop(pending);
    drop(permit);
    assert!(session.authorize(Some("old")).await.is_ok());
    assert!(matches!(
        session.authorize(Some("new")).await,
        Err(ServiceError::Unauthenticated)
    ));
    source.allow();
    session.initialize(&bytes).await.unwrap();
    assert!(session.authorize(Some("new")).await.is_ok());
}

#[tokio::test(start_paused = true)]
async fn init_deadline_does_not_grant_a_partially_prepared_session() {
    let source = Source::new("token");
    let session = Session::new(source.clone(), limits()).unwrap();
    let bytes = body("token");
    let mut request = Box::pin(session.initialize(&bytes));
    assert_pending(request.as_mut()).await;
    tokio::time::advance(Duration::from_secs(6)).await;
    assert_eq!(request.await, Err(ServiceError::DeadlineExceeded));
    assert!(matches!(
        session.authorize(Some("token")).await,
        Err(ServiceError::Unauthenticated)
    ));
    source.allow();
    session.initialize(&bytes).await.unwrap();
}

#[tokio::test]
async fn changed_instance_or_template_and_restart_never_inherit_old_authorization() {
    let source = Source::new("old");
    let session = Session::new(source.clone(), limits()).unwrap();
    source.allow();
    session.initialize(&body("old")).await.unwrap();
    for (instance, template) in [
        ("other-vm", "template-fixture"),
        ("vm-fixture", "other-template"),
    ] {
        *source.reply.lock().unwrap() = Ok((instance.into(), template.into(), "new".into()));
        source.allow();
        assert_eq!(
            session.initialize(&body("new")).await,
            Err(ServiceError::Unauthenticated)
        );
        assert!(session.authorize(Some("old")).await.is_ok());
    }
    let restarted = Session::new(source.clone(), limits()).unwrap();
    assert!(matches!(
        restarted.authorize(Some("old")).await,
        Err(ServiceError::Unauthenticated)
    ));
    *source.reply.lock().unwrap() = Err(ServiceError::Unavailable);
    source.allow();
    assert_eq!(
        restarted.initialize(&body("old")).await,
        Err(ServiceError::Unavailable)
    );
}

#[tokio::test]
async fn malformed_input_and_oversize_bodies_fail_before_metadata_io() {
    let source = Source::new("token");
    let session = Session::new(source.clone(), limits()).unwrap();
    for bytes in [
        b"[]".as_slice(),
        b"null",
        b"{}{}",
        br#"{"accessToken":"token","accessToken":"other"}"#,
        br#"{"accessToken":"token","timestamp":"2026-10-13T00:00:00Z"}"#,
        br#"{"accessToken":5}"#,
    ] {
        assert_eq!(
            session.initialize(bytes).await,
            Err(ServiceError::InvalidArgument)
        );
    }
    assert_eq!(
        session.initialize(&vec![b' '; 4097]).await,
        Err(ServiceError::PayloadTooLarge)
    );
    for token in ["a\nb", " a", "\0", "é"] {
        assert_eq!(
            session.initialize(&body(token)).await,
            Err(ServiceError::Unauthenticated)
        );
    }
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn invalid_limits_are_refused_before_semaphore_or_network_creation() {
    for change in 0..8 {
        let mut limit = limits();
        match change {
            0 => limit.body_bytes = 0,
            1 => limit.token_bytes = 0,
            2 => limit.state_bytes = 0,
            3 => limit.env_entries = 0,
            4 => limit.in_flight = 0,
            5 => limit.in_flight = usize::MAX,
            6 => limit.timeout = Duration::ZERO,
            _ => limit.token_bytes = limit.body_bytes + 1,
        }
        assert!(matches!(
            Session::new(Source::new("token"), limit),
            Err(ServiceError::InvalidArgument)
        ));
    }
}

#[tokio::test]
async fn exact_retained_budget_includes_metadata_environment_entries() {
    let source = Source::new("token");
    let mut limits = limits();
    // root + the two fixed metadata key/value pairs contain 59 UTF-8 bytes.
    limits.state_bytes = 59;
    limits.env_entries = 2;
    let session = Session::new(source.clone(), limits).unwrap();
    source.allow();
    session.initialize(&body("token")).await.unwrap();
    assert_eq!(
        session
            .initialize(br#"{"accessToken":"token","defaultUser":"roots"}"#)
            .await,
        Err(ServiceError::ResourceExhausted)
    );
    assert_eq!(
        session
            .initialize(br#"{"accessToken":"token","envVars":{"X":"Y"}}"#)
            .await,
        Err(ServiceError::ResourceExhausted)
    );
    assert_eq!(
        session
            .authorize(Some("token"))
            .await
            .unwrap()
            .defaults()
            .user,
        "root"
    );
    limits.state_bytes = 58;
    let insufficient = Session::new(source.clone(), limits).unwrap();
    source.allow();
    assert_eq!(
        insufficient.initialize(&body("token")).await,
        Err(ServiceError::ResourceExhausted)
    );
    assert!(matches!(
        insufficient.authorize(Some("token")).await,
        Err(ServiceError::Unauthenticated)
    ));
}
