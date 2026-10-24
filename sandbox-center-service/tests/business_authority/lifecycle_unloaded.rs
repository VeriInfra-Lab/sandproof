//! Real Redis persistence with a controlled host result; not a VM-exit test.
use super::*;
use sandproof_center_store::authority::AuthorityState;
use sandproof_protocol::lifecycle::{ContentDigest, UnloadObservation, UnloadOutcome, UnloadedFacts};

fn unloaded_facts_mut(observation: &mut UnloadObservation) -> &mut UnloadedFacts {
    match &mut observation.outcome {
        UnloadOutcome::Unloaded { facts } => facts,
        _ => panic!("expected completed Unloaded fixture"),
    }
}

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
            outcome: UnloadOutcome::Unloaded { facts },
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
    assert_eq!(saved.unloaded, observation.outcome.unloaded().cloned());
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
                observation.outcome.unloaded().unwrap().end_at_ms,
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
    let response = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/sandproof/lifecycle/sessions/{}",
                    session.as_protocol_id().as_str()
                ))
                .header("X-API-Key", "unloaded-query-component-only-key")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
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
            tenant: "8".into(),
            secret: "unloaded-query-component-only-key".into(),
        }],
        Arc::new(sandproof_api::admission::RequestAdmission::default()),
    )
    .unwrap();
    let before = public_query(&router, &session).await;
    assert_eq!(before.0, 200);
    assert_eq!(before.1["phase"], "unloaded");
    assert_eq!(
        before.1["unloaded"],
        serde_json::to_value(&observation.outcome.unloaded().cloned()).unwrap()
    );
    if lost_reply {
        // Deliver the real reaffirm CAS, then lose only its original reply.
        proxy.lose_after(0);
    } else {
        // A known configuration mismatch precedes any reaffirm CAS.
        let _: () = redis::cmd("CONFIG")
            .arg("SET")
            .arg("maxmemory")
            .arg(super::super::engine::MEMORY + 1)
            .query(&mut super::super::engine::connect(fixture.address))
            .unwrap();
    }
    let failed = public_query(&router, &session).await;
    assert_eq!(failed.0, 500);
    assert_eq!(
        failed.1,
        serde_json::json!({"code":500, "message":"Internal Server Error"})
    );
    if !lost_reply {
        let _: () = redis::cmd("CONFIG")
            .arg("SET")
            .arg("maxmemory")
            .arg(super::super::engine::MEMORY)
            .query(&mut super::super::engine::connect(fixture.address))
            .unwrap();
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

#[test]
fn controlled_unloading_to_unloaded_queries_keep_original_material_and_lifetime() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, clock) = application(&authority);
    let (session, _, mut observation) = accept_unload(&app);
    // Same scalar magnitude as the retained real capture, without a VM/file.
    unloaded_facts_mut(&mut observation).material_bytes = 3_221_239_189;
    let (observed, received) = mpsc::channel();
    let (proceed, wait) = mpsc::channel();
    let release = Release(Some(proceed));
    let query_app = app.clone();
    let query_session = session.clone();
    let query = std::thread::spawn(move || {
        observed
            .send(query_app.query_lifecycle("8", &query_session))
            .unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
        query_app.query_lifecycle("8", &query_session)
    });
    let pending = received
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    assert_eq!(pending.phase, PublicSessionPhase::Unloading);
    assert!(pending.unloaded.is_none());
    *host.unloaded.lock().unwrap() = Some(observation.clone());
    app.reconcile_once().unwrap();
    release.complete();
    let completed = query.join().unwrap().unwrap();
    assert_eq!(completed.phase, PublicSessionPhase::Unloaded);
    assert_eq!(completed.unloaded, observation.outcome.unloaded().cloned());
    assert_eq!(completed.current_version, pending.current_version);
    assert_eq!(completed.operation, pending.operation);
    assert_eq!(completed.session, pending.session);
    assert!(clock.0.load(Ordering::SeqCst) < completed.unloaded.unwrap().end_at_ms);
}

