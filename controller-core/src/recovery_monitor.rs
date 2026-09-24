use crate::model::{EffectId, Phase, World};
use crate::{Generation, SandboxId};

/// Controller-runtime inputs represented by `SandboxModel.RuntimeController`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeInput {
    Crash,
    Recover,
    DuplicateOrLateDelivery(EffectId),
}

/// Runtime events emitted by an accepted input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeEvent {
    ControllerCrash,
    ControllerRecover,
    DuplicateOrLateDelivery(EffectId),
}

/// Rejections from the ordered runtime guards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeReject {
    ControllerAlreadyDown,
    ControllerAlreadyUp,
    EffectNotFound,
}

/// Successful pure reduction of a controller-runtime input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeDecision {
    pub event: RuntimeEvent,
    pub next: World,
}

/// Rust counterpart of `RuntimeController.handleCrash`.
///
/// # Errors
///
/// Returns [`RuntimeReject::ControllerAlreadyDown`] when the controller is
/// already down. Rejection leaves the supplied world untouched.
pub fn handle_controller_crash(world: &World) -> Result<RuntimeDecision, RuntimeReject> {
    if !world.controller_up {
        return Err(RuntimeReject::ControllerAlreadyDown);
    }

    let mut next = world.clone();
    next.controller_up = false;
    Ok(RuntimeDecision {
        event: RuntimeEvent::ControllerCrash,
        next,
    })
}

/// Rust counterpart of `RuntimeController.handleRecover`.
///
/// # Errors
///
/// Returns [`RuntimeReject::ControllerAlreadyUp`] when the controller is
/// already up. Rejection leaves the supplied world untouched.
pub fn handle_controller_recover(world: &World) -> Result<RuntimeDecision, RuntimeReject> {
    if world.controller_up {
        return Err(RuntimeReject::ControllerAlreadyUp);
    }

    let mut next = world.clone();
    next.controller_up = true;
    Ok(RuntimeDecision {
        event: RuntimeEvent::ControllerRecover,
        next,
    })
}

/// Rust counterpart of `RuntimeController.handleDuplicateOrLateDelivery`.
///
/// This observation is deliberately available while the controller is down:
/// the matching Lean `Step` has only the persisted-effect guard and is a
/// state-preserving transition.
///
/// # Errors
///
/// Returns [`RuntimeReject::EffectNotFound`] unless the exact attempt is in
/// the durable effect table.
pub fn handle_duplicate_or_late_delivery(
    world: &World,
    effect: EffectId,
) -> Result<RuntimeDecision, RuntimeReject> {
    if !world.effects.contains_key(&effect) {
        return Err(RuntimeReject::EffectNotFound);
    }

    Ok(RuntimeDecision {
        event: RuntimeEvent::DuplicateOrLateDelivery(effect),
        next: world.clone(),
    })
}

/// Deterministic coordinator for controller-runtime inputs.
///
/// # Errors
///
/// Returns the first rejection from the selected handler's Lean guard.
pub fn handle_runtime(
    world: &World,
    input: RuntimeInput,
) -> Result<RuntimeDecision, RuntimeReject> {
    match input {
        RuntimeInput::Crash => handle_controller_crash(world),
        RuntimeInput::Recover => handle_controller_recover(world),
        RuntimeInput::DuplicateOrLateDelivery(effect) => {
            handle_duplicate_or_late_delivery(world, effect)
        }
    }
}

/// Observable sandbox projection used by the verified trace monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SandboxView {
    pub phase: Phase,
    pub terminal: bool,
    pub generation: Generation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MonitorState {
    accepted: bool,
    seen_terminal: bool,
}

fn monitor_observation(seen_terminal: bool, observation: Option<SandboxView>) -> MonitorState {
    match observation {
        None => MonitorState {
            accepted: true,
            seen_terminal,
        },
        Some(current) if !seen_terminal => MonitorState {
            accepted: true,
            seen_terminal: current.terminal,
        },
        Some(current) => MonitorState {
            accepted: current.terminal && current.phase != Phase::Running,
            seen_terminal: true,
        },
    }
}

/// Rust counterpart of `Monitor.view`.
#[must_use]
#[allow(clippy::manual_map)]
pub fn view(world: &World, sandbox: SandboxId) -> Option<SandboxView> {
    // Keep the projection as a direct match. Besides mirroring Lean's
    // definition, this avoids adding `Option::map` to the extraction boundary.
    match world.sandboxes.get(&sandbox) {
        None => None,
        Some(record) => Some(SandboxView {
            phase: record.phase,
            terminal: record.terminal,
            generation: record.generation,
        }),
    }
}

