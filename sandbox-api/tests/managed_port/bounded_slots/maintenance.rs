//! Real production maintenance coordinator, reducer and `SQLite`; only exact
//! physical release observations use a controlled, incarnation-aware host.
use super::*;
use sandproof_controller_service::MaintenanceConfig;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicI64;

#[derive(Clone)]
struct FleetHost {
    hosts: Arc<Mutex<BTreeMap<u64, BoundedHost>>>,
    directory: PathBuf,
}

impl FleetHost {
    fn new(directory: &Path) -> Self {
        Self {
            hosts: Arc::default(),
            directory: directory.into(),
        }
    }

    fn sandbox(&self, sandbox: core::SandboxId) -> Result<BoundedHost, PortError> {
        self.hosts
            .lock()
            .unwrap()
            .values()
            .find(|host| {
                host.0
                    .lock()
                    .unwrap()
                    .row
                    .is_some_and(|(context, _)| context.binding.sandbox == sandbox)
            })
            .cloned()
            .ok_or(PortError::Busy)
    }
}

impl WorkEvidence for FleetHost {
    type Evidence = Evidence;
}
impl WorkExecution for FleetHost {
    fn execute(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Evidence>, WorkerTransportError> {
        self.sandbox(action.command.command().sandbox())
            .map_err(|_| WorkerTransportError::ConnectionLost)?
            .execute(action)
    }
}

impl WorkRecovery for FleetHost {
    fn observe(&self, _: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Evidence>> {
        None
    }

    fn reclaim_incomplete_prepare(
        &self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }

    fn reclaim_incomplete_cleanup(
        &self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }
}

impl HostSlots for FleetHost {
    fn bind_host_slot(&self, context: &HostSlotContext, allow: bool) -> Result<(), PortError> {
        let mut hosts = self.hosts.lock().unwrap();
        let host = hosts.entry(context.position.0).or_insert_with(|| {
            let host = BoundedHost::default();
            let path = self.directory.join(format!("gate-{}", context.position.0));
            std::fs::write(&path, []).unwrap();
            host.0.lock().unwrap().gate_path = Some(path);
            host
        });
        host.bind_host_slot(context, allow)
    }

    fn release_host_slot(&self, context: &HostSlotContext) -> Result<(), PortError> {
        self.sandbox(context.binding.sandbox)?
            .release_host_slot(context)
    }
}

impl DataRelay for FleetHost {
    fn acquire_data_gate(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        generation: core::Generation,
    ) -> Result<Option<std::fs::File>, PortError> {
        self.sandbox(sandbox)?
            .acquire_data_gate(sandbox, vm, generation)
    }

    fn data_gate_drained(&self, context: &HostSlotContext) -> Result<bool, PortError> {
        self.sandbox(context.binding.sandbox)?
            .data_gate_drained(context)
    }
}

impl HostCapacity for FleetHost {
    fn available_disk_bytes(&self) -> Result<u64, PortError> {
        Ok(16 * 1024 * 1024 * 1024)
    }

