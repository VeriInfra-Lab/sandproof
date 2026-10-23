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

fn bounded(path: &std::path::Path) -> BoundedHostConfig {
    // The exact fixture pool is present before the production complete opener.
    let mut config = complete_fixture::host(path, NAMESPACE, &config());
    config.uid_base = 2000;
    config.uplink_base = 12;
    config.maintenance = MaintenanceLimits {
        max_host_slots: 1,
        max_retained_results: 128,
        windows: WindowLimits::new(2, 1000).unwrap(),
    };
    config.physical_storage.as_mut().unwrap().pools[0].max_host_slots = 1;
    config
}

fn open_journal(path: &std::path::Path) -> ControllerJournal {
    ControllerJournal::open_complete(
        path,
        NAMESPACE,
        bounded(path),
        complete_fixture::maintenance(),
        0,
    )
    .unwrap()
}

#[derive(Default, PartialEq)]
enum BindingFault {
    #[default]
    None,
    Refused,
    LostAck,
}

#[derive(Default)]
struct DurableHost {
    physical_error: bool,
    physical_profile:
        Option<sandproof_controller_service::physical_storage::PhysicalStorageProfile>,
    physical_free: Option<u64>,
    row: Option<(HostSlotContext, HostSlotPhase)>,
    secrets: BTreeMap<u64, String>,
    actions: Vec<WorkAction<core::EffectId>>,
    binding_fault: BindingFault,
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

impl WorkEvidence for BoundedHost {
    type Evidence = Evidence;
}
impl WorkExecution for BoundedHost {
    fn execute(
        &self,
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
}

impl WorkRecovery for BoundedHost {
    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Evidence>> {
        if !matches!(
            action.command.command(),
            core::Command::InstallFence { terminal: true, .. }
        ) || !self.0.lock().unwrap().actions.contains(action)
        {
            return None;
        }
        Some(WorkReceipt {
            identity: action.identity.clone(),
            outcome: WorkOutcome::ObservedSuccess(Evidence {
                resource: action.identity.resource.clone(),
                command: action.command.command().clone(),
            }),
        })
    }

    fn reclaim_incomplete_prepare(
        &self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }

    fn reclaim_incomplete_terminal_fence(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        assert!(matches!(
            action.command.command(),
            core::Command::InstallFence { terminal: true, .. }
        ));
        self.execute(action)
            .map(|_| ())
            .map_err(|_| PortError::Busy)
    }

    fn reclaim_incomplete_cleanup(
        &self,
        _action: &WorkAction<core::EffectId>,
    ) -> Result<(), PortError> {
        Err(PortError::Busy)
    }
}

impl HostSlots for BoundedHost {
    fn bind_host_slot(
        &self,
        context: &HostSlotContext,
        allow_install: bool,
    ) -> Result<(), PortError> {
        let mut state = self.0.lock().unwrap();
        if state.binding_fault == BindingFault::Refused {
            return Err(PortError::Busy);
        }
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
        if std::mem::take(&mut state.binding_fault) == BindingFault::LostAck {
            return Err(PortError::Busy);
        }
        Ok(())
    }

    fn release_host_slot(&self, context: &HostSlotContext) -> Result<(), PortError> {
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
}

impl DataRelay for BoundedHost {
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
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        generation: core::Generation,
        _control: bool,
    ) -> Result<std::process::Child, PortError> {
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
            .map_err(|_| PortError::Busy)
    }

    fn data_gate_drained(&self, context: &HostSlotContext) -> Result<bool, PortError> {
        let state = self.0.lock().unwrap();
        if state.row.is_none_or(|(bound, _)| bound != *context) {
            return Err(PortError::Busy);
        }
        Ok(state.gate_observation == Some(true))
    }
}

impl HostCapacity for BoundedHost {
    fn available_disk_bytes(&self) -> Result<u64, PortError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .physical_free
            .unwrap_or(16 * 1024 * 1024 * 1024))
    }

    fn validate_physical_storage(
        &self,
        profile: &sandproof_controller_service::physical_storage::PhysicalStorageProfile,
        _pool: &sandproof_controller_service::physical_storage::PhysicalPool,
    ) -> Result<(), PortError> {
        profile.validate().map_err(|_| PortError::Busy)?;
        let state = self.0.lock().unwrap();
        if !state.physical_error
            && state
                .physical_profile
                .as_ref()
                .is_none_or(|expected| expected == profile)
        {
            Ok(())
        } else {
            Err(PortError::Busy)
        }
    }
}

