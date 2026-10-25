//! Real Redis persistence with a controlled host result; not a VM-exit test.
use super::*;
use sandproof_center_store::authority::AuthorityState;
use sandproof_protocol::lifecycle::{
    ContentDigest, UnloadObservation, UnloadOutcome, UnloadedFacts,
    CurrentResumeReadiness, ResumeObservation, PublicResumeRequest,
};

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

fn raced_unload(
    late_request: PublicUnloadRequest,
    fence_after_winner: bool,
) -> (
    Result<sandproof_protocol::lifecycle::PublicAcceptance, PortError>,
    sandproof_protocol::lifecycle::PublicAcceptance,
) {
    raced_unload_with_deadline(late_request, fence_after_winner, false)
}

fn raced_unload_with_deadline(
    late_request: PublicUnloadRequest,
    fence_after_winner: bool,
    exhaust_deadline: bool,
) -> (
    Result<sandproof_protocol::lifecycle::PublicAcceptance, PortError>,
    sandproof_protocol::lifecycle::PublicAcceptance,
) {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let created = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &created.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    let request = PublicUnloadRequest {
        window: 1.try_into().unwrap(),
        sequence: 2.try_into().unwrap(),
        expected_version: 1,
    };
    let (entered, release) = host.block_next(created.session.clone());
    let late_app = Arc::clone(&app);
    let late_session = created.session.clone();
    let late =
        std::thread::spawn(move || late_app.unload_lifecycle("8", &late_session, &late_request));
    await_observation(&entered);
    let blocked_budget = *host.observation_budgets.lock().unwrap().last().unwrap();
    let accepted = app
        .unload_lifecycle("8", &created.session, &request)
        .unwrap();
    app.reconcile_once().unwrap();
    let before = app.snapshot().unwrap();
    // The real Host may already report the winner's newer current request,
    // rather than returning the old Create view captured before this barrier.
    *host.current_request_after_block.lock().unwrap() = Some(
        before
            .lifecycle_session(&"8".try_into().unwrap(), &created.session)
            .unwrap()
            .operations()
            .last()
            .unwrap()
            .intent()
            .clone(),
    );
    if fence_after_winner {
        proxy.armed.store(1, Ordering::SeqCst);
        assert!(matches!(
            authority.snapshot(),
            Err(PortError::OutcomeUnknown)
        ));
    }
    if exhaust_deadline {
        exhaust_observation_budget(blocked_budget);
    }
    release.complete();
    let result = late.join().unwrap();
    if fence_after_winner {
        assert!(matches!(
            authority.snapshot(),
            Err(PortError::OutcomeUnknown)
        ));
    } else {
        assert_eq!(app.snapshot().unwrap(), before);
    }
    (result, accepted)
}

// The observer is already inside the original Host barrier. Its supplied
// remaining budget started before this wait, so a full timed receive guarantees
// the original deadline has elapsed before we release it. No polling race.
fn exhaust_observation_budget(budget: Duration) {
    assert!(!budget.is_zero());
    let (sender, receiver) = mpsc::channel::<()>();
    assert_eq!(
        receiver.recv_timeout(budget),
        Err(mpsc::RecvTimeoutError::Timeout)
    );
    drop(sender);
}

fn racing_request(sequence: u64, expected_version: u64) -> PublicUnloadRequest {
    PublicUnloadRequest {
        window: 1.try_into().unwrap(),
        sequence: sequence.try_into().unwrap(),
        expected_version,
    }
}

#[test]
fn duplicate_unload_replays_acceptance_committed_during_detached_observation() {
    let (result, accepted) = raced_unload(racing_request(2, 1), false);
    assert_eq!(result, Ok(accepted));
}

#[test]
fn raced_unload_cannot_replay_a_changed_full_request() {
    let (result, _) = raced_unload(racing_request(2, 2), false);
    assert_eq!(result, Err(PortError::Conflict));
}

#[test]
fn raced_unload_does_not_borrow_another_request_keys_acceptance() {
    let (result, _) = raced_unload(racing_request(3, 1), false);
    assert_eq!(result, Err(PortError::Busy));
}