    fn validate_physical_storage(
        &self,
        profile: &sandproof_controller_service::physical_storage::PhysicalStorageProfile,
        _: &sandproof_controller_service::physical_storage::PhysicalPool,
    ) -> Result<(), PortError> {
        profile.validate().map_err(|_| PortError::Busy)
    }
}

impl LaunchConfiguration for FleetHost {
    fn put_secret(&self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError> {
        self.sandbox(sandbox)?.put_secret(sandbox, token)
    }

    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, PortError> {
        self.sandbox(sandbox)?.has_secret(sandbox)
    }

    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, PortError> {
        self.sandbox(sandbox)?.token_for(sandbox)
    }
}

impl TemplateArtifacts for FleetHost {}

fn host_config(path: &Path) -> BoundedHostConfig {
    let mut config = bounded(path);
    config.maintenance.max_host_slots = 2;
    config.maintenance.max_retained_results = 64;
    config.physical_storage.as_mut().unwrap().pools[0].max_host_slots = 2;
    config
}

fn maintenance_config() -> MaintenanceConfig {
    let mut policy = complete_fixture::maintenance();
    policy.max_tail_inputs = 8;
    policy.max_tail_bytes = 64 * 1024;
    policy.prune_batch_rows = 3;
    policy
}

fn maintained_application(path: &Path, host: FleetHost) -> ControllerApplication<FleetHost> {
    let mut limits = config();
    limits.limits.max_active_per_tenant = 2;
    limits.limits.max_disk_bytes_per_tenant = 2_000;
    ControllerApplication::new(
        ControllerJournal::open_complete(
            path,
            NAMESPACE,
            host_config(path),
            maintenance_config(),
            0,
        )
        .unwrap(),
        host,
        limits,
    )
}

fn open_maintained(path: &Path, host: FleetHost) -> ManagedPort<FleetHost> {
    ManagedPort::new(Arc::new(maintained_application(path, host)))
}

struct MaintenanceClock(AtomicI64);

impl sandproof_controller_service::application::Clock for MaintenanceClock {
    fn now_ms(&self) -> Result<i64, sandproof_controller_service::application::ApplicationError> {
        Ok(self.0.load(Ordering::SeqCst))
    }
}

fn open_with_clock(
    path: &Path,
    host: FleetHost,
    clock: Arc<MaintenanceClock>,
) -> ManagedPort<FleetHost> {
    ManagedPort::new(Arc::new(
        maintained_application(path, host).with_clock(clock),
    ))
}

fn journal(path: &Path) -> ControllerJournal {
    ControllerJournal::open_complete(path, NAMESPACE, host_config(path), maintenance_config(), 0)
        .unwrap()
}

#[test]
fn production_tick_retires_newer_objects_while_old_live_or_unknown_object_stays_exact() {
    for unknown in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("controller.db");
        let host = FleetHost::new(dir.path());
        // This checks retirement while the old instance remains live, not
        // wall-clock expiry during slow durable SQLite I/O.
        let clock = Arc::new(MaintenanceClock(AtomicI64::new(1_000_000)));
        let port = open_with_clock(&path, host.clone(), clock);
        let old = port.create("1", request()).unwrap();
        if unknown {
            host.sandbox(core::SandboxId(1))
                .unwrap()
                .0
                .lock()
                .unwrap()
                .unknown_cleanup = true;
            assert!(matches!(
                port.kill("1", &old.sandbox_id),
                Err(PortError::OutcomeUnknown)
            ));
        }
        for _ in 0..8 {
            let new = port.create("1", request()).unwrap();
            port.kill("1", &new.sandbox_id).unwrap();
            let report = port.application().maintain().unwrap();
            assert_eq!(report.retired_sandboxes, 1);
            assert_eq!(report.tracked_data_sandboxes, 0);
            if !unknown {
                assert_eq!(
                    port.get("1", &old.sandbox_id).unwrap().envd_access_token,
                    old.envd_access_token
                );
            }
        }
        drop(port);
        let journal = journal(&path);
        let checkpoint = journal.checkpoint_progress().unwrap().unwrap();
        // Full mode rotates/prunes throughout admitted writes, not only in
        // this explicit tick. The persistent prefix must actually shrink.
        assert!(checkpoint.deleted_through > 3);
        assert!(
            checkpoint.input_high_water - checkpoint.input_cut
                <= maintenance_config().max_tail_inputs
        );
        let world = journal.world().unwrap();
        assert_eq!(world.sandbox_records().len(), 1);
        assert!(world.sandbox_record(core::SandboxId(1)).is_some());
        assert_eq!(world.maintenance_state().unwrap().retained_count(), 16);
        assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(10));
    }
}

#[test]
fn controlled_maintenance_clock_still_refuses_an_actually_expired_instance() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = FleetHost::new(dir.path());
    let clock = Arc::new(MaintenanceClock(AtomicI64::new(1_000_000)));
    let port = open_with_clock(&path, host, clock.clone());
    let created = port.create("1", request()).unwrap();
    assert_eq!(
        port.get("1", &created.sandbox_id)
            .unwrap()
            .envd_access_token,
        created.envd_access_token
    );
    clock.0.store(1_300_001, Ordering::SeqCst);
    assert!(matches!(
        port.get("1", &created.sandbox_id),
        Err(PortError::NotFound)
    ));
}

