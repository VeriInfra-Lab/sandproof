//! Real Redis revision/CAS checks for the single confirmed Unloaded view.
//! Host completion remains controlled; this is not physical unload acceptance.
use super::*;

fn router(app: &Arc<Application>) -> axum::Router {
    sandproof_api::lifecycle::router_with_admission(
        Arc::new(sandproof_center_service::CenterPort::new(Arc::clone(app))),
        vec![sandproof_api::ApiCredential {
            tenant: "8".into(),
            secret: "unloaded-query-component-only-key".into(),
        }],
        Arc::new(sandproof_api::admission::RequestAdmission::default()),
    )
    .unwrap()
}

fn revision(fixture: &Engine) -> u64 {
    fixture.revision().parse().unwrap()
}

#[tokio::test]
async fn normal_unloaded_projection_uses_exactly_one_durable_confirmation() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let (session, _, observation) = accept_unload(&app);
    *host.unloaded.lock().unwrap() = Some(observation.clone());
    app.reconcile_once().unwrap();
    let router = router(&app);
    let before = revision(&fixture);
    let view = public_query(&router, &session).await;
    assert_eq!(revision(&fixture) - before, 1);
    assert_eq!(view.0, 200);
    assert_eq!(view.1["phase"], "unloaded");
    assert_eq!(
        view.1["unload_outcome"],
        serde_json::to_value(observation.outcome).unwrap()
    );
    let before = revision(&fixture);
    assert_eq!(public_query(&router, &session).await, view);
    assert_eq!(
        revision(&fixture) - before,
        1,
        "each new request confirms again"
    );
}

#[tokio::test]
async fn due_unloaded_query_uses_two_commits_and_never_reallocates_termination() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, clock) = application(&authority);
    let (session, _, observation) = accept_unload(&app);
    let end = observation.outcome.unloaded().unwrap().end_at_ms;
    *host.unloaded.lock().unwrap() = Some(observation);
    app.reconcile_once().unwrap();
    clock.0.store(end, Ordering::SeqCst);
    let router = router(&app);
    let before = revision(&fixture);
    let view = public_query(&router, &session).await;
    assert_eq!(revision(&fixture) - before, 2);
    assert_eq!(view.0, 200);
    assert_eq!(view.1["phase"], "terminating");
    assert_eq!(view.1["current_version"], 3);
    assert_eq!(public_query(&router, &session).await, view);
    let state = authority.snapshot().unwrap();
    let record = state
        .center
        .lifecycle_session(&"8".try_into().unwrap(), &session)
        .unwrap();
    assert_eq!(record.operations().len(), 3);
    assert!(matches!(
        record
            .operations()
            .last()
            .unwrap()
            .intent()
            .request
            .operation,
        LifecycleOperation::Destroy {}
    ));
}

#[tokio::test]
async fn unloaded_query_still_expires_other_due_rows_in_the_same_envelope() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, clock) = application(&authority);
    let (first, _, first_observation) = accept_unload(&app);
    *host.unloaded.lock().unwrap() = Some(first_observation.clone());
    app.reconcile_once().unwrap();
    let mut short = lifecycle_public::request();
    short.sequence = 3.try_into().unwrap();
    short.parameters.timeout_ms = 1000;
    let second = app.create_lifecycle("8", &short).unwrap().session;
    let running = app.query_lifecycle("8", &second).unwrap();
    assert_eq!(running.phase, PublicSessionPhase::Running);
    let state = authority.snapshot().unwrap();
    let due = state
        .center
        .lifecycle_session(&"8".try_into().unwrap(), &second)
        .unwrap()
        .last_running()
        .unwrap()
        .end_at_ms;
    clock.0.store(due, Ordering::SeqCst);
    // The queried row is still live, but the original whole-envelope expiry
    // must terminate the other, shorter-lived row in the same commit.
    let router = router(&app);
    let before = revision(&fixture);
    let view = public_query(&router, &first).await;
    assert_eq!(view.0, 200);
    assert_eq!(view.1["phase"], "unloaded");
    assert_eq!(
        view.1["unload_outcome"],
        serde_json::to_value(first_observation.outcome).unwrap()
    );
    assert_eq!(revision(&fixture) - before, 2);
    assert_eq!(
        app.query_lifecycle("8", &second).unwrap().phase,
        PublicSessionPhase::Terminating
    );
}

#[tokio::test]
async fn confirmed_unloaded_query_keeps_owner_and_missing_session_checks() {
    use tower::ServiceExt;
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let (session, _, observation) = accept_unload(&app);
    *host.unloaded.lock().unwrap() = Some(observation);
    app.reconcile_once().unwrap();
    assert!(matches!(
        app.query_lifecycle("9", &session),
        Err(PortError::NotFound)
    ));
    let missing = SessionId::from_protocol_id("missing-session".try_into().unwrap());
    assert_eq!(public_query(&router(&app), &missing).await.0, 404);
    let response = router(&app)
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/sandproof/lifecycle/sessions/{}",
                    session.as_protocol_id().as_str()
                ))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);
}

struct QueryClock {
    time: Arc<Time>,
    gate: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

impl sandproof_controller_service::application::Clock for QueryClock {
    fn now_ms(&self) -> Result<i64, sandproof_controller_service::application::ApplicationError> {
        if let Some((entered, release)) = self.gate.lock().unwrap().take() {
            entered.send(()).unwrap();
            release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
        Ok(i64::try_from(self.time.0.load(Ordering::SeqCst)).unwrap())
    }
}

#[test]
fn unloaded_query_and_direct_shared_destroy_cannot_mix_two_authority_versions() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, clock) = application(&authority);
    let (session, _, observation) = accept_unload(&app);
    *host.unloaded.lock().unwrap() = Some(observation);
    app.reconcile_once().unwrap();
    let expected = app.query_lifecycle("8", &session).unwrap();
    let (entered, paused) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let app = Arc::new(
        Arc::try_unwrap(app)
            .unwrap_or_else(|_| panic!("unshared fixture app"))
            .with_clock(Arc::new(QueryClock {
                time: clock,
                gate: Mutex::new(Some((entered, wait))),
            })),
    );
    let query_app = Arc::clone(&app);
    let query_session = session.clone();
    let query = std::thread::spawn(move || query_app.query_lifecycle("8", &query_session));
    paused.recv_timeout(Duration::from_secs(5)).unwrap();
    let (started, attempting) = mpsc::channel();
    let (completed, finished) = mpsc::channel();
    let destroy_session = session.clone();
    let destroy = std::thread::spawn(move || {
        started.send(()).unwrap();
        // Bypass the application's outer mutex, as a shared background adapter
        // does. The inner authority mutex must still serialize this update.
        let accepted = authority.accept_public_destroy(
            &"8".try_into().unwrap(),
            destroy_session,
            &PublicDestroyRequest {
                window: 1.try_into().unwrap(),
                sequence: 3.try_into().unwrap(),
                expected_version: 2,
            },
        );
        completed.send(()).unwrap();
        accepted
    });
    attempting.recv_timeout(Duration::from_secs(5)).unwrap();
    let before_release = finished.recv_timeout(Duration::from_millis(100));
    release.send(()).unwrap();
    let view = query.join().unwrap().unwrap();
    let accepted = destroy.join().unwrap().unwrap();
    assert_eq!(before_release, Err(mpsc::RecvTimeoutError::Timeout));
    assert_eq!(view, expected);
    assert_eq!(accepted.accepted_version, 3);
    assert_eq!(
        app.query_lifecycle("8", &session).unwrap().phase,
        PublicSessionPhase::Terminating
    );
}
