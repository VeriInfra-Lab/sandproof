//! Already-Free history remains distinct from permission to release anew.
use super::*;
use crate::maintenance::slots::{HostBinding, SlotInput};

fn unloaded() -> (World, HostBinding) {
    let world = unload::drain(&unload::commit_planned());
    let target = unload::target(&world);
    let world = step(
        &world,
        ControllerInput::Publication(PublicationInput::Paused(SandboxId(10), target)),
    );
    let binding = world
        .maintenance_state()
        .unwrap()
        .host_slot(HostSlotId(0))
        .unwrap()
        .binding;
    (cleanup(&world, binding), binding)
}
fn accept_kill(world: &World) -> World {
    handle_controller(
        world,
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SandboxId(10),
            operation: OperationId(3),
        })),
    )
    .unwrap()
    .next
}

#[test]
fn released_unload_roundtrips_through_new_terminal_fence_uncertainty_without_erasing_material() {
    let (world, binding) = unloaded();
    let world = release(&world, 0, binding);
    let snapshots = world.resource_index(binding.sandbox).snapshots.clone();
    assert!(!snapshots.is_empty());
    let world = accept_kill(&world);
    roundtrip(&world);
    assert!(crate::maintenance::released_unload_termination_history(
        &world, binding
    ));
    let fence = effect(&world, binding.sandbox, |c| {
        matches!(c, Command::InstallFence { terminal: true, .. })
    });
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(fence.clone())),
    );
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(fence.clone())),
    );
    assert!(crate::maintenance::released_unload_termination_history(
        &world, binding
    ));
    assert_eq!(
        world.effect_status(&fence),
        Some(EffectStatus::OutcomeUnknown)
    );
    let world = step(&world, ControllerInput::Worker(fence.clone()));
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Receipt(
            fence,
            EffectOutcome::Succeeded(ResultDigest(1)),
        )),
    );
    let world = step(
        &world,
        ControllerInput::Publication(PublicationInput::Destroyed(binding.sandbox)),
    );
    assert_eq!(world.resource_index(binding.sandbox).snapshots, snapshots);
    assert_eq!(
        world
            .maintenance_state()
            .unwrap()
            .host_slot(HostSlotId(0))
            .unwrap()
            .phase,
        HostSlotPhase::Free
    );
}

#[test]
fn released_unload_does_not_accept_wrong_generation_vm_network_owner_or_old_uncertainty() {
    let (world, binding) = unloaded();
    let world = release(&world, 0, binding);
    let (world, other) = reserve(&world, SandboxId(11));
    let world = running(&bind(&world, 1, other), other);
    let world = accept_kill(&world);
    for wrong in [
        HostBinding {
            generation: 2,
            ..binding
        },
        HostBinding {
            vm: other.vm,
            ..binding
        },
        HostBinding {
            network: other.network,
            ..binding
        },
        HostBinding {
            owner: TenantId(99),
            ..binding
        },
    ] {
        assert!(!crate::maintenance::released_unload_termination_history(
            &world, wrong
        ));
        let mut image = capture(&world, LIMITS).unwrap();
        image.0.maintenance.as_mut().unwrap().slots[0].1.binding = wrong;
        assert!(restore(image, LIMITS).is_err());
    }
    let mut old_unknown = world.clone();
    let seal = effect(&world, binding.sandbox, |c| {
        matches!(c, Command::SealSnapshot { .. })
    });
    old_unknown.effects.put(seal, EffectStatus::OutcomeUnknown);
    assert!(!crate::maintenance::released_unload_termination_history(
        &old_unknown,
        binding
    ));
    assert!(capture(&old_unknown, LIMITS).is_err());
}