#[test]
fn production_tick_waits_for_body_and_unknown_relay_then_removes_local_tracking() {
    for unknown in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("controller.db");
        let host = FleetHost::new(dir.path());
        let port = open_maintained(&path, host.clone());
        let old = port.create("1", request()).unwrap();
        let permit = port
            .application()
            .data_permit(
                &old.envd_access_token,
                sandproof_controller_service::application::DataKind::Exec,
            )
            .unwrap();
        if unknown {
            permit.drain.mark_unknown();
        }
        port.kill("1", &old.sandbox_id).unwrap();
        let report = port.application().maintain().unwrap();
        assert_eq!(report.retired_sandboxes, 0);
        assert_eq!(report.tracked_data_sandboxes, 1);
        drop(permit);
        if unknown {
            assert_eq!(port.application().maintain().unwrap().retired_sandboxes, 0);
            host.sandbox(core::SandboxId(1))
                .unwrap()
                .0
                .lock()
                .unwrap()
                .gate_observation = Some(true);
        }
        let report = port.application().maintain().unwrap();
        assert_eq!(report.retired_sandboxes, 1);
        assert_eq!(report.tracked_data_sandboxes, 0);
        assert!(matches!(
            port.application().authorize_data(
                &old.envd_access_token,
                sandproof_controller_service::application::DataKind::Exec
            ),
            Err(PortError::NotFound)
        ));
    }
}

#[test]
fn release_witness_survives_rebind_and_retirement_preserves_newer_launch_and_authority() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = FleetHost::new(dir.path());
    let port = open_maintained(&path, host.clone());
    let old = port.create("1", request()).unwrap();
    let stale = host
        .sandbox(core::SandboxId(1))
        .unwrap()
        .0
        .lock()
        .unwrap()
        .actions
        .clone();
    port.kill("1", &old.sandbox_id).unwrap();
    let current = port.create("1", request()).unwrap();
    assert_eq!(
        host.sandbox(core::SandboxId(2))
            .unwrap()
            .0
            .lock()
            .unwrap()
            .row
            .unwrap()
            .0
            .position
            .0,
        0
    );
    assert_eq!(port.application().maintain().unwrap().retired_sandboxes, 1);
    for action in stale {
        assert!(host.clone().execute(&action).is_err());
    }
    assert_eq!(
        port.get("1", &current.sandbox_id)
            .unwrap()
            .envd_access_token,
        current.envd_access_token
    );
    drop(port);
    let port = open_maintained(&path, host);
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes,
        0
    );
    assert_eq!(port.application().maintain().unwrap().retired_sandboxes, 0);
    assert_eq!(
        port.get("1", &current.sandbox_id)
            .unwrap()
            .envd_access_token,
        current.envd_access_token
    );
}

#[test]
fn lost_host_release_ack_cannot_authorize_retirement_before_recovery() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = FleetHost::new(dir.path());
    let port = open_maintained(&path, host.clone());
    let old = port.create("1", request()).unwrap();
    host.sandbox(core::SandboxId(1))
        .unwrap()
        .0
        .lock()
        .unwrap()
        .lose_release = true;
    port.kill("1", &old.sandbox_id).unwrap();
    drop(port);
    let db = journal(&path);
    assert!(db.retirement_candidates().unwrap().is_empty());
    assert_eq!(
        db.host_slot_inventory().unwrap()[0].1,
        HostSlotPhase::Releasing
    );
    drop(db);
    let port = open_maintained(&path, host);
    assert_eq!(port.application().maintain().unwrap().retired_sandboxes, 1);
    drop(port);
    assert!(journal(&path).world().unwrap().sandbox_records().is_empty());
}

