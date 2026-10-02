//! Exercise the production admission/workflow/recovery path with one physical
//! position. Only physical observation is replaced, using the shared reducer.
use super::*;
use sandbox_controller_core::maintenance::{
    MaintenanceLimits,
    slots::{HostSlotPhase, HostSlotRecord, SlotInput, reduce_slot},
    windows::WindowLimits,
};
use sandproof_controller_service::{BoundedHostConfig, HostSlotContext};
use std::sync::Mutex;

fn bounded() -> BoundedHostConfig {
    BoundedHostConfig {
        physical_storage: None,
        console_policy: None,
        uid_base: 2000,
        uplink_base: 12,
        max_data_routes_per_slot: 4,
        max_data_routes_global: 8,
        data_route_control_reserve: 1,
        maintenance: MaintenanceLimits {
            max_host_slots: 1,
            max_retained_results: 16,
            windows: WindowLimits::new(2, 1000).unwrap(),
        },
    }
}

#[derive(Default)]
struct DurableHost {
    physical_profile:
        Option<sandproof_controller_service::physical_storage::PhysicalStorageProfile>,
    physical_free: Option<u64>,
    row: Option<(HostSlotContext, HostSlotPhase)>,
    secrets: BTreeMap<u64, String>,
    actions: Vec<WorkAction<core::EffectId>>,
    lose_bind: bool,
    lose_release: bool,
    unknown_cleanup: bool,
    gate_observation: Option<bool>,
    gate_path: Option<std::path::PathBuf>,
    relay_spawns: u64,
}

#[derive(Clone, Default)]
struct BoundedHost(Arc<Mutex<DurableHost>>);

impl BoundedHost {
    fn eligible(state: &DurableHost, action: &WorkAction<core::EffectId>) -> bool {
        let Some((context, HostSlotPhase::Occupied)) = state.row else {
            return false;
        };
        if context.binding.sandbox != action.command.command().sandbox()
            || context.binding.generation != action.identity.generation
            || action.identity.owner.as_str() != format!("tenant-{}", context.binding.owner.0)
        {
            return false;
        }
        match action.command.command() {
            core::Command::PrepareVm { vm, slot, .. }
            | core::Command::AbortPrepare { vm, slot, .. }
            | core::Command::ConfigureNetwork { vm, slot, .. } => {
                *vm == context.binding.vm && *slot == context.binding.network
            }
            core::Command::StopVm { vm, .. }
            | core::Command::CleanupVm { vm, .. }
            | core::Command::ActivateVm { vm, .. } => *vm == context.binding.vm,
            core::Command::CleanupSlot { slot, .. } => *slot == context.binding.network,
            core::Command::InstallFence { .. } | core::Command::ApplyFilePolicy { .. } => true,
            _ => false,
        }
    }
}

impl HostAdapter for BoundedHost {
    type Evidence = Evidence;
    fn bind_host_slot(
        &mut self,
        context: &HostSlotContext,
        allow_install: bool,
    ) -> Result<(), PortError> {
        let mut state = self.0.lock().unwrap();
        if state.row != Some((*context, HostSlotPhase::Occupied)) {
            if !allow_install {
                return Err(PortError::Busy);
            }
            let previous = state.row.map(|(context, phase)| HostSlotRecord {
                binding: context.binding,
                phase,
            });
            let row = reduce_slot(
                previous,
                SlotInput::Bind {
                    position: context.position,
                    expected_previous: context.expected_previous,
                    binding: context.binding,
                },
            )
            .map_err(|_| PortError::Busy)?;
            state.row = Some((*context, row.phase));
        }
        if std::mem::take(&mut state.lose_bind) {
            return Err(PortError::Busy);
        }
        Ok(())
    }
    fn release_host_slot(&mut self, context: &HostSlotContext) -> Result<(), PortError> {
        let mut state = self.0.lock().unwrap();
        let (current, phase) = state.row.ok_or(PortError::Busy)?;
        if current != *context {
            return Err(PortError::Busy);
        }
        if phase != HostSlotPhase::Free {
            let row = reduce_slot(
                Some(HostSlotRecord {
                    binding: current.binding,
                    phase,
                }),
                SlotInput::BeginRelease {
                    position: context.position,
                    binding: context.binding,
                },
            )
            .map_err(|_| PortError::Busy)?;
            let free = reduce_slot(
                Some(row),
                SlotInput::ConfirmRelease {
                    position: context.position,
                    binding: context.binding,
                },
            )
            .map_err(|_| PortError::Busy)?;
            state.row = Some((current, free.phase));
            state.secrets.remove(&context.binding.sandbox.0);
        }
        if std::mem::take(&mut state.lose_release) {
            return Err(PortError::Busy);
        }
        Ok(())
    }
    fn acquire_data_gate(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        generation: core::Generation,
    ) -> Result<Option<std::fs::File>, PortError> {
        let state = self.0.lock().unwrap();
        if state.row.is_none_or(|(context, phase)| {
            context.binding.sandbox != sandbox
                || context.binding.vm != vm
                || context.binding.generation != generation
                || phase != HostSlotPhase::Occupied
        }) {
            return Err(PortError::Busy);
        }
        let file = std::fs::File::open(state.gate_path.as_ref().ok_or(PortError::Busy)?)
            .map_err(|_| PortError::Busy)?;
        file.try_lock_shared().map_err(|_| PortError::Busy)?;
        Ok(Some(file))
    }

