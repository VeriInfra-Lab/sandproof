use super::CheckpointReject as Reject;
use crate::maintenance::slots::{HostBinding, HostSlotPhase};
use crate::{
    AppliedFilePolicy, Authority, Command, EffectId, EffectOrigin, EffectOutcome, EffectStatus,
    Generation, LogicalEffectId, LogicalEffectResult, NetSlotId, OperationInput, OperationKey,
    OperationKind, OperationLedgerRecord, OperationPlan, OperationResult, OperationStatus, Phase,
    SandboxId, SandboxRecord, StopCoverage, VmId, VmStatus, WorkerFence, World, effect, terminal,
};

mod effects;
mod maintenance;
mod resources;

pub(super) fn world(world: &World) -> Result<(), Reject> {
    if !world.snapshots.entries().is_empty() {
        return Err(Reject::UnsupportedSnapshotHistory);
    }
    for (sandbox, record) in world.sandboxes.entries() {
        sandbox_record(world, *sandbox, record)?;
    }
    operations(world)?;
    resources::validate(world)?;
    effects::validate(world)?;
    maintenance::validate(world)
}

fn unique<T: PartialEq>(values: &[T]) -> Result<(), Reject> {
    for (index, value) in values.iter().enumerate() {
        if values[..index].contains(value) {
            return Err(Reject::DuplicateKey);
        }
    }
    Ok(())
}

fn record(world: &World, sandbox: SandboxId) -> Result<&SandboxRecord, Reject> {
    world
        .sandbox_record(sandbox)
        .ok_or(Reject::InvalidReference)
}

fn no_snapshot_ledger(ledger: &OperationLedgerRecord) -> Result<(), Reject> {
    if matches!(
        ledger.operation.kind,
        OperationKind::Pause | OperationKind::Resume
    ) || matches!(
        ledger.input,
        OperationInput::Pause { .. } | OperationInput::Resume { .. }
    ) || matches!(ledger.result, Some(OperationResult::Paused(_)))
    {
        return Err(Reject::UnsupportedSnapshotHistory);
    }
    Ok(())
}

fn ledger_shape(key: OperationKey, ledger: &OperationLedgerRecord) -> Result<(), Reject> {
    no_snapshot_ledger(ledger)?;
    if ledger.sandbox != key.sandbox || ledger.operation.id != key.operation {
        return Err(Reject::InvalidLedger);
    }
    let valid = match (
        ledger.operation.kind,
        ledger.operation.status,
        ledger.result,
    ) {
        (
            OperationKind::Create | OperationKind::Kill,
            OperationStatus::Pending | OperationStatus::OutcomeUnknown,
            None,
        )
        | (OperationKind::Create, OperationStatus::Compensating(_), None)
        | (OperationKind::Create, OperationStatus::Succeeded, Some(OperationResult::Running(1)))
        | (OperationKind::Kill, OperationStatus::Succeeded, Some(OperationResult::Destroyed)) => {
            true
        }
        (
            OperationKind::Create,
            OperationStatus::Failed(expected),
            Some(OperationResult::Failed(found)),
        ) => expected == found,
        _ => false,
    };
    let input_valid = matches!(
        (&ledger.operation.kind, &ledger.input),
        (OperationKind::Create, OperationInput::Create { .. })
            | (OperationKind::Kill, OperationInput::Kill { generation: 1 })
    );
    if !valid
        || !input_valid
        || ledger.operation.epoch
            != match ledger.operation.kind {
                OperationKind::Create => 1,
                OperationKind::Kill => 2,
                OperationKind::Pause | OperationKind::Resume => {
                    unreachable!("snapshot kinds rejected")
                }
            }
    {
        return Err(Reject::InvalidLedger);
    }
    Ok(())
}