#[test]
fn raced_unload_keeps_unknown_storage_fenced_despite_an_existing_acceptance() {
    let (result, _) = raced_unload(racing_request(2, 1), true);
    assert_eq!(result, Err(PortError::OutcomeUnknown));
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
    assert_eq!(saved.unload_outcome, Some(observation.outcome.clone()));
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
        assert!(view.unload_outcome.is_none());
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
        before.1["unload_outcome"],
        serde_json::to_value(&observation.outcome).unwrap()
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
    assert_eq!(pending.unload_outcome, Some(UnloadOutcome::Pending));
    *host.unloaded.lock().unwrap() = Some(observation.clone());
    app.reconcile_once().unwrap();
    release.complete();
    let completed = query.join().unwrap().unwrap();
    assert_eq!(completed.phase, PublicSessionPhase::Unloaded);
    assert_eq!(completed.unload_outcome, Some(observation.outcome.clone()));
    assert_eq!(completed.current_version, pending.current_version);
    assert_eq!(completed.operation, pending.operation);
    assert_eq!(completed.session, pending.session);
    assert!(
        clock.0.load(Ordering::SeqCst)
            < completed
                .unload_outcome
                .as_ref()
                .unwrap()
                .unloaded()
                .unwrap()
                .end_at_ms
    );
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
    assert_eq!(pending.unload_outcome, Some(UnloadOutcome::Pending));
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
    assert_eq!(completed.unload_outcome, Some(observation.outcome.clone()));
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
    assert!(
        clock.0.load(Ordering::SeqCst)
            < completed
                .unload_outcome
                .as_ref()
                .unwrap()
                .unloaded()
                .unwrap()
                .end_at_ms
    );
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
    let expected_outcome = observation.outcome.clone();
    *host.unloaded.lock().unwrap() = Some(observation);
    app.reconcile_once().unwrap();
    let saved = app.query_lifecycle("8", &session).unwrap();
    assert_eq!(saved.phase, PublicSessionPhase::Running);
    assert_eq!(saved.unload_outcome, Some(expected_outcome.clone()));
    let replay = app.unload_lifecycle("8", &session, &request).unwrap();
    assert_eq!(replay.key, saved.operation);
    assert_eq!(replay.session, session);
    let detail = app.get("8", session.as_protocol_id().as_str()).unwrap();
    let route = app.resolve_data_route(&detail.envd_access_token).unwrap();
    assert_eq!(route.identity.operation, saved.operation);
    assert_eq!(route.identity.accepted_version, saved.current_version);
    let LifecycleOperation::Unload {
        source_incarnation, ..
    } = &snapshot
        .lifecycle_session(&"8".try_into().unwrap(), &session)
        .unwrap()
        .operations()
        .last()
        .unwrap()
        .intent()
        .request
        .operation
    else {
        unreachable!()
    };
    assert_eq!(route.identity.incarnation, *source_incarnation);
    host.ready.store(false, Ordering::SeqCst);
    assert_eq!(app.query_lifecycle("8", &session), Err(PortError::Busy));
    let retained = app.snapshot().unwrap();
    assert_eq!(
        retained
            .lifecycle_session(&"8".try_into().unwrap(), &session)
            .unwrap()
            .operations()
            .last()
            .unwrap()
            .unloaded()
            .unwrap()
            .outcome,
        expected_outcome
    );
}

async fn assert_timeout_recovered_by_original_round(preceding_writes: usize) {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let (session, _, observation) = accept_unload(&app);
    *host.unloaded.lock().unwrap() = Some(observation);
    app.reconcile_once().unwrap();
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
    let before = public_query(&router, &session).await;
    assert_eq!(before.0, 200);
    proxy.timeout_after(preceding_writes);
    let failed = public_query(&router, &session).await;
    proxy.release_timeout();
    assert_eq!(
        failed,
        (
            500,
            serde_json::json!({"code":500,"message":"Internal Server Error"})
        )
    );
    assert_eq!(public_query(&router, &session).await.0, 500);
    // This is the exact production round, not a test calling recover directly.
    let refreshes = AtomicUsize::new(0);
    app.lifecycle_background_round(|| {
        refreshes.fetch_add(1, Ordering::SeqCst);
        authority.refresh_node(&node.identity, node.healthy_until_ms, node.limit)
    });
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(public_query(&router, &session).await, before);
    app.lifecycle_background_round(|| {
        authority.refresh_node(&node.identity, node.healthy_until_ms, node.limit)
    });
    assert_eq!(public_query(&router, &session).await, before);
}