    fn spawn_data_relay(
        &mut self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        generation: core::Generation,
        _control: bool,
    ) -> Result<Option<std::process::Child>, PortError> {
        let mut state = self.0.lock().unwrap();
        if state.row.is_none_or(|(context, phase)| {
            context.binding.sandbox != sandbox
                || context.binding.vm != vm
                || context.binding.generation != generation
                || phase != HostSlotPhase::Occupied
        }) {
            return Err(PortError::Busy);
        }
        state.relay_spawns += 1;
        std::process::Command::new("/bin/cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(Some)
            .map_err(|_| PortError::Busy)
    }

    fn data_gate_drained(&self, context: &HostSlotContext) -> Result<bool, PortError> {
        let state = self.0.lock().unwrap();
        if state.row.is_none_or(|(bound, _)| bound != *context) {
            return Err(PortError::Busy);
        }
        Ok(state.gate_observation == Some(true))
    }

    fn available_disk_bytes(&self) -> Result<u64, PortError> {
        Ok(self.0.lock().unwrap().physical_free.unwrap_or(10_000))
    }
    fn validate_physical_storage(
        &self,
        profile: &sandproof_controller_service::physical_storage::PhysicalStorageProfile,
        _pool: &sandproof_controller_service::physical_storage::PhysicalPool,
    ) -> Result<(), PortError> {
        if self.0.lock().unwrap().physical_profile.as_ref() == Some(profile) {
            Ok(())
        } else {
            Err(PortError::Busy)
        }
    }
    fn put_secret(&mut self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError> {
        let mut state = self.0.lock().unwrap();
        if state.row.is_none_or(|(context, phase)| {
            context.binding.sandbox != sandbox || phase != HostSlotPhase::Occupied
        }) {
            return Err(PortError::Busy);
        }
        if state.secrets.insert(sandbox.0, token.into()).is_some() {
            return Err(PortError::Busy);
        }
        Ok(())
    }
    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, PortError> {
        Ok(self.0.lock().unwrap().secrets.contains_key(&sandbox.0))
    }
    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, PortError> {
        self.0
            .lock()
            .unwrap()
            .secrets
            .get(&sandbox.0)
            .cloned()
            .ok_or(PortError::Busy)
    }
    fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Evidence>, WorkerTransportError> {
        let mut state = self.0.lock().unwrap();
        if !Self::eligible(&state, action) {
            return Err(WorkerTransportError::ConnectionLost);
        }
        state.actions.push(action.clone());
        if state.unknown_cleanup
            && matches!(action.command.command(), core::Command::CleanupVm { .. })
        {
            return Ok(unknown_receipt(action));
        }
        Ok(WorkReceipt {
            identity: action.identity.clone(),
            outcome: WorkOutcome::ObservedSuccess(Evidence {
                resource: action.identity.resource.clone(),
                command: action.command.command().clone(),
            }),
        })
    }
    fn observe(&self, _action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Evidence>> {
        None
    }
    fn reclaim_incomplete_prepare(
        &mut self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }
    fn reclaim_incomplete_cleanup(
        &mut self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }
}

fn open(path: &std::path::Path, host: BoundedHost) -> ManagedPort<BoundedHost> {
    {
        let mut state = host.0.lock().unwrap();
        if state.gate_path.is_none() {
            let gate_path = path.with_extension("data-gate");
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&gate_path)
                .unwrap();
            state.gate_path = Some(gate_path);
        }
    }
    let mut config = config();
    config.limits.max_vm_id = 1; // Legacy physical ceiling must not cap permanent IDs.
    ManagedPort::new(
        ControllerJournal::open_bounded(path, NAMESPACE, bounded(), 0).unwrap(),
        host,
        config,
    )
}

#[test]
fn one_position_serves_many_permanent_ids_and_delayed_old_commands_never_touch_successor() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = BoundedHost::default();
    let port = open(&path, host.clone());
    let mut old = Vec::new();
    for number in 1..=32 {
        let session = port.create("1", request()).unwrap();
        assert_eq!(session.sandbox_id, format!("sb-{NAMESPACE}-{number}"));
        let row = host.0.lock().unwrap().row.unwrap();
        assert_eq!(row.0.position.0, 0);
        assert_eq!(row.0.binding.vm.0, number - 1);
        assert_eq!(
            row.0.expected_previous.map(|vm| vm.0),
            number.checked_sub(2)
        );
        for stale in &old {
            assert!(host.clone().execute(stale).is_err());
        }
        port.kill("1", &session.sandbox_id).unwrap();
        let state = host.0.lock().unwrap();
        assert_eq!(state.row.unwrap().1, HostSlotPhase::Free);
        assert!(state.secrets.is_empty());
        old.clone_from(&state.actions);
    }
    drop(port);
    let journal = ControllerJournal::open_bounded(&path, NAMESPACE, bounded(), 0).unwrap();
    assert_eq!(journal.host_slot_contexts().unwrap().len(), 1);
    assert_eq!(journal.next_sandbox_id().unwrap().0, 33);
    assert_eq!(
        journal.host_slot_inventory().unwrap()[0].1,
        HostSlotPhase::Free
    );
}

#[test]
fn lost_bind_ack_restarts_before_first_fence_without_allocating_another_identity() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = BoundedHost::default();
    host.0.lock().unwrap().lose_bind = true;
    let port = open(&path, host.clone());
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::OutcomeUnknown)
    ));
    assert!(host.0.lock().unwrap().actions.is_empty());
    drop(port);
    let port = open(&path, host.clone());
    assert_eq!(port.recover_on_startup().unwrap().unresolved_sandboxes, 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1"))
            .unwrap()
            .info
            .state,
        "running"
    );
    assert_eq!(host.0.lock().unwrap().row.unwrap().0.binding.vm.0, 0);
}

