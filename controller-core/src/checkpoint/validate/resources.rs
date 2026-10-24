use super::{Reject, history, record, reserved_slot, reserved_vm, startup_for_vm, unique};
use crate::{Command, EffectOutcome, EffectStatus, Phase, SandboxId, Vm, VmId, VmStatus, World};

pub(super) fn validate(world: &World) -> Result<(), Reject> {
    for (workspace, fs) in world.filesystems.entries() {
        let row = record(world, fs.sandbox)?;
        if row.workspace != *workspace || row.owner != fs.owner || fs.created_generation != 1 {
            return Err(Reject::InvalidResource);
        }
    }
    for (vm, machine) in world.vms.entries() {
        machine_record(world, *vm, machine)?;
    }
    networks(world)?;
    indexes(world)?;
    policies_and_fences(world)
}

// Worker application precedes acknowledgement, so a persisted resource can
// legitimately have a Dispatched or OutcomeUnknown effect. An unexecuted
// Committed command or a failure receipt alone is not a resource witness.
pub(super) fn observed(world: &World, predicate: impl Fn(&Command) -> bool) -> bool {
    world.effects.entries().iter().any(|(id, status)| {
        predicate(id.command())
            && matches!(
                status,
                EffectStatus::Dispatched
                    | EffectStatus::OutcomeUnknown
                    | EffectStatus::Acknowledged(EffectOutcome::Succeeded(_))
            )
    })
}

fn prepared_or_aborted(world: &World, sandbox: SandboxId, vm: VmId) -> bool {
    observed(world, |command| {
        matches!(command,
        Command::PrepareVm { sandbox: owner, vm: found, .. } | Command::AbortPrepare { sandbox: owner, vm: found, .. }
        if *owner == sandbox && *found == vm)
    })
}

fn machine_record(world: &World, vm: VmId, machine: &Vm) -> Result<(), Reject> {
    let row = record(world, machine.sandbox)?;
    reserved_vm(world, machine.sandbox, vm)?;
    let startup = startup_for_vm(world, machine.sandbox, vm).ok_or(Reject::InvalidReference)?;
    let slot_id = startup.slot.ok_or(Reject::InvalidReference)?;
    let slot = world.slot_record(slot_id).ok_or(Reject::InvalidReference)?;
    if machine.owner != row.owner
        || machine.generation != startup.generation
        || slot.vm != vm
        || !world.resource_index(machine.sandbox).vms.contains(&vm)
        || !prepared_or_aborted(world, machine.sandbox, vm)
        || (row.phase == Phase::Destroyed
            && !matches!(machine.status, VmStatus::Stopped | VmStatus::Reclaimed))
    {
        return Err(Reject::InvalidResource);
    }
    let witnessed = match machine.status {
        VmStatus::Prepared => true,
        VmStatus::Executing => {
            slot.configured
                && !slot.reclaimed
                && observed(
                    world,
                    |command| matches!(command, Command::ActivateVm { vm: found, .. } if *found == vm),
                )
        }
        VmStatus::Stopped => observed(
            world,
            |command| matches!(command, Command::StopVm { vm: found, .. } if *found == vm),
        ),
        VmStatus::Reclaimed => observed(
            world,
            |command| matches!(command, Command::CleanupVm { vm: found, .. } | Command::AbortPrepare { vm: found, .. } if *found == vm),
        ),
        VmStatus::Frozen => observed(world, |command| {
            matches!(command, Command::FreezeVm { sandbox, generation, vm: found, .. }
                if *sandbox == machine.sandbox && *generation == machine.generation && *found == vm)
        }),
    };
    if !witnessed {
        return Err(Reject::InvalidResource);
    }
    Ok(())
}

fn networks(world: &World) -> Result<(), Reject> {
    for (slot, network) in world.net_slots.entries() {
        let row = record(world, network.sandbox)?;
        let vm = world
            .vm_record(network.vm)
            .ok_or(Reject::InvalidReference)?;
        reserved_slot(world, network.sandbox, *slot)?;
        if network.owner != row.owner
            || network.generation == 0
            || network.generation > row.generation
            || network.policy != row.policy
            || vm.sandbox != network.sandbox
            || vm.owner != network.owner
            || vm.generation != network.generation
            || !world.resource_index(network.sandbox).slots.contains(slot)
            || (network.active
                && (!network.configured
                    || network.reclaimed
                    || vm.status != VmStatus::Executing
                    || row.net_slot != Some(*slot)))
            || (row.phase == Phase::Destroyed && network.active)
        {
            return Err(Reject::InvalidResource);
        }
        if network.configured
            && !observed(
                world,
                |command| matches!(command, Command::ConfigureNetwork { slot: found, .. } if found == slot),
            )
        {
            return Err(Reject::InvalidResource);
        }
        if network.reclaimed
            && !observed(
                world,
                |command| matches!(command, Command::CleanupSlot { slot: found, .. } | Command::AbortPrepare { slot: found, .. } if found == slot),
            )
        {
            return Err(Reject::InvalidResource);
        }
    }
    Ok(())
}

fn indexes(world: &World) -> Result<(), Reject> {
    for (sandbox, index) in world.resource_indexes.entries() {
        record(world, *sandbox)?;
        unique(&index.vms)?;
        unique(&index.slots)?;
        unique(&index.snapshots)?;
        if index.snapshots.iter().any(|id| {
            world
                .snapshots
                .get(id)
                .is_none_or(|row| row.sandbox != *sandbox)
        }) {
            return Err(Reject::InvalidReference);
        }
        if index.vms.iter().any(|id| {
            world
                .vm_record(*id)
                .is_none_or(|row| row.sandbox != *sandbox)
        }) || index.slots.iter().any(|id| {
            world
                .slot_record(*id)
                .is_none_or(|row| row.sandbox != *sandbox)
        }) {
            return Err(Reject::InvalidReference);
        }
    }
    Ok(())
}

fn policies_and_fences(world: &World) -> Result<(), Reject> {
    for (sandbox, applied) in world.applied_file_policies.entries() {
        let row = record(world, *sandbox)?;
        let startup = history::startup(world, *sandbox, applied.generation)
            .ok_or(Reject::InvalidReference)?;
        let command = Command::ApplyFilePolicy {
            sandbox: *sandbox,
            epoch: startup.epoch,
            generation: applied.generation,
            workspace: applied.workspace,
            policy: applied.policy,
        };
        if applied.sandbox != *sandbox
            || applied.generation == 0
            || applied.generation > row.generation
            || applied.workspace != row.workspace
            || applied.policy != row.file_policy
            || !observed(world, |found| *found == command)
        {
            return Err(Reject::InvalidResource);
        }
    }
    for (sandbox, fence) in world.worker_fences.entries() {
        let row = record(world, *sandbox)?;
        let command = Command::InstallFence {
            sandbox: *sandbox,
            epoch: fence.epoch,
            terminal: fence.terminal,
        };
        if fence.epoch == 0
            || fence.epoch > row.epoch
            || (fence.terminal && (!row.terminal || fence.epoch != row.epoch))
            || !observed(world, |found| *found == command)
        {
            return Err(Reject::InvalidAuthority);
        }
    }
    Ok(())
}