#[tokio::test]
async fn lookup_transport_timeout_recovers_through_original_background_round() {
    assert_timeout_recovered_by_original_round(0).await;
}
#[tokio::test]
async fn expiry_transport_timeout_recovers_through_original_background_round() {
    assert_timeout_recovered_by_original_round(1).await;
}
#[tokio::test]
async fn projection_transport_timeout_recovers_through_original_background_round() {
    assert_timeout_recovered_by_original_round(3).await;
}

#[test]
fn background_transport_timeout_uses_original_recovery_without_replaying_refresh() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let (session, _, observation) = accept_unload(&app);
    *host.unloaded.lock().unwrap() = Some(observation);
    app.reconcile_once().unwrap();
    let before = app.query_lifecycle("8", &session).unwrap();
    let node = authority.snapshot().unwrap().center.nodes()[0].clone();
    proxy.timeout_after(0);
    let refreshes = AtomicUsize::new(0);
    app.lifecycle_background_round(|| {
        refreshes.fetch_add(1, Ordering::SeqCst);
        let failed = authority.refresh_node(&node.identity, node.healthy_until_ms, node.limit);
        proxy.release_timeout();
        assert!(failed.is_err());
        failed
    });
    assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(app.query_lifecycle("8", &session).unwrap(), before);
}

fn complete_continued(app: &Application, host: &Host) -> (SessionId, u64) {
    let (session, _, mut observation) = accept_unload(app);
    let snapshot = app.snapshot().unwrap();
    let facts = snapshot
        .lifecycle_session(&"8".try_into().unwrap(), &session)
        .unwrap()
        .last_running()
        .unwrap()
        .clone();
    let end = facts.end_at_ms;
    observation.outcome = UnloadOutcome::OriginalContinued { facts };
    *host.unloaded.lock().unwrap() = Some(observation);
    app.reconcile_once().unwrap();
    (session, end)
}

// Original public acceptance, Redis CAS and query paths; only the physical Host
// facts are controlled. The first completed Unload remains the Resume source.
fn complete_resumed_continued(app: &Application, host: &Host) -> (SessionId, ResumeObservation, PublicUnloadRequest) {
    let (session, _, unloaded) = accept_unload(app);
    let source = unloaded.outcome.unloaded().unwrap().clone();
    *host.unloaded.lock().unwrap() = Some(unloaded);
    app.reconcile_once().unwrap();
    assert_eq!(app.query_lifecycle("8", &session).unwrap().phase, PublicSessionPhase::Unloaded);
    app.resume_lifecycle("8", &session, &PublicResumeRequest {
        window: 1.try_into().unwrap(), sequence: 3.try_into().unwrap(), expected_version: 2,
    }).unwrap();
    let state = app.snapshot().unwrap();
    let origin = state.lifecycle_session(&"8".try_into().unwrap(), &session).unwrap().operations().last().unwrap().intent().clone();
    let ready = ResumeObservation {
        request: origin.clone(), current_request: origin,
        readiness: CurrentResumeReadiness::Observed { facts: CurrentReadyFacts {
            started_at_ms: source.started_at_ms, end_at_ms: source.end_at_ms,
            max_end_at_ms: source.max_end_at_ms, startup_deadline_ms: source.startup_deadline_ms,
            observed_at_ms: host.clock.0.load(Ordering::SeqCst),
        } },
    };
    *host.resumed.lock().unwrap() = Some(ready.clone());
    assert_eq!(app.query_lifecycle("8", &session).unwrap().phase, PublicSessionPhase::Running);
    let request = PublicUnloadRequest {
        window: 1.try_into().unwrap(), sequence: 4.try_into().unwrap(), expected_version: 3,
    };
    app.unload_lifecycle("8", &session, &request).unwrap();
    let state = app.snapshot().unwrap();
    let record = state.lifecycle_session(&"8".try_into().unwrap(), &session).unwrap();
    let current = record.operations().last().unwrap().intent().clone();
    let mut ready = ready;
    ready.current_request = current.clone();
    let CurrentResumeReadiness::Observed { facts } = ready.readiness.clone() else { unreachable!() };
    *host.unloaded.lock().unwrap() = Some(UnloadObservation {
        request: current, outcome: UnloadOutcome::OriginalContinued { facts },
    });
    *host.resumed.lock().unwrap() = Some(ready.clone());
    app.reconcile_once().unwrap();
    assert_eq!(app.query_lifecycle("8", &session).unwrap().phase, PublicSessionPhase::Running);
    (session, ready, request)
}

