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
