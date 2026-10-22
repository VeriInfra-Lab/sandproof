//! Bounded V3 work selection and the original recorded-input advance path.
#[allow(dead_code, reason = "shared original application fixture")]
#[path = "support/lifecycle_application.rs"]
mod v3;
use sandproof_controller_service::{
    CreateReadiness, JournalError, application::ApplicationError, capture::SourceProcessIdentity,
};
use sandproof_protocol::lifecycle::{ExpectedSession, LifecycleOperation};
use std::sync::{Arc, Mutex};
use v3::fixture::{State, TestEpoch};

#[test]
fn recorded_creates_reappear_after_restart_and_foreign_node_cannot_read_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State::default()));
    let app = v3::application(&path, state.clone(), true);
    let node = v3::config().node;
    app.record_lifecycle_intent(&v3::request(), &v3::context())
        .unwrap();
    assert_eq!(
        app.lifecycle_pending_creates(&node).unwrap(),
        vec![v3::request()]
    );
    let mut foreign = node.clone();
    foreign.node_generation = 2.try_into().unwrap();
    assert!(matches!(
        app.lifecycle_pending_creates(&foreign),
        Err(JournalError::Conflict)
    ));
    drop(app);
    let app = v3::application(&path, state.clone(), false);
    assert_eq!(
        app.lifecycle_pending_creates(&node).unwrap(),
        vec![v3::request()]
    );
    assert!(state.lock().unwrap().actions.is_empty());
}

#[test]
fn busy_first_candidate_does_not_prevent_second_from_original_finite_batch_advancing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        ..State::default()
    }));
    let app = v3::application(&path, state.clone(), true);
    let first = v3::request();
    let mut second = first.clone();
    second.request.binding.session = "session-b".try_into().unwrap();
    second.request.binding.key.owner = "account-beta".try_into().unwrap();
    for request in [&first, &second] {
        app.record_lifecycle_intent(request, &v3::context())
            .unwrap();
    }
    app.establish_lifecycle_host_epoch().unwrap();
    let batch = app.lifecycle_pending_creates(&v3::config().node).unwrap();
    assert_eq!(batch.len(), 2);
    let mut busy = 0;
    let mut advanced = 0;
    for request in batch {
        let template = (request == second).then(v3::fixture::template);
        match app.advance_recorded_lifecycle_create_with_verification(
            &request,
            &v3::config().node,
            template,
            v3::fixture::compute(),
            None,
        ) {
            Err(ApplicationError::Busy) => busy += 1,
            Ok(result) => {
                assert!(result.local.is_some());
                advanced += 1;
            }
            Err(error) => panic!("unexpected result: {error:?}"),
        }
    }
    assert_eq!((busy, advanced), (1, 1));
    assert_eq!(
        app.lifecycle_pending_creates(&v3::config().node).unwrap(),
        vec![first.clone()]
    );
    assert!(
        app.lifecycle_work_state(&first, &v3::context())
            .unwrap()
            .local
            .is_none()
    );
    assert!(
        app.lifecycle_work_state(&second, &v3::context())
            .unwrap()
            .local
            .is_some()
    );
}

#[test]
fn stale_enumeration_is_rechecked_after_destroy_without_native_allocation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        ..State::default()
    }));
    let app = v3::application(&path, state.clone(), true);
    app.record_lifecycle_intent(&v3::request(), &v3::context())
        .unwrap();
    let old = app
        .lifecycle_pending_creates(&v3::config().node)
        .unwrap()
        .pop()
        .unwrap();
    let mut destroy = old.clone();
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    app.record_lifecycle_intent(&destroy, &v3::context())
        .unwrap();
    assert!(
        app.lifecycle_pending_creates(&v3::config().node)
            .unwrap()
            .is_empty()
    );
    let result = app
        .advance_recorded_lifecycle_create_with_verification(
            &old,
            &v3::config().node,
            Some(v3::fixture::template()),
            v3::fixture::compute(),
            None,
        )
        .unwrap();
    assert!(!result.current);
    assert!(result.terminal);
    assert!(result.local.is_none());
    assert!(state.lock().unwrap().actions.is_empty());
    let mut changed = old;
    changed.request.binding.session = "another-session".try_into().unwrap();
    assert!(
        app.advance_recorded_lifecycle_create_with_verification(
            &changed,
            &v3::config().node,
            None,
            v3::fixture::compute(),
            None
        )
        .is_err()
    );
}

#[test]
fn mapped_unknown_startup_is_pending_after_restart_until_original_work_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        lose_activation: true,
        ..State::default()
    }));
    let app = v3::application(&path, state.clone(), true);
    app.record_lifecycle_intent(&v3::request(), &v3::context())
        .unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    v3::advance(&app).unwrap();
    assert_eq!(
        app.lifecycle_pending_creates(&v3::config().node).unwrap(),
        vec![v3::request()]
    );
    drop(app);
    let app = v3::application(&path, state.clone(), false);
    assert_eq!(
        app.lifecycle_pending_creates(&v3::config().node).unwrap(),
        vec![v3::request()]
    );
    assert_eq!(
        app.advance_recorded_lifecycle_create_with_verification(
            &v3::request(),
            &v3::config().node,
            None,
            v3::fixture::compute(),
            None
        ),
        Err(ApplicationError::Busy)
    );
    app.establish_lifecycle_host_epoch().unwrap();
    app.advance_recorded_lifecycle_create_with_verification(
        &v3::request(),
        &v3::config().node,
        None,
        v3::fixture::compute(),
        None,
    )
    .unwrap();
    assert!(
        app.lifecycle_pending_creates(&v3::config().node)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        state
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|action| matches!(
                action.command.command(),
                sandbox_controller_core::Command::PrepareVm { .. }
            ))
            .count(),
        1
    );
}

#[test]
fn running_before_ready_publication_is_queried_after_restart_without_readmission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        source: Some(SourceProcessIdentity {
            pid: 1234,
            start_time_ticks: 5678,
            boot_id: "fixture-boot".into(),
        }),
        ..State::default()
    }));
    let app = v3::application(&path, state.clone(), true);
    app.record_lifecycle_intent(&v3::request(), &v3::context())
        .unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    v3::advance(&app).unwrap();
    // The center's existing ready=None operation remains its query duty. No
    // Ready has been observed/published by this fixture before the restart.
    let calls = state.lock().unwrap().actions.len();
    drop(app);
    let app = v3::application(&path, state.clone(), false);
    assert!(
        app.lifecycle_pending_creates(&v3::config().node)
            .unwrap()
            .is_empty()
    );
    let result = app
        .observe_lifecycle_create(&v3::request(), &v3::context())
        .unwrap();
    assert!(matches!(result.readiness, CreateReadiness::Observed(_)));
    assert_eq!(state.lock().unwrap().actions.len(), calls);
    assert_eq!(
        result.historical_result,
        Some(sandbox_controller_core::OperationResult::Running(1))
    );
}