impl LaunchConfiguration for BoundedHost {
    fn put_secret(&self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError> {
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
}

impl TemplateArtifacts for BoundedHost {}

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
    let config = config();
    new_port(open_journal(path), host, config)
}

#[test]
fn one_position_serves_many_permanent_ids_and_delayed_old_commands_never_touch_successor() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = BoundedHost::default();
    let port = open(&path, host.clone());
    let mut old = Vec::new();
    for number in 1..=32 {
        let session = port
            .create("1", request())
            .unwrap_or_else(|error| panic!("create incarnation {number}: {error:?}"));
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
        // The service's ordinary coordinator retires completed metadata;
        // physical-slot reuse is exercised without an unmaintained journal.
        assert_eq!(port.application().maintain().unwrap().retired_sandboxes, 1);
        let state = host.0.lock().unwrap();
        assert_eq!(state.row.unwrap().1, HostSlotPhase::Free);
        assert!(state.secrets.is_empty());
        old.clone_from(&state.actions);
    }
    drop(port);
    let journal = open_journal(&path);
    assert_eq!(journal.host_slot_contexts().unwrap().len(), 1);
    assert_eq!(journal.next_sandbox_id().unwrap().0, 33);
    assert_eq!(
        journal.host_slot_inventory().unwrap()[0].1,
        HostSlotPhase::Free
    );
}

#[test]
fn cancelled_unbound_create_waits_without_claiming_actions_and_recovers_without_launch() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = BoundedHost::default();
    let port = open(&path, host.clone());
    let first = port.create("1", request()).unwrap();
    port.kill("1", &first.sandbox_id).unwrap();
    let previous = host.0.lock().unwrap().row;
    host.0.lock().unwrap().binding_fault = BindingFault::Refused;
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::OutcomeUnknown)
    ));
    assert!(matches!(
        port.kill("1", &format!("sb-{NAMESPACE}-2")),
        Err(PortError::OutcomeUnknown)
    ));
    assert_eq!(host.0.lock().unwrap().row, previous);
    drop(port);
    let journal = open_journal(&path);
    let world = journal.world().unwrap();
    assert!(
        world
            .effect_ids()
            .iter()
            .filter(|effect| effect.command().sandbox() == core::SandboxId(2))
            .all(|effect| world.effect_status(effect) == Some(core::EffectStatus::Committed))
    );
    drop(journal);
    host.0.lock().unwrap().binding_fault = BindingFault::None;
    let port = open(&path, host.clone());
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes,
        0
    );
    let state = host.0.lock().unwrap();
    assert_eq!(state.row.unwrap().1, HostSlotPhase::Free);
    assert!(state.secrets.is_empty());
    assert!(
        state
            .actions
            .iter()
            .filter(|action| action.command.command().sandbox() == core::SandboxId(2))
            .all(|action| matches!(
                action.command.command(),
                core::Command::InstallFence { terminal: true, .. }
            ))
    );
}

#[test]
fn unknown_terminal_fence_before_binding_recovers_only_the_original_cancel() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = BoundedHost::default();
    host.0.lock().unwrap().binding_fault = BindingFault::Refused;
    let port = open(&path, host.clone());
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::OutcomeUnknown)
    ));
    drop(port);
    let mut journal = open_journal(&path);
    // Inject cancellation's claim/host-loss boundary: it claimed its fence even
    // though physical binding had never succeeded. It never dispatched prepare.
    let failure = kill::<Evidence, _>(
        &mut journal,
        core::KillRequest {
            actor: core::Actor::Tenant(core::TenantId(1)),
            sandbox: core::SandboxId(1),
            operation: core::OperationId(2),
        },
        &mut |_| Err(WorkerTransportError::ConnectionLost),
    );
    assert!(failure.is_err());
    assert_eq!(journal.uncertain_actions().unwrap().len(), 1);
    drop(journal);
    host.0.lock().unwrap().binding_fault = BindingFault::None;
    let port = open(&path, host.clone());
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes,
        0
    );
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes,
        0
    );
    let state = host.0.lock().unwrap();
    assert!(state.secrets.is_empty());
    assert_eq!(state.row.unwrap().1, HostSlotPhase::Free);
    assert_eq!(state.row.unwrap().0.binding.sandbox, core::SandboxId(1));
    assert_eq!(state.actions.len(), 1);
    assert!(matches!(
        state.actions[0].command.command(),
        core::Command::InstallFence { terminal: true, .. }
    ));
    drop(state);
    drop(port);
    let journal = open_journal(&path);
    assert_eq!(journal.next_sandbox_id().unwrap().0, 2);
    assert_eq!(
        journal
            .world()
            .unwrap()
            .sandbox_record(core::SandboxId(1))
            .unwrap()
            .phase,
        core::Phase::Destroyed
    );
}

#[test]
fn lost_binding_after_launch_cannot_be_reinstalled_for_terminal_cleanup() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = BoundedHost::default();
    let port = open(&path, host.clone());
    let session = port.create("1", request()).unwrap();
    host.0.lock().unwrap().row = None;
    let before = host.0.lock().unwrap().actions.len();
    assert!(matches!(
        port.kill("1", &session.sandbox_id),
        Err(PortError::OutcomeUnknown)
    ));
    drop(port);
    let port = open(&path, host.clone());
    assert!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes
            > 0
    );
    let state = host.0.lock().unwrap();
    assert!(state.row.is_none());
    assert_eq!(state.actions.len(), before);
}

