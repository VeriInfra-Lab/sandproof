//! Exact channel-controlled interleavings around original pure transitions.
//! A delayed packet is not another host outcome or a Redis/KVM attestation.
use super::*;
use sandproof_controller_service::application::Session;
use std::{sync::mpsc, thread::JoinHandle};

struct PausedCreate {
    fixture: Fixture,
    request: LifecycleRequest,
    creator: JoinHandle<Result<Session, PortError>>,
    release: mpsc::SyncSender<()>,
}

fn paused_create() -> PausedCreate {
    let mut fixture = fixture();
    // This explicit unit-test worker budget accommodates the barrier, not a
    // change to any production lifetime, freshness or reply configuration.
    Arc::get_mut(&mut fixture.app).unwrap().config.reply_timeout = Duration::from_secs(2);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    *fixture.host.gate.lock().unwrap() = Some(NetworkGate {
        entered: entered_tx,
        release: resume,
    });
    let app = Arc::clone(&fixture.app);
    let creator = std::thread::spawn(move || app.create("1", command()));
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let request = fixture.host.last.lock().unwrap().clone().unwrap();
    *fixture.host.late_response.lock().unwrap() = Some(LifecycleResponse {
        identity: request.identity.clone(),
        observation_version: 1.try_into().unwrap(),
        fact: LifecycleFact::Recorded,
    });
    PausedCreate {
        fixture,
        request,
        creator,
        release,
    }
}

fn save_newer_ready(run: &PausedCreate) -> LifecycleResponse {
    let response = LifecycleResponse {
        identity: run.request.identity.clone(),
        observation_version: 2.try_into().unwrap(),
        fact: LifecycleFact::Ready {
            started_at_ms: 1000,
            end_at_ms: 1000 + run.request.create.timeout_ms,
            max_end_at_ms: 1000 + run.request.create.max_lifetime_ms,
        },
    };
    // The background already applied the real original reducer. The host's
    // durable-current response is also Ready; only the in-flight packet is old.
    run.fixture
        .app
        .authority
        .lock()
        .unwrap()
        .apply_response(&response)
        .unwrap();
    run.fixture.host.responses.lock().unwrap().insert(
        run.request.identity.operation_id.as_str().to_owned(),
        response.clone(),
    );
    response
}

fn save_terminal(run: &PausedCreate) -> LifecycleResponse {
    save_newer_ready(run);
    let mut authority = run.fixture.app.authority.lock().unwrap();
    let destroy = authority
        .destroy(
            &run.request.identity.tenant_id,
            &run.request.identity.sandbox_id,
        )
        .unwrap();
    let response = LifecycleResponse {
        identity: destroy.identity,
        observation_version: 3.try_into().unwrap(),
        fact: LifecycleFact::Cleaned,
    };
    authority.apply_response(&response).unwrap();
    response
}

#[test]
fn same_request_ready_committed_before_recorded_reply_requeries_original_not_superseded() {
    let run = paused_create();
    let latest = save_newer_ready(&run);
    run.release.send(()).unwrap();
    let session = run.creator.join().unwrap().unwrap();
    assert_eq!(session.sandbox_id, run.request.identity.sandbox_id.as_str());
    assert_eq!(run.fixture.host.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        run.fixture.host.last.lock().unwrap().as_ref(),
        Some(&run.request)
    );
    let state = run.fixture.app.snapshot().unwrap();
    assert_eq!(state.identity_frontiers(), (1, 1));
    assert_eq!(state.sandboxes().len(), 1);
    assert_eq!(state.sandboxes()[0].operations.len(), 1);
    assert_eq!(state.sandboxes()[0].operations[0].request, run.request);
    assert_eq!(
        state.sandboxes()[0].operations[0].response.as_ref(),
        Some(&latest)
    );
}

#[test]
fn destroy_really_supersedes_create_and_late_recorded_cannot_restore_a_session() {
    let run = paused_create();
    let terminal = save_terminal(&run);
    run.release.send(()).unwrap();
    assert_eq!(run.creator.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert_eq!(run.fixture.host.calls.load(Ordering::SeqCst), 1);
    let state = run.fixture.app.snapshot().unwrap();
    let record = &state.sandboxes()[0];
    assert!(record.released);
    assert_eq!(
        record.operations.last().unwrap().response.as_ref(),
        Some(&terminal)
    );
    assert_eq!(
        run.fixture.app.get("1", "sandbox-1"),
        Err(PortError::NotFound)
    );
}

#[test]
fn retired_original_record_is_not_recreated_after_a_late_recorded_reply() {
    let run = paused_create();
    let terminal = save_terminal(&run);
    {
        let mut authority = run.fixture.app.authority.lock().unwrap();
        authority.confirm_terminal(&terminal).unwrap();
        let close = authority.begin_close(&terminal).unwrap();
        authority
            .apply_close_ack(&WindowCloseAck {
                terminal: close.terminal,
            })
            .unwrap();
    }
    run.release.send(()).unwrap();
    assert_eq!(run.creator.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert_eq!(run.fixture.host.calls.load(Ordering::SeqCst), 1);
    let state = run.fixture.app.snapshot().unwrap();
    assert!(state.sandboxes().is_empty());
    assert_eq!(state.identity_frontiers(), (1, 2));
}

#[test]
fn shared_snapshot_fault_after_stale_decision_cannot_reuse_a_cached_ready() {
    let run = paused_create();
    let latest = save_newer_ready(&run);
    run.fixture.app.authority.lock().unwrap().fence_after_stale = true;
    run.release.send(()).unwrap();
    assert_eq!(run.creator.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert_eq!(run.fixture.host.calls.load(Ordering::SeqCst), 1);
    assert_eq!(run.fixture.app.snapshot(), Err(PortError::OutcomeUnknown));
    // The exact stored state survives for recovery but cannot authorize success.
    let authority = run.fixture.app.authority.lock().unwrap();
    assert_eq!(authority.state.identity_frontiers(), (1, 1));
    assert_eq!(
        authority.state.sandboxes()[0].operations[0]
            .response
            .as_ref(),
        Some(&latest)
    );
}

#[test]
fn same_identity_stale_fact_without_lower_observation_is_not_blindly_retried() {
    let run = paused_create();
    let latest = save_newer_ready(&run);
    run.fixture
        .host
        .late_response
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .observation_version = 3.try_into().unwrap();
    run.release.send(()).unwrap();
    assert_eq!(run.creator.join().unwrap(), Err(PortError::OutcomeUnknown));
    assert_eq!(run.fixture.host.calls.load(Ordering::SeqCst), 1);
    let state = run.fixture.app.snapshot().unwrap();
    assert_eq!(
        state.sandboxes()[0].operations[0].response.as_ref(),
        Some(&latest)
    );
    assert_eq!(state.identity_frontiers(), (1, 1));
}