/// Rust counterpart of `Monitor.monitorFrom`.
///
/// `seen_terminal` is a latch. Missing observations are skipped in either
/// state. Before the latch is set, an observation only updates it; after it is
/// set, every observable state must remain terminal and must not be running.
#[must_use]
pub fn monitor_from(mut seen_terminal: bool, views: &[Option<SandboxView>]) -> bool {
    let mut index = 0;
    while index < views.len() {
        let next = monitor_observation(seen_terminal, views[index]);
        if !next.accepted {
            return false;
        }
        seen_terminal = next.seen_terminal;
        index += 1;
    }
    true
}

/// Check an observation sequence from an initially clear terminal latch.
#[must_use]
pub fn monitor(views: &[Option<SandboxView>]) -> bool {
    monitor_from(false, views)
}

#[cfg(test)]
mod tests {
    use super::{
        RuntimeEvent, RuntimeInput, RuntimeReject, SandboxView, handle_controller_crash,
        handle_controller_recover, handle_duplicate_or_late_delivery, handle_runtime, monitor,
        monitor_from, view,
    };
    use crate::model::{
        AppliedFilePolicy, Command, FilePolicyRef, NetSlot, NetworkPolicy, Phase, ResourceIndex,
        Snapshot, SnapshotManifestRef, SnapshotStage, Vm, VmStatus, WorkerFence, World,
    };
    use crate::transition::begin_create;
    use crate::{
        FsId, ManifestDigest, NetSlotId, OperationId, PolicyDigest, SandboxId, SnapshotId,
        TenantId, VmId,
    };

    const OWNER: TenantId = TenantId::new(1);
    const SANDBOX: SandboxId = SandboxId::new(2);
    const WORKSPACE: FsId = FsId::new(3);
    const VM: VmId = VmId::new(4);
    const SLOT: NetSlotId = NetSlotId::new(5);
    const SNAPSHOT: SnapshotId = SnapshotId::new(6);
    const OPERATION: OperationId = OperationId::new(7);
    const FILE_POLICY: FilePolicyRef = FilePolicyRef {
        digest: PolicyDigest::new(8),
    };
    const MANIFEST: SnapshotManifestRef = SnapshotManifestRef {
        digest: ManifestDigest::new(9),
    };

