//! The next public Unload observes the actual resumed origin in the original Redis CAS.
use super::*;

fn next_request() -> PublicUnloadRequest {
    PublicUnloadRequest {
        window: 1.try_into().unwrap(),
        sequence: 5.try_into().unwrap(),
        expected_version: 4,
    }
}

fn accept_next(
    app: &Application,
    authority: &SharedBusinessAuthority,
    session: &SessionId,
    request: &PublicUnloadRequest,
    before: &sandproof_center_core::CenterState,
) -> sandproof_protocol::lifecycle::PublicAcceptance {
    let result = app.unload_lifecycle("8", session, request);
    if result.is_err() {
        // Preserve the original failure's transaction boundary as evidence:
        // the readonly wrong-source rejection has not appended an acceptance.
        assert_eq!(app.snapshot().unwrap(), *before);
        assert!(authority.replay_public_unload(&"8".try_into().unwrap(), session, request).unwrap().is_none());
        assert_eq!(result, Err(PortError::Busy));
    }
    result.expect("a fresh exact Resume observation must permit the next public Unload")
}

#[test]
fn next_unload_after_resumed_continuation_accepts_only_fresh_resume_and_preserves_history() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    // Five nonterminal inputs still reserve a sixth position for Destroy.
    let authority = lifecycle_public::setup_with_operation_limit(&fixture, &proxy, 6);
    let (app, host, _) = application(&authority);
    let (session, ready, _) = complete_resumed_continued(&app, &host);
    // The original Create belongs to an older execution, as on the real Host.
    // Returning a fresh Create here would hide the routing error.
    host.create_superseded.store(true, Ordering::SeqCst);
    let before = app.snapshot().unwrap();
    let old_create_queries = host.create_requests.lock().unwrap().len();
    let old_resume_queries = host.resume_requests.lock().unwrap().len();
    let request = next_request();
    let accepted = accept_next(&app, &authority, &session, &request, &before);
    assert_eq!(accepted.accepted_version, 5);
    assert_eq!(host.create_requests.lock().unwrap().len(), old_create_queries);
    assert_eq!(host.resume_requests.lock().unwrap().len(), old_resume_queries + 1);
    assert_eq!(host.resume_requests.lock().unwrap().last(), Some(&ready.request));
    let after = app.snapshot().unwrap();
    let owner = "8".try_into().unwrap();
    let prior = before.lifecycle_session(&owner, &session).unwrap();
    let current = after.lifecycle_session(&owner, &session).unwrap();
    assert_eq!(&current.operations()[..prior.operations().len()], prior.operations());
    assert_eq!(current.operations().len(), prior.operations().len() + 1);
    assert_eq!(current.current_execution_origin(), Some(&ready.request));
    assert_eq!(current.phase(), SessionPhase::Unloading);
    let queries = host.resume_requests.lock().unwrap().len();
    assert_eq!(app.unload_lifecycle("8", &session, &request).unwrap(), accepted);
    assert_eq!(host.resume_requests.lock().unwrap().len(), queries, "same complete input replays before observing again");
    let snapshot = authority.snapshot().unwrap();
    let limit = snapshot.required_block_bytes();
    drop(app);
    drop(authority);
    let (ledger, _) = super::super::super::RedisLedger::open(
        super::super::super::engine::endpoint(proxy.address), Engine::config(limit), fixture.lease(),
    ).unwrap();
    let reopened = SharedBusinessAuthority::new(super::super::super::CenterAuthority::attach(ledger).unwrap());
    let (app, _, _) = application(&reopened);
    assert_eq!(app.unload_lifecycle("8", &session, &request).unwrap(), accepted);
    assert_eq!(app.snapshot().unwrap(), snapshot.center);
}

#[test]
fn next_unload_after_resumed_continuation_rejects_unknown_changed_and_destroy_races_without_acceptance() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup_with_operation_limit(&fixture, &proxy, 6);
    let (app, host, _) = application(&authority);
    let (session, ready, _) = complete_resumed_continued(&app, &host);
    host.create_superseded.store(true, Ordering::SeqCst);
    let before = app.snapshot().unwrap();
    *host.observation_error_once.lock().unwrap() = Some(PortError::OutcomeUnknown);
    assert_eq!(app.unload_lifecycle("8", &session, &next_request()), Err(PortError::OutcomeUnknown));
    assert_eq!(app.snapshot().unwrap(), before);
    let mut changed = ready.clone();
    changed.current_request = changed.request.clone();
    *host.resumed.lock().unwrap() = Some(changed);
    assert_eq!(app.unload_lifecycle("8", &session, &next_request()), Err(PortError::Busy));
    assert_eq!(app.snapshot().unwrap(), before);
    *host.resumed.lock().unwrap() = Some(ready);
    let (observed, release) = host.block_next(session.clone());
    let calling = Arc::clone(&app);
    let called = session.clone();
    let pending = std::thread::spawn(move || calling.unload_lifecycle("8", &called, &next_request()));
    await_observation(&observed);
    let destroyed = app.destroy_lifecycle("8", session.clone(), &PublicDestroyRequest {
        window: 1.try_into().unwrap(), sequence: 6.try_into().unwrap(), expected_version: 4,
    }).unwrap();
    release.complete();
    assert!(matches!(pending.join().unwrap(), Err(PortError::Conflict | PortError::Busy)));
    let current = app.query_lifecycle("8", &session).unwrap();
    assert_eq!(current.phase, PublicSessionPhase::Terminating);
    assert_eq!(current.operation, destroyed.key);
    let after = app.snapshot().unwrap();
    let record = after.lifecycle_session(&"8".try_into().unwrap(), &session).unwrap();
    assert_eq!(record.operations().len(), 5, "only Destroy was appended; no next Unload acceptance");
    assert!(matches!(record.operations().last().unwrap().intent().request.operation, LifecycleOperation::Destroy {}));
}
