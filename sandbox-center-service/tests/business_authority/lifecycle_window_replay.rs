//! E2: actual authenticated HTTP/CenterPort/Redis retirement and replay.
//! Terminal host evidence is controlled by the existing fixture, not KVM.
use super::*;
use axum::{Router, http::StatusCode};
use sandproof_api::admission::RequestAdmission;
use sandproof_center_service::CenterPort;
use sandproof_controller_service::application::{ApplicationError, Clock};
use sandproof_protocol::lifecycle::{
    PublicAcceptance, PublicDestroyRequest, PublicRequestWindow, PublicSessionView,
};
use std::sync::{Arc, atomic::AtomicU64};

struct Time(AtomicU64);
impl Clock for Time {
    fn now_ms(&self) -> Result<i64, ApplicationError> {
        i64::try_from(self.0.load(Ordering::SeqCst)).map_err(|_| ApplicationError::Busy)
    }
}

fn router(authority: &SharedBusinessAuthority, clock: &Arc<Time>) -> Router {
    let (_, app, _) = templates::application(authority, false);
    sandproof_api::lifecycle::router_with_admission(
        Arc::new(CenterPort::new(Arc::new(app.with_clock(clock.clone())))),
        public_credentials(),
        Arc::new(RequestAdmission::default()),
    )
    .unwrap()
}

