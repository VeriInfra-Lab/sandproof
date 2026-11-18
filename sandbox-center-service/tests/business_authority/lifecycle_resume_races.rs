//! Original application entry points and Redis; controlled physical facts.
//! Hold the existing Host reply, without adding a production synchronization point.
use super::{decoded_resume_terminal::durable_block, *};
use sandproof_center_service::business::RegistryMetadata;
use sandproof_center_store::business::BusinessEnvelope;
use sandproof_protocol::lifecycle::{ExpectedSession, PublicSessionView};
use std::net::SocketAddrV4;

#[derive(Clone, Copy)]
enum Entry {
    Background,
    Query,
}

enum Outcome {
    Background(Result<usize, PortError>),
    Query(Result<PublicSessionView, PortError>),
}

fn observe_resume(
    entry: Entry,
    app: &Arc<Application>,
    session: &SessionId,
) -> std::thread::JoinHandle<Outcome> {
    let worker = Arc::clone(app);
    let target = session.clone();
    std::thread::spawn(move || match entry {
        Entry::Background => Outcome::Background(worker.reconcile_once()),
        Entry::Query => Outcome::Query(worker.query_lifecycle("8", &target)),
    })
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
    // The original reply deadline is created AFTER this instant. Finishing
    // before this stricter bound rules out a timeout masquerading as rejection.
    // Keep the fixture's original 200ms; do not retry a slow/failed case.
    let started = Instant::now();
    let call = observe_resume(entry, &app, &session);
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

fn assert_outcome(outcome: Outcome, rejected: bool, ready: &ResumeObservation) {
    match outcome {
        Outcome::Background(result) => assert_eq!(result, Ok(usize::from(!rejected))),
        Outcome::Query(result) => {
            if rejected {
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

fn assert_data_closed(app: &Application, session: &SessionId, previous_token: Option<&str>) {
    assert_eq!(
        app.get("8", session.as_protocol_id().as_str()),
        Err(PortError::NotFound)
    );
    if let Some(token) = previous_token {
        assert_eq!(app.resolve_data_route(token), Err(PortError::NotFound));
    }
}

fn assert_original_resume_replay(
    app: &Application,
    ready: &ResumeObservation,
    saved: &PublicSessionView,
) {
    let binding = &ready.request.request.binding;
    let ExpectedSession::Version(expected_version) = &binding.expected else {
        panic!("the original Resume must use a version")
    };
    let replay = app
        .resume_lifecycle(
            "8",
            &saved.session,
            &PublicResumeRequest {
                window: binding.key.window,
                sequence: binding.key.sequence,
                expected_version: *expected_version,
            },
        )
        .unwrap();
    assert_eq!(replay.session, saved.session);
    assert_eq!(replay.key, saved.operation);
    assert_eq!(replay.accepted_version, saved.current_version);
    assert_eq!(app.query_lifecycle("8", &saved.session).unwrap(), *saved);
}

fn assert_reopened_loss(
    engine: &Engine,
    address: SocketAddrV4,
    original: &BusinessEnvelope<RegistryMetadata>,
    view: &PublicSessionView,
    previous_token: Option<&str>,
) {
    let (ledger, _) = crate::RedisLedger::open(
        crate::engine::endpoint(address),
        Engine::config(original.required_block_bytes()),
        engine.lease(),
    )
    .unwrap();
    let reopened = SharedBusinessAuthority::new(crate::CenterAuthority::attach(ledger).unwrap());
    let (app, host, _) = application(&reopened);
    assert_eq!(app.query_lifecycle("8", &view.session).unwrap(), *view);
    assert_data_closed(&app, &view.session, previous_token);
    assert!(host.resume_requests.lock().unwrap().is_empty());
    lifecycle_public::assert_same_business_state(&reopened.snapshot().unwrap(), original);
}

fn resume_reply_after_state_lost(entry: Entry) {
    let engine = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(engine.address);
    let authority = lifecycle_public::setup(&engine, &proxy);
    let (app, host, _) = application(&authority);
    let (session, ready) = decoded_resume_terminal::pending_resume(&app, &host);
    assert!(ready.matches_request(&ready.request));
    *host.resumed.lock().unwrap() = Some(ready.clone());
    let token = matches!(entry, Entry::Query).then(|| {
        assert_eq!(
            app.query_lifecycle("8", &session).unwrap().phase,
            PublicSessionPhase::Running
        );
        let detail = app.get("8", session.as_protocol_id().as_str()).unwrap();
        let route = app.resolve_data_route(&detail.envd_access_token).unwrap();
        assert_eq!(route.identity.operation, ready.request.request.binding.key);
        assert_eq!(
            route.identity.accepted_version,
            ready.request.accepted_version
        );
        detail.envd_access_token
    });
    let prior = authority.snapshot().unwrap();
    let before = durable_block(&engine);
    let mut lost = ready.clone();
    lost.readiness = CurrentResumeReadiness::StateLost;
    let expected = prior.with_center(
        prior
            .center
            .accept_lifecycle_resume_observation(&lost)
            .unwrap(),
    );

    let observations = host.resume_requests.lock().unwrap().len();
    let (entered, release) = host.block_next(session.clone());
    let started = Instant::now();
    let late = observe_resume(entry, &app, &session);
    await_observation(&entered);
    *host.resumed.lock().unwrap() = Some(lost);
    let saved = app.query_lifecycle("8", &session).unwrap();
    let committed = durable_block(&engine);
    *host.resumed.lock().unwrap() = Some(ready.clone());
    release.complete();
    let outcome = late.join().unwrap();
    assert!(
        started.elapsed() < REPLY_TIMEOUT,
        "old Ready exceeded its original deadline; timeout is not a stale-observation result"
    );
    assert_outcome(outcome, true, &ready);
    assert_eq!(saved.phase, PublicSessionPhase::StateLost);
    assert_eq!(saved.operation, ready.request.request.binding.key);
    assert_eq!(saved.current_version, ready.request.accepted_version);
    assert_ne!(committed.1, before.1);
    assert_eq!(durable_block(&engine).1, committed.1);
    assert_data_closed(&app, &session, token.as_deref());
    let snapshot = authority.snapshot().unwrap();
    lifecycle_public::assert_same_business_state(&snapshot, &expected);
    assert_eq!(
        snapshot.center.reserved(None, None),
        prior.center.reserved(None, None)
    );
    let record = snapshot
        .center
        .lifecycle_session(&"8".try_into().unwrap(), &session)
        .unwrap();
    assert_eq!(record.operations().len(), 3);
    assert_eq!(record.operations().last().unwrap().intent(), &ready.request);
    assert!(!record.released());
    assert_original_resume_replay(&app, &ready, &saved);
    assert_eq!(
        &host.resume_requests.lock().unwrap()[observations..],
        &[ready.request.clone(), ready.request.clone()]
    );
    drop(app);
    drop(authority);

    assert_reopened_loss(&engine, proxy.address, &snapshot, &saved, token.as_deref());
    assert_eq!(durable_block(&engine).1, committed.1);
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

#[test]
fn background_ready_cannot_revive_same_resume_after_state_lost() {
    resume_reply_after_state_lost(Entry::Background);
}

#[test]
fn query_ready_cannot_revive_same_resume_after_state_lost_or_restore_token() {
    resume_reply_after_state_lost(Entry::Query);
}