#[test]
fn lost_release_ack_keeps_durable_releasing_until_exact_host_recovery() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = BoundedHost::default();
    let port = open(&path, host.clone());
    let session = port.create("1", request()).unwrap();
    host.0.lock().unwrap().lose_release = true;
    port.kill("1", &session.sandbox_id).unwrap();
    drop(port);
    let journal = ControllerJournal::open_bounded(&path, NAMESPACE, bounded(), 0).unwrap();
    assert_eq!(
        journal.host_slot_inventory().unwrap()[0].1,
        HostSlotPhase::Releasing
    );
    drop(journal);
    let port = open(&path, host.clone());
    port.recover_on_startup().unwrap();
    assert!(port.create("1", request()).is_ok());
}

#[test]
fn unknown_cleanup_and_data_or_relay_pins_block_slot_reuse() {
    for unknown_cleanup in [false, true] {
        let dir = TempDir::new().unwrap();
        let host = BoundedHost::default();
        host.0.lock().unwrap().unknown_cleanup = unknown_cleanup;
        let port = open(&dir.path().join("controller.db"), host.clone());
        let session = port.create("1", request()).unwrap();
        let permit = port
            .data_permit(
                &session.envd_access_token,
                sandproof_api::managed::DataKind::Exec,
            )
            .unwrap();
        let _ = port.kill("1", &session.sandbox_id);
        assert!(matches!(
            port.create("1", request()),
            Err(PortError::QuotaExceeded)
        ));
        drop(permit);
        if unknown_cleanup {
            assert!(matches!(
                port.create("1", request()),
                Err(PortError::QuotaExceeded)
            ));
        } else {
            assert!(port.create("1", request()).is_ok());
        }
    }
}

