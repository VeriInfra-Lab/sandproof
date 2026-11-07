use super::snapshot::ReadyCandidate;
use super::*;
use crate::{
    CreateReadiness, CreateReadinessPending, NeverCrash, application::WorkEvidence,
    capture::SourceProcessIdentity,
};
use sandproof_protocol::{
    WorkOutcome, WorkReceipt,
    lifecycle::{CaptureId, ExpectedSession, IncarnationId, LifecycleOperation},
};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
};

use crate::controller::tests_support::lifecycle_application as v3;
use v3::fixture::{State, TestClock, TestEpoch, TestHost};
type Evidence = <TestHost as WorkEvidence>::Evidence;
type App = crate::application::ControllerApplication<TestHost>;
fn source() -> SourceProcessIdentity {
    SourceProcessIdentity {
        pid: 1234,
        start_time_ticks: 5678,
        boot_id: "fixture-boot".into(),
    }
}
fn running(path: &Path) -> (App, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        source: Some(source()),
        ..State::default()
    }));
    let app = v3::application(path, state.clone(), true);
    app.record_lifecycle_intent(&v3::request(), &v3::context())
        .unwrap();
    app.establish_lifecycle_host_epoch().unwrap();
    v3::advance(&app).unwrap();
    (app, state)
}
fn unload() -> sandproof_protocol::lifecycle::AcceptedLifecycleIntent {
    let mut request = v3::request();
    request.accepted_version = 2;
    request.request.binding.expected = ExpectedSession::Version(1);
    request.request.binding.key.sequence = 2.try_into().unwrap();
    request.request.operation = LifecycleOperation::Unload {
        source_incarnation: IncarnationId::try_from(101).unwrap(),
        capture: CaptureId::try_from(1).unwrap(),
    };
    request
}
fn probes(candidate: &ReadyCandidate) -> Vec<Option<WorkReceipt<Evidence>>> {
    candidate
        .actions
        .iter()
        .map(|action| {
            Some(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::ObservedSuccess(Evidence {
                    resource: action.identity.resource.clone(),
                    command: action.command.command().clone(),
                    established: true,
                    source: Some(source()),
                }),
            })
        })
        .collect()
}

#[test]
fn current_ready_is_read_only_and_preserves_original_lifetime_on_replay_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (app, state) = running(&path);
    let observer = rusqlite::Connection::open(&path).unwrap();
    let version: i64 = observer
        .query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap();
    let executions = state.lock().unwrap().actions.len();
    let first = app
        .observe_lifecycle_create(&v3::request(), &v3::context())
        .unwrap();
    let CreateReadiness::Observed(ready) = &first.readiness else {
        panic!("not ready: {:?}", first.readiness);
    };
    assert_eq!(ready.source, source());
    assert_eq!(ready.basis.original_startup_actions.len(), 5);
    assert_eq!(ready.basis.original_startup_receipts.len(), 5);
    assert_eq!(
        first.historical_result,
        Some(core::OperationResult::Running(1))
    );
    assert_eq!(
        app.observe_lifecycle_create(&v3::request(), &v3::context())
            .unwrap(),
        first
    );
    assert_eq!(state.lock().unwrap().actions.len(), executions);
    assert_eq!(
        observer
            .query_row("PRAGMA data_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        version
    );
    drop(observer);
    drop(app);
    let app = v3::application(&path, state, false);
    assert_eq!(
        app.observe_lifecycle_create(&v3::request(), &v3::context())
            .unwrap(),
        first
    );
}

#[test]
fn original_create_success_stays_historical_after_unload_without_current_ready() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (app, state) = running(&path);
    let first = app
        .observe_lifecycle_create(&v3::request(), &v3::context())
        .unwrap();
    let calls = state.lock().unwrap().recovery_observations;
    app.record_lifecycle_intent(&unload(), &v3::context())
        .unwrap();
    let after = app
        .observe_lifecycle_create(&v3::request(), &v3::context())
        .unwrap();
    assert_eq!(after.current_request, unload());
    assert_eq!(after.readiness, CreateReadiness::NotCurrent);
    assert_eq!(after.historical_result, first.historical_result);
    assert_eq!(after.native.unwrap().phase, core::Phase::Running);
    assert_eq!(state.lock().unwrap().recovery_observations, calls);
}

