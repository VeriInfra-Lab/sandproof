//! Deterministic detached-query races around the actual current center reducer.
use super::*;
use sandproof_controller_service::application::Session;
use std::{sync::mpsc, thread::JoinHandle};
struct PausedCreate {
    fixture: Fixture,
    request: AcceptedLifecycleIntent,
    creator: JoinHandle<Result<Session, PortError>>,
    release: mpsc::SyncSender<()>,
}
fn paused_create() -> PausedCreate {
    let mut fixture = fixture();
    Arc::get_mut(&mut fixture.app).unwrap().config.reply_timeout = Duration::from_secs(2);
    let (entered, observed) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    *fixture.host.gate.lock().unwrap() = Some(NetworkGate { entered, release: resume });
    let app = Arc::clone(&fixture.app);
    let creator = std::thread::spawn(move || app.create("1", command()));
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    let request = fixture.host.last.lock().unwrap().clone().unwrap();
    *fixture.host.late_response.lock().unwrap() = Some(CreateObservation { request: request.clone(), current_request: request.clone(), historical_create_succeeded: false, readiness: CurrentCreateReadiness::Pending });
    PausedCreate { fixture, request, creator, release }
}
fn save_newer_ready(run: &PausedCreate) -> CreateObservation {
    let result = ready_observation(&run.request, &run.request, run.fixture.clock.0.load(Ordering::SeqCst));
    run.fixture.app.authority.lock().unwrap().accept_lifecycle_create_observation(&result).unwrap();
    run.fixture.host.ready.lock().unwrap().push(result.clone());
    result
}
fn save_terminal(run: &PausedCreate) -> TerminalLifecycleObservation {
    save_newer_ready(run);
    let mut authority = run.fixture.app.authority.lock().unwrap();
    let destroy = authority.destroy_current(&run.request.request.binding.key.owner, run.request.request.binding.session.as_protocol_id()).unwrap();
    let result = TerminalLifecycleObservation { request: destroy, status: TerminalStatus::Cleaned };
    authority.accept_lifecycle_terminal_observation(&result, run.fixture.clock.0.load(Ordering::SeqCst)).unwrap();
    result
}
fn assert_original_ready(run: &PausedCreate, latest: &CreateObservation) {
    let state = run.fixture.app.snapshot().unwrap();
    assert_eq!(state.identity_frontiers(), (1, 1));
    assert_eq!(state.sandboxes().len(), 1);
    assert_eq!(state.sandboxes()[0].operations().len(), 1);
    assert_eq!(state.sandboxes()[0].operations()[0].intent(), &run.request);
    let CurrentCreateReadiness::Observed { facts } = &latest.readiness else { unreachable!() };
    assert_eq!(state.sandboxes()[0].current_running(), Some(facts));
}
#[test]
fn pending_detached_query_keeps_concurrently_committed_original_ready() {
    let run = paused_create();
    let latest = save_newer_ready(&run);
    run.release.send(()).unwrap();
    // Current Pending cannot overwrite Ready saved by another exact observer.
    let session = run.creator.join().unwrap().unwrap();
    assert_eq!(session.sandbox_id, run.request.request.binding.session.as_protocol_id().as_str());
    assert_eq!(run.fixture.host.calls.load(Ordering::SeqCst), 1);
    assert_eq!(run.fixture.host.last.lock().unwrap().as_ref(), Some(&run.request));
    let state = run.fixture.app.snapshot().unwrap();
    let CurrentCreateReadiness::Observed { facts } = latest.readiness else { unreachable!() };
    assert_eq!(state.sandboxes()[0].current_running(), Some(&facts));
    assert_eq!(state.identity_frontiers(), (1, 1));
}
#[test]
fn destroy_supersedes_create_and_late_pending_cannot_restore_a_session() {
    let run = paused_create();
    let terminal = save_terminal(&run);
    run.release.send(()).unwrap();
    assert_eq!(run.creator.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert_eq!(run.fixture.host.calls.load(Ordering::SeqCst), 1);
    let state = run.fixture.app.snapshot().unwrap();
    let record = &state.sandboxes()[0];
    assert!(record.released());
    assert_eq!(record.operations().last().unwrap().terminal(), Some(&terminal));
    assert_eq!(run.fixture.app.get("1", "sandbox-1"), Err(PortError::NotFound));
}
#[test]
fn retired_original_record_is_not_recreated_after_a_late_pending_reply() {
    let run = paused_create();
    let terminal = save_terminal(&run);
    advance_retention(&run.fixture);
    {
        let mut authority = run.fixture.app.authority.lock().unwrap();
        authority.confirm_terminal(&ConfirmedLifecycleResult { result: terminal.clone() }).unwrap();
        let now = run.fixture.clock.0.load(Ordering::SeqCst);
        let close = authority.begin_close(&terminal, now).unwrap();
        authority.apply_close_ack(&ClosedLifecycleWindow { retired_through: close.terminal.request.retention.window, terminal: close.terminal }, now).unwrap();
    }
    run.release.send(()).unwrap();
    assert_eq!(run.creator.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert_eq!(run.fixture.host.calls.load(Ordering::SeqCst), 1);
    let state = run.fixture.app.snapshot().unwrap();
    assert!(state.sandboxes().is_empty());
    assert_eq!(state.identity_frontiers(), (1, 2));
}
#[test]
fn shared_snapshot_fault_after_detached_query_cannot_reuse_a_cached_ready() {
    let run = paused_create();
    let latest = save_newer_ready(&run);
    *run.fixture.host.fence_after_send.lock().unwrap() = Some(Arc::clone(&run.fixture.unreadable));
    run.release.send(()).unwrap();
    assert_eq!(run.creator.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert_eq!(run.fixture.host.calls.load(Ordering::SeqCst), 1);
    assert_eq!(run.fixture.app.snapshot(), Err(PortError::OutcomeUnknown));
    let authority = run.fixture.app.authority.lock().unwrap();
    assert_eq!(authority.state.identity_frontiers(), (1, 1));
    let CurrentCreateReadiness::Observed { facts } = latest.readiness else { unreachable!() };
    assert_eq!(authority.state.sandboxes()[0].current_running(), Some(&facts));
}
#[test]
fn conflicting_detached_lifetime_cannot_replace_the_saved_original() {
    let run = paused_create();
    let latest = save_newer_ready(&run);
    let mut wrong = latest.clone();
    let CurrentCreateReadiness::Observed { facts } = &mut wrong.readiness else { unreachable!() };
    facts.started_at_ms -= 1;
    facts.end_at_ms -= 1;
    facts.max_end_at_ms -= 1;
    assert!(wrong.matches_request(&run.request));
    *run.fixture.host.late_response.lock().unwrap() = Some(wrong);
    assert_original_ready(&run, &latest);
    run.release.send(()).unwrap();
    let session = run.creator.join().unwrap().unwrap();
    assert_eq!(session.sandbox_id, "sandbox-1");
    assert_eq!(run.fixture.host.calls.load(Ordering::SeqCst), 1);
    let state = run.fixture.app.snapshot().unwrap();
    let CurrentCreateReadiness::Observed { facts } = latest.readiness else { unreachable!() };
    assert_eq!(state.sandboxes()[0].current_running(), Some(&facts));
    assert_eq!(state.identity_frontiers(), (1, 1));
}