#[test]
fn lost_bind_ack_restarts_before_first_fence_without_allocating_another_identity() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let host = BoundedHost::default();
    host.0.lock().unwrap().binding_fault = BindingFault::LostAck;
    let port = open(&path, host.clone());
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::OutcomeUnknown)
    ));
    assert!(host.0.lock().unwrap().actions.is_empty());
    drop(port);
    let port = open(&path, host.clone());
    assert_eq!(
        port.application()
            .recover_on_startup()
            .unwrap()
            .unresolved_sandboxes,
        0
    );
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
    let journal = open_journal(&path);
    assert_eq!(
        journal.host_slot_inventory().unwrap()[0].1,
        HostSlotPhase::Releasing
    );
    drop(journal);
    let port = open(&path, host.clone());
    port.application().recover_on_startup().unwrap();
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
            .application()
            .data_permit(
                &session.envd_access_token,
                sandproof_controller_service::application::DataKind::Exec,
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
        .application()
        .data_permit(
            &session.envd_access_token,
            sandproof_controller_service::application::DataKind::Exec,
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
fn current_configuration_cannot_be_changed_and_legacy_stores_are_rejected() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("bounded.db");
    drop(open(&path, BoundedHost::default()));
    // A different complete profile is refused, not treated as a mode change.
    assert!(complete_fixture::open(&path, NAMESPACE).is_err());
    for changed in [
        BoundedHostConfig {
            uid_base: 2001,
            ..bounded(&path)
        },
        BoundedHostConfig {
            uplink_base: 13,
            ..bounded(&path)
        },
    ] {
        assert!(
            ControllerJournal::open_complete(
                &path,
                NAMESPACE,
                changed,
                complete_fixture::maintenance(),
                0
            )
            .is_err()
        );
    }
    let legacy_path = dir.path().join("legacy.db");
    rusqlite::Connection::open(&legacy_path).unwrap().execute_batch(
        "CREATE TABLE controller_meta(id INTEGER PRIMARY KEY,version INTEGER,namespace TEXT); PRAGMA application_id=0x53504631; PRAGMA user_version=1;"
    ).unwrap();
    let before = std::fs::read(&legacy_path).unwrap();
    assert!(
        ControllerJournal::open_complete(
            &legacy_path,
            NAMESPACE,
            bounded(&legacy_path),
            complete_fixture::maintenance(),
            0
        )
        .is_err()
    );
    assert!(complete_fixture::open(&legacy_path, NAMESPACE).is_err());
    assert_eq!(std::fs::read(&legacy_path).unwrap(), before);
}

#[test]
fn unknown_relay_exit_resolves_only_after_observed_durable_gate_drain() {
    let dir = TempDir::new().unwrap();
    let host = BoundedHost::default();
    let port = open(&dir.path().join("controller.db"), host.clone());
    let session = port.create("1", request()).unwrap();
    let permit = port
        .application()
        .data_permit(
            &session.envd_access_token,
            sandproof_controller_service::application::DataKind::Exec,
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
        .application()
        .data_permit(
            &session.envd_access_token,
            sandproof_controller_service::application::DataKind::Exec,
        )
        .unwrap();
    let mut child = port.application().spawn_data_relay(&permit, false).unwrap();
    drop(child.take_stdin());
    assert!(child.wait().unwrap().success());
    assert_eq!(host.0.lock().unwrap().relay_spawns, 1);
    port.kill("1", &session.sandbox_id).unwrap();
    assert!(matches!(
        port.application().spawn_data_relay(&permit, false),
        Err(PortError::NotFound)
    ));
    assert_eq!(host.0.lock().unwrap().relay_spawns, 1);
}

#[test]
fn expired_permit_cannot_spawn_a_relay_before_the_expiry_worker_runs() {
    let dir = TempDir::new().unwrap();
    let host = BoundedHost::default();
    let port = open(&dir.path().join("controller.db"), host.clone());
    let session = port.create("1", request()).unwrap();
    let permit = port
        .application()
        .data_permit(
            &session.envd_access_token,
            sandproof_controller_service::application::DataKind::Exec,
        )
        .unwrap();
    // The SDK's nonpositive timeout expires the durable deadline immediately;
    // no sleep, guessed scanner timing, or physical stop is needed here.
    port.set_timeout("1", &session.sandbox_id, 0).unwrap();
    assert!(*permit.revocation.borrow());
    assert!(matches!(
        port.application().spawn_data_relay(&permit, false),
        Err(PortError::NotFound)
    ));
    assert_eq!(host.0.lock().unwrap().relay_spawns, 0);
    assert_eq!(port.application().expire_due().unwrap(), 1);
}

#[path = "bounded_slots/maintenance.rs"]
mod maintenance;

#[path = "bounded_slots/build_retry.rs"]
mod build_retry;

#[path = "bounded_slots/physical_storage.rs"]
mod physical_storage;
