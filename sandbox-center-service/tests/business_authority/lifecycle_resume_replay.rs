//! Public Resume HTTP routing, original Redis commit/recovery and controlled Host facts.
use super::{decoded_resume_terminal::durable_block, *};
use axum::http::StatusCode;
use sandproof_api::admission::RequestAdmission;
use sandproof_center_core::LifecycleSubmission;
use sandproof_center_service::{AuthorityPort, CenterPort};
use sandproof_protocol::lifecycle::{OperationKey, PublicAcceptance, UnloadedFacts};
use tokio::sync::Barrier;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_same_full_resume_uses_one_redis_acceptance_even_after_commit_reply_loss() {
    for lose_commit_reply in [false, true] {
        let engine = Engine::new();
        let proxy = reply_loss::ReplyLoss::new(engine.address);
        let authority = lifecycle_public::setup(&engine, &proxy);
        let (app, host, clock) = application(&authority);
        let (session, _, unloaded) = accept_unload(&app);
        let source = unloaded.outcome.unloaded().unwrap().clone();
        *host.unloaded.lock().unwrap() = Some(unloaded);
        app.reconcile_once().unwrap();
        let owner = "8".try_into().unwrap();
        let unloaded_view = authority.query_public_lifecycle(&owner, &session).unwrap();
        assert_eq!(unloaded_view.phase, PublicSessionPhase::Unloaded);
        let recordings = host.recordings.load(Ordering::SeqCst);
        let input = PublicResumeRequest {
            window: unloaded_view.operation.window,
            sequence: 3.try_into().unwrap(),
            expected_version: unloaded_view.current_version,
        };
        let key = OperationKey {
            deployment: unloaded_view.operation.deployment.clone(),
            owner: owner.clone(),
            window: input.window,
            sequence: input.sequence,
        };
        let before = authority.snapshot().unwrap();
        let (once, intent) = before
            .center
            .accept_lifecycle(
                &owner,
                &LifecycleSubmission::Resume {
                    key: key.clone(),
                    session: session.clone(),
                    expected_version: input.expected_version,
                },
                clock.0.load(Ordering::SeqCst),
            )
            .unwrap();
        let expected = PublicAcceptance {
            session: session.clone(),
            key,
            accepted_version: input.expected_version + 1,
        };
        assert_resume_source(&intent.request.operation, &source);
        let expected_state = before.with_center(once);
        let baseline_block = durable_block(&engine);

        let router = sandproof_api::lifecycle::router_with_admission(
            Arc::new(CenterPort::new(Arc::clone(&app))),
            lifecycle_public::public_credentials(),
            Arc::new(RequestAdmission::default()),
        )
        .unwrap();
        let path = format!(
            "/sandproof/lifecycle/sessions/{}/resume",
            session.as_protocol_id().as_str()
        );
        let body = serde_json::to_value(&input).unwrap();
        if lose_commit_reply {
            // Keep the preceding read reaffirmation. Lose the reply to the
            // actual Resume CAS after Redis has applied it.
            proxy.lose_after(1);
        }
        let ready = Barrier::new(2);
        let (left, right) = tokio::join!(
            send_after_barrier(&router, &path, &body, &ready),
            send_after_barrier(&router, &path, &body, &ready),
        );
        let committed = durable_block(&engine);
        assert_ne!(committed.1, baseline_block.1);
        if lose_commit_reply {
            for unknown in [left, right] {
                assert_eq!(unknown.status(), StatusCode::INTERNAL_SERVER_ERROR);
            }
            assert_eq!(committed.0, baseline_block.0 + 2);
            assert_eq!(
                authority.query_public_lifecycle(&owner, &session),
                Err(PortError::OutcomeUnknown)
            );
            assert_eq!(durable_block(&engine), committed);
            AuthorityPort::recover(&mut authority.clone()).unwrap();
        } else {
            for accepted in [left, right] {
                assert_eq!(accepted.status(), StatusCode::ACCEPTED);
                assert_eq!(
                    lifecycle_public::read_public::<PublicAcceptance>(accepted).await,
                    expected
                );
            }
        }

        // This clock cannot be converted by the application's Clock adapter.
        // A same-key replay or full-input conflict must use the durable decision.
        clock.0.store(u64::MAX, Ordering::SeqCst);
        assert_replay_and_conflict(&router, &path, body, &expected).await;
        let state = authority.snapshot().unwrap();
        lifecycle_public::assert_same_business_state(&state, &expected_state);
        assert_eq!(state.center.pending_lifecycle(), vec![intent]);
        assert_eq!(durable_block(&engine).1, committed.1);
        assert_eq!(host.recordings.load(Ordering::SeqCst), recordings);
        assert!(host.resume_requests.lock().unwrap().is_empty());
    }
}

fn assert_resume_source(operation: &LifecycleOperation, source: &UnloadedFacts) {
    let LifecycleOperation::Resume {
        source_incarnation,
        capture,
        manifest_digest,
        ..
    } = operation
    else {
        panic!("one original Resume intent required");
    };
    assert_eq!(*source_incarnation, source.source_incarnation);
    assert_eq!(*capture, source.capture);
    assert_eq!(*manifest_digest, source.manifest_digest);
}

async fn send_after_barrier(
    router: &axum::Router,
    path: &str,
    body: &serde_json::Value,
    ready: &Barrier,
) -> axum::response::Response {
    ready.wait().await;
    send_public_resume(router, path, body).await
}

async fn assert_replay_and_conflict(
    router: &axum::Router,
    path: &str,
    mut body: serde_json::Value,
    expected: &PublicAcceptance,
) {
    let replay = send_public_resume(router, path, &body).await;
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    assert_eq!(
        &lifecycle_public::read_public::<PublicAcceptance>(replay).await,
        expected
    );
    body["expected_version"] = expected.accepted_version.into();
    assert_eq!(
        send_public_resume(router, path, &body).await.status(),
        StatusCode::CONFLICT
    );
}

async fn send_public_resume(
    router: &axum::Router,
    path: &str,
    body: &serde_json::Value,
) -> axum::response::Response {
    lifecycle_public::send_public(
        router,
        "POST",
        path,
        lifecycle_public::OWNER_KEY,
        Some(body),
    )
    .await
}