#[test]
fn resumed_continued_loss_is_durable_and_old_ready_cannot_revive_it() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let (session, ready, request) = complete_resumed_continued(&app, &host);
    let before = app.query_lifecycle("8", &session).unwrap();
    let mut lost = ready.clone();
    lost.readiness = CurrentResumeReadiness::StateLost;
    *host.resumed.lock().unwrap() = Some(lost);
    let saved = app.query_lifecycle("8", &session).unwrap();
    assert_eq!(saved.phase, PublicSessionPhase::StateLost);
    assert_eq!(saved.unload_outcome, before.unload_outcome);
    assert_eq!(app.unload_lifecycle("8", &session, &request).unwrap().key, saved.operation);
    *host.resumed.lock().unwrap() = Some(ready.clone());
    assert_eq!(app.query_lifecycle("8", &session).unwrap(), saved);
    let snapshot = authority.snapshot().unwrap();
    // The original CAS itself also refuses the old successful observation.
    assert!(sandproof_center_service::AuthorityPort::accept_lifecycle_resume_observation(&mut authority.clone(), &ready).is_err());
    assert_eq!(authority.snapshot().unwrap().center, snapshot.center);
    let limit = snapshot.required_block_bytes();
    drop(app);
    drop(authority);
    let (ledger, _) = super::super::RedisLedger::open(
        super::super::engine::endpoint(proxy.address), Engine::config(limit), fixture.lease(),
    ).unwrap();
    let reopened = SharedBusinessAuthority::new(super::super::CenterAuthority::attach(ledger).unwrap());
    let (app, host, _) = application(&reopened);
    *host.resumed.lock().unwrap() = Some(ready);
    assert_eq!(app.query_lifecycle("8", &session).unwrap(), saved);
    assert_eq!(app.snapshot().unwrap(), snapshot.center);
}

#[test]
fn destroy_during_resumed_continued_loss_observation_wins_the_original_redis_cas() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let (session, mut lost, _) = complete_resumed_continued(&app, &host);
    lost.readiness = CurrentResumeReadiness::StateLost;
    *host.resumed.lock().unwrap() = Some(lost);
    let (observed, release) = host.block_next(session.clone());
    let querying = Arc::clone(&app);
    let queried = session.clone();
    let query = std::thread::spawn(move || querying.query_lifecycle("8", &queried));
    await_observation(&observed);
    let destroyed = app.destroy_lifecycle("8", session.clone(), &PublicDestroyRequest {
        window: 1.try_into().unwrap(), sequence: 5.try_into().unwrap(), expected_version: 4,
    }).unwrap();
    release.complete();
    assert!(query.join().unwrap().is_err());
    let current = app.query_lifecycle("8", &session).unwrap();
    assert_eq!(current.phase, PublicSessionPhase::Terminating);
    assert_eq!(current.operation, destroyed.key);
    assert!(current.unload_outcome.is_none());
}

#[test]
fn continued_query_does_not_publish_stale_or_expired_ready() {
    for expired in [false, true] {
        let fixture = Engine::new();
        let proxy = reply_loss::ReplyLoss::new(fixture.address);
        let authority = lifecycle_public::setup(&fixture, &proxy);
        let (app, host, clock) = application(&authority);
        let (session, end) = complete_continued(&app, &host);
        if expired {
            clock.0.store(end, Ordering::SeqCst);
        } else {
            *host.observed_at_override.lock().unwrap() = Some(0);
        }
        let result = app.query_lifecycle("8", &session);
        assert!(match result {
            Ok(view) => view.phase != PublicSessionPhase::Running,
            Err(_) => true,
        });
    }
}

#[test]
fn continued_query_cannot_overwrite_a_destroy_accepted_during_observation() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let (session, _) = complete_continued(&app, &host);
    let (observed, release) = host.block_next(session.clone());
    let querying = Arc::clone(&app);
    let queried = session.clone();
    let query = std::thread::spawn(move || querying.query_lifecycle("8", &queried));
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    let destroyed = app
        .destroy_lifecycle(
            "8",
            session.clone(),
            &PublicDestroyRequest {
                window: 1.try_into().unwrap(),
                sequence: 3.try_into().unwrap(),
                expected_version: 2,
            },
        )
        .unwrap();
    release.complete();
    assert!(query.join().unwrap().is_err());
    let current = app.query_lifecycle("8", &session).unwrap();
    assert_eq!(current.phase, PublicSessionPhase::Terminating);
    assert_eq!(current.operation, destroyed.key);
    assert!(current.unload_outcome.is_none());
}