#[test]
fn unknown_relay_exit_stays_blocked_after_every_pin_drops() {
    let dir = TempDir::new().unwrap();
    let host = BoundedHost::default();
    let port = open(&dir.path().join("controller.db"), host);
    let session = port.create("1", request()).unwrap();
    let permit = port
        .data_permit(
            &session.envd_access_token,
            sandproof_api::managed::DataKind::Exec,
        )
        .unwrap();
    permit.drain.mark_unknown();
    port.kill("1", &session.sandbox_id).unwrap();
    drop(permit);
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::QuotaExceeded)
    ));
}

#[test]
fn bounded_format_and_configuration_cannot_be_disabled_changed_or_adopt_nonempty_legacy() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bounded.db");
    drop(open(&path, BoundedHost::default()));
    assert!(ControllerJournal::open(&path, NAMESPACE).is_err());
    for changed in [
        BoundedHostConfig {
            physical_storage: None,
            console_policy: None,
            uid_base: 2001,
            ..bounded()
        },
        BoundedHostConfig {
            uplink_base: 13,
            ..bounded()
        },
    ] {
        assert!(ControllerJournal::open_bounded(&path, NAMESPACE, changed, 0).is_err());
    }
    let legacy_path = dir.path().join("legacy.db");
    let legacy = ManagedPort::new(
        ControllerJournal::open(&legacy_path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    legacy.create("1", request()).unwrap();
    drop(legacy);
    assert!(ControllerJournal::open_bounded(&legacy_path, NAMESPACE, bounded(), 0).is_err());
    assert!(ControllerJournal::open(&legacy_path, NAMESPACE).is_ok());
}

#[test]
fn unknown_relay_exit_resolves_only_after_observed_durable_gate_drain() {
    let dir = TempDir::new().unwrap();
    let host = BoundedHost::default();
    let port = open(&dir.path().join("controller.db"), host.clone());
    let session = port.create("1", request()).unwrap();
    let permit = port
        .data_permit(
            &session.envd_access_token,
            sandproof_api::managed::DataKind::Exec,
        )
        .unwrap();
    permit.drain.mark_unknown();
    port.kill("1", &session.sandbox_id).unwrap();
    drop(permit);
    host.0.lock().unwrap().gate_observation = Some(false);
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::QuotaExceeded)
    ));
    host.0.lock().unwrap().gate_observation = Some(true);
    assert!(port.create("1", request()).is_ok());
}

#[test]
fn relay_spawn_rechecks_authority_after_the_initial_route_permit() {
    let dir = TempDir::new().unwrap();
    let host = BoundedHost::default();
    let port = open(&dir.path().join("controller.db"), host.clone());
    let session = port.create("1", request()).unwrap();
    let permit = port
        .data_permit(
            &session.envd_access_token,
            sandproof_api::managed::DataKind::Exec,
        )
        .unwrap();
    let mut child = port.spawn_data_relay(&permit, false).unwrap().unwrap();
    drop(child.take_stdin());
    assert!(child.wait().unwrap().success());
    assert_eq!(host.0.lock().unwrap().relay_spawns, 1);
    port.kill("1", &session.sandbox_id).unwrap();
    assert!(matches!(
        port.spawn_data_relay(&permit, false),
        Err(PortError::NotFound)
    ));
    assert_eq!(host.0.lock().unwrap().relay_spawns, 1);
}

#[path = "bounded_slots/maintenance.rs"]
mod maintenance;

#[path = "bounded_slots/build_retry.rs"]
mod build_retry;

#[path = "bounded_slots/physical_storage.rs"]
mod physical_storage;