async fn create(router: &Router, secret: &str, request: &PublicCreateRequest) -> PublicAcceptance {
    let response = send_public(
        router,
        "POST",
        "/sandproof/lifecycle/create",
        secret,
        Some(&serde_json::to_value(request).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    read_public(response).await
}

async fn window(router: &Router, secret: &str) -> u64 {
    let response = send_public(
        router,
        "GET",
        "/sandproof/lifecycle/request-window",
        secret,
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    read_public::<PublicRequestWindow>(response)
        .await
        .window
        .unwrap()
        .get()
}

fn session_path(accepted: &PublicAcceptance) -> String {
    format!(
        "/sandproof/lifecycle/sessions/{}",
        accepted.session.as_protocol_id().as_str()
    )
}

async fn replay_checks(
    router: &Router,
    business: &SharedBusinessAuthority,
    original: &PublicCreateRequest,
    accepted: &PublicAcceptance,
    peer_request: &PublicCreateRequest,
    peer: &PublicAcceptance,
    peer_secret: &str,
) {
    let before = business.snapshot().unwrap();
    let mut changed = original.clone();
    changed.parameters.timeout_ms -= 1;
    for request in [original, &changed] {
        let response = send_public(
            router,
            "POST",
            "/sandproof/lifecycle/create",
            OWNER_KEY,
            Some(&serde_json::to_value(request).unwrap()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_same_business_state(&business.snapshot().unwrap(), &before);
    }
    // A retained operation in the old public window still replays. A window
    // mismatch alone must not erase it or reject its complete original input.
    assert_eq!(create(router, peer_secret, peer_request).await, *peer);
    assert_same_business_state(&business.snapshot().unwrap(), &before);
    for secret in [OWNER_KEY, OTHER_KEY] {
        assert_eq!(
            send_public(router, "GET", &session_path(accepted), secret, None)
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_same_business_state(&business.snapshot().unwrap(), &before);
    }
    // A foreign owner must not inspect or terminate the still-live peer.
    let foreign = if peer_secret == OWNER_KEY {
        OTHER_KEY
    } else {
        OWNER_KEY
    };
    assert_eq!(
        send_public(router, "GET", &session_path(peer), foreign, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    let destroy = serde_json::to_value(PublicDestroyRequest {
        window: window(router, foreign).await.try_into().unwrap(),
        sequence: 4.try_into().unwrap(),
        expected_version: 1,
    })
    .unwrap();
    assert_eq!(
        send_public(
            router,
            "POST",
            &format!("{}/destroy", session_path(peer)),
            foreign,
            Some(&destroy),
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_same_business_state(&business.snapshot().unwrap(), &before);
}

async fn retire_original(
    business: &SharedBusinessAuthority,
    public: &Router,
    clock: &Arc<Time>,
    original: &PublicCreateRequest,
    accepted: &PublicAcceptance,
    confirm_before_expiry: bool,
) -> BusinessEnvelope<RegistryMetadata> {
    let destroy = PublicDestroyRequest {
        window: original.window,
        sequence: 3.try_into().unwrap(),
        expected_version: 1,
    };
    let response = send_public(
        public,
        "POST",
        &format!("{}/destroy", session_path(accepted)),
        OWNER_KEY,
        Some(&serde_json::to_value(&destroy).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let saved = business.snapshot().unwrap();
    let intent = saved
        .center
        .lifecycle_session(&accepted.key.owner, &accepted.session)
        .unwrap()
        .operations()
        .last()
        .unwrap()
        .intent()
        .clone();
    let terminal = cleaned_observation(&intent);
    clock.0.store(100, Ordering::SeqCst);
    let mut authority = business.clone();
    AuthorityPort::accept_lifecycle_terminal_observation(&mut authority, &terminal, 100).unwrap();
    drop(authority);
    if confirm_before_expiry {
        business
            .confirm_terminal(&ConfirmedLifecycleResult {
                result: terminal.clone(),
            })
            .unwrap();
        clock.0.store(199, Ordering::SeqCst);
        let before = business.snapshot().unwrap();
        assert_eq!(business.begin_close(&terminal, 199), Err(PortError::Busy));
        assert_same_business_state(&business.snapshot().unwrap(), &before);
    } else {
        clock.0.store(200, Ordering::SeqCst);
        let before = business.snapshot().unwrap();
        assert_eq!(
            business.begin_close(&terminal, 200),
            Err(PortError::Conflict)
        );
        assert_same_business_state(&business.snapshot().unwrap(), &before);
    }
    // Expiry alone cannot turn a still-retained request into a new acceptance.
    clock.0.store(200, Ordering::SeqCst);
    let before = business.snapshot().unwrap();
    assert_eq!(create(public, OWNER_KEY, original).await, *accepted);
    assert_same_business_state(&business.snapshot().unwrap(), &before);
    if !confirm_before_expiry {
        business
            .confirm_terminal(&ConfirmedLifecycleResult {
                result: terminal.clone(),
            })
            .unwrap();
    }
    let close = business.begin_close(&terminal, 200).unwrap();
    let before = business.snapshot().unwrap();
    assert_eq!(create(public, OWNER_KEY, original).await, *accepted);
    assert_same_business_state(&business.snapshot().unwrap(), &before);
    let mut ack = ClosedLifecycleWindow {
        terminal: close.terminal.clone(),
        retired_through: close.terminal.request.retention.window,
    };
    ack.retired_through = (ack.retired_through.get() + 1).try_into().unwrap();
    assert_eq!(
        business.apply_close_ack(&ack, 200),
        Err(PortError::Conflict)
    );
    assert_same_business_state(&business.snapshot().unwrap(), &before);
    ack.retired_through = close.terminal.request.retention.window;
    business.apply_close_ack(&ack, 200).unwrap();
    business.snapshot().unwrap()
}

async fn accept_successor(
    public: &Router,
    business: &SharedBusinessAuthority,
    original: &PublicCreateRequest,
    accepted: &PublicAcceptance,
    peer_request: &PublicCreateRequest,
    peer: &PublicAcceptance,
    peer_secret: &str,
) {
    let mut fresh = original.clone();
    fresh.window = window(public, OWNER_KEY).await.try_into().unwrap();
    let next = create(public, OWNER_KEY, &fresh).await;
    assert_ne!(next.session, accepted.session);
    assert_ne!(next.session, peer.session);
    assert_ne!(next.key, accepted.key);
    assert_eq!(next.key.sequence, accepted.key.sequence);
    let view = send_public(public, "GET", &session_path(&next), OWNER_KEY, None).await;
    assert_eq!(view.status(), StatusCode::OK);
    assert_eq!(
        read_public::<PublicSessionView>(view).await.phase,
        PublicSessionPhase::Creating
    );
    replay_checks(
        public,
        business,
        original,
        accepted,
        peer_request,
        peer,
        peer_secret,
    )
    .await;
    assert_eq!(business.snapshot().unwrap().center.sandboxes().len(), 2);
}

async fn retired_window(peer_secret: &str, confirm_before_expiry: bool) {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = setup(&fixture, &proxy);
    let clock = Arc::new(Time(AtomicU64::new(10)));
    let public = router(&business, &clock);
    let original = request();
    let accepted = create(&public, OWNER_KEY, &original).await;
    let mut peer_request = original.clone();
    if peer_secret == OWNER_KEY {
        peer_request.sequence = 2.try_into().unwrap();
    }
    let peer = create(&public, peer_secret, &peer_request).await;
    assert_ne!(accepted.session, peer.session);
    assert_ne!(accepted.key, peer.key);
    assert_eq!(window(&public, OWNER_KEY).await, 1);
    assert_eq!(window(&public, OTHER_KEY).await, 1);

    let retired = retire_original(
        &business,
        &public,
        &clock,
        &original,
        &accepted,
        confirm_before_expiry,
    )
    .await;
    assert!(
        retired
            .center
            .lifecycle_session(&accepted.key.owner, &accepted.session)
            .is_none()
    );
    assert_eq!(retired.center.sandboxes().len(), 1);
    assert_eq!(window(&public, OWNER_KEY).await, 3);
    assert_eq!(window(&public, OTHER_KEY).await, 1);
    replay_checks(
        &public,
        &business,
        &original,
        &accepted,
        &peer_request,
        &peer,
        peer_secret,
    )
    .await;

    // Reopen the real Redis authority under a new writer lease; do not rebuild
    // its state from a fixture snapshot or restore an old in-memory cache.
    drop(public);
    drop(business);
    let (ledger, _) = RedisLedger::open(
        engine::endpoint(proxy.address),
        Engine::config(retired.required_block_bytes()),
        fixture.lease(),
    )
    .unwrap();
    let business = SharedBusinessAuthority::new(CenterAuthority::attach(ledger).unwrap());
    assert_same_business_state(&business.snapshot().unwrap(), &retired);
    let public = router(&business, &clock);
    replay_checks(
        &public,
        &business,
        &original,
        &accepted,
        &peer_request,
        &peer,
        peer_secret,
    )
    .await;
    accept_successor(
        &public,
        &business,
        &original,
        &accepted,
        &peer_request,
        &peer,
        peer_secret,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_window_keeps_a_retained_peer_and_refuses_retired_replay_after_reopen() {
    retired_window(OWNER_KEY, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expiry_needs_confirmation_and_preserves_the_other_authenticated_owner() {
    retired_window(OTHER_KEY, false).await;
}