fn sandbox_record(world: &World, sandbox: SandboxId, value: &SandboxRecord) -> Result<(), Reject> {
    if value.durable_snapshot.is_some()
        || value.restore_from.is_some()
        || matches!(value.phase, Phase::Pausing | Phase::Paused)
    {
        return Err(Reject::UnsupportedSnapshotHistory);
    }
    let operation = value.current_operation.ok_or(Reject::InvalidLedger)?;
    let key = OperationKey {
        sandbox,
        operation: operation.id,
    };
    let ledger = world.operation_record(key).ok_or(Reject::InvalidLedger)?;
    ledger_shape(key, ledger)?;
    if value.generation != 1 || value.epoch != operation.epoch || operation != ledger.operation {
        return Err(Reject::InvalidLedger);
    }
    let fs = world
        .filesystems
        .get(&value.workspace)
        .ok_or(Reject::InvalidReference)?;
    if fs.owner != value.owner || fs.sandbox != sandbox || fs.created_generation != 1 {
        return Err(Reject::InvalidResource);
    }
    if value.current_vm.is_some() != value.net_slot.is_some() {
        return Err(Reject::InvalidResource);
    }
    if let (Some(vm), Some(slot)) = (value.current_vm, value.net_slot) {
        let machine = world.vm_record(vm).ok_or(Reject::InvalidReference)?;
        let network = world.slot_record(slot).ok_or(Reject::InvalidReference)?;
        if machine.owner != value.owner
            || machine.sandbox != sandbox
            || machine.generation != 1
            || machine.status == VmStatus::Reclaimed
            || network.vm != vm
            || network.reclaimed
            || network.owner != value.owner
            || network.sandbox != sandbox
            || network.generation != 1
        {
            return Err(Reject::InvalidResource);
        }
    }
    authority(world, sandbox, value, &value.desired)?;
    authority(world, sandbox, value, &value.effective)?;
    sandbox_phase(world, sandbox, value, operation)
}

fn sandbox_phase(
    world: &World,
    sandbox: SandboxId,
    value: &SandboxRecord,
    operation: crate::Operation,
) -> Result<(), Reject> {
    let valid_phase = match value.phase {
        Phase::Starting => {
            !value.terminal
                && operation.kind == OperationKind::Create
                && matches!(
                    operation.status,
                    OperationStatus::Pending | OperationStatus::OutcomeUnknown
                )
                && value.desired == Authority::Closed
                && value.effective == Authority::Closed
        }
        Phase::Running => {
            !value.terminal
                && operation.kind == OperationKind::Create
                && operation.status == OperationStatus::Succeeded
                && value.desired == value.effective
                && matches!(value.effective, Authority::Running { .. })
        }
        Phase::Revoking => {
            value.terminal
                && value.desired == Authority::Closed
                && match operation.kind {
                    OperationKind::Create => {
                        matches!(operation.status, OperationStatus::Compensating(_))
                            && value.effective == Authority::Closed
                    }
                    OperationKind::Kill => matches!(
                        operation.status,
                        OperationStatus::Pending | OperationStatus::OutcomeUnknown
                    ),
                    _ => false,
                }
        }
        Phase::Destroyed => {
            value.terminal
                && value.desired == Authority::Closed
                && value.effective == Authority::Closed
                && value.current_vm.is_none()
                && value.net_slot.is_none()
                && matches!(
                    (operation.kind, operation.status),
                    (OperationKind::Kill, OperationStatus::Succeeded)
                        | (OperationKind::Create, OperationStatus::Failed(_))
                )
        }
        Phase::Pausing | Phase::Paused => false,
    };
    if !valid_phase {
        return Err(Reject::InvalidAuthority);
    }
    if value.phase == Phase::Running {
        running_evidence(world, sandbox, value)?;
    }
    if value.phase == Phase::Destroyed {
        let command = Command::InstallFence {
            sandbox,
            epoch: value.epoch,
            terminal: true,
        };
        if world.worker_fences.get(&sandbox)
            != Some(&WorkerFence {
                epoch: value.epoch,
                terminal: true,
            })
            || !effect::effect_succeeded_check(world, &command)
        {
            return Err(Reject::InvalidAuthority);
        }
    }
    Ok(())
}

