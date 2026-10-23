//! Actual application and `SQLite`; host facts below are controlled test inputs.
#[allow(
    dead_code,
    reason = "shared V3 application fixture serves several suites"
)]
#[path = "support/lifecycle_application.rs"]
mod v3;
use sandbox_controller_core as core;
use sandproof_controller_service::{JournalError, application::ApplicationError};
use sandproof_protocol::lifecycle::{CreateRefusalReason, ExpectedSession, LifecycleOperation};
use std::sync::{Arc, Mutex};
use v3::fixture::{self, State, TestEpoch};
use v3::{advance, application, context, open, request};

#[test]
fn renewal_keeps_original_create_and_replays_exact_deadline_without_revival() {
    use sandproof_controller_service::{
        CreateReadiness, application::DataKind, capture::SourceProcessIdentity,
    };
    use sandproof_protocol::lifecycle::LifecycleAccessIdentity;
    use std::sync::atomic::{AtomicI64, Ordering};
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
    let clock = Arc::new(fixture::TestClock(AtomicI64::new(10)));
    let app = application(&path, state.clone(), true).with_clock(clock.clone());
    let create = request();
    app.record_lifecycle_intent(&create, &context()).unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    let original = advance(&app).unwrap().local.unwrap();
    let startup_calls = state.lock().unwrap().actions.len();
    let mut renew = create.clone();
    renew.accepted_version = 2;
    renew.request.binding.expected = ExpectedSession::Version(1);
    renew.request.binding.key.sequence = 2.try_into().unwrap();
    let LifecycleOperation::Create {
        target_incarnation: incarnation,
        ..
    } = create.request.operation
    else {
        panic!()
    };
    renew.request.operation = LifecycleOperation::Renew {
        incarnation,
        end_at_ms: 120_010,
        extend_only: false,
    };
    app.record_lifecycle_intent(&renew, &context()).unwrap();
    assert!(matches!(
        app.observe_lifecycle_create(&create, &context())
            .unwrap()
            .readiness,
        CreateReadiness::NotCurrent
    ));
    let (work, end) = app.apply_lifecycle_renew(&renew, &context()).unwrap();
    assert_eq!(work.local, Some(original));
    assert_eq!(end, 120_010);
    clock.0.store(20, Ordering::SeqCst);
    assert_eq!(
        app.apply_lifecycle_renew(&renew, &context()).unwrap().1,
        end
    );
    let observed = app.observe_lifecycle_create(&create, &context()).unwrap();
    assert_eq!(observed.request, create);
    assert_eq!(observed.current_request, renew);
    let CreateReadiness::Observed(ready) = observed.readiness else {
        panic!("original physical Create remains current after applied renewal")
    };
    assert_eq!(ready.started_at_ms, 10);
    assert_eq!(ready.end_at_ms, 120_010);
    assert_eq!(ready.max_end_at_ms, 300_010);
    let access = LifecycleAccessIdentity {
        protocol_version: renew.request.binding.protocol_version,
        center: renew.center.clone(),
        node: renew.request.binding.node.clone(),
        session: renew.request.binding.session.clone(),
        operation: renew.request.binding.key.clone(),
        accepted_version: renew.accepted_version,
        incarnation,
    };
    drop(
        app.lifecycle_data_authorization(&access, &context(), DataKind::FileRead)
            .unwrap(),
    );
    let mut extended = renew.clone();
    extended.accepted_version = 3;
    extended.request.binding.expected = ExpectedSession::Version(2);
    extended.request.binding.key.sequence = 3.try_into().unwrap();
    extended.request.operation = LifecycleOperation::Renew {
        incarnation,
        end_at_ms: 60_010,
        extend_only: true,
    };
    app.record_lifecycle_intent(&extended, &context()).unwrap();
    assert_eq!(
        app.apply_lifecycle_renew(&extended, &context()).unwrap().1,
        120_010
    );
    assert!(matches!(
        app.lifecycle_data_authorization(&access, &context(), DataKind::FileRead),
        Err(ApplicationError::NotFound)
    ));
    let mut excessive = extended.clone();
    excessive.accepted_version = 4;
    excessive.request.binding.expected = ExpectedSession::Version(3);
    excessive.request.binding.key.sequence = 4.try_into().unwrap();
    excessive.request.operation = LifecycleOperation::Renew {
        incarnation,
        end_at_ms: 400_010,
        extend_only: false,
    };
    app.record_lifecycle_intent(&excessive, &context()).unwrap();
    assert_eq!(
        app.apply_lifecycle_renew(&excessive, &context()),
        Err(ApplicationError::Invalid)
    );
    let mut late = excessive.clone();
    late.accepted_version = 5;
    late.request.binding.expected = ExpectedSession::Version(4);
    late.request.binding.key.sequence = 5.try_into().unwrap();
    late.request.operation = LifecycleOperation::Renew {
        incarnation,
        end_at_ms: 200_010,
        extend_only: false,
    };
    app.record_lifecycle_intent(&late, &context()).unwrap();
    clock.0.store(120_010, Ordering::SeqCst);
    assert_eq!(
        app.apply_lifecycle_renew(&late, &context()),
        Err(ApplicationError::NotFound)
    );
    // Historical replay says only that this update committed, not that it is
    // current or the VM is alive; a new unapplied update cannot revive expiry.
    assert!(
        !app.apply_lifecycle_renew(&renew, &context())
            .unwrap()
            .0
            .current
    );
    assert_eq!(startup_calls, state.lock().unwrap().actions.len());
}

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
    let refused = advance(&app).unwrap();
    assert_eq!(refused.intent, request());
    assert_eq!(refused.refusal, Some(CreateRefusalReason::Capacity));
    assert!(refused.local.is_none());
    assert_eq!(
        app.observe_lifecycle_create(&request(), &context())
            .unwrap()
            .readiness,
        sandproof_controller_service::CreateReadiness::Refused {
            reason: CreateRefusalReason::Capacity
        }
    );
    state.lock().unwrap().disk_available = None;
    state.lock().unwrap().template_error = Some(ApplicationError::Invalid);
    assert_eq!(advance(&app).unwrap(), refused);
    assert!(state.lock().unwrap().actions.is_empty());
    let checks = state.lock().unwrap().template_checks;
    drop(app);
    assert!(open(&path).world().unwrap().sandbox_records().is_empty());
    let app = application(&path, state.clone(), false);
    assert_eq!(advance(&app).unwrap(), refused);
    assert_eq!(state.lock().unwrap().template_checks, checks);
    drop(app);

    // Independent dependency/template failures remain their actual errors, not
    // the permanent capacity refusal created by the preceding exact sample.
    for error in [
        ApplicationError::Invalid,
        ApplicationError::Busy,
        ApplicationError::OutcomeUnknown,
        ApplicationError::QuotaExceeded,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let state = Arc::new(Mutex::new(State {
            epoch: TestEpoch::Matching,
            template_error: Some(error),
            ..State::default()
        }));
        let app = application(&path, state.clone(), true);
        app.record_lifecycle_intent(&request(), &context()).unwrap();
        app.establish_lifecycle_host_epoch().unwrap();
        assert_eq!(advance(&app), Err(error));
        let saved = app.lifecycle_work_state(&request(), &context()).unwrap();
        assert_eq!(saved.refusal, None);
        assert!(saved.local.is_none());
        assert!(state.lock().unwrap().actions.is_empty());
        drop(app);
        assert!(open(&path).world().unwrap().sandbox_records().is_empty());
    }
}

