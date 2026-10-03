use super::{
    Command, EffectId, EffectOrigin, EffectOutcome, EffectStatus, Generation, LogicalEffectId,
    LogicalEffectResult, NetSlotId, OperationKind, OperationPlan, OperationStatus, Reject,
    SandboxId, StopCoverage, VmId, World, effect, record, reserved_slot, reserved_vm, terminal,
    unique,
};

pub(super) fn validate(world: &World) -> Result<(), Reject> {
    unique(&world.effect_keys)?;
    unique(&world.inflight)?;
    if world.effect_keys.len() != world.effects.entries().len()
        || world.inflight.len() != world.effects.entries().len()
        || world
            .effect_keys
            .iter()
            .chain(world.inflight.iter())
            .any(|id| world.effect_status(id).is_none())
    {
        return Err(Reject::InvalidEffect);
    }
    for (id, logical) in world.logical_effects.entries() {
        command(world, &id.command, logical.origin)?;
        if logical
            .current_attempt
            .checked_add(logical.remaining_retries)
            != Some(crate::transition::DEFAULT_RETRY_LIMIT)
        {
            return Err(Reject::InvalidEffect);
        }
        for attempt in 0..=logical.current_attempt {
            let effect = EffectId {
                logical: id.clone(),
                attempt,
            };
            let status = world.effect_status(&effect).ok_or(Reject::InvalidEffect)?;
            if attempt < logical.current_attempt
                && !matches!(status, EffectStatus::Acknowledged(EffectOutcome::Failed(failure)) if failure.retryable())
            {
                return Err(Reject::InvalidEffect);
            }
        }
        let current = EffectId {
            logical: id.clone(),
            attempt: logical.current_attempt,
        };
        let status = world.effect_status(&current).ok_or(Reject::InvalidEffect)?;
        let consistent = match (logical.result, status) {
            (
                None,
                EffectStatus::Committed
                | EffectStatus::Dispatched
                | EffectStatus::OutcomeUnknown
                | EffectStatus::Acknowledged(EffectOutcome::Failed(_)),
            ) => true,
            (
                Some(LogicalEffectResult::Succeeded { attempt, result }),
                EffectStatus::Acknowledged(EffectOutcome::Succeeded(found)),
            ) => attempt == current.attempt && result == found,
            (
                Some(LogicalEffectResult::Failed { attempt, failure }),
                EffectStatus::Acknowledged(EffectOutcome::Failed(found)),
            ) => {
                attempt == current.attempt
                    && failure == found
                    && (!failure.retryable() || logical.remaining_retries == 0)
            }
            (
                Some(LogicalEffectResult::RetiredByStop { attempt, coverage }),
                EffectStatus::RetiredByStop(found),
            ) => attempt == current.attempt && coverage == found,
            _ => false,
        };
        if !consistent {
            return Err(Reject::InvalidEffect);
        }
    }
    for (id, status) in world.effects.entries() {
        let logical = world
            .logical_effect(&id.logical)
            .ok_or(Reject::InvalidEffect)?;
        if id.attempt > logical.current_attempt
            || !world.effect_keys.contains(id)
            || !world.inflight.contains(id)
        {
            return Err(Reject::InvalidEffect);
        }
        if matches!(
            status,
            EffectStatus::Acknowledged(EffectOutcome::Succeeded(_))
        ) {
            historical_success(world, id.command())?;
        }
        if let EffectStatus::RetiredByStop(coverage) = status {
            retired_stop(world, id, *coverage)?;
        }
    }
    Ok(())
}

fn command_origin<'a>(
    world: &'a World,
    command: &Command,
    origin: EffectOrigin,
) -> Result<Option<&'a OperationPlan>, Reject> {
    let sandbox = command.sandbox();
    Ok(match origin {
        EffectOrigin::Operation(key) => {
            let plan = world.operation_plan(key).ok_or(Reject::InvalidReference)?;
            if key.sandbox != sandbox || command.epoch() != Some(plan.epoch) {
                return Err(Reject::InvalidEffect);
            }
            Some(plan)
        }
        EffectOrigin::Maintenance(found) => {
            if found != sandbox
                || !matches!(
                    command,
                    Command::CleanupVm { .. } | Command::CleanupSlot { .. }
                )
            {
                return Err(Reject::InvalidEffect);
            }
            None
        }
    })
}

