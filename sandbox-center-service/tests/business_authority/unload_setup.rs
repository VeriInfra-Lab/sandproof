//! Preparing an Unloaded test is separate from asserting a one-call result.
use super::*;
use sandproof_protocol::lifecycle::PublicAcceptance;

fn request() -> PublicUnloadRequest {
    racing_request(2, 1)
}

/// At most two public calls with exactly the same input and original budgets.
/// A second call is allowed only after a known Busy and a confirmed, completely
/// unchanged CenterState containing just the original Running Create. Unknown
/// storage/host outcomes and every other error return without another call.
pub(super) fn accept(
    app: &Application,
    session: &SessionId,
    request: &PublicUnloadRequest,
) -> Result<PublicAcceptance, PortError> {
    let before = app.snapshot()?;
    let original = before
        .lifecycle_session(&"8".try_into().unwrap(), session)
        .unwrap();
    assert_eq!(original.phase(), SessionPhase::Running);
    assert_eq!(original.operations().len(), 1);
    let create = original.operations()[0].intent();
    assert!(matches!(
        create.request.operation,
        LifecycleOperation::Create { .. }
    ));
    assert_eq!(create.accepted_version, request.expected_version);
    let result = app.unload_lifecycle("8", session, request);
    if result != Err(PortError::Busy) {
        return result;
    }
    // This confirmed full state includes operation receipts, pending work,
    // request windows, identity frontiers and resource/material accounting.
    // Busy alone is not evidence that nothing was accepted.
    assert_eq!(
        app.snapshot()?,
        before,
        "Unload setup Busy changed authority"
    );
    app.unload_lifecycle("8", session, request)
}

fn running(app: &Application) -> SessionId {
    let created = app
        .create_lifecycle("8", &lifecycle_public::request())
        .unwrap();
    assert_eq!(
        app.query_lifecycle("8", &created.session).unwrap().phase,
        PublicSessionPhase::Running
    );
    created.session
}

fn assert_single_unload(
    app: &Application,
    session: &SessionId,
    request: &PublicUnloadRequest,
    accepted: &PublicAcceptance,
) {
    assert_eq!(accepted.session, *session);
    assert_eq!(accepted.accepted_version, 2);
    assert_eq!(accepted.key.window, request.window);
    assert_eq!(accepted.key.sequence, request.sequence);
    let state = app.snapshot().unwrap();
    let record = state
        .lifecycle_session(&"8".try_into().unwrap(), session)
        .unwrap();
    assert_eq!(record.phase(), SessionPhase::Unloading);
    assert_eq!(record.operations().len(), 2);
    assert_eq!(
        record.operations()[1].intent().request.binding.key,
        accepted.key
    );
    assert_eq!(
        app.unload_lifecycle("8", session, request),
        Ok(accepted.clone())
    );
    assert_eq!(app.snapshot().unwrap(), state);
}

#[test]
fn prepared_unload_ready_uses_one_original_observation() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let session = running(&app);
    host.observation_budgets.lock().unwrap().clear();
    let request = request();
    let accepted = accept(&app, &session, &request).unwrap();
    assert_single_unload(&app, &session, &request, &accepted);
    assert_eq!(host.observation_budgets.lock().unwrap().len(), 1);
}

#[test]
fn prepared_unload_known_busy_confirms_absence_before_same_request() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let session = running(&app);
    host.observation_budgets.lock().unwrap().clear();
    *host.observation_error_once.lock().unwrap() = Some(PortError::Busy);
    let request = request();
    let accepted = accept(&app, &session, &request).unwrap();
    assert_single_unload(&app, &session, &request, &accepted);
    assert_eq!(host.observation_budgets.lock().unwrap().len(), 2);
}

#[test]
fn prepared_unload_late_ready_uses_same_request_after_confirmed_busy() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let session = running(&app);
    let before = app.snapshot().unwrap();
    let original = before
        .lifecycle_session(&"8".try_into().unwrap(), &session)
        .unwrap()
        .operations()[0]
        .intent()
        .clone();
    host.observation_budgets.lock().unwrap().clear();
    let (entered, release) = host.block_next(session.clone());
    let (notice, budget) = mpsc::channel();
    host.blocked
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .create_reply_budget = Some(notice);
    let run = Arc::clone(&app);
    let target = session.clone();
    let prepared = std::thread::spawn(move || accept(&run, &target, &request()));
    await_observation(&entered);
    let (blocked_intent, sampled_at, remaining) =
        budget.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(blocked_intent, original);
    assert!(!remaining.is_zero() && remaining <= Duration::from_millis(200));
    assert_eq!(app.snapshot().unwrap(), before);
    let expired_by = sampled_at.checked_add(remaining).unwrap();
    while Instant::now() < expired_by {
        std::thread::park_timeout(expired_by.saturating_duration_since(Instant::now()));
    }
    release.complete();
    let accepted = prepared.join().unwrap().unwrap();
    assert_single_unload(&app, &session, &request(), &accepted);
    assert_eq!(host.observation_budgets.lock().unwrap().len(), 2);
}

#[test]
fn prepared_unload_unknown_stops_without_a_second_call_or_new_key() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = lifecycle_public::setup(&fixture, &proxy);
    let (app, host, _) = application(&authority);
    let session = running(&app);
    let before = app.snapshot().unwrap();
    host.observation_budgets.lock().unwrap().clear();
    *host.observation_error_once.lock().unwrap() = Some(PortError::OutcomeUnknown);
    assert_eq!(
        accept(&app, &session, &request()),
        Err(PortError::OutcomeUnknown)
    );
    assert_eq!(host.observation_budgets.lock().unwrap().len(), 1);
    assert_eq!(app.snapshot().unwrap(), before);
}