#[test]
fn terminal_history_cannot_release_occupied_or_validate_releasing_slot() {
    let (world, binding) = unloaded();
    assert_eq!(
        world
            .maintenance_state()
            .unwrap()
            .host_slot(HostSlotId(0))
            .unwrap()
            .phase,
        HostSlotPhase::Occupied
    );
    let killed = accept_kill(&world);
    roundtrip(&killed);
    for input in [
        SlotInput::BeginRelease {
            position: HostSlotId(0),
            binding,
        },
        SlotInput::ConfirmRelease {
            position: HostSlotId(0),
            binding,
        },
    ] {
        assert!(
            handle_controller(
                &killed,
                ControllerInput::Maintenance(MaintenanceInput::Slot(input))
            )
            .is_err()
        );
    }
    // A corrupt image cannot relabel already Free as Releasing. Production
    // admission separately refuses Kill while a real release is in flight.
    let free = accept_kill(&release(&world, 0, binding));
    let mut image = capture(&free, LIMITS).unwrap();
    image.0.maintenance.as_mut().unwrap().slots[0].1.phase = HostSlotPhase::Releasing;
    assert_eq!(
        restore(image, LIMITS),
        Err(CheckpointReject::InvalidMaintenance)
    );
}

#[test]
fn released_execution_closure_roundtrips_without_manufacturing_fence_success() {
    use crate::maintenance::released_execution::{ReleasedExecution, discharged};
    let (world, binding) = unloaded();
    let world = accept_kill(&release(&world, 0, binding));
    let fence = effect(&world, binding.sandbox, |c| {
        matches!(c, Command::InstallFence { terminal: true, .. })
    });
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(fence.clone())),
    );
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(fence.clone())),
    );
    let basis = ReleasedExecution {
        key: OperationKey {
            sandbox: binding.sandbox,
            operation: OperationId(3),
        },
        epoch: world.sandbox_record(binding.sandbox).unwrap().epoch,
        position: HostSlotId(0),
        expected_previous: None,
        binding,
        release_input_sequence: 1,
    };
    let world = step(
        &world,
        ControllerInput::Maintenance(MaintenanceInput::CloseReleasedExecution(basis)),
    );
    assert!(discharged(&world, &fence));
    assert_eq!(
        world.effect_status(&fence),
        Some(EffectStatus::OutcomeUnknown)
    );
    assert!(!effect::effect_succeeded_check(&world, fence.command()));
    assert!(effect::handle_effect_dispatch(&world, fence.clone()).is_err());
    assert!(worker::handle_worker(&world, &fence).is_err());
    roundtrip(&world);
    let destroyed = step(
        &world,
        ControllerInput::Publication(PublicationInput::Destroyed(binding.sandbox)),
    );
    assert!(
        destroyed
            .worker_fences
            .get(&binding.sandbox)
            .is_some_and(|f| f.epoch < basis.epoch && !f.terminal)
    );
    roundtrip(&destroyed);
    let crashed = step(&destroyed, ControllerInput::Runtime(RuntimeInput::Crash));
    roundtrip(&crashed);
}

#[test]
fn released_execution_closure_cannot_discharge_an_old_unknown_or_a_new_generation() {
    use crate::maintenance::released_execution::{ReleasedExecution, valid_basis};
    let (world, binding) = unloaded();
    let world = release(&world, 0, binding);
    let killed = accept_kill(&world);
    let basis = ReleasedExecution {
        key: OperationKey {
            sandbox: binding.sandbox,
            operation: OperationId(3),
        },
        epoch: killed.sandbox_record(binding.sandbox).unwrap().epoch,
        position: HostSlotId(0),
        expected_previous: None,
        binding,
        release_input_sequence: 1,
    };
    assert!(valid_basis(&killed, basis));
    let seal = effect(&killed, binding.sandbox, |c| {
        matches!(c, Command::SealSnapshot { .. })
    });
    let mut unknown = killed.clone();
    unknown.effects.put(seal, EffectStatus::OutcomeUnknown);
    assert!(!valid_basis(&unknown, basis));
    let resumed = step(
        &world,
        ControllerInput::Request(Request::Resume(ResumeRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: binding.sandbox,
            operation: OperationId(3),
        })),
    );
    let new_kill = step(
        &resumed,
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: binding.sandbox,
            operation: OperationId(4),
        })),
    );
    let wrong = ReleasedExecution {
        key: OperationKey {
            sandbox: binding.sandbox,
            operation: OperationId(4),
        },
        epoch: new_kill.sandbox_record(binding.sandbox).unwrap().epoch,
        ..basis
    };
    assert!(!valid_basis(&new_kill, wrong));
    assert!(
        handle_controller(
            &new_kill,
            ControllerInput::Maintenance(MaintenanceInput::CloseReleasedExecution(wrong))
        )
        .is_err()
    );
}
