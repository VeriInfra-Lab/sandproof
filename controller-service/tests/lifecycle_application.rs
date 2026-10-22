//! Actual application and `SQLite`; host facts below are controlled test inputs.
#[allow(
    dead_code,
    reason = "shared V3 application fixture serves several suites"
)]
#[path = "support/lifecycle_application.rs"]
mod v3;
use sandbox_controller_core as core;
use sandproof_controller_service::{JournalError, application::ApplicationError};
use sandproof_protocol::lifecycle::{ExpectedSession, LifecycleOperation};
use std::sync::{Arc, Mutex};
use v3::fixture::{self, State, TestEpoch};
use v3::{advance, application, context, open, request};

#[test]
fn original_application_records_queries_and_starts_through_one_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State::default()));
    let app = application(&path, state.clone(), true);
    let recorded = app.record_lifecycle_intent(&request(), &context()).unwrap();
    assert_eq!(
        recorded,
        app.lifecycle_work_state(&request(), &context()).unwrap()
    );
    assert!(matches!(
        app.establish_lifecycle_host_epoch(),
        Err(JournalError::UnresolvedAction)
    ));
    assert_eq!(advance(&app), Err(ApplicationError::Busy));
    assert!(state.lock().unwrap().actions.is_empty());
    state.lock().unwrap().epoch = TestEpoch::Matching;
    app.establish_lifecycle_host_epoch().unwrap();
    let result = advance(&app).unwrap();
    let local = result.local.unwrap();
    assert_eq!(local.sandbox, core::SandboxId(1));
    let calls = state.lock().unwrap().actions.len();
    let replay = app
        .advance_lifecycle_create_with_verification(
            &request(),
            &context(),
            None,
            fixture::compute(),
            None,
        )
        .unwrap();
    assert_eq!(replay, result);
    assert_eq!(calls, state.lock().unwrap().actions.len());
    drop(app);
    let journal = open(&path);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(local.sandbox)
            .unwrap()
            .phase,
        core::Phase::Running
    );
    assert_eq!(
        journal
            .lifecycle_work_state(&request(), &context())
            .unwrap(),
        result
    );
}

#[test]
fn restarted_application_does_not_recover_dispatched_startup_before_fresh_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        lose_activation: true,
        ..State::default()
    }));
    let app = application(&path, state.clone(), true);
    app.record_lifecycle_intent(&request(), &context()).unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    let result = advance(&app).unwrap();
    let local = result.local.unwrap();
    drop(app);
    let actions = state.lock().unwrap().actions.len();
    let observations = state.lock().unwrap().recovery_observations;
    let app = application(&path, state.clone(), false);
    let recovery = app.recover_on_startup().unwrap();
    assert_eq!(recovery.unresolved_sandboxes, 1);
    assert_eq!(recovery.receipts_recovered, 0);
    assert_eq!(state.lock().unwrap().actions.len(), actions);
    assert_eq!(state.lock().unwrap().recovery_observations, observations);
    assert_eq!(advance(&app), Err(ApplicationError::Busy));
    app.establish_lifecycle_host_epoch().unwrap();
    state.lock().unwrap().epoch = TestEpoch::WrongBoot;
    assert!(matches!(
        app.establish_lifecycle_host_epoch(),
        Err(JournalError::InvalidHostEvidence)
    ));
    assert_eq!(app.recover_on_startup().unwrap().unresolved_sandboxes, 1);
    assert_eq!(state.lock().unwrap().recovery_observations, observations);
    state.lock().unwrap().epoch = TestEpoch::Matching;
    app.establish_lifecycle_host_epoch().unwrap();
    app.recover_on_startup().unwrap();
    drop(app);
    let journal = open(&path);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(local.sandbox)
            .unwrap()
            .phase,
        core::Phase::Running
    );
    assert_eq!(
        state
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|a| matches!(a.command.command(), core::Command::PrepareVm { .. }))
            .count(),
        1
    );
}

#[test]
fn original_application_capacity_and_template_veto_never_allocate_or_launch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        disk_available: Some(0),
        ..State::default()
    }));
    let app = application(&path, state.clone(), true);
    app.record_lifecycle_intent(&request(), &context()).unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    assert_eq!(advance(&app), Err(ApplicationError::QuotaExceeded));
    assert!(
        app.lifecycle_work_state(&request(), &context())
            .unwrap()
            .local
            .is_none()
    );
    state.lock().unwrap().disk_available = None;
    state.lock().unwrap().template_error = Some(ApplicationError::Invalid);
    assert_eq!(advance(&app), Err(ApplicationError::Invalid));
    assert!(state.lock().unwrap().actions.is_empty());
    drop(app);
    assert!(open(&path).world().unwrap().sandbox_records().is_empty());
}

#[test]
fn mapped_destroy_revokes_routes_and_restarted_cleanup_needs_no_startup_epoch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        ..State::default()
    }));
    let app = application(&path, state.clone(), true);
    app.record_lifecycle_intent(&request(), &context()).unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    let local = advance(&app).unwrap().local.unwrap();
    let secret = state.lock().unwrap().configurations[&local.sandbox.0]
        .0
        .clone();
    let permit = app
        .data_permit(&secret, sandproof_protocol::DataKind::Exec)
        .unwrap();
    assert!(!*permit.revocation.borrow());
    let mut destroy = request();
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    assert!(
        app.record_lifecycle_intent(&destroy, &context())
            .unwrap()
            .terminal
    );
    assert!(*permit.revocation.borrow());
    assert!(
        app.data_permit(&secret, sandproof_protocol::DataKind::Exec)
            .is_err()
    );
    drop(permit);
    drop(app);
    state.lock().unwrap().epoch = TestEpoch::Unavailable;
    let app = application(&path, state.clone(), false);
    app.recover_on_startup().unwrap();
    drop(app);
    let journal = open(&path);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(local.sandbox)
            .unwrap()
            .phase,
        core::Phase::Destroyed
    );
    assert!(
        journal
            .host_slot_inventory()
            .unwrap()
            .iter()
            .all(|(_, phase)| *phase == core::maintenance::slots::HostSlotPhase::Free)
    );
    assert!(
        state
            .lock()
            .unwrap()
            .actions
            .iter()
            .any(|a| matches!(a.command.command(), core::Command::StopVm { .. }))
    );
    assert_eq!(state.lock().unwrap().epoch_checks, 1);
}