#[test]
fn unavailable_physical_sample_is_not_a_capacity_refusal() {
    for error in [
        ApplicationError::Busy,
        ApplicationError::OutcomeUnknown,
        ApplicationError::QuotaExceeded,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let state = Arc::new(Mutex::new(State {
            epoch: TestEpoch::Matching,
            disk_probe: Some(Box::new(move || Err(error))),
            ..State::default()
        }));
        let app = application(&path, state.clone(), true);
        app.record_lifecycle_intent(&request(), &context()).unwrap();
        app.establish_lifecycle_host_epoch().unwrap();
        assert_eq!(advance(&app), Err(error));
        let saved = app.lifecycle_work_state(&request(), &context()).unwrap();
        assert!(saved.refusal.is_none() && saved.local.is_none());
        assert_eq!(
            app.lifecycle_pending_creates(&request().request.binding.node)
                .unwrap(),
            vec![request()]
        );
        assert!(state.lock().unwrap().actions.is_empty());
    }
}

#[test]
fn destroy_during_physical_capacity_sample_wins_without_false_refusal_or_launch() {
    use std::sync::mpsc;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        disk_probe: Some(Box::new(move || {
            entered_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(15))
                .unwrap();
            Ok(0)
        })),
        ..State::default()
    }));
    let app = Arc::new(application(&path, state.clone(), true));
    app.record_lifecycle_intent(&request(), &context()).unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    let task = {
        let app = app.clone();
        std::thread::spawn(move || advance(&app))
    };
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(15))
        .unwrap();
    let mut destroy = request();
    destroy.accepted_version = 2;
    destroy.request.binding.expected = ExpectedSession::Version(1);
    destroy.request.binding.key.sequence = 2.try_into().unwrap();
    destroy.request.operation = LifecycleOperation::Destroy {};
    app.record_lifecycle_intent(&destroy, &context()).unwrap();
    release_tx.send(()).unwrap();
    let result = task.join().unwrap().unwrap();
    assert!(!result.current && result.terminal);
    assert!(result.refusal.is_none() && result.local.is_none());
    assert!(app.lifecycle_destroy_cleaned(&destroy, &context()).unwrap());
    assert!(state.lock().unwrap().actions.is_empty());
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
