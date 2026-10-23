//! Real application mutex, `SQLite` and original workflow; controlled host effects.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "support/lifecycle_application.rs"]
mod lifecycle;
#[path = "lifecycle_concurrency/source_observation.rs"]
mod source_observation;
use lifecycle::fixture::{State, TestEpoch};
use sandproof_controller_service::{
    CreateReadiness, application::ApplicationError, capture::SourceProcessIdentity,
};
use sandproof_protocol::lifecycle::{ExpectedSession, LifecycleOperation};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

struct Release(Option<mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

fn destroy(
    mut request: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
) -> sandproof_protocol::lifecycle::AcceptedLifecycleIntent {
    request.accepted_version += 1;
    request.request.binding.expected = ExpectedSession::Version(request.accepted_version - 1);
    request.request.binding.key.sequence = request.accepted_version.try_into().unwrap();
    request.request.operation = LifecycleOperation::Destroy {};
    request
}

#[test]
fn physical_release_does_not_hold_management_lock_or_publish_cleaned_early() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        ..State::default()
    }));
    let app = Arc::new(lifecycle::application(&path, Arc::clone(&state), true));
    let first = lifecycle::request();
    let mut second = first.clone();
    second.request.binding.session = "session-b".try_into().unwrap();
    second.retention.slot = 2;
    second.request.binding.key.owner = "account-beta".try_into().unwrap();
    for request in [&first, &second] {
        app.record_lifecycle_intent(request, &lifecycle::context())
            .unwrap();
    }
    app.establish_lifecycle_host_epoch().unwrap();
    let local = lifecycle::advance(&app).unwrap().local.unwrap();
    let terminal = destroy(first);
    app.record_lifecycle_intent(&terminal, &lifecycle::context())
        .unwrap();
    assert!(
        !app.lifecycle_destroy_cleaned(&terminal, &lifecycle::context())
            .unwrap()
    );
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut release = Release(Some(release_tx));
    state.lock().unwrap().block_release = Some((entered_tx, release_rx));
    let worker_app = Arc::clone(&app);
    let worker = std::thread::spawn(move || worker_app.advance_current_workflow(local.sandbox));
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (recorded_tx, recorded_rx) = mpsc::channel();
    let other_app = Arc::clone(&app);
    let other = std::thread::spawn(move || {
        let request = destroy(second);
        let result = other_app.record_lifecycle_intent(&request, &lifecycle::context());
        recorded_tx.send(result).unwrap();
    });
    assert!(
        recorded_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("termination progresses during host release")
            .unwrap()
            .terminal
    );
    other.join().unwrap();
    assert!(
        !app.lifecycle_destroy_cleaned(&terminal, &lifecycle::context())
            .unwrap()
    );
    release.0.take().unwrap().send(()).unwrap();
    assert!(worker.join().unwrap().unwrap());
    assert!(
        app.lifecycle_destroy_cleaned(&terminal, &lifecycle::context())
            .unwrap()
    );
    drop(app);
    let journal = lifecycle::open(&path);
    assert!(
        journal
            .lifecycle_destroy_cleaned(&terminal, &lifecycle::context())
            .unwrap()
    );
}