#[tokio::test]
async fn original_background_round_and_public_queries_keep_unloaded_without_storage_faults() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, clock) = application(&authority);
    let (session, _, mut observation) = accept_unload(&app);
    unloaded_facts_mut(&mut observation).material_bytes = 3_221_239_189;
    *host.unloaded.lock().unwrap() = Some(observation.clone());
    let node = authority.snapshot().unwrap().center.nodes()[0].clone();
    let router = sandproof_api::lifecycle::router_with_admission(
        Arc::new(sandproof_center_service::CenterPort::new(app.clone())),
        vec![sandproof_api::ApiCredential {
            tenant: "8".into(),
            secret: "unloaded-query-component-only-key".into(),
        }],
        Arc::new(sandproof_api::admission::RequestAdmission::default()),
    )
    .unwrap();
    let (refreshed, refresh_seen) = mpsc::channel();
    let (after_refresh, refresh_wait) = mpsc::channel();
    let release_refresh = Release(Some(after_refresh));
    let (reconciled, reconcile_seen) = mpsc::channel();
    let (after_reconcile, reconcile_wait) = mpsc::channel();
    let release_reconcile = Release(Some(after_reconcile));
    let background_app = app.clone();
    let background = std::thread::spawn(move || {
        // Use the runtime's actual three calls in their original order. Free
        // physical capacity is a veto observation, not release of ownership.
        authority
            .refresh_node(&node.identity, node.healthy_until_ms, node.limit)
            .unwrap();
        refreshed.send(()).unwrap();
        refresh_wait.recv_timeout(Duration::from_secs(5)).unwrap();
        background_app.reconcile_once().unwrap();
        reconciled.send(()).unwrap();
        reconcile_wait.recv_timeout(Duration::from_secs(5)).unwrap();
        background_app.retire_terminal_history_once()
    });
    // Gates are between the original background calls. They do not claim to
    // force a write between the two snapshots inside a single public GET.
    refresh_seen.recv_timeout(Duration::from_secs(5)).unwrap();
    let pending = public_query(&router, &session).await;
    assert_eq!(pending.0, 200);
    let pending: sandproof_protocol::lifecycle::PublicSessionView =
        serde_json::from_value(pending.1).unwrap();
    assert_eq!(pending.phase, PublicSessionPhase::Unloading);
    assert!(pending.unloaded.is_none());
    release_refresh.complete();
    reconcile_seen.recv_timeout(Duration::from_secs(5)).unwrap();
    let revision_before: u64 = fixture.revision().parse().unwrap();
    let completed = public_query(&router, &session).await;
    let revision_after: u64 = fixture.revision().parse().unwrap();
    let revision_cost = revision_after.checked_sub(revision_before).unwrap();
    eprintln!("unloaded_query_reaffirm_revision_delta={revision_cost}");
    // Measure the real original Redis CAS path without injecting any fault.
    // Do not make a future safe reduction in redundant reaffirms a failure.
    assert!(revision_cost > 0, "public authority is durably reaffirmed");
    assert_eq!(completed.0, 200);
    let completed: sandproof_protocol::lifecycle::PublicSessionView =
        serde_json::from_value(completed.1).unwrap();
    assert_eq!(completed.phase, PublicSessionPhase::Unloaded);
    assert_eq!(completed.unloaded, observation.outcome.unloaded().cloned());
    assert_eq!(completed.session, pending.session);
    assert_eq!(completed.operation, pending.operation);
    assert_eq!(completed.current_version, pending.current_version);
    release_reconcile.complete();
    background.join().unwrap().unwrap();
    let retired = public_query(&router, &session).await;
    assert_eq!(retired.0, 200);
    assert_eq!(
        serde_json::from_value::<sandproof_protocol::lifecycle::PublicSessionView>(retired.1)
            .unwrap(),
        completed
    );
    assert!(clock.0.load(Ordering::SeqCst) < completed.unloaded.unwrap().end_at_ms);
}


#[test]
fn original_continued_result_restores_running_without_unloaded_material() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let (session, request, mut observation) = accept_unload(&app);
    let snapshot = app.snapshot().unwrap();
    let facts = snapshot
        .lifecycle_session(&"8".try_into().unwrap(), &session)
        .unwrap()
        .last_running()
        .unwrap()
        .clone();
    observation.outcome = UnloadOutcome::OriginalContinued { facts };
    *host.unloaded.lock().unwrap() = Some(observation);
    app.reconcile_once().unwrap();
    let saved = app.query_lifecycle("8", &session).unwrap();
    assert_eq!(saved.phase, PublicSessionPhase::Running);
    assert!(saved.unloaded.is_none());
    let replay = app.unload_lifecycle("8", &session, &request).unwrap();
    assert_eq!(replay.key, saved.operation);
    assert_eq!(replay.session, session);
}