#[test]
fn pending_before_unload_acceptance_waits_for_original_ready_without_new_key() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let created = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &created.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    host.observation_budgets.lock().unwrap().clear();
    host.pending_once.store(true, Ordering::SeqCst);
    let request = racing_request(2, 1);
    let result = app.unload_lifecycle("8", &created.session, &request);
    assert!(
        result.is_ok(),
        "original transient Pending must not reject unload: {result:?}"
    );
    let accepted = result.unwrap();
    assert_eq!(accepted.accepted_version, 2);
    assert_eq!(
        app.unload_lifecycle("8", &created.session, &request)
            .unwrap(),
        accepted
    );
    let budgets = host.observation_budgets.lock().unwrap();
    assert_eq!(budgets.len(), 2);
    assert!(budgets[1] < budgets[0]);
    assert_eq!(
        app.snapshot()
            .unwrap()
            .lifecycle_session(&"8".try_into().unwrap(), &created.session)
            .unwrap()
            .operations()
            .len(),
        2
    );
}

#[test]
fn pending_unload_exhausts_original_deadline_without_accepting_or_resetting_budget() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let created = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &created.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    let before = app.snapshot().unwrap();
    host.observation_budgets.lock().unwrap().clear();
    host.ready.store(false, Ordering::SeqCst);
    assert_eq!(
        app.unload_lifecycle("8", &created.session, &racing_request(2, 1)),
        Err(PortError::Busy)
    );
    let budgets = host.observation_budgets.lock().unwrap();
    assert!(budgets.len() > 1);
    assert!(budgets.windows(2).all(|pair| pair[1] < pair[0]));
    assert_eq!(app.snapshot().unwrap(), before);
}

#[test]
fn unload_transport_busy_is_not_treated_as_authenticated_pending() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let created = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &created.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    let before = app.snapshot().unwrap();
    host.observation_budgets.lock().unwrap().clear();
    *host.observation_error_once.lock().unwrap() = Some(PortError::Busy);
    assert_eq!(
        app.unload_lifecycle("8", &created.session, &racing_request(2, 1)),
        Err(PortError::Busy)
    );
    assert_eq!(host.observation_budgets.lock().unwrap().len(), 1);
    assert_eq!(app.snapshot().unwrap(), before);
}

#[test]
fn unload_observation_deadline_is_busy_only_after_confirmed_replay_absence() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let created = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &created.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    let before = app.snapshot().unwrap();
    host.observation_budgets.lock().unwrap().clear();
    host.ready.store(false, Ordering::SeqCst);
    let (entered, release) = host.block_next(created.session.clone());
    let late_app = Arc::clone(&app);
    let late_session = created.session;
    let late = std::thread::spawn(move || {
        late_app.unload_lifecycle("8", &late_session, &racing_request(2, 1))
    });
    await_observation(&entered);
    let budget = *host.observation_budgets.lock().unwrap().last().unwrap();
    exhaust_observation_budget(budget);
    release.complete();
    assert_eq!(late.join().unwrap(), Err(PortError::Busy));
    assert_eq!(host.observation_budgets.lock().unwrap().len(), 1);
    assert_eq!(app.snapshot().unwrap(), before);
}

#[test]
fn unload_observation_deadline_replays_the_same_full_acceptance() {
    let (result, accepted) = raced_unload_with_deadline(racing_request(2, 1), false, true);
    assert_eq!(result, Ok(accepted));
}

#[test]
fn unload_observation_deadline_keeps_unknown_replay_fenced() {
    let (result, _) = raced_unload_with_deadline(racing_request(2, 1), true, true);
    assert_eq!(result, Err(PortError::OutcomeUnknown));
}

#[test]
fn unload_host_unknown_is_not_relabelled_as_a_readonly_deadline() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let created = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &created.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    let before = app.snapshot().unwrap();
    host.observation_budgets.lock().unwrap().clear();
    *host.observation_error_once.lock().unwrap() = Some(PortError::OutcomeUnknown);
    assert_eq!(
        app.unload_lifecycle("8", &created.session, &racing_request(2, 1)),
        Err(PortError::OutcomeUnknown)
    );
    assert_eq!(host.observation_budgets.lock().unwrap().len(), 1);
    assert_eq!(app.snapshot().unwrap(), before);
}