fn authority(
    world: &World,
    sandbox: SandboxId,
    row: &SandboxRecord,
    authority: &Authority,
) -> Result<(), Reject> {
    match authority {
        Authority::Closed => Ok(()),
        Authority::Paused { .. } => Err(Reject::UnsupportedSnapshotHistory),
        Authority::Running {
            sandbox: found,
            generation,
            vm,
            workspace,
            access,
            slot,
            policy,
        } => {
            if *found != sandbox
                || *generation != row.generation
                || Some(*vm) != row.current_vm
                || Some(*slot) != row.net_slot
                || *workspace != row.workspace
                || *access != row.workspace_access
                || *policy != row.policy
            {
                return Err(Reject::InvalidAuthority);
            }
            let startup = startup_plan(world, sandbox).ok_or(Reject::InvalidReference)?;
            if world
                .operation_record(startup.key)
                .is_none_or(|ledger| ledger.operation.status != OperationStatus::Succeeded)
            {
                return Err(Reject::InvalidAuthority);
            }
            let machine = world.vm_record(*vm).ok_or(Reject::InvalidReference)?;
            let network = world.slot_record(*slot).ok_or(Reject::InvalidReference)?;
            if machine.status != VmStatus::Executing
                || !network.active
                || !network.configured
                || network.reclaimed
            {
                return Err(Reject::InvalidAuthority);
            }
            Ok(())
        }
    }
}

fn running_evidence(world: &World, sandbox: SandboxId, row: &SandboxRecord) -> Result<(), Reject> {
    let vm = row.current_vm.ok_or(Reject::InvalidResource)?;
    let slot = row.net_slot.ok_or(Reject::InvalidResource)?;
    let commands = [
        Command::InstallFence {
            sandbox,
            epoch: row.epoch,
            terminal: false,
        },
        Command::PrepareVm {
            sandbox,
            epoch: row.epoch,
            generation: row.generation,
            vm,
            slot,
            source: None,
        },
        Command::ConfigureNetwork {
            sandbox,
            epoch: row.epoch,
            generation: row.generation,
            vm,
            slot,
            policy: row.policy.clone(),
        },
        Command::ActivateVm {
            sandbox,
            epoch: row.epoch,
            generation: row.generation,
            vm,
        },
        Command::ApplyFilePolicy {
            sandbox,
            epoch: row.epoch,
            generation: row.generation,
            workspace: row.workspace,
            policy: row.file_policy,
        },
    ];
    if commands
        .iter()
        .any(|command| !effect::effect_succeeded_check(world, command))
        || world.worker_fences.get(&sandbox)
            != Some(&WorkerFence {
                epoch: row.epoch,
                terminal: false,
            })
        || world.applied_file_policies.get(&sandbox)
            != Some(&AppliedFilePolicy {
                sandbox,
                generation: row.generation,
                workspace: row.workspace,
                policy: row.file_policy,
            })
    {
        return Err(Reject::InvalidAuthority);
    }
    Ok(())
}

fn operations(world: &World) -> Result<(), Reject> {
    for (key, ledger) in world.operations.entries() {
        ledger_shape(*key, ledger)?;
        let row = record(world, key.sandbox)?;
        let plan = world.operation_plan(*key).ok_or(Reject::InvalidReference)?;
        if ledger.operation.epoch > row.epoch
            || plan.key != *key
            || plan.epoch != ledger.operation.epoch
        {
            return Err(Reject::InvalidLedger);
        }
        if let OperationInput::Create {
            workspace,
            network,
            file_policy,
        } = &ledger.input
            && (*workspace != row.workspace
                || *network != row.policy
                || *file_policy != row.file_policy)
        {
            return Err(Reject::InvalidLedger);
        }
        if row
            .current_operation
            .is_none_or(|current| current.id != key.operation)
            && !matches!(
                ledger.operation.status,
                OperationStatus::Succeeded | OperationStatus::Failed(_)
            )
        {
            return Err(Reject::InvalidLedger);
        }
    }
    for (sandbox, _) in world.sandboxes.entries() {
        let creates = world
            .operations
            .entries()
            .iter()
            .filter(|(key, row)| {
                key.sandbox == *sandbox && row.operation.kind == OperationKind::Create
            })
            .count();
        let kills = world
            .operations
            .entries()
            .iter()
            .filter(|(key, row)| {
                key.sandbox == *sandbox && row.operation.kind == OperationKind::Kill
            })
            .count();
        if creates != 1 || kills > 1 {
            return Err(Reject::InvalidLedger);
        }
    }
    for (key, plan) in world.operation_plans.entries() {
        plan_record(world, *key, plan)?;
    }
    Ok(())
}

