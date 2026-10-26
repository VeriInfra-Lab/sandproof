//! Terminal closure for a completed Unload whose physical position was already
//! released. The journal supplies the ORIGINAL ConfirmRelease witness under its
//! writer transaction; a presently Free position is never such a witness.
use super::{MaintenanceReject, slots::{HostBinding, HostSlotId}};
use crate::{Authority, Command, EffectId, EffectOrigin, EffectStatus, Epoch,
    LogicalEffectId, LogicalEffectResult, OperationInput, OperationKey, OperationKind,
    OperationResult, OperationStatus, Phase, SandboxId, VmId, VmStatus, World};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ReleasedExecution {
    pub key: OperationKey,
    pub epoch: Epoch,
    pub position: HostSlotId,
    pub expected_previous: Option<VmId>,
    pub binding: HostBinding,
    pub release_input_sequence: u64,
}

impl ReleasedExecution {
    #[must_use]
    pub fn command(self) -> Command {
        Command::InstallFence { sandbox: self.key.sandbox, epoch: self.epoch, terminal: true }
    }
}

// Structural validation is shared before publication and during recovery. It
// does not authenticate a release: the same journal transaction and its saved
// HostReleaseWitness must additionally bind every field and input_sequence.
#[must_use]
pub fn valid_basis(world: &World, basis: ReleasedExecution) -> bool {
    let Some(row) = world.sandbox_record(basis.key.sandbox) else { return false };
    let Some(operation) = row.current_operation else { return false };
    let Some(ledger) = world.operation_record(basis.key) else { return false };
    let Some(plan) = world.operation_plan(basis.key) else { return false };
    let Some(source) = row.durable_snapshot else { return false };
    let Some(state) = world.maintenance_state() else { return false };
    let before = row.phase == Phase::Revoking
        && matches!(operation.status, OperationStatus::Pending | OperationStatus::OutcomeUnknown)
        && ledger.result.is_none();
    let after = row.phase == Phase::Destroyed && operation.status == OperationStatus::Succeeded
        && ledger.result == Some(OperationResult::Destroyed);
    if !(before || after) || !row.terminal
        || row.owner != basis.binding.owner || row.generation != basis.binding.generation
        || basis.binding.sandbox != basis.key.sandbox || row.epoch != basis.epoch
        || row.current_vm.is_some() || row.net_slot.is_some()
        || row.desired != Authority::Closed || row.effective != Authority::Closed
        || operation.id != basis.key.operation || operation.kind != OperationKind::Kill
        || operation.epoch != basis.epoch || ledger.operation != operation
        || ledger.sandbox != basis.key.sandbox
        || ledger.input != (OperationInput::Kill { generation: basis.binding.generation })
        || plan.key != basis.key || plan.epoch != basis.epoch || !plan.sealed
        || plan.generation != basis.binding.generation || plan.capture.is_some()
        || plan.vm.is_some() || plan.slot.is_some() || plan.restore_from.is_some()
        || plan.target_snapshot.is_some() || basis.release_input_sequence == 0
        || basis.position.0 >= state.max_host_slots
        || basis.expected_previous.is_some_and(|old| old >= basis.binding.vm)
        || !plan.stop_vms.contains(&basis.binding.vm)
        || !plan.close_slots.contains(&basis.binding.network)
    { return false; }
    if world.vm_record(basis.binding.vm).is_none_or(|vm| vm.owner != row.owner
        || vm.sandbox != basis.key.sandbox || vm.generation != row.generation
        || vm.status != VmStatus::Reclaimed)
        || world.slot_record(basis.binding.network).is_none_or(|slot|
            slot.owner != row.owner || slot.sandbox != basis.key.sandbox
            || slot.generation != row.generation || slot.vm != basis.binding.vm
            || !slot.reclaimed || slot.active)
        || super::unload_history::completed_pause_epoch(world, basis.binding, basis.epoch, source).is_none()
    { return false; }
    let resources = world.resource_index(basis.key.sandbox);
    if plan.stop_vms != resources.vms || plan.close_slots != resources.slots
        || resources.vms.iter().any(|id| world.vm_record(*id).is_none_or(|v| v.status != VmStatus::Reclaimed))
        || resources.slots.iter().any(|id| world.slot_record(*id).is_none_or(|s| !s.reclaimed || s.active))
        || !crate::publication::indexed_stops_succeeded(world, basis.key.sandbox)
    { return false; }
    // Only this exact metadata-only terminal fence may be unresolved. No
    // physical action from an older or newer execution is discharged here.
    let command = basis.command();
    world.effect_ids().iter().all(|id| {
        if id.command().sandbox() != basis.key.sandbox { return true; }
        let Some(logical) = world.logical_effect(&id.logical) else { return false };
        if *id.command() == command {
            logical.origin == EffectOrigin::Operation(basis.key)
                && logical.current_attempt == id.attempt
                && matches!(world.effect_status(id), Some(EffectStatus::Committed
                    | EffectStatus::Dispatched | EffectStatus::OutcomeUnknown))
                && (logical.result.is_none()
                    || logical.result == Some(LogicalEffectResult::ReleasedExecution(basis)))
        } else {
            !matches!(world.effect_status(id), None | Some(EffectStatus::Committed
                | EffectStatus::Dispatched | EffectStatus::OutcomeUnknown))
                && logical.result.is_some()
        }
    })
}

#[must_use]
pub fn discharged(world: &World, id: &EffectId) -> bool {
    let Some(logical) = world.logical_effect(&id.logical) else { return false };
    let Some(LogicalEffectResult::ReleasedExecution(basis)) = logical.result else { return false };
    logical.current_attempt == id.attempt && *id.command() == basis.command()
        && valid_basis(world, basis)
}

#[must_use]
pub fn terminal_closed(world: &World, sandbox: SandboxId) -> bool {
    let Some(row) = world.sandbox_record(sandbox) else { return false };
    let logical = LogicalEffectId { command: Command::InstallFence { sandbox, epoch: row.epoch, terminal: true } };
    let Some(effect) = world.logical_effect(&logical) else { return false };
    discharged(world, &EffectId { logical, attempt: effect.current_attempt })
}

pub(super) fn close(world: &World, basis: ReleasedExecution) -> Result<World, MaintenanceReject> {
    if !world.controller_up { return Err(MaintenanceReject::ControllerDown); }
    if !valid_basis(world, basis) { return Err(MaintenanceReject::ResourcesNotReclaimed); }
    let logical_id = LogicalEffectId { command: basis.command() };
    let mut logical = world.logical_effect(&logical_id).ok_or(MaintenanceReject::ResourceMismatch)?;
    if logical.result == Some(LogicalEffectResult::ReleasedExecution(basis)) { return Ok(world.clone()); }
    if world.sandbox_record(basis.key.sandbox).is_none_or(|r| r.phase != Phase::Revoking) {
        return Err(MaintenanceReject::ResourceMismatch);
    }
    logical.result = Some(LogicalEffectResult::ReleasedExecution(basis));
    let mut next = world.clone();
    next.logical_effects.put(logical_id, logical);
    Ok(next)
}
