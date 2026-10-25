//! Real application + `SQLite`; only physical effects and trusted time are controlled.
#[cfg(feature = "contention-diagnostics")]
#[path = "application_lifecycle/contention.rs"]
mod contention;
#[allow(
    dead_code,
    reason = "existing worker support is shared across test executables"
)]
mod support;
use sandbox_controller_core as core;
use sandproof_controller_service::{
    AdmissionLimits, ControllerJournal, HostSlotContext, WorkerTransportError,
    application::{
        ApplicationConfig, ApplicationError, Clock, ConnectCommand, ControllerApplication,
        CreateCommand, DataKind, DataRelay, HostCapacity, HostSlots, LaunchConfiguration,
        ListSelection, TemplateArtifacts, WorkEvidence, WorkExecution, WorkRecovery,
    },
    physical_storage::{PhysicalPool, PhysicalStorageProfile},
};
use sandproof_protocol::{WorkAction, WorkReceipt};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering},
    },
};
use tempfile::TempDir;

#[derive(Default)]
struct TestClock {
    milliseconds: AtomicI64,
    unavailable: AtomicBool,
}
impl TestClock {
    fn at(value: i64) -> Arc<Self> {
        let clock = Arc::new(Self::default());
        clock.set(value);
        clock
    }
    fn set(&self, value: i64) {
        self.milliseconds.store(value, Ordering::SeqCst);
    }
}
impl Clock for TestClock {
    fn now_ms(&self) -> Result<i64, ApplicationError> {
        if self.unavailable.load(Ordering::SeqCst) {
            Err(ApplicationError::Busy)
        } else {
            Ok(self.milliseconds.load(Ordering::SeqCst))
        }
    }
}

#[derive(Default)]
struct PhysicalState {
    secrets: BTreeMap<(u64, core::Generation), String>,
    actions: Vec<WorkAction<core::EffectId>>,
}
#[derive(Clone)]
struct TestHost {
    state: Arc<Mutex<PhysicalState>>,
    clock: Arc<TestClock>,
    relay_calls: Arc<AtomicUsize>,
    activate_time: Option<i64>,
    lose_activation: bool,
    fail_clock_on_activation: bool,
    observe_time: Option<i64>,
    #[cfg(feature = "contention-diagnostics")]
    slow: Option<Arc<contention::SlowOperation>>,
}
impl TestHost {
    fn new(clock: Arc<TestClock>) -> Self {
        Self {
            state: Arc::default(),
            clock,
            relay_calls: Arc::default(),
            activate_time: None,
            lose_activation: false,
            fail_clock_on_activation: false,
            observe_time: None,
            #[cfg(feature = "contention-diagnostics")]
            slow: None,
        }
    }
    fn launches(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .actions
            .iter()
            .filter(|a| matches!(a.command.command(), core::Command::PrepareVm { .. }))
            .count()
    }
}
impl WorkEvidence for TestHost {
    type Evidence = support::FakeHostEvidence;
}
impl WorkExecution for TestHost {
    fn execute(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Self::Evidence>, WorkerTransportError> {
        #[cfg(feature = "contention-diagnostics")]
        {
            sandproof_controller_service::application::diagnostics::host_call(|| {
                self.execute_work(action)
            })
        }
        #[cfg(not(feature = "contention-diagnostics"))]
        self.execute_work(action)
    }
}
impl TestHost {
    fn execute_work(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<support::FakeHostEvidence>, WorkerTransportError> {
        self.state.lock().unwrap().actions.push(action.clone());
        #[cfg(feature = "contention-diagnostics")]
        if let Some(slow) = &self.slow {
            slow.pause(action);
        }
        if matches!(action.command.command(), core::Command::ActivateVm { .. }) {
            if let Some(time) = self.activate_time {
                self.clock.set(time);
            }
            if self.fail_clock_on_activation {
                self.clock.unavailable.store(true, Ordering::SeqCst);
            }
            if self.lose_activation {
                return Err(WorkerTransportError::ReplyLost);
            }
        }
        support::worker(action)
    }
}

impl WorkRecovery for TestHost {
    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Self::Evidence>> {
        if !self.state.lock().unwrap().actions.contains(action) {
            return None;
        }
        if let Some(time) = self.observe_time {
            self.clock.set(time);
        }
        Some(support::success(action))
    }

