//! Actual application and `SQLite`; host facts below are controlled test inputs.
#[allow(
    dead_code,
    reason = "shared original fixture serves several executables"
)]
#[path = "support/internal_application.rs"]
mod fixture;
use fixture::{State, TestClock, TestEpoch, TestHost, complete_fixture};
use sandbox_controller_core as core;
use sandproof_controller_service::{
    CheckpointProfile, ControllerJournal, JournalError, LifecycleInboxConfig, NeverCrash,
    application::{ApplicationConfig, ApplicationError, ControllerApplication},
    capture::HostEpochRegistration,
};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, CenterIdentity, ContentDigest,
    ExpectedSession, IncarnationId, LifecycleOperation, LifecycleRequest, OperationBinding,
    OperationKey,
};
use std::{
    path::Path,
    sync::{Arc, Mutex, atomic::AtomicI64},
};
const NS: &str = "0123456789abcdef";

fn config() -> LifecycleInboxConfig {
    let node = fixture::wire::node("host-a");
    LifecycleInboxConfig {
        center: CenterIdentity {
            deployment: node.deployment_id.clone(),
            center: "primary".try_into().unwrap(),
        },
        node,
        host_epoch: Some(HostEpochRegistration {
            version: 1,
            boot_id: "fixture-boot".into(),
            policy: ContentDigest::from_sha256([7; 32]),
        }),
        max_sessions: 2,
        max_options_bytes: 1024,
        max_profile_bytes: 4096,
        max_manifest_bytes: 16384,
    }
}
fn context() -> AuthenticatedCenterContext {
    let config = config();
    AuthenticatedCenterContext::from_verified_peer_registration(config.center, config.node)
}
fn request() -> AcceptedLifecycleIntent {
    AcceptedLifecycleIntent {
        center: config().center,
        accepted_version: 1,
        request: LifecycleRequest {
            binding: OperationBinding {
                protocol_version: 3,
                node: config().node,
                session: "session-a".try_into().unwrap(),
                key: OperationKey {
                    deployment: config().node.deployment_id,
                    owner: "account-alpha".try_into().unwrap(),
                    window: 1.try_into().unwrap(),
                    sequence: 1.try_into().unwrap(),
                },
                expected: ExpectedSession::Absent,
            },
            operation: LifecycleOperation::Create {
                create: fixture::request().create,
                target_incarnation: IncarnationId::try_from(101).unwrap(),
            },
        },
    }
}
fn open(path: &Path) -> ControllerJournal {
    let reserve = complete_fixture::CLEANUP_BYTES + 1000;
    let mut maintenance = complete_fixture::maintenance();
    maintenance.checkpoint_profile = CheckpointProfile::FirstUnload;
    ControllerJournal::open_complete(
        path,
        NS,
        complete_fixture::host(path, NS, 64, 2 * 1024 * 1024, reserve),
        maintenance,
        0,
    )
    .unwrap()
}
fn application(
    path: &Path,
    state: Arc<Mutex<State>>,
    provision: bool,
) -> ControllerApplication<TestHost> {
    let mut journal = open(path);
    if provision {
        journal
            .initialize_lifecycle_inbox(config(), &mut NeverCrash)
            .unwrap();
    }
    let reserve = complete_fixture::CLEANUP_BYTES + 1000;
    let app_config = ApplicationConfig {
        public_ipv4_enabled: false,
        limits: sandproof_controller_service::AdmissionLimits {
            max_active_global: 64,
            max_active_per_tenant: 64,
            max_disk_bytes_global: reserve + 2 * 1024 * 1024,
            max_disk_bytes_per_tenant: 2 * 1024 * 1024,
            cleanup_reserve_bytes: reserve,
            max_timeout_secs: 300,
            max_lifetime_secs: 300,
            startup_timeout_secs: 300,
        },
        disk_bytes: 4096,
        file_policy_digest: core::PolicyDigest(1),
        cpu_count: 1,
        memory_mb: 128,
        disk_size_mb: 1,
        envd_version: "fixed-envd".into(),
        domain: "example.invalid".into(),
    };
    ControllerApplication::new(journal, TestHost(state), app_config)
        .with_clock(Arc::new(TestClock(AtomicI64::new(10))))
}
fn advance(
    app: &ControllerApplication<TestHost>,
) -> Result<sandproof_controller_service::LifecycleWorkState, ApplicationError> {
    app.advance_lifecycle_create_with_verification(
        &request(),
        &context(),
        Some(fixture::template()),
        fixture::compute(),
        None,
    )
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
