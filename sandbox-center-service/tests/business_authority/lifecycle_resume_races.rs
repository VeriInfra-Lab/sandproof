//! Original application entry points and Redis; controlled physical facts.
//! Hold the existing Host reply, without adding a production synchronization point.
use super::*;

#[derive(Clone, Copy)]
enum Entry {
    Background,
    Query,
}

enum Outcome {
    Background(Result<usize, PortError>),
    Query(Result<sandproof_protocol::lifecycle::PublicSessionView, PortError>),
}

fn resume_reply(entry: Entry, destroy: bool) {
    let engine = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(engine.address);
    let authority = lifecycle_public::setup(&engine, &proxy);
    let (app, host, _) = application(&authority);
    let (session, ready) = decoded_resume_terminal::pending_resume(&app, &host);
    let owner = "8".try_into().unwrap();
    assert!(ready.matches_request(&ready.request));
    assert_eq!(ready.current_request, ready.request);
    *host.resumed.lock().unwrap() = Some(ready.clone());
    let initial = app.snapshot().unwrap();
    assert_eq!(initial.pending_lifecycle(), vec![ready.request.clone()]);
    let prior_observations = host.resume_requests.lock().unwrap().len();
    let (entered, release) = host.block_next(session.clone());
    let worker = Arc::clone(&app);
    let target = session.clone();
    // The original reply deadline is created AFTER this instant. Finishing
    // before this stricter bound rules out a timeout masquerading as rejection.
    // Keep the fixture's original 200ms; do not retry a slow/failed case.
    let started = Instant::now();
    let call = std::thread::spawn(move || match entry {
        Entry::Background => Outcome::Background(worker.reconcile_once()),
        Entry::Query => Outcome::Query(worker.query_lifecycle("8", &target)),
    });
    await_observation(&entered);
    assert_eq!(
        &host.resume_requests.lock().unwrap()[prior_observations..],
        std::slice::from_ref(&ready.request)
    );
    let termination = destroy.then(|| {
        app.destroy_lifecycle(
            "8",
            session.clone(),
            &PublicDestroyRequest {
                window: 1.try_into().unwrap(),
                sequence: 4.try_into().unwrap(),
                expected_version: 3,
            },
        )
        .unwrap()
    });
    let before_release = app.snapshot().unwrap();
    let record = before_release.lifecycle_session(&owner, &session).unwrap();
    assert_eq!(
        record.phase(),
        if destroy {
            SessionPhase::Terminating
        } else {
            SessionPhase::Resuming
        }
    );
    // Host copied this exact reply before entering its existing barrier;
    // neither Destroy nor the fixture rewrites it to a newer operation.
    assert_eq!(*host.resumed.lock().unwrap(), Some(ready.clone()));
    release.complete();
    let outcome = call.join().unwrap();
    assert!(
        started.elapsed() < REPLY_TIMEOUT,
        "race exceeded original reply budget; cannot count timeout as stale rejection"
    );
    assert_outcome(outcome, destroy, &ready);
    let after = app.snapshot().unwrap();
    let current = after.lifecycle_session(&owner, &session).unwrap();
    assert_eq!(current.operations()[2].intent(), &ready.request);
    assert_eq!(
        &host.resume_requests.lock().unwrap()[prior_observations..],
        std::slice::from_ref(&ready.request)
    );
    if let Some(termination) = termination {
        assert_eq!(after, before_release);
        assert_eq!(current.phase(), SessionPhase::Terminating);
        assert_eq!(
            current.current_version(),
            Some(termination.accepted_version)
        );
        assert!(!current.released());
        let pending = after.pending_lifecycle();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request.binding.key, termination.key);
        assert!(matches!(
            pending[0].request.operation,
            LifecycleOperation::Destroy {}
        ));
        let public = app.query_lifecycle("8", &session).unwrap();
        assert_eq!(public.phase, PublicSessionPhase::Terminating);
        assert_eq!(public.operation, termination.key);
    } else {
        assert_eq!(current.phase(), SessionPhase::Running);
        assert_eq!(current.current_version(), Some(3));
        assert_eq!(current.operations().len(), 3);
        assert!(after.pending_lifecycle().is_empty());
        let CurrentResumeReadiness::Observed { facts } = &ready.readiness else {
            unreachable!()
        };
        assert_eq!(current.last_running(), Some(facts));
    }
}

fn assert_outcome(outcome: Outcome, destroy: bool, ready: &ResumeObservation) {
    match outcome {
        Outcome::Background(result) => assert_eq!(result, Ok(usize::from(!destroy))),
        Outcome::Query(result) => {
            if destroy {
                assert_eq!(result, Err(PortError::Conflict));
            } else {
                let view = result.unwrap();
                assert_eq!(view.phase, PublicSessionPhase::Running);
                assert_eq!(view.operation, ready.current_request.request.binding.key);
                assert_eq!(view.current_version, ready.current_request.accepted_version);
            }
        }
    }
}

#[test]
fn background_accepts_current_resume_reply() {
    resume_reply(Entry::Background, false);
}

#[test]
fn background_cannot_revive_resume_after_destroy_during_observation() {
    resume_reply(Entry::Background, true);
}

#[test]
fn query_accepts_current_resume_reply() {
    resume_reply(Entry::Query, false);
}

#[test]
fn query_cannot_revive_resume_after_destroy_during_observation() {
    resume_reply(Entry::Query, true);
}
