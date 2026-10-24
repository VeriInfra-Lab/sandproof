//! Real Redis persistence with a controlled host result; not a VM-exit test.
use super::*;
use sandproof_center_store::authority::AuthorityState;
use sandproof_protocol::lifecycle::{ContentDigest, UnloadObservation, UnloadedFacts};

fn accept_unload(app: &Application) -> (SessionId, PublicUnloadRequest, UnloadObservation) {
    let accepted = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &accepted.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    let request = PublicUnloadRequest {
        window: 1.try_into().unwrap(),
        sequence: 2.try_into().unwrap(),
        expected_version: 1,
    };
    app.unload_lifecycle("8", &accepted.session, &request)
        .unwrap();
    let state = app.snapshot().unwrap();
    let record = state
        .lifecycle_session(&"8".try_into().unwrap(), &accepted.session)
        .unwrap();
    let intent = record.operations().last().unwrap().intent().clone();
    let LifecycleOperation::Unload {
        source_incarnation,
        capture,
    } = intent.request.operation
    else {
        unreachable!()
    };
    let prior = record.last_running().unwrap();
    let facts = UnloadedFacts {
        source_incarnation,
        capture,
        manifest_digest: ContentDigest::from_sha256([7; 32]),
        material_bytes: 7,
        started_at_ms: prior.started_at_ms,
        end_at_ms: prior.end_at_ms,
        max_end_at_ms: prior.max_end_at_ms,
        startup_deadline_ms: prior.startup_deadline_ms,
    };
    (
        accepted.session,
        request,
        UnloadObservation {
            request: intent,
            unloaded: Some(facts),
        },
    )
}

#[test]
fn unload_requires_completed_fact_and_reopens_original_authorized_result() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, clock) = application(&authority);
    let (session, request, observation) = accept_unload(&app);
    app.reconcile_once().unwrap();
    assert_eq!(
        app.query_lifecycle("8", &session).unwrap().phase,
        PublicSessionPhase::Unloading
    );
    *host.unloaded.lock().unwrap() = Some(observation.clone());
    app.reconcile_once().unwrap();
    let saved = app.query_lifecycle("8", &session).unwrap();
    assert_eq!(saved.phase, PublicSessionPhase::Unloaded);
    assert_eq!(saved.unloaded, observation.unloaded);
    let replay = app.unload_lifecycle("8", &session, &request).unwrap();
    assert_eq!(replay.key, saved.operation);
    assert_eq!(app.query_lifecycle("9", &session), Err(PortError::NotFound));
    let before = authority.snapshot().unwrap();
    let limit = before.required_block_bytes();
    drop(app);
    drop(authority);
    let (ledger, _) = super::super::RedisLedger::open(
        super::super::engine::endpoint(proxy.address),
        Engine::config(limit),
        fixture.lease(),
    )
    .unwrap();
    let reopened =
        SharedBusinessAuthority::new(super::super::CenterAuthority::attach(ledger).unwrap());
    let (app, _, restarted_clock) = application(&reopened);
    restarted_clock
        .0
        .store(clock.0.load(Ordering::SeqCst), Ordering::SeqCst);
    assert_eq!(app.query_lifecycle("8", &session).unwrap(), saved);
    assert_eq!(app.query_lifecycle("9", &session), Err(PortError::NotFound));
    assert_eq!(app.snapshot().unwrap(), before.center);
}

#[test]
fn late_unload_cannot_replace_destroy_or_expiry() {
    for expire in [false, true] {
        let fixture = Engine::new();
        let proxy = reply_loss::ReplyLoss::new(fixture.address);
        let authority = lifecycle_public::setup(&fixture, &proxy);
        let (app, host, clock) = application(&authority);
        let (session, _, observation) = accept_unload(&app);
        if expire {
            clock.0.store(
                observation.unloaded.as_ref().unwrap().end_at_ms,
                Ordering::SeqCst,
            );
        } else {
            app.destroy_lifecycle(
                "8",
                session.clone(),
                &PublicDestroyRequest {
                    window: 1.try_into().unwrap(),
                    sequence: 3.try_into().unwrap(),
                    expected_version: 2,
                },
            )
            .unwrap();
        }
        *host.unloaded.lock().unwrap() = Some(observation);
        app.reconcile_once().unwrap();
        let view = app.query_lifecycle("8", &session).unwrap();
        assert_eq!(view.phase, PublicSessionPhase::Terminating);
        assert!(view.unloaded.is_none());
    }
}


async fn public_query(router: &axum::Router, session: &SessionId) -> (u16, serde_json::Value) {
    use tower::ServiceExt;
    let response = router.clone().oneshot(axum::http::Request::builder()
        .uri(format!("/sandproof/lifecycle/sessions/{}", session.as_protocol_id().as_str()))
        .header("X-API-Key", "unloaded-query-component-only-key")
        .body(axum::body::Body::empty()).unwrap()).await.unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 8192).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn assert_unloaded_query_fault(lost_reply: bool) {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let (session, _, observation) = accept_unload(&app);
    *host.unloaded.lock().unwrap() = Some(observation.clone());
    app.reconcile_once().unwrap();
    let router = sandproof_api::lifecycle::router_with_admission(
        Arc::new(sandproof_center_service::CenterPort::new(app.clone())),
        vec![sandproof_api::ApiCredential {
            tenant: "8".into(), secret: "unloaded-query-component-only-key".into(),
        }],
        Arc::new(sandproof_api::admission::RequestAdmission::default()),
    ).unwrap();
    let before = public_query(&router, &session).await;
    assert_eq!(before.0, 200);
    assert_eq!(before.1["phase"], "unloaded");
    assert_eq!(before.1["unloaded"], serde_json::to_value(&observation.unloaded).unwrap());
    if lost_reply {
        // Deliver the real reaffirm CAS, then lose only its original reply.
        proxy.lose_after(0);
    } else {
        // A known configuration mismatch precedes any reaffirm CAS.
        let _: () = redis::cmd("CONFIG").arg("SET").arg("maxmemory")
            .arg(super::super::engine::MEMORY + 1)
            .query(&mut super::super::engine::connect(fixture.address)).unwrap();
    }
    let failed = public_query(&router, &session).await;
    assert_eq!(failed.0, 500);
    assert_eq!(failed.1, serde_json::json!({"code":500, "message":"Internal Server Error"}));
    if !lost_reply {
        let _: () = redis::cmd("CONFIG").arg("SET").arg("maxmemory")
            .arg(super::super::engine::MEMORY)
            .query(&mut super::super::engine::connect(fixture.address)).unwrap();
    }
    // Restoring availability cannot turn a fenced cached value into success.
    assert_eq!(public_query(&router, &session).await.0, 500);
    app.recover_authority().unwrap();
    assert_eq!(public_query(&router, &session).await, before);
}

#[tokio::test]
async fn unloaded_public_query_keeps_500_after_pre_cas_health_failure_until_explicit_recovery() {
    assert_unloaded_query_fault(false).await;
}

#[tokio::test]
async fn unloaded_public_query_keeps_500_after_lost_reaffirm_reply_until_explicit_recovery() {
    assert_unloaded_query_fault(true).await;
}