#[tokio::test]
async fn ordinary_sdk_create_stays_unchanged_and_http_cannot_supply_retirement_attestations() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = FleetHost::new(dir.path());
    let port = Arc::new(open_maintained(&path, host));
    let app = router(
        Arc::clone(&port),
        vec![ApiCredential {
            tenant: "1".into(),
            secret: KEY_ONE.into(),
        }],
    )
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v2/sandboxes")
                .header("x-api-key", KEY_ONE)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"templateID":"base","allow_internet_access":false}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
    let sandbox = body["sandboxID"].as_str().unwrap();
    port.kill("1", sandbox).unwrap();
    let response = app.oneshot(
        Request::builder().method("POST").uri("/maintenance")
            .header("x-api-key", KEY_ONE).header("content-type", "application/json")
            .body(Body::from(format!(r#"{{"sandbox":"{sandbox}","worker_drained":true,"data_connections_drained":true,"host_resources_reclaimed":true,"external_pins_released":true}}"#))).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // The trusted coordinator, rather than that request, performs retirement.
    assert_eq!(port.application().maintain().unwrap().retired_sandboxes, 1);
}

#[test]
fn full_result_retention_refuses_new_admission_without_evicting_promised_results() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = FleetHost::new(dir.path());
    let mut bounded = host_config(&path);
    bounded.maintenance.max_retained_results = 2;
    let port = new_port(
        ControllerJournal::open_complete(
            &path,
            NAMESPACE,
            bounded.clone(),
            maintenance_config(),
            0,
        )
        .unwrap(),
        host.clone(),
        config(),
    );
    let first = port.create("1", request()).unwrap();
    port.kill("1", &first.sandbox_id).unwrap();
    assert_eq!(port.application().maintain().unwrap().retired_sandboxes, 1);
    let physical = host.sandbox(core::SandboxId(1)).unwrap();
    let actions_before = physical.0.lock().unwrap().actions.len();
    // Full mode reserves eventual create/kill outcomes before admitting work,
    // so this request is refused before another identity or command exists.
    assert!(matches!(port.create("1", request()), Err(PortError::Busy)));
    assert_eq!(physical.0.lock().unwrap().actions.len(), actions_before);
    let report = port.application().maintain().unwrap();
    assert_eq!(report.retired_sandboxes, 0);
    assert_eq!(report.pending_sandboxes, 0);
    assert_eq!(report.expired_results, 0);
    drop(port);
    let journal =
        ControllerJournal::open_complete(&path, NAMESPACE, bounded, maintenance_config(), 0)
            .unwrap();
    let world = journal.world().unwrap();
    assert_eq!(world.maintenance_state().unwrap().retained_count(), 2);
    assert!(world.sandbox_record(core::SandboxId(1)).is_none());
    assert!(world.sandbox_record(core::SandboxId(2)).is_none());
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(2));
    assert!(
        world
            .maintenance_state()
            .unwrap()
            .retained_result(core::OperationKey {
                sandbox: core::SandboxId(1),
                operation: core::OperationId(1),
            })
            .is_some()
    );
}

#[test]
fn full_capacity_idle_maintenance_does_not_spend_input_or_checkpoint_frontiers() {
    use sandproof_controller_service::{CapacityConfig, ConsolePolicy};
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let mut host = host_config(&path);
    host.console_policy = Some(ConsolePolicy::Discard);
    // Explicit test-only envelope, matching the component workload rather than
    // recommending deployment values.
    let policy = MaintenanceConfig {
        retention_ms: 100,
        max_checkpoint_bytes: 4 * 1024 * 1024,
        max_records: 10_000,
        max_references: 1_000_000,
        max_tail_inputs: 8,
        max_tail_bytes: 64 * 1024,
        max_input_bytes: 8192,
        prune_batch_rows: 3,
        capacity: Some(CapacityConfig {
            max_live_sandboxes: 4,
            max_sql_rows: 1024,
            max_sql_bytes: 16 * 1024 * 1024,
            max_sql_value_bytes: 256 * 1024,
            max_receipt_bytes: 16 * 1024,
            max_transaction_inputs: 4,
            max_transaction_bytes: 32 * 1024,
            max_database_bytes: 512 * 1024 * 1024,
            max_wal_bytes: 520 * 1024 * 1024,
            max_shm_bytes: 2 * 1024 * 1024,
            checkpoint_scratch_bytes: 8 * 1024 * 1024,
            max_online_storage_bytes: 1042 * 1024 * 1024,
        }),
    };
    let initial =
        ControllerJournal::open_complete(&path, NAMESPACE, host.clone(), policy.clone(), 0)
            .unwrap();
    let before = initial.checkpoint_progress().unwrap().unwrap();
    let port = new_port(initial, FleetHost::new(dir.path()), config());
    let first = port.application().maintain().unwrap();
    assert_eq!(
        first.pruned_inputs as u64,
        before.input_cut - before.deleted_through
    );
    for _ in 0..8 {
        let report = port.application().maintain().unwrap();
        assert_eq!(
            (
                report.retired_sandboxes,
                report.expired_results,
                report.pruned_inputs
            ),
            (0, 0, 0)
        );
    }
    drop(port);
    let reopened = ControllerJournal::open_complete(&path, NAMESPACE, host, policy, 0).unwrap();
    let after = reopened.checkpoint_progress().unwrap().unwrap();
    assert_eq!(after.input_high_water, before.input_high_water);
    assert_eq!(after.input_cut, before.input_cut);
    assert_eq!(after.deleted_through, before.input_cut);
}