    fn reclaim_incomplete_prepare(
        &self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }

    fn reclaim_incomplete_activation(
        &self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        // Only the recorded exact action is observed below, never executed again.
        Ok(())
    }

    fn reclaim_incomplete_cleanup(
        &self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }
}

impl HostSlots for TestHost {
    fn bind_host_slot(&self, _: &HostSlotContext, _: bool) -> Result<(), ApplicationError> {
        Ok(())
    }

    fn release_host_slot(&self, _: &HostSlotContext) -> Result<(), ApplicationError> {
        Ok(())
    }
}

impl DataRelay for TestHost {
    fn acquire_data_gate(
        &self,
        _: core::SandboxId,
        _: core::VmId,
        _: core::Generation,
    ) -> Result<Option<std::fs::File>, ApplicationError> {
        let gate = tempfile::tempfile().map_err(|_| ApplicationError::Busy)?;
        gate.try_lock_shared().map_err(|_| ApplicationError::Busy)?;
        Ok(Some(gate))
    }

    fn spawn_data_relay(
        &self,
        _: core::SandboxId,
        _: core::VmId,
        _: core::Generation,
        _: bool,
    ) -> Result<std::process::Child, ApplicationError> {
        self.relay_calls.fetch_add(1, Ordering::SeqCst);
        Err(ApplicationError::Busy)
    }

    fn data_gate_drained(&self, _: &HostSlotContext) -> Result<bool, ApplicationError> {
        Ok(true)
    }
}

impl HostCapacity for TestHost {
    fn available_disk_bytes(&self) -> Result<u64, ApplicationError> {
        Ok(u64::MAX)
    }

    fn validate_physical_storage(
        &self,
        profile: &PhysicalStorageProfile,
        _: &PhysicalPool,
    ) -> Result<(), ApplicationError> {
        profile.validate().map_err(|_| ApplicationError::Busy)
    }
}

impl LaunchConfiguration for TestHost {
    fn put_secret(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        token: &str,
    ) -> Result<(), ApplicationError> {
        let mut state = self.state.lock().unwrap();
        if state.secrets.contains_key(&(sandbox.0, generation)) {
            return Err(ApplicationError::Conflict);
        }
        state.secrets.insert((sandbox.0, generation), token.into());
        Ok(())
    }

    fn has_secret(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> Result<bool, ApplicationError> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .secrets
            .contains_key(&(sandbox.0, generation)))
    }