fn command(world: &World, command: &Command, origin: EffectOrigin) -> Result<(), Reject> {
    let sandbox = command.sandbox();
    let row = record(world, sandbox)?;
    let plan = command_origin(world, command, origin)?;
    match command {
        Command::InstallFence {
            epoch, terminal, ..
        } => {
            fence_command(world, row, plan, *epoch, *terminal)?;
        }
        Command::PrepareVm {
            generation,
            vm,
            slot,
            source,
            ..
        } => {
            if source.is_some() {
                return Err(Reject::UnsupportedSnapshotHistory);
            }
            startup_command(plan, *generation, *vm, *slot)?;
            reserved_vm(world, sandbox, *vm)?;
            reserved_slot(world, sandbox, *slot)?;
        }
        Command::AbortPrepare {
            generation,
            vm,
            slot,
            ..
        } => {
            startup_command(plan, *generation, *vm, *slot)?;
            reserved_vm(world, sandbox, *vm)?;
            reserved_slot(world, sandbox, *slot)?;
            if !row.terminal {
                return Err(Reject::InvalidEffect);
            }
        }
        Command::ConfigureNetwork {
            generation,
            vm,
            slot,
            policy,
            ..
        } => {
            startup_command(plan, *generation, *vm, *slot)?;
            vm_reference(world, sandbox, *vm, *generation)?;
            slot_reference(world, sandbox, *slot, *generation, Some(*vm))?;
            if *policy != row.policy {
                return Err(Reject::InvalidEffect);
            }
        }
        Command::ActivateVm { generation, vm, .. } => {
            let plan = plan.ok_or(Reject::InvalidEffect)?;
            if !plan.sealed || plan.vm != Some(*vm) || plan.generation != *generation {
                return Err(Reject::InvalidEffect);
            }
            vm_reference(world, sandbox, *vm, *generation)?;
        }
        Command::ApplyFilePolicy {
            generation,
            workspace,
            policy,
            ..
        } => {
            let plan = plan.ok_or(Reject::InvalidEffect)?;
            if !plan.sealed
                || plan.vm.is_none()
                || plan.slot.is_none()
                || *workspace != row.workspace
                || *policy != row.file_policy
                || plan.generation != *generation
            {
                return Err(Reject::InvalidEffect);
            }
        }
        Command::StopVm { generation, vm, .. } => {
            let plan = plan.ok_or(Reject::InvalidEffect)?;
            if !plan.stop_vms.contains(vm) || plan.generation != *generation || !row.terminal {
                return Err(Reject::InvalidEffect);
            }
            vm_reference(world, sandbox, *vm, *generation)?;
        }
        Command::CleanupVm { generation, vm, .. } => {
            if plan.is_some() {
                return Err(Reject::InvalidEffect);
            }
            vm_reference(world, sandbox, *vm, *generation)?;
        }
        Command::CleanupSlot {
            generation, slot, ..
        } => {
            if plan.is_some() {
                return Err(Reject::InvalidEffect);
            }
            slot_reference(world, sandbox, *slot, *generation, None)?;
        }
        Command::FreezeVm { .. }
        | Command::SealSnapshot { .. }
        | Command::CommitSnapshot { .. } => return Err(Reject::UnsupportedSnapshotHistory),
    }
    Ok(())
}

fn fence_command(
    world: &World,
    row: &crate::SandboxRecord,
    plan: Option<&OperationPlan>,
    epoch: u64,
    terminal: bool,
) -> Result<(), Reject> {
    let origin = plan.ok_or(Reject::InvalidEffect)?;
    let operation = world
        .operation_record(origin.key)
        .ok_or(Reject::InvalidEffect)?;
    if (!terminal && operation.operation.kind != OperationKind::Create)
        || epoch == 0
        || epoch > row.epoch
        || (terminal && (!row.terminal || epoch != row.epoch))
    {
        return Err(Reject::InvalidEffect);
    }
    Ok(())
}

