//! Public SDK renewal and lifecycle calls use the original Redis authority.
//! Host renewal application/confirmation is a controlled, fully bound provider fact.
use super::*;

struct Case {
    _engine: Engine,
    _proxy: reply_loss::ReplyLoss,
    authority: SharedBusinessAuthority,
    app: Arc<Application>,
    host: Host,
    clock: Arc<Time>,
    session: SessionId,
    ready: ResumeObservation,
    source_end: u64,
    effective_end: u64,
}

impl Case {
    fn new() -> Self {
        let engine = Engine::new();
        let proxy = reply_loss::ReplyLoss::new(engine.address);
        let authority = lifecycle_public::setup_with_operation_limit(&engine, &proxy, 8);
        let (app, host, clock) = application(&authority);
        let mut create = lifecycle_public::request();
        create.parameters.timeout_ms = 1000;
        let (session, _, unloaded) = accept_unload_with_create(&app, &create);
        let source = unloaded.outcome.unloaded().unwrap().clone();
        *host.unloaded.lock().unwrap() = Some(unloaded);
        app.reconcile_once().unwrap();
        app.resume_lifecycle("8", &session, &PublicResumeRequest {
            window: 1.try_into().unwrap(), sequence: 3.try_into().unwrap(), expected_version: 2,
        }).unwrap();
        let snapshot = app.snapshot().unwrap();
        let origin = snapshot.lifecycle_session(&"8".try_into().unwrap(), &session).unwrap()
            .current_execution_origin().unwrap().clone();
        let ready = ResumeObservation {
            request: origin.clone(), current_request: origin,
            readiness: CurrentResumeReadiness::Observed { facts: CurrentReadyFacts {
                started_at_ms: source.started_at_ms, end_at_ms: source.end_at_ms,
                max_end_at_ms: source.max_end_at_ms, startup_deadline_ms: source.startup_deadline_ms,
                observed_at_ms: 100,
            } },
        };
        *host.resumed.lock().unwrap() = Some(ready);
        host.create_superseded.store(true, Ordering::SeqCst);
        assert_eq!(app.query_lifecycle("8", &session).unwrap().phase, PublicSessionPhase::Running);
        host.apply_renewals.store(true, Ordering::SeqCst);
        app.set_timeout("8", session.as_protocol_id().as_str(), 2).unwrap();
        let snapshot = app.snapshot().unwrap();
        let record = snapshot.lifecycle_session(&"8".try_into().unwrap(), &session).unwrap();
        assert!(matches!(record.operations().last().unwrap().intent().request.operation, LifecycleOperation::Renew { .. }));
        let effective_end = record.current_running().unwrap().end_at_ms;
        assert!(effective_end > source.end_at_ms);
        app.unload_lifecycle("8", &session, &PublicUnloadRequest {
            window: 1.try_into().unwrap(), sequence: 4.try_into().unwrap(), expected_version: 4,
        }).unwrap();
        let snapshot = app.snapshot().unwrap();
        let record = snapshot.lifecycle_session(&"8".try_into().unwrap(), &session).unwrap();
        let current = record.operations().last().unwrap().intent().clone();
        let mut ready = host.resumed.lock().unwrap().clone().unwrap();
        ready.current_request = current.clone();
        let CurrentResumeReadiness::Observed { facts } = &ready.readiness else { unreachable!() };
        assert_eq!(facts.end_at_ms, effective_end);
        *host.unloaded.lock().unwrap() = Some(UnloadObservation {
            request: current, outcome: UnloadOutcome::OriginalContinued { facts: facts.clone() },
        });
        *host.resumed.lock().unwrap() = Some(ready.clone());
        app.reconcile_once().unwrap();
        clock.0.store(source.end_at_ms + 1, Ordering::SeqCst);
        let CurrentResumeReadiness::Observed { facts } = &mut ready.readiness else { unreachable!() };
        facts.observed_at_ms = source.end_at_ms + 1;
        *host.resumed.lock().unwrap() = Some(ready.clone());
        assert_eq!(app.query_lifecycle("8", &session).unwrap().phase, PublicSessionPhase::Running);
        Self { _engine: engine, _proxy: proxy, authority, app, host, clock, session, ready,
            source_end: source.end_at_ms, effective_end }
    }

    fn next(&self) -> PublicUnloadRequest {
        PublicUnloadRequest { window: 1.try_into().unwrap(), sequence: 5.try_into().unwrap(), expected_version: 5 }
    }
}

#[test]
fn renewed_continued_public_reunload_preserves_effective_lifetime_after_source_deadline() {
    let case = Case::new();
    let before = case.app.snapshot().unwrap();
    let result = case.app.unload_lifecycle("8", &case.session, &case.next());
    if result.is_err() {
        assert_eq!(result, Err(PortError::Conflict));
        assert_eq!(case.app.snapshot().unwrap(), before);
        assert!(case.authority.replay_public_unload(&"8".try_into().unwrap(), &case.session, &case.next()).unwrap().is_none());
    }
    let accepted = result.expect("renewed continuation must preserve its current effective end");
    assert_eq!(accepted.accepted_version, 6);
    let after = case.app.snapshot().unwrap();
    let owner = "8".try_into().unwrap();
    let previous = before.lifecycle_session(&owner, &case.session).unwrap();
    let current = after.lifecycle_session(&owner, &case.session).unwrap();
    assert_eq!(&current.operations()[..previous.operations().len()], previous.operations());
    assert_eq!(current.last_running().unwrap().end_at_ms, case.effective_end);
    assert_ne!(case.effective_end, case.source_end);
    assert_eq!(case.app.unload_lifecycle("8", &case.session, &case.next()).unwrap(), accepted);
}

#[test]
fn renewed_continued_public_reunload_rejects_expiry_and_wrong_full_observation() {
    for fault in 0..4 {
        let case = Case::new();
        let before = case.app.snapshot().unwrap();
        let mut view = case.ready.clone();
        match fault {
            0 => { case.clock.0.store(case.effective_end, Ordering::SeqCst); }
            1 => view.current_request.accepted_version += 1,
            2 => view.current_request.request.binding.key.owner = "9".try_into().unwrap(),
            _ => view.current_request.request.binding.session = "other-session".try_into().unwrap(),
        }
        *case.host.resumed.lock().unwrap() = Some(view);
        assert_eq!(case.app.unload_lifecycle("8", &case.session, &case.next()), Err(PortError::Busy));
        assert_eq!(case.app.snapshot().unwrap(), before);
        assert!(case.authority.replay_public_unload(&"8".try_into().unwrap(), &case.session, &case.next()).unwrap().is_none());
    }
}

#[test]
fn renewed_continued_public_reunload_late_observation_cannot_override_expiry_or_destroy() {
    for expire in [false, true] {
        let case = Case::new();
        let (observed, release) = case.host.block_next(case.session.clone());
        let app = Arc::clone(&case.app);
        let session = case.session.clone();
        let request = case.next();
        let pending = std::thread::spawn(move || app.unload_lifecycle("8", &session, &request));
        await_observation(&observed);
        if expire { case.clock.0.store(case.effective_end, Ordering::SeqCst); }
        else { case.app.destroy_lifecycle("8", case.session.clone(), &PublicDestroyRequest {
            window: 1.try_into().unwrap(), sequence: 6.try_into().unwrap(), expected_version: 5,
        }).unwrap(); }
        release.complete();
        assert!(matches!(pending.join().unwrap(), Err(PortError::Busy | PortError::Conflict)));
        assert!(case.authority.replay_public_unload(&"8".try_into().unwrap(), &case.session, &case.next()).unwrap().is_none());
    }
}
