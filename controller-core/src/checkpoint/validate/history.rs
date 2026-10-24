//! Consecutive operation epochs and native generations of one logical session.
//! IDs need not be numerically ordered. Allocation plans retain their source
//! after Running clears the row's transient `restore_from` field.
use super::{Reject, record, unique};
use crate::{
    Generation, OperationInput, OperationKey, OperationKind, OperationPlan, OperationResult,
    OperationStatus, SandboxId, World,
};

pub(super) fn startup(
    world: &World,
    sandbox: SandboxId,
    generation: Generation,
) -> Option<&OperationPlan> {
    world
        .operation_plans
        .entries()
        .iter()
        .find_map(|(key, plan)| {
            (key.sandbox == sandbox
                && plan.generation == generation
                && world.operation_record(*key).is_some_and(|row| {
                    matches!(
                        row.operation.kind,
                        OperationKind::Create | OperationKind::Resume
                    )
                }))
            .then_some(plan)
        })
}

pub(super) fn validate(world: &World, sandbox: SandboxId) -> Result<(), Reject> {
    let mut epochs = Vec::new();
    for (key, row) in world.operations.entries() {
        if key.sandbox == sandbox {
            epochs.push(row.operation.epoch);
        }
    }
    unique(&epochs)?;
    let latest = epochs.iter().copied().max().ok_or(Reject::InvalidLedger)?;
    if u64::try_from(epochs.len()).map_err(|_| Reject::InvalidLedger)? != latest
        || record(world, sandbox)?.epoch != latest
    {
        return Err(Reject::InvalidLedger);
    }
    for (key, row) in world.operations.entries() {
        if key.sandbox != sandbox {
            continue;
        }
        let plan = world.operation_plan(*key).ok_or(Reject::InvalidReference)?;
        if let Some(OperationResult::Running(found)) = row.result
            && found != plan.generation
        {
            return Err(Reject::InvalidLedger);
        }
        if row.operation.epoch == 1 {
            if row.operation.kind != OperationKind::Create || plan.generation != 1 {
                return Err(Reject::InvalidLedger);
            }
            continue;
        }
        let (prior_key, prior) = world
            .operations
            .entries()
            .iter()
            .find(|(other, prior)| {
                other.sandbox == sandbox
                    && prior.operation.epoch.checked_add(1) == Some(row.operation.epoch)
            })
            .ok_or(Reject::InvalidLedger)?;
        let previous = world
            .operation_plan(*prior_key)
            .ok_or(Reject::InvalidReference)?;
        let expected = match row.operation.kind {
            OperationKind::Create => return Err(Reject::InvalidLedger),
            OperationKind::Resume => {
                let OperationInput::Resume { source } = row.input else {
                    return Err(Reject::InvalidLedger);
                };
                if prior.operation.kind != OperationKind::Pause
                    || prior.operation.status != OperationStatus::Succeeded
                    || prior.result != Some(OperationResult::Paused(source))
                    || previous.target_snapshot != Some(source)
                {
                    return Err(Reject::InvalidLedger);
                }
                previous
                    .generation
                    .checked_add(1)
                    .ok_or(Reject::InvalidLedger)?
            }
            OperationKind::Pause => {
                if !matches!(
                    prior.operation.kind,
                    OperationKind::Create | OperationKind::Resume
                ) || prior.operation.status != OperationStatus::Succeeded
                {
                    return Err(Reject::InvalidLedger);
                }
                previous.generation
            }
            OperationKind::Kill => previous.generation,
        };
        if plan.generation != expected || expected == 0 {
            return Err(Reject::InvalidLedger);
        }
        match row.input {
            OperationInput::Pause { generation } | OperationInput::Kill { generation }
                if generation != expected =>
            {
                return Err(Reject::InvalidLedger);
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn resume_plan(
    world: &World,
    key: OperationKey,
    plan: &OperationPlan,
) -> Result<(), Reject> {
    let row = world
        .operation_record(key)
        .ok_or(Reject::InvalidReference)?;
    let OperationInput::Resume { source } = row.input else {
        return Err(Reject::InvalidLedger);
    };
    if plan.key != key
        || plan.epoch != row.operation.epoch
        || plan.generation < 2
        || plan.retry_limit != crate::transition::DEFAULT_RETRY_LIMIT
        || plan.capture.is_some()
        || plan.target_snapshot.is_some()
        || plan.restore_from != Some(source)
        || plan.vm.is_some() != plan.slot.is_some()
        || plan.sealed != plan.vm.is_some()
    {
        return Err(Reject::InvalidLedger);
    }
    super::unload::source(world, key.sandbox, source)?;
    if world
        .snapshots
        .get(&source.snapshot)
        .is_none_or(|s| s.source_generation.checked_add(1) != Some(plan.generation))
    {
        return Err(Reject::InvalidResource);
    }
    if plan.vm.is_some_and(|vm| !plan.stop_vms.contains(&vm))
        || plan
            .slot
            .is_some_and(|slot| !plan.close_slots.contains(&slot))
    {
        return Err(Reject::InvalidLedger);
    }
    Ok(())
}
