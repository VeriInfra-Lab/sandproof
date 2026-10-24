//! Historical validity only: never permission to release an occupied position.
use super::slots::HostBinding;
use crate::{
    Authority, EffectStatus, Epoch, OperationInput, OperationKey, OperationKind, OperationResult,
    OperationStatus, Phase, RestoreSource, VmStatus, World,
};

/// Recognize an already released Unloaded execution while its next native
/// operation terminates it. Callers must independently possess an existing
/// Free slot or the original successful release witness. This check creates
/// neither evidence of physical release nor permission to release a position.
#[must_use]
pub fn released_unload_termination_history(world: &World, binding: HostBinding) -> bool {
    let Some((epoch, source)) = terminal_binding(world, binding) else {
        return false;
    };
    let Some(previous) = completed_pause_epoch(world, binding, epoch, source) else {
        return false;
    };
    // New termination uncertainty cannot undo the older completed release.
    !world.effect_ids().iter().any(|id| {
        id.command().sandbox() == binding.sandbox
            && matches!(
                world.effect_status(id),
                Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown)
            )
            && id.command().epoch().is_none_or(|epoch| epoch <= previous)
    })
}

fn terminal_binding(world: &World, binding: HostBinding) -> Option<(Epoch, RestoreSource)> {
    let row = world.sandbox_record(binding.sandbox)?;
    let kill = row.current_operation?;
    let source = row.durable_snapshot?;
    let key = OperationKey {
        sandbox: binding.sandbox,
        operation: kill.id,
    };
    if row.owner != binding.owner
        || row.generation != binding.generation
        || row.phase != Phase::Revoking
        || !row.terminal
        || row.current_vm.is_some()
        || row.net_slot.is_some()
        || row.desired != Authority::Closed
        || row.effective != Authority::Closed
        || kill.kind != OperationKind::Kill
        || kill.epoch != row.epoch
        || !matches!(
            kill.status,
            OperationStatus::Pending | OperationStatus::OutcomeUnknown
        )
        || world.operation_record(key).is_none_or(|ledger| {
            ledger.operation != kill
                || ledger.input
                    != (OperationInput::Kill {
                        generation: binding.generation,
                    })
                || ledger.result.is_some()
        })
        || world.operation_plan(key).is_none_or(|plan| {
            plan.key != key
                || !plan.sealed
                || plan.epoch != row.epoch
                || plan.generation != binding.generation
                || !plan.stop_vms.contains(&binding.vm)
                || !plan.close_slots.contains(&binding.network)
        })
        || world.vm_record(binding.vm).is_none_or(|vm| {
            vm.owner != binding.owner
                || vm.sandbox != binding.sandbox
                || vm.generation != binding.generation
                || vm.status != VmStatus::Reclaimed
        })
        || world.slot_record(binding.network).is_none_or(|slot| {
            slot.owner != binding.owner
                || slot.sandbox != binding.sandbox
                || slot.generation != binding.generation
                || slot.vm != binding.vm
                || slot.active
                || !slot.reclaimed
        })
    {
        return None;
    }
    Some((row.epoch, source))
}

fn completed_pause_epoch(
    world: &World,
    binding: HostBinding,
    epoch: Epoch,
    source: RestoreSource,
) -> Option<Epoch> {
    let (prior, ledger) = world.operation_records().iter().find(|(prior, ledger)| {
        prior.sandbox == binding.sandbox && ledger.operation.epoch.checked_add(1) == Some(epoch)
    })?;
    if ledger.operation.kind != OperationKind::Pause
        || ledger.operation.status != OperationStatus::Succeeded
        || ledger.result != Some(OperationResult::Paused(source))
        || ledger.input
            != (OperationInput::Pause {
                generation: binding.generation,
            })
        || world.operation_plan(*prior).is_none_or(|plan| {
            plan.generation != binding.generation
                || plan.vm != Some(binding.vm)
                || plan.target_snapshot != Some(source)
                || !plan.close_slots.contains(&binding.network)
                || plan.capture.is_none_or(|capture| {
                    capture.commit() != crate::capture::CaptureCommit::Registered
                        || !capture.has_unload_source()
                        || capture.eligibility() != crate::capture::RestoreEligibility::Revoked
                })
        })
    {
        return None;
    }
    Some(ledger.operation.epoch)
}