    fn token_for(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> Result<String, ApplicationError> {
        self.state
            .lock()
            .unwrap()
            .secrets
            .get(&(sandbox.0, generation))
            .cloned()
            .ok_or(ApplicationError::Busy)
    }
}

impl TemplateArtifacts for TestHost {}

// This observer deliberately does NOT implement WorkExecution or other host
// capabilities. An observation consumer cannot dispatch work through it.
struct RecoveryOnly;
impl WorkEvidence for RecoveryOnly {
    type Evidence = support::FakeHostEvidence;
}
impl WorkRecovery for RecoveryOnly {
    fn observe(&self, _: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Self::Evidence>> {
        None
    }
    fn reclaim_incomplete_prepare(
        &self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }
    fn reclaim_incomplete_cleanup(
        &self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), ApplicationError> {
        Err(ApplicationError::Busy)
    }
}

#[test]
fn recovery_port_requires_no_execution_capability_and_unknown_stays_unknown() {
    fn observe_only<H: WorkRecovery>(host: &mut H, action: &WorkAction<core::EffectId>) {
        assert!(host.observe(action).is_none());
        assert_eq!(
            host.reclaim_incomplete_activation(action),
            Err(ApplicationError::Busy)
        );
        assert_eq!(
            host.reclaim_incomplete_stop(action),
            Err(ApplicationError::Busy)
        );
    }
    let directory = TempDir::new().unwrap();
    let host = TestHost::new(TestClock::at(1000));
    let app = application(&directory, host.clone());
    let session = app.create("1", request()).unwrap();
    let action = host.state.lock().unwrap().actions[0].clone();
    let calls = host.state.lock().unwrap().actions.len();
    observe_only(&mut RecoveryOnly, &action);
    assert_eq!(host.state.lock().unwrap().actions.len(), calls);
    app.kill("1", &session.sandbox_id).unwrap();
}

#[test]
fn data_port_without_backend_refuses_relay_instead_of_starting_fallback() {
    struct NoRelay;
    impl DataRelay for NoRelay {}
    fn spawn_only<H: DataRelay>(host: &mut H) {
        assert!(matches!(
            host.spawn_data_relay(core::SandboxId(1), core::VmId(1), 1, false),
            Err(ApplicationError::Busy)
        ));
    }
    spawn_only(&mut NoRelay);
}

fn config() -> ApplicationConfig {
    ApplicationConfig {
        public_ipv4_enabled: false,
        limits: AdmissionLimits {
            max_active_global: 2,
            max_active_per_tenant: 2,
            max_disk_bytes_global: support::CLEANUP_BYTES + 1000 + 2 * 1024 * 1024,
            max_disk_bytes_per_tenant: 2 * 1024 * 1024,
            cleanup_reserve_bytes: support::CLEANUP_BYTES + 1000,
            max_timeout_secs: 20,
            max_lifetime_secs: 20,
            startup_timeout_secs: 10,
        },
        disk_bytes: 1000,
        file_policy_digest: core::PolicyDigest(1),
        cpu_count: 1,
        memory_mb: 512,
        disk_size_mb: 1,
        envd_version: "0.9.0".into(),
        domain: "example.invalid".into(),
    }
}
fn request() -> CreateCommand {
    CreateCommand {
        template_id: "base".into(),
        timeout: Some(2),
        allow_internet_access: Some(false),
        network: None,
        metadata: None,
        env_vars: None,
    }
}
fn application(directory: &TempDir, host: TestHost) -> ControllerApplication<TestHost> {
    let clock = host.clock.clone();
    let path = directory.path().join("controller.db");
    let bounded = support::complete_fixture::host(
        &path,
        support::NAMESPACE,
        2,
        2 * 1024 * 1024,
        support::CLEANUP_BYTES + 1000,
    );
    ControllerApplication::new(
        ControllerJournal::open_complete(
            &path,
            support::NAMESPACE,
            bounded,
            support::complete_fixture::maintenance(),
            0,
        )
        .unwrap(),
        host,
        config(),
    )
    .with_clock(clock)
}

#[test]
fn permit_expires_at_exact_deadline_before_scanner_and_pins_cleanup_until_drop() {
    let directory = TempDir::new().unwrap();
    let clock = TestClock::at(1000);
    let host = TestHost::new(clock.clone());
    let app = application(&directory, host.clone());
    let session = app.create("1", request()).unwrap();
    let permit = app
        .data_permit(&session.envd_access_token, DataKind::Exec)
        .unwrap();
    clock.set(2999);
    assert!(
        app.authorize_data(&session.envd_access_token, DataKind::Exec)
            .is_ok()
    );
    clock.set(3000);
    assert!(!*permit.revocation.borrow(), "scanner has not revoked yet");
    assert!(matches!(
        app.spawn_data_relay(&permit, false),
        Err(ApplicationError::NotFound)
    ));
    assert_eq!(host.relay_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        app.get("1", &session.sandbox_id),
        Err(ApplicationError::NotFound)
    );
    assert!(
        app.list("1", ListSelection::default())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(app.expire_due().unwrap(), 1);
    assert!(*permit.revocation.borrow());
    assert_eq!(app.maintain().unwrap().retired_sandboxes, 0);
    drop(permit);
    assert_eq!(app.maintain().unwrap().retired_sandboxes, 1);
}

#[test]
fn publication_samples_time_after_guest_activation_not_once_at_admission() {
    let directory = TempDir::new().unwrap();
    let clock = TestClock::at(1000);
    let mut host = TestHost::new(clock.clone());
    host.activate_time = Some(5000);
    let app = application(&directory, host.clone());
    let session = app.create("1", request()).unwrap();
    let detail = app.get("1", &session.sandbox_id).unwrap();
    assert_eq!(detail.info.started_at_ms, 5000);
    assert_eq!(detail.info.end_at_ms, 7000);
    assert_eq!(host.launches(), 1);
    assert_eq!(
        app.get("2", &session.sandbox_id),
        Err(ApplicationError::NotFound)
    );
}

#[test]
fn unavailable_clock_before_admission_consumes_no_identity_or_host_effect() {
    let directory = TempDir::new().unwrap();
    let clock = TestClock::at(1000);
    let host = TestHost::new(clock.clone());
    let app = application(&directory, host.clone());
    clock.unavailable.store(true, Ordering::SeqCst);
    assert_eq!(app.create("1", request()), Err(ApplicationError::Busy));
    assert_eq!(host.launches(), 0);
    clock.unavailable.store(false, Ordering::SeqCst);
    assert!(
        app.create("1", request())
            .unwrap()
            .sandbox_id
            .ends_with("-1")
    );
}

#[test]
fn unavailable_publication_clock_is_unknown_and_recovers_original_vm() {
    let directory = TempDir::new().unwrap();
    let clock = TestClock::at(1000);
    let mut host = TestHost::new(clock.clone());
    host.fail_clock_on_activation = true;
    let app = application(&directory, host.clone());
    assert_eq!(
        app.create("1", request()),
        Err(ApplicationError::OutcomeUnknown)
    );
    assert_eq!(host.launches(), 1);
    clock.unavailable.store(false, Ordering::SeqCst);
    clock.set(5000);
    assert_eq!(app.recover_on_startup().unwrap().workflows_completed, 1);
    let page = app.list("1", ListSelection::default()).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].started_at_ms, 5000);
    assert_eq!(host.launches(), 1);
}

#[test]
fn lost_activation_receipt_after_restart_keeps_identity_and_uses_fresh_publication_time() {
    let directory = TempDir::new().unwrap();
    let clock = TestClock::at(1000);
    let mut host = TestHost::new(clock.clone());
    host.lose_activation = true;
    let app = application(&directory, host.clone());
    assert_eq!(
        app.create("1", request()),
        Err(ApplicationError::OutcomeUnknown)
    );
    drop(app);
    clock.set(5000);
    let app = application(&directory, host.clone());
    assert_eq!(app.recover_on_startup().unwrap().workflows_completed, 1);
    let session = &app.list("1", ListSelection::default()).unwrap().items[0];
    assert!(session.sandbox_id.ends_with("-1"));
    assert_eq!(session.started_at_ms, 5000);
    assert_eq!(host.launches(), 1);
}

#[test]
fn recovery_rechecks_startup_deadline_after_observation() {
    let directory = TempDir::new().unwrap();
    let clock = TestClock::at(1000);
    let mut host = TestHost::new(clock.clone());
    host.lose_activation = true;
    let app = application(&directory, host.clone());
    assert_eq!(
        app.create("1", request()),
        Err(ApplicationError::OutcomeUnknown)
    );
    drop(app);
    host.observe_time = Some(11_000);
    let app = application(&directory, host.clone());
    let report = app.recover_on_startup().unwrap();
    assert_eq!(report.unresolved_sandboxes, 0);
    assert_eq!(report.workflows_completed, 1);
    assert!(
        app.list("1", ListSelection::default())
            .unwrap()
            .items
            .is_empty()
    );
    assert_eq!(host.launches(), 1);
}

#[test]
fn renewal_and_nonpositive_timeout_share_authority_and_exact_clock() {
    let directory = TempDir::new().unwrap();
    let clock = TestClock::at(1000);
    let host = TestHost::new(clock.clone());
    let app = application(&directory, host);
    let session = app.create("1", request()).unwrap();
    clock.set(2999);
    app.connect(
        "1",
        &session.sandbox_id,
        ConnectCommand { timeout: Some(4) },
    )
    .unwrap();
    assert_eq!(
        app.get("1", &session.sandbox_id).unwrap().info.end_at_ms,
        6999
    );
    let permit = app
        .data_permit(&session.envd_access_token, DataKind::FileWrite)
        .unwrap();
    app.set_timeout("1", &session.sandbox_id, 0).unwrap();
    assert!(*permit.revocation.borrow());
    assert!(matches!(
        app.spawn_data_relay(&permit, false),
        Err(ApplicationError::NotFound)
    ));
    assert_eq!(
        app.connect(
            "1",
            &session.sandbox_id,
            ConnectCommand { timeout: Some(1) }
        ),
        Err(ApplicationError::NotFound)
    );
}