#[test]
fn destroy_with_unknown_stop_never_revives_old_ready_or_releases_position() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (app, state) = running(&path);
    let mut destroy = unload();
    destroy.request.operation = LifecycleOperation::Destroy {};
    app.record_lifecycle_intent(&destroy, &v3::context())
        .unwrap();
    state.lock().unwrap().lose_stop = Some(core::SandboxId(1));
    app.recover_on_startup().unwrap();
    let calls = state.lock().unwrap().recovery_observations;
    let after = app
        .observe_lifecycle_create(&v3::request(), &v3::context())
        .unwrap();
    assert_eq!(after.readiness, CreateReadiness::NotCurrent);
    assert_eq!(
        after.historical_result,
        Some(core::OperationResult::Running(1))
    );
    assert!(
        after
            .native
            .unwrap()
            .uncertain_actions
            .iter()
            .any(|effect| matches!(effect.command(), core::Command::StopVm { .. }))
    );
    assert_eq!(state.lock().unwrap().recovery_observations, calls);
    drop(app);
    let journal = v3::open(&path);
    assert!(
        journal
            .host_slot_inventory()
            .unwrap()
            .iter()
            .any(|(_, phase)| *phase != core::maintenance::slots::HostSlotPhase::Free)
    );
}

#[test]
fn credential_failure_host_timeout_and_changed_source_only_make_current_observation_pending() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (app, state) = running(&path);
    let original = state.lock().unwrap().configurations[&(1, 1)].clone();
    let calls = state.lock().unwrap().actions.len();
    state
        .lock()
        .unwrap()
        .configurations
        .get_mut(&(1, 1))
        .unwrap()
        .0 = "0".repeat(64);
    let result = app
        .observe_lifecycle_create(&v3::request(), &v3::context())
        .unwrap();
    assert_eq!(
        result.readiness,
        CreateReadiness::Pending(CreateReadinessPending::CredentialUnobserved)
    );
    assert_eq!(
        result.historical_result,
        Some(core::OperationResult::Running(1))
    );
    assert_eq!(
        state.lock().unwrap().configurations[&(1, 1)].0,
        "0".repeat(64)
    );
    state
        .lock()
        .unwrap()
        .configurations
        .insert((1, 1), original);
    state.lock().unwrap().observe_unavailable = true;
    assert_eq!(
        app.observe_lifecycle_create(&v3::request(), &v3::context())
            .unwrap()
            .readiness,
        CreateReadiness::Pending(CreateReadinessPending::HostUnobserved)
    );
    state.lock().unwrap().observe_unavailable = false;
    state.lock().unwrap().source.as_mut().unwrap().pid += 1;
    assert_eq!(
        app.observe_lifecycle_create(&v3::request(), &v3::context())
            .unwrap()
            .readiness,
        CreateReadiness::Pending(CreateReadinessPending::HostUnobserved)
    );
    assert_eq!(state.lock().unwrap().actions.len(), calls);
}

#[test]
fn observer_io_holds_no_sql_write_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (app, state) = running(&path);
    let target = path.clone();
    state.lock().unwrap().observe_hook = Some(Box::new(move || {
        let mut connection = rusqlite::Connection::open(target).unwrap();
        connection.busy_timeout(std::time::Duration::ZERO).unwrap();
        connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap()
            .rollback()
            .unwrap();
    }));
    assert!(matches!(
        app.observe_lifecycle_create(&v3::request(), &v3::context())
            .unwrap()
            .readiness,
        CreateReadiness::Observed(_)
    ));
}

#[test]
fn external_checkpoint_proves_no_live_reader_and_still_fences_foreign_store_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (app, state) = running(&path);
    let target = path.clone();
    let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = observed.clone();
    let executions = state.lock().unwrap().actions.len();
    state.lock().unwrap().observe_hook = Some(Box::new(move || {
        let connection = rusqlite::Connection::open(target).unwrap();
        connection.busy_timeout(std::time::Duration::ZERO).unwrap();
        let result: (i64, i64, i64) = connection
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap();
        assert_eq!(
            result.0, 0,
            "a live SQL reader prevents truncation in this isolated fixture"
        );
        flag.store(true, Ordering::SeqCst);
    }));
    // This isolated TRUNCATE changes this fixture's observed data_version.
    // Preserve the original identity fence; this is not a general checkpoint contract.
    assert!(matches!(
        app.observe_lifecycle_create(&v3::request(), &v3::context()),
        Err(crate::JournalError::CorruptStore)
    ));
    assert!(observed.load(Ordering::SeqCst));
    assert_eq!(state.lock().unwrap().actions.len(), executions);
    drop(app);
    assert_eq!(
        v3::open(&path)
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(1))
            .unwrap()
            .phase,
        core::Phase::Running
    );
}