fn startup_command(
    plan: Option<&OperationPlan>,
    generation: Generation,
    vm: VmId,
    slot: NetSlotId,
) -> Result<(), Reject> {
    let plan = plan.ok_or(Reject::InvalidEffect)?;
    if !plan.sealed
        || plan.generation != generation
        || plan.vm != Some(vm)
        || plan.slot != Some(slot)
    {
        return Err(Reject::InvalidEffect);
    }
    Ok(())
}
fn vm_reference(
    world: &World,
    sandbox: SandboxId,
    vm: VmId,
    generation: Generation,
) -> Result<(), Reject> {
    if world
        .vm_record(vm)
        .is_none_or(|row| row.sandbox != sandbox || row.generation != generation)
    {
        return Err(Reject::InvalidReference);
    }
    Ok(())
}
fn slot_reference(
    world: &World,
    sandbox: SandboxId,
    slot: NetSlotId,
    generation: Generation,
    vm: Option<VmId>,
) -> Result<(), Reject> {
    if world.slot_record(slot).is_none_or(|row| {
        row.sandbox != sandbox || row.generation != generation || vm.is_some_and(|vm| row.vm != vm)
    }) {
        return Err(Reject::InvalidReference);
    }
    Ok(())
}

// Success is historical. In particular cleanup invalidates the immediate
// PrepareVm success predicate, and revocation can close an earlier authority.
fn historical_success(world: &World, command: &Command) -> Result<(), Reject> {
    let valid = match command {
        Command::PrepareVm {
            sandbox,
            generation,
            vm,
            slot,
            ..
        } => {
            vm_reference(world, *sandbox, *vm, *generation)?;
            slot_reference(world, *sandbox, *slot, *generation, Some(*vm))?;
            true
        }
        _ => effect::command_result_present(world, command),
    };
    if !valid {
        return Err(Reject::InvalidEffect);
    }
    Ok(())
}

fn retired_stop(world: &World, id: &EffectId, coverage: StopCoverage) -> Result<(), Reject> {
    let command = terminal::activation_stop(id).ok_or(Reject::InvalidEffect)?;
    let stop = EffectId {
        logical: LogicalEffectId {
            command: command.clone(),
        },
        attempt: coverage.attempt,
    };
    let stop_logical = world
        .logical_effect(&stop.logical)
        .ok_or(Reject::InvalidEffect)?;
    let activation = world
        .logical_effect(&id.logical)
        .ok_or(Reject::InvalidEffect)?;
    let row = record(world, command.sandbox())?;
    let ledger = world
        .operation_record(coverage.operation)
        .ok_or(Reject::InvalidEffect)?;
    let valid = command.epoch() == Some(coverage.epoch)
        && coverage.operation.sandbox == command.sandbox()
        && row.terminal
        && row.epoch == coverage.epoch
        && ledger.operation.kind == OperationKind::Create
        && matches!(
            ledger.operation.status,
            OperationStatus::Compensating(_) | OperationStatus::Failed(_)
        )
        && stop_logical.current_attempt == coverage.attempt
        && stop_logical.origin == EffectOrigin::Operation(coverage.operation)
        && activation.origin == stop_logical.origin
        && activation.current_attempt == id.attempt
        && activation.result
            == Some(LogicalEffectResult::RetiredByStop {
                attempt: id.attempt,
                coverage,
            })
        && stop_logical.result
            == Some(LogicalEffectResult::Succeeded {
                attempt: coverage.attempt,
                result: coverage.result,
            })
        && world.effect_status(&stop)
            == Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(
                coverage.result,
            )))
        && effect::command_result_present(world, &command)
        && effect::effect_succeeded_check(
            world,
            &Command::InstallFence {
                sandbox: command.sandbox(),
                epoch: coverage.epoch,
                terminal: true,
            },
        );
    if !valid {
        return Err(Reject::InvalidEffect);
    }
    Ok(())
}
