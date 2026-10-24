use super::{
    Authority, EffectStatus, HostBinding, HostSlotPhase, OperationInput, OperationStatus, Phase,
    Reject, VmStatus, World, binding_current, binding_history, ledger_shape, record,
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
    retained(world, state)?;
    for (position, slot) in state.slots.entries() {
        if position.0 >= state.max_host_slots || slot.binding.generation == 0 {
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
                    free_binding(world, slot.binding)?;
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
    let rows = state
        .retained
        .entries()
        .iter()
        .filter(|(key, _)| key.sandbox == sandbox)
        .collect::<Vec<_>>();
    let epochs = rows
        .iter()
        .map(|(_, row)| row.ledger.operation.epoch)
        .collect::<Vec<_>>();
    super::unique(&epochs)?;
    let latest = epochs
        .iter()
        .copied()
        .max()
        .ok_or(Reject::InvalidMaintenance)?;
    if u64::try_from(rows.len()).map_err(|_| Reject::InvalidMaintenance)? != latest {
        return Err(Reject::InvalidMaintenance);
    }
    let mut creates = 0;
    let mut kills = 0;
    for (_, retained) in &rows {
        let ledger = &retained.ledger;
        let epoch = ledger.operation.epoch;
        let generation = 1 + u64::try_from(
            rows.iter()
                .filter(|(_, r)| {
                    r.ledger.operation.kind == OperationKind::Resume
                        && r.ledger.operation.epoch <= epoch
                })
                .count(),
        )
        .map_err(|_| Reject::InvalidMaintenance)?;
        match ledger.operation.kind {
            OperationKind::Create => {
                creates += 1;
                if epoch != 1 {
                    return Err(Reject::InvalidMaintenance);
                }
            }
            OperationKind::Kill => kills += 1,
            OperationKind::Pause | OperationKind::Resume => {}
        }
        match ledger.input {
            OperationInput::Pause { generation: found }
            | OperationInput::Kill { generation: found }
                if found != generation =>
            {
                return Err(Reject::InvalidMaintenance);
            }
            _ => {}
        }
        if let Some(crate::OperationResult::Running(found)) = ledger.result
            && found != generation
        {
            return Err(Reject::InvalidMaintenance);
        }
        if epoch > 1 {
            let prior = &rows
                .iter()
                .find(|(_, r)| r.ledger.operation.epoch.checked_add(1) == Some(epoch))
                .ok_or(Reject::InvalidMaintenance)?
                .1
                .ledger;
            retained_successor(prior, ledger)?;
        }
    }
    let last = &rows
        .iter()
        .find(|(_, r)| r.ledger.operation.epoch == latest)
        .ok_or(Reject::InvalidMaintenance)?
        .1
        .ledger;
    if creates != 1
        || kills > 1
        || !matches!(
            (last.operation.kind, last.operation.status),
            (OperationKind::Kill, OperationStatus::Succeeded)
                | (
                    OperationKind::Create | OperationKind::Resume,
                    OperationStatus::Failed(_)
                )
        )
    {
        return Err(Reject::InvalidMaintenance);
    }
    Ok(())
}

fn released_history(world: &World, binding: HostBinding) -> Result<(), Reject> {
    let epoch = world
        .operation_plans
        .entries()
        .iter()
        .filter(|(key, p)| key.sandbox == binding.sandbox && p.generation == binding.generation)
        .map(|(_, p)| p.epoch)
        .max()
        .ok_or(Reject::InvalidMaintenance)?;
    if world
        .vm_record(binding.vm)
        .is_none_or(|vm| vm.status != VmStatus::Reclaimed)
        || world
            .slot_record(binding.network)
            .is_none_or(|slot| slot.active || !slot.reclaimed)
        || world.effects.entries().iter().any(|(id, status)| {
            id.command().sandbox() == binding.sandbox
                && matches!(
                    status,
                    EffectStatus::Dispatched | EffectStatus::OutcomeUnknown
                )
                && id.command().epoch().is_none_or(|found| found <= epoch)
        })
    {
        return Err(Reject::InvalidMaintenance);
    }
    Ok(())
}

fn free_binding(world: &World, binding: HostBinding) -> Result<(), Reject> {
    binding_history(world, binding)?;
    if record(world, binding.sandbox)?.generation > binding.generation {
        released_history(world, binding)
    } else if crate::maintenance::released_unload_termination_history(world, binding) {
        // Only a slot already Free reaches this branch. The original release
        // permit and Releasing validation remain unchanged.
        Ok(())
    } else {
        released(world, binding)
    }
}

fn retained(world: &World, state: &MaintenanceState) -> Result<(), Reject> {
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
        let source = match retained.ledger.input {
            OperationInput::Resume { source } => Some(source),
            _ => match retained.ledger.result {
                Some(crate::OperationResult::Paused(source)) => Some(source),
                _ => None,
            },
        };
        if source.is_some_and(|source| {
            source.snapshot.0 >= world.next_snapshot_id
                || source.manifest.digest.0 >= world.next_manifest_digest
        }) {
            return Err(Reject::InvalidFrontier);
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
    Ok(())
}

fn retained_successor(
    prior: &crate::OperationLedgerRecord,
    ledger: &crate::OperationLedgerRecord,
) -> Result<(), Reject> {
    match ledger.operation.kind {
        OperationKind::Resume => {
            let OperationInput::Resume { source } = ledger.input else {
                return Err(Reject::InvalidMaintenance);
            };
            if prior.operation.kind != OperationKind::Pause
                || prior.operation.status != OperationStatus::Succeeded
                || prior.result != Some(crate::OperationResult::Paused(source))
            {
                return Err(Reject::InvalidMaintenance);
            }
        }
        OperationKind::Pause => {
            if !matches!(
                prior.operation.kind,
                OperationKind::Create | OperationKind::Resume
            ) || prior.operation.status != OperationStatus::Succeeded
            {
                return Err(Reject::InvalidMaintenance);
            }
        }
        OperationKind::Kill => {}
        OperationKind::Create => return Err(Reject::InvalidMaintenance),
    }
    Ok(())
}