#[test]
fn foreign_commit_during_host_io_refuses_old_snapshot_without_ready_or_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (app, state) = running(&path);
    let target = path.clone();
    let executions = state.lock().unwrap().actions.len();
    state.lock().unwrap().observe_hook = Some(Box::new(move || {
        let connection = rusqlite::Connection::open(target).unwrap();
        connection.busy_timeout(std::time::Duration::ZERO).unwrap();
        assert_eq!(
            connection
                .execute("UPDATE sandbox_lifetimes SET deadline_ms=deadline_ms-1", [])
                .unwrap(),
            1
        );
    }));
    assert!(matches!(
        app.observe_lifecycle_create(&v3::request(), &v3::context()),
        Err(crate::JournalError::CorruptStore)
    ));
    assert_eq!(state.lock().unwrap().actions.len(), executions);
}

#[test]
fn sql_only_lifetime_change_and_later_intent_invalidate_old_observation_basis() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (app, _) = running(&path);
    drop(app);
    let mut journal = v3::open(&path);
    let before = journal
        .lifecycle_ready_candidate::<Evidence>(&v3::request(), &v3::context(), 10)
        .unwrap();
    assert!(before.can_probe());
    assert!(journal.journal.connection.is_autocommit());
    let receipts = probes(&before);
    let local = before.view.local.unwrap();
    journal
        .connect_lifetime(
            &core::Actor::Tenant(local.owner),
            local.sandbox,
            20,
            Some(120),
            v3::limits(),
        )
        .unwrap();
    let after = journal
        .lifecycle_ready_candidate::<Evidence>(&v3::request(), &v3::context(), 20)
        .unwrap();
    assert_eq!(
        before.view.native_input_high_water,
        after.view.native_input_high_water
    );
    assert_ne!(
        before.view.native.as_ref().unwrap().lifetime,
        after.view.native.as_ref().unwrap().lifetime
    );
    let result = journal
        .finish_lifecycle_ready(&before, &v3::context(), &receipts, true, 20)
        .unwrap();
    assert_eq!(
        result.readiness,
        CreateReadiness::Pending(CreateReadinessPending::SnapshotChanged)
    );
    journal
        .record_lifecycle_intent(&unload(), &v3::context(), &mut NeverCrash)
        .unwrap();
    assert_eq!(
        journal
            .finish_lifecycle_ready(&before, &v3::context(), &receipts, true, 20)
            .unwrap()
            .readiness,
        CreateReadiness::NotCurrent
    );
}

#[test]
fn expiry_during_host_observation_does_not_extend_lifetime_or_stop_the_vm() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (app, state) = running(&path);
    let clock = Arc::new(TestClock(AtomicI64::new(10)));
    let app = app.with_clock(clock.clone());
    let before = app
        .observe_lifecycle_create(&v3::request(), &v3::context())
        .unwrap();
    let lifetime = before.native.as_ref().unwrap().lifetime.unwrap();
    let calls = state.lock().unwrap().actions.len();
    state.lock().unwrap().observe_hook = Some(Box::new(move || {
        clock.0.store(lifetime.deadline_ms, Ordering::SeqCst);
    }));
    let after = app
        .observe_lifecycle_create(&v3::request(), &v3::context())
        .unwrap();
    assert_eq!(after.readiness, CreateReadiness::Expired);
    assert_eq!(after.historical_result, before.historical_result);
    assert_eq!(after.native.unwrap().lifetime, Some(lifetime));
    assert_eq!(state.lock().unwrap().actions.len(), calls);
}

#[test]
fn mapped_running_without_original_boot_source_is_not_current_physical_ready() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let state = Arc::new(Mutex::new(State {
        epoch: TestEpoch::Matching,
        ..State::default()
    }));
    let app = v3::application(&path, state.clone(), true);
    app.record_lifecycle_intent(&v3::request(), &v3::context())
        .unwrap();
    assert_eq!(
        app.observe_lifecycle_create(&v3::request(), &v3::context())
            .unwrap()
            .readiness,
        CreateReadiness::Pending(CreateReadinessPending::NoLocalMapping)
    );
    app.establish_lifecycle_host_epoch().unwrap();
    v3::advance(&app).unwrap();
    let calls = state.lock().unwrap().recovery_observations;
    let result = app
        .observe_lifecycle_create(&v3::request(), &v3::context())
        .unwrap();
    assert_eq!(
        result.historical_result,
        Some(core::OperationResult::Running(1))
    );
    assert_eq!(
        result.readiness,
        CreateReadiness::Pending(CreateReadinessPending::OriginalEvidenceMissing)
    );
    assert_eq!(state.lock().unwrap().recovery_observations, calls);
}