#[test]
fn physical_health_sampling_does_not_block_terminal_intent_registration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut release = Release(Some(release_tx));
    let state = Arc::new(Mutex::new(State {
        block_health: Some((entered_tx, release_rx)),
        ..State::default()
    }));
    let app = Arc::new(lifecycle::application(&path, state, true));
    let request = lifecycle::request();
    app.record_lifecycle_intent(&request, &lifecycle::context())
        .unwrap();
    let sample_app = Arc::clone(&app);
    let sample = std::thread::spawn(move || {
        sample_app.internal_free_resources(&lifecycle::config().node, lifecycle::fixture::compute())
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let terminal = destroy(request);
    let record_app = Arc::clone(&app);
    let (recorded_tx, recorded_rx) = mpsc::channel();
    let record = std::thread::spawn(move || {
        recorded_tx
            .send(record_app.record_lifecycle_intent(&terminal, &lifecycle::context()))
            .unwrap();
    });
    assert!(
        recorded_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("termination progresses during health sample")
            .unwrap()
            .terminal
    );
    record.join().unwrap();
    release.0.take().unwrap().send(()).unwrap();
    sample.join().unwrap().unwrap();
}

#[test]
fn blocked_original_prepare_does_not_block_second_session_ready_or_destroy_acceptance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut release = Release(Some(release_tx));
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        source: Some(SourceProcessIdentity {
            pid: 1234,
            start_time_ticks: 5678,
            boot_id: "fixture-boot".into(),
        }),
        block_prepare: Some((entered_tx, release_rx)),
        ..State::default()
    }));
    let app = Arc::new(lifecycle::application(&path, Arc::clone(&state), true));
    let first = lifecycle::request();
    let mut second = first.clone();
    second.request.binding.session = "session-b".try_into().unwrap();
    second.retention.slot = 2;
    second.request.binding.key.owner = "account-beta".try_into().unwrap();
    for request in [&first, &second] {
        app.record_lifecycle_intent(request, &lifecycle::context())
            .unwrap();
    }
    app.establish_lifecycle_host_epoch().unwrap();
    let first_app = Arc::clone(&app);
    let first_work = first.clone();
    let worker = std::thread::spawn(move || {
        first_app.advance_recorded_lifecycle_create_with_verification(
            &first_work,
            &lifecycle::config().node,
            Some(lifecycle::fixture::template()),
            lifecycle::fixture::compute(),
            None,
        )
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let first_local = app
        .lifecycle_work_state(&first, &lifecycle::context())
        .unwrap()
        .local
        .unwrap();
    assert_eq!(
        app.advance_current_workflow(first_local.sandbox),
        Err(ApplicationError::Busy)
    );
    let (ready_tx, ready_rx) = mpsc::channel();
    let second_app = Arc::clone(&app);
    let second_work = second.clone();
    let other = std::thread::spawn(move || {
        let result = second_app.advance_recorded_lifecycle_create_with_verification(
            &second_work,
            &lifecycle::config().node,
            Some(lifecycle::fixture::template()),
            lifecycle::fixture::compute(),
            None,
        );
        ready_tx.send(result).unwrap();
    });
    let second_state = ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("B progresses while A is held")
        .unwrap();
    assert_ne!(first_local.sandbox, second_state.local.unwrap().sandbox);
    other.join().unwrap();
    assert!(matches!(
        app.observe_lifecycle_create(&second, &lifecycle::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
    let mut destroy = first.clone();
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    let terminal = app
        .record_lifecycle_intent(&destroy, &lifecycle::context())
        .unwrap();
    assert!(terminal.terminal);
    release.0.take().unwrap().send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert_original_prepare_then_stop(&state, first_local.sandbox);
}

fn assert_original_prepare_then_stop(
    state: &Mutex<State>,
    sandbox: sandbox_controller_core::SandboxId,
) {
    let actions = &state.lock().unwrap().actions;
    assert_eq!(
        actions
            .iter()
            .filter(|a| a.command.command().sandbox() == sandbox
                && matches!(
                    a.command.command(),
                    sandbox_controller_core::Command::PrepareVm { .. }
                ))
            .count(),
        1
    );
    assert!(
        !actions
            .iter()
            .any(|a| a.command.command().sandbox() == sandbox
                && matches!(
                    a.command.command(),
                    sandbox_controller_core::Command::ConfigureNetwork { .. }
                        | sandbox_controller_core::Command::ActivateVm { .. }
                ))
    );
    assert!(
        actions
            .iter()
            .any(|a| a.command.command().sandbox() == sandbox
                && matches!(
                    a.command.command(),
                    sandbox_controller_core::Command::StopVm { .. }
                ))
    );
}
