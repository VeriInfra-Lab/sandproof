use super::{
    Authority, EffectStatus, HostBinding, HostSlotPhase, OperationInput, OperationStatus, Phase,
    Reject, VmStatus, World, binding_current, ledger_shape, record,
};
use crate::maintenance::MaintenanceState;
use crate::{OperationKind, SandboxId};

pub(super) fn validate(world: &World) -> Result<(), Reject> {
    let Some(state) = &world.maintenance else {
        return Ok(());
    };
    limits_and_clock(state)?;
    if world.sandbox_records().iter().any(|(id, _)| {
        state
            .sandbox_frontier
            .is_none_or(|frontier| id.0 > frontier)
    }) || world.filesystems.entries().iter().any(|(id, _)| {
        state
            .workspace_frontier
            .is_none_or(|frontier| id.0 > frontier)
    }) || state.sandbox_frontier.is_some() != state.workspace_frontier.is_some()
    {
        return Err(Reject::InvalidFrontier);
    }
    for (key, retained) in state.retained.entries() {
        retained_group(state, key.sandbox)?;
        ledger_shape(*key, &retained.ledger)?;
        if world.sandbox_record(key.sandbox).is_some()
            || !matches!(
                retained.ledger.operation.status,
                OperationStatus::Succeeded | OperationStatus::Failed(_)
            )
            // Advancing window time can make a retained result inaccessible
            // before the separate result-pruning transition removes its row.
            // Such over-retention is bounded and must remain recoverable.
            || retained.expires_at == 0
            || state.retained.entries().iter().any(|(other, row)| {
                other.sandbox == key.sandbox
                    && (row.owner != retained.owner || row.expires_at != retained.expires_at)
            })
        {
            return Err(Reject::InvalidMaintenance);
        }
        if let OperationInput::Create { workspace, .. } = retained.ledger.input
            && (world.filesystems.contains_key(&workspace)
                || state.retained.entries().iter().any(|(other, row)| other.sandbox != key.sandbox
                    && matches!(row.ledger.input, OperationInput::Create { workspace: found, .. } if found == workspace))) {
            return Err(Reject::InvalidReference);
        }
        if state
            .sandbox_frontier
            .is_none_or(|frontier| key.sandbox.0 > frontier)
            || matches!(retained.ledger.input, OperationInput::Create { workspace, .. }
                if state.workspace_frontier.is_none_or(|frontier| workspace.0 > frontier))
        {
            return Err(Reject::InvalidFrontier);
        }
    }
    for (position, slot) in state.slots.entries() {
        if position.0 >= state.max_host_slots || slot.binding.generation != 1 {
            return Err(Reject::InvalidMaintenance);
        }
        if slot.binding.vm.0 >= world.next_vm_id
            || slot.binding.network.0 >= world.next_slot_id
            || state
                .sandbox_frontier
                .is_none_or(|frontier| slot.binding.sandbox.0 > frontier)
        {
            return Err(Reject::InvalidFrontier);
        }
        if state.slots.entries().iter().any(|(other, row)| {
            other != position
                && (row.binding.vm == slot.binding.vm
                    || row.binding.network == slot.binding.network)
        }) {
            return Err(Reject::DuplicateKey);
        }
        if world.operation_plans.entries().iter().any(|(key, plan)| {
            key.sandbox != slot.binding.sandbox
                && (plan.vm == Some(slot.binding.vm) || plan.slot == Some(slot.binding.network))
        }) {
            return Err(Reject::InvalidReference);
        }
        match slot.phase {
            HostSlotPhase::Occupied => binding_current(world, slot.binding)?,
            HostSlotPhase::Releasing => {
                binding_current(world, slot.binding)?;
                released(world, slot.binding)?;
            }
            HostSlotPhase::Free => {
                if world.sandbox_record(slot.binding.sandbox).is_some() {
                    binding_current(world, slot.binding)?;
                    released(world, slot.binding)?;
                } else if world.vm_record(slot.binding.vm).is_some()
                    || world.slot_record(slot.binding.network).is_some()
                    || state.retained.entries().iter().any(|(key, row)| {
                        key.sandbox == slot.binding.sandbox && row.owner != slot.binding.owner
                    })
                {
                    return Err(Reject::InvalidMaintenance);
                }
            }
        }
    }
    Ok(())
}

fn limits_and_clock(state: &MaintenanceState) -> Result<(), Reject> {
    if state.max_retained_results == 0
        || state.max_host_slots == 0
        || usize::try_from(state.max_retained_results).is_err()
        || usize::try_from(state.max_host_slots).is_err()
        || u64::try_from(state.retained.entries().len()).map_err(|_| Reject::InvalidMaintenance)?
            > state.max_retained_results
        || state.time_floor != state.windows.time_floor()
        || state.windows.validate().is_err()
    {
        return Err(Reject::InvalidMaintenance);
    }
    Ok(())
}

fn released(world: &World, binding: HostBinding) -> Result<(), Reject> {
    let row = record(world, binding.sandbox)?;
    // Reuse the same exact released-runtime state accepted by the reducer.
    // Paused alone is insufficient: material custody and absence of current
    // runtime bindings must also hold, with every physical/unknown check below.
    let terminal = row.terminal
        && row.phase == Phase::Destroyed
        && row.desired == Authority::Closed
        && row.effective == Authority::Closed;
    if !(terminal || crate::maintenance::paused_release_state(world, binding.sandbox, row))
        || world
            .vm_record(binding.vm)
            .is_some_and(|vm| vm.status != VmStatus::Reclaimed)
        || world
            .slot_record(binding.network)
            .is_some_and(|slot| slot.active || !slot.reclaimed)
        || world.effects.entries().iter().any(|(id, status)| {
            id.command().sandbox() == binding.sandbox
                && matches!(
                    status,
                    EffectStatus::Dispatched | EffectStatus::OutcomeUnknown
                )
        })
    {
        return Err(Reject::InvalidMaintenance);
    }
    Ok(())
}

fn retained_group(state: &MaintenanceState, sandbox: SandboxId) -> Result<(), Reject> {
    let mut creates = 0_u64;
    let mut kills = 0_u64;
    let mut failed_create = false;
    let mut pauses = 0_u64;
    for (key, retained) in state.retained.entries() {
        if key.sandbox != sandbox {
            continue;
        }
        match retained.ledger.operation.kind {
            OperationKind::Create => {
                creates += 1;
                failed_create =
                    matches!(retained.ledger.operation.status, OperationStatus::Failed(_));
            }
            OperationKind::Kill => kills += 1,
            OperationKind::Pause => pauses += 1,
            OperationKind::Resume => {
                return Err(Reject::UnsupportedSnapshotHistory);
            }
        }
    }
    if creates != 1 || kills > 1 || pauses > 1 || (kills == 0 && !failed_create) {
        return Err(Reject::InvalidMaintenance);
    }
    Ok(())
}