    fn populated_world() -> World {
        let mut world = begin_create(
            &World::default(),
            SANDBOX,
            OWNER,
            WORKSPACE,
            FILE_POLICY,
            NetworkPolicy::DenyAll,
            OPERATION,
        );
        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SANDBOX,
                generation: 1,
                status: VmStatus::Prepared,
            },
        );
        world.snapshots.put(
            SNAPSHOT,
            Snapshot {
                owner: OWNER,
                sandbox: SANDBOX,
                source_generation: 1,
                workspace: WORKSPACE,
                manifest: MANIFEST,
                stage: SnapshotStage::Durable,
                immutable: true,
            },
        );
        world.net_slots.put(
            SLOT,
            NetSlot {
                owner: OWNER,
                sandbox: SANDBOX,
                generation: 1,
                vm: VM,
                policy: NetworkPolicy::DenyAll,
                configured: true,
                active: true,
                reclaimed: false,
            },
        );
        world.applied_file_policies.put(
            SANDBOX,
            AppliedFilePolicy {
                sandbox: SANDBOX,
                generation: 1,
                workspace: WORKSPACE,
                policy: FILE_POLICY,
            },
        );
        world.worker_fences.put(
            SANDBOX,
            WorkerFence {
                epoch: 1,
                terminal: false,
            },
        );
        world.resource_indexes.put(
            SANDBOX,
            ResourceIndex {
                vms: vec![VM],
                slots: vec![SLOT],
                snapshots: vec![SNAPSHOT],
            },
        );
        world.next_vm_id = 11;
        world.next_slot_id = 12;
        world.next_snapshot_id = 13;
        world.next_manifest_digest = 14;
        world
    }

    #[test]
    fn crash_and_recovery_only_toggle_runtime_availability() {
        let world = populated_world();

        let crashed = handle_controller_crash(&world).expect("up controller must crash");
        let mut expected_crashed = world.clone();
        expected_crashed.controller_up = false;
        assert_eq!(crashed.event, RuntimeEvent::ControllerCrash);
        assert_eq!(crashed.next, expected_crashed);
        assert!(world.controller_up);

        let recovered =
            handle_controller_recover(&crashed.next).expect("down controller must recover");
        assert_eq!(recovered.event, RuntimeEvent::ControllerRecover);
        assert_eq!(recovered.next, world);
    }

    #[test]
    fn lifecycle_handlers_enforce_the_exact_runtime_guard() {
        let up = populated_world();
        assert_eq!(
            handle_controller_recover(&up),
            Err(RuntimeReject::ControllerAlreadyUp)
        );

        let mut down = up.clone();
        down.controller_up = false;
        assert_eq!(
            handle_controller_crash(&down),
            Err(RuntimeReject::ControllerAlreadyDown)
        );
    }

    #[test]
    fn runtime_coordinator_selects_the_matching_handler() {
        let world = populated_world();
        let crashed = handle_runtime(&world, RuntimeInput::Crash)
            .expect("coordinator must route a legal crash");
        assert_eq!(crashed.event, RuntimeEvent::ControllerCrash);

        let recovered = handle_runtime(&crashed.next, RuntimeInput::Recover)
            .expect("coordinator must route a legal recovery");
        assert_eq!(recovered.event, RuntimeEvent::ControllerRecover);
        assert_eq!(recovered.next, world);
    }

    #[test]
    fn duplicate_delivery_requires_the_exact_persisted_attempt() {
        let mut world = populated_world();
        world.controller_up = false;
        let persisted = world.effect_keys[0].clone();

        let accepted = handle_duplicate_or_late_delivery(&world, persisted.clone())
            .expect("persisted delivery must be observable even while down");
        assert_eq!(
            accepted.event,
            RuntimeEvent::DuplicateOrLateDelivery(persisted)
        );
        assert_eq!(accepted.next, world);

        let missing = crate::EffectId::initial(Command::CleanupVm {
            sandbox: SANDBOX,
            generation: 99,
            vm: VmId::new(99),
        });
        assert_eq!(
            handle_duplicate_or_late_delivery(&world, missing),
            Err(RuntimeReject::EffectNotFound)
        );
    }

    #[test]
    fn view_is_the_exact_observable_projection() {
        let world = populated_world();
        assert_eq!(
            view(&world, SANDBOX),
            Some(SandboxView {
                phase: Phase::Starting,
                terminal: false,
                generation: 1,
            })
        );
        assert_eq!(view(&world, SandboxId::new(404)), None);
    }

    #[test]
    fn monitor_accepts_the_model_normal_lifecycle_example() {
        let observations = [
            Some(SandboxView {
                phase: Phase::Starting,
                terminal: false,
                generation: 1,
            }),
            Some(SandboxView {
                phase: Phase::Running,
                terminal: false,
                generation: 1,
            }),
            Some(SandboxView {
                phase: Phase::Paused,
                terminal: false,
                generation: 1,
            }),
            Some(SandboxView {
                phase: Phase::Starting,
                terminal: false,
                generation: 2,
            }),
            Some(SandboxView {
                phase: Phase::Running,
                terminal: false,
                generation: 2,
            }),
        ];
        assert!(monitor(&observations));
    }

    #[test]
    fn monitor_rejects_running_after_terminal() {
        let observations = [
            Some(SandboxView {
                phase: Phase::Paused,
                terminal: false,
                generation: 1,
            }),
            Some(SandboxView {
                phase: Phase::Revoking,
                terminal: true,
                generation: 1,
            }),
            Some(SandboxView {
                phase: Phase::Running,
                terminal: false,
                generation: 1,
            }),
        ];
        assert!(!monitor(&observations));
    }

    #[test]
    fn monitor_skips_missing_observations_without_clearing_the_latch() {
        let terminal = SandboxView {
            phase: Phase::Revoking,
            terminal: true,
            generation: 1,
        };
        let destroyed = SandboxView {
            phase: Phase::Destroyed,
            terminal: true,
            generation: 1,
        };
        assert!(monitor(&[None, Some(terminal), None, Some(destroyed)]));
        assert!(!monitor(&[
            Some(terminal),
            None,
            Some(SandboxView {
                phase: Phase::Paused,
                terminal: false,
                generation: 1,
            }),
        ]));
    }

    #[test]
    fn monitor_from_matches_the_model_latch_boundary() {
        let terminal_running = SandboxView {
            phase: Phase::Running,
            terminal: true,
            generation: 1,
        };

        // `monitorFrom false` records the first terminal observation; the
        // consistency check starts with the next observable entry.
        assert!(monitor_from(false, &[Some(terminal_running)]));
        assert!(!monitor_from(true, &[Some(terminal_running)]));
        assert!(monitor_from(true, &[None]));
        assert!(monitor_from(false, &[]));
    }
}