fn plan_record(world: &World, key: OperationKey, plan: &OperationPlan) -> Result<(), Reject> {
    let ledger = world
        .operation_record(key)
        .ok_or(Reject::InvalidReference)?;
    if plan.restore_from.is_some() || plan.target_snapshot.is_some() {
        return Err(Reject::UnsupportedSnapshotHistory);
    }
    if plan.key != key
        || plan.epoch != ledger.operation.epoch
        || plan.generation != 1
        || plan.retry_limit != crate::transition::DEFAULT_RETRY_LIMIT
    {
        return Err(Reject::InvalidLedger);
    }
    unique(&plan.stop_vms)?;
    unique(&plan.close_slots)?;
    match ledger.operation.kind {
        OperationKind::Create => {
            if plan.vm.is_some() != plan.slot.is_some()
                || plan.sealed != plan.vm.is_some()
                || plan.stop_vms.as_slice() != plan.vm.as_slice()
                || plan.close_slots.as_slice() != plan.slot.as_slice()
            {
                return Err(Reject::InvalidLedger);
            }
        }
        OperationKind::Kill => {
            if !plan.sealed || plan.vm.is_some() || plan.slot.is_some() {
                return Err(Reject::InvalidLedger);
            }
        }
        _ => return Err(Reject::UnsupportedSnapshotHistory),
    }
    for vm in &plan.stop_vms {
        reserved_vm(world, key.sandbox, *vm)?;
    }
    for slot in &plan.close_slots {
        reserved_slot(world, key.sandbox, *slot)?;
    }
    Ok(())
}

fn startup_plan(world: &World, sandbox: SandboxId) -> Option<&OperationPlan> {
    world
        .operation_plans
        .entries()
        .iter()
        .find_map(|(key, plan)| {
            (key.sandbox == sandbox
                && world
                    .operation_record(*key)
                    .is_some_and(|row| row.operation.kind == OperationKind::Create))
            .then_some(plan)
        })
}

fn reserved_vm(world: &World, sandbox: SandboxId, vm: VmId) -> Result<(), Reject> {
    if vm.0 >= world.next_vm_id {
        return Err(Reject::InvalidFrontier);
    }
    if startup_plan(world, sandbox).is_none_or(|plan| plan.vm != Some(vm))
        || world
            .operation_plans
            .entries()
            .iter()
            .any(|(key, plan)| key.sandbox != sandbox && plan.vm == Some(vm))
    {
        return Err(Reject::InvalidReference);
    }
    Ok(())
}
fn reserved_slot(world: &World, sandbox: SandboxId, slot: NetSlotId) -> Result<(), Reject> {
    if slot.0 >= world.next_slot_id {
        return Err(Reject::InvalidFrontier);
    }
    if startup_plan(world, sandbox).is_none_or(|plan| plan.slot != Some(slot))
        || world
            .operation_plans
            .entries()
            .iter()
            .any(|(key, plan)| key.sandbox != sandbox && plan.slot == Some(slot))
    {
        return Err(Reject::InvalidReference);
    }
    Ok(())
}

fn binding_current(world: &World, binding: HostBinding) -> Result<(), Reject> {
    let row = record(world, binding.sandbox)?;
    if row.owner != binding.owner || row.generation != binding.generation {
        return Err(Reject::InvalidMaintenance);
    }
    reserved_vm(world, binding.sandbox, binding.vm)?;
    reserved_slot(world, binding.sandbox, binding.network)
}
