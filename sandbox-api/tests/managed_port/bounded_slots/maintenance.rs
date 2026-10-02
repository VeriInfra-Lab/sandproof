//! Real production maintenance coordinator, reducer and `SQLite`; only exact
//! physical release observations use a controlled, incarnation-aware host.
use super::*;
use sandproof_controller_service::MaintenanceConfig;
use std::path::{Path, PathBuf};

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

impl HostAdapter for FleetHost {
    type Evidence = Evidence;

    fn bind_host_slot(&mut self, context: &HostSlotContext, allow: bool) -> Result<(), PortError> {
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
    fn release_host_slot(&mut self, context: &HostSlotContext) -> Result<(), PortError> {
        self.sandbox(context.binding.sandbox)?
            .release_host_slot(context)
    }
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
    fn available_disk_bytes(&self) -> Result<u64, PortError> {
        Ok(10_000)
    }
    fn put_secret(&mut self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError> {
        self.sandbox(sandbox)?.put_secret(sandbox, token)
    }
    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, PortError> {
        self.sandbox(sandbox)?.has_secret(sandbox)
    }
    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, PortError> {
        self.sandbox(sandbox)?.token_for(sandbox)
    }
    fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Evidence>, WorkerTransportError> {
        self.sandbox(action.command.command().sandbox())
            .map_err(|_| WorkerTransportError::ConnectionLost)?
            .execute(action)
    }
    fn observe(&self, _: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Evidence>> {
        None
    }
    fn reclaim_incomplete_prepare(
        &mut self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }
    fn reclaim_incomplete_cleanup(
        &mut self,
        _: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }
}

fn host_config() -> BoundedHostConfig {
    let mut config = bounded();
    config.maintenance.max_host_slots = 2;
    config.maintenance.max_retained_results = 64;
    config
}

fn maintenance_config() -> MaintenanceConfig {
    MaintenanceConfig {
        retention_ms: 60_000,
        max_checkpoint_bytes: 8_000_000,
        max_records: 10_000,
        max_references: 100_000,
        max_tail_inputs: 10_000,
        max_tail_bytes: 8_000_000,
        max_input_bytes: 128_000,
        prune_batch_rows: 3,
    }
}

fn open_maintained(path: &Path, host: FleetHost) -> ManagedPort<FleetHost> {
    let mut limits = config();
    limits.limits.max_active_per_tenant = 2;
    limits.limits.max_disk_bytes_per_tenant = 2_000;
    ManagedPort::new(
        ControllerJournal::open_maintained(path, NAMESPACE, host_config(), maintenance_config(), 0)
            .unwrap(),
        host,
        limits,
    )
}

fn journal(path: &Path) -> ControllerJournal {
    ControllerJournal::open_maintained(path, NAMESPACE, host_config(), maintenance_config(), 0)
        .unwrap()
}

#[test]
fn production_tick_retires_newer_objects_while_old_live_or_unknown_object_stays_exact() {
    for unknown in [false, true] {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("controller.db");
        let host = FleetHost::new(dir.path());
        let port = open_maintained(&path, host.clone());
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
            let report = port.maintain().unwrap();
            assert_eq!(report.retired_sandboxes, 1);
            assert!(report.pruned_inputs > 3);
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
        let world = journal.world().unwrap();
        assert_eq!(world.sandbox_records().len(), 1);
        assert!(world.sandbox_record(core::SandboxId(1)).is_some());
        assert_eq!(world.maintenance_state().unwrap().retained_count(), 16);
        assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(10));
    }
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
            .data_permit(
                &old.envd_access_token,
                sandproof_api::managed::DataKind::Exec,
            )
            .unwrap();
        if unknown {
            permit.drain.mark_unknown();
        }
        port.kill("1", &old.sandbox_id).unwrap();
        let report = port.maintain().unwrap();
        assert_eq!(report.retired_sandboxes, 0);
        assert_eq!(report.tracked_data_sandboxes, 1);
        drop(permit);
        if unknown {
            assert_eq!(port.maintain().unwrap().retired_sandboxes, 0);
            host.sandbox(core::SandboxId(1))
                .unwrap()
                .0
                .lock()
                .unwrap()
                .gate_observation = Some(true);
        }
        let report = port.maintain().unwrap();
        assert_eq!(report.retired_sandboxes, 1);
        assert_eq!(report.tracked_data_sandboxes, 0);
        assert!(matches!(
            port.authorize_data(
                &old.envd_access_token,
                sandproof_api::managed::DataKind::Exec
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
    assert_eq!(port.maintain().unwrap().retired_sandboxes, 1);
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
    assert_eq!(port.recover_on_startup().unwrap().unresolved_sandboxes, 0);
    assert_eq!(port.maintain().unwrap().retired_sandboxes, 0);
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
    assert_eq!(port.maintain().unwrap().retired_sandboxes, 1);
    drop(port);
    assert!(journal(&path).world().unwrap().sandbox_records().is_empty());
}

#[test]
fn private_build_retry_anchor_remains_pinned_and_keeps_same_identity() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = FleetHost::new(dir.path());
    let port = open_maintained(&path, host).with_internal_build_vms(true);
    let key = "123456789abcdef0123456789abcdef0";
    let input = sandproof_api::BuildVmRequest {
        timeout: 300,
        allow_internet_access: false,
    };
    let build = port.build_vm("1", key, Some(input.clone())).unwrap();
    port.kill("1", &build.sandbox_id).unwrap();
    assert_eq!(port.maintain().unwrap().retired_sandboxes, 0);
    assert_eq!(
        port.build_vm("1", key, Some(input)).unwrap().sandbox_id,
        build.sandbox_id
    );
    drop(port);
    let db = journal(&path);
    assert_eq!(db.world().unwrap().sandbox_records().len(), 1);
    assert_eq!(
        db.host_slot_inventory().unwrap()[0].1,
        HostSlotPhase::Occupied
    );
}

#[test]
fn unconfigured_production_tick_preserves_legacy_history() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let port = open(&path, BoundedHost::default());
    let session = port.create("1", request()).unwrap();
    port.kill("1", &session.sandbox_id).unwrap();
    assert_eq!(
        port.maintain().unwrap(),
        sandproof_api::managed::MaintenanceReport::default()
    );
    drop(port);
    let db = ControllerJournal::open_bounded(&path, NAMESPACE, bounded(), 0).unwrap();
    assert_eq!(db.world().unwrap().sandbox_records().len(), 1);
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
    assert_eq!(port.maintain().unwrap().retired_sandboxes, 1);
}

#[test]
fn full_result_retention_never_evicts_promised_results_to_retire_another_object() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = FleetHost::new(dir.path());
    let mut bounded = host_config();
    bounded.maintenance.max_retained_results = 2;
    let port = ManagedPort::new(
        ControllerJournal::open_maintained(
            &path,
            NAMESPACE,
            bounded.clone(),
            maintenance_config(),
            0,
        )
        .unwrap(),
        host,
        config(),
    );
    let first = port.create("1", request()).unwrap();
    port.kill("1", &first.sandbox_id).unwrap();
    assert_eq!(port.maintain().unwrap().retired_sandboxes, 1);
    let second = port.create("1", request()).unwrap();
    port.kill("1", &second.sandbox_id).unwrap();
    let report = port.maintain().unwrap();
    assert_eq!(report.retired_sandboxes, 0);
    assert_eq!(report.pending_sandboxes, 1);
    assert_eq!(report.expired_results, 0);
    drop(port);
    let journal =
        ControllerJournal::open_maintained(&path, NAMESPACE, bounded, maintenance_config(), 0)
            .unwrap();
    let world = journal.world().unwrap();
    assert_eq!(world.maintenance_state().unwrap().retained_count(), 2);
    assert!(world.sandbox_record(core::SandboxId(1)).is_none());
    assert!(world.sandbox_record(core::SandboxId(2)).is_some());
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
