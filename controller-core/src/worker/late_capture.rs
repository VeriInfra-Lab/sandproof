//! Account for a completed old capture after termination, without restoring access.
use super::{
    Command, EffectId, Phase, RestoreSource, SandboxRecord, Snapshot, SnapshotStage, VmStatus,
    WorkerDecision, WorkerEvent, WorkerReject, World, put_snapshot, put_vm,
};
use crate::OperationPlan;

pub(super) fn record(world: &World, id: &EffectId) -> Option<Result<WorkerDecision, WorkerReject>> {
    let command = id.command();
    let sandbox = command.sandbox();
    let row = world.sandbox_record(sandbox)?;
    if !row.terminal
        || row.phase != Phase::Revoking
        || !matches!(
            command,
            Command::FreezeVm { .. }
                | Command::SealSnapshot { .. }
                | Command::CommitSnapshot { .. }
        )
    {
        return None;
    }
    Some(record_inner(world, id, row))
}
fn record_inner(
    world: &World,
    id: &EffectId,
    row: &SandboxRecord,
) -> Result<WorkerDecision, WorkerReject> {
    let command = id.command();
    let sandbox = command.sandbox();
    let logical = world
        .logical_effect(&id.logical)
        .ok_or(WorkerReject::PlanMismatch)?;
    let crate::EffectOrigin::Operation(key) = logical.origin else {
        return Err(WorkerReject::PlanMismatch);
    };
    let plan = world
        .operation_plan(key)
        .ok_or(WorkerReject::PlanMismatch)?;
    if plan.key != key
        || key.sandbox != sandbox
        || key.operation == row.current_operation.ok_or(WorkerReject::PlanMismatch)?.id
        || plan.generation != row.generation
        || command.epoch() != Some(plan.epoch)
        || !plan.capture.is_some_and(|c| {
            c.eligibility() == crate::capture::RestoreEligibility::Revoked && c.has_capture_work()
        })
    {
        return Err(WorkerReject::PlanMismatch);
    }
    match command {
        Command::FreezeVm { .. } => freeze(world, row, plan, command),
        Command::SealSnapshot { .. } => seal(world, row, plan, command),
        Command::CommitSnapshot { .. } => commit(world, row, plan, command),
        _ => Err(WorkerReject::PlanMismatch),
    }
}

fn freeze(
    world: &World,
    row: &SandboxRecord,
    plan: &OperationPlan,
    command: &Command,
) -> Result<WorkerDecision, WorkerReject> {
    let Command::FreezeVm { generation, vm, .. } = *command else {
        return Err(WorkerReject::PlanMismatch);
    };
    let sandbox = command.sandbox();

    let old = world.vm_record(vm).ok_or(WorkerReject::ResourceNotFound)?;
    if old.sandbox != sandbox
        || old.owner != row.owner
        || old.generation != generation
        || generation != plan.generation
        || row.current_vm != Some(vm)
        || old.status != VmStatus::Executing
    {
        return Err(WorkerReject::IdentityMismatch);
    }
    let mut next = world.clone();
    let mut frozen = *old;
    frozen.status = VmStatus::Frozen;
    put_vm(&mut next, vm, frozen);
    Ok(WorkerDecision {
        event: WorkerEvent::FreezeVm { sandbox, vm },
        next,
    })
}

fn seal(
    world: &World,
    row: &SandboxRecord,
    plan: &OperationPlan,
    command: &Command,
) -> Result<WorkerDecision, WorkerReject> {
    let Command::SealSnapshot {
        generation,
        vm,
        workspace,
        snapshot,
        manifest,
        ..
    } = *command
    else {
        return Err(WorkerReject::PlanMismatch);
    };
    let sandbox = command.sandbox();

    if generation != plan.generation
        || workspace != row.workspace
        || plan.vm != Some(vm)
        || plan.target_snapshot != Some(RestoreSource { snapshot, manifest })
        || world.snapshots.get(&snapshot).is_some()
    {
        return Err(WorkerReject::PlanMismatch);
    }
    let old = world.vm_record(vm).ok_or(WorkerReject::ResourceNotFound)?;
    if old.sandbox != sandbox
        || old.owner != row.owner
        || old.generation != generation
        || old.status != VmStatus::Frozen
    {
        return Err(WorkerReject::IdentityMismatch);
    }
    let mut next = world.clone();
    put_snapshot(
        &mut next,
        snapshot,
        Snapshot {
            owner: row.owner,
            sandbox,
            source_generation: generation,
            workspace,
            manifest,
            stage: SnapshotStage::LocalSealed,
            immutable: false,
        },
    );
    Ok(WorkerDecision {
        event: WorkerEvent::SealSnapshot {
            sandbox,
            snapshot,
            manifest,
        },
        next,
    })
}

fn commit(
    world: &World,
    row: &SandboxRecord,
    plan: &OperationPlan,
    command: &Command,
) -> Result<WorkerDecision, WorkerReject> {
    let Command::CommitSnapshot {
        generation,
        snapshot,
        manifest,
        ..
    } = *command
    else {
        return Err(WorkerReject::PlanMismatch);
    };
    let sandbox = command.sandbox();

    if generation != plan.generation
        || plan.target_snapshot != Some(RestoreSource { snapshot, manifest })
    {
        return Err(WorkerReject::PlanMismatch);
    }
    let old = world
        .snapshots
        .get(&snapshot)
        .ok_or(WorkerReject::ResourceNotFound)?;
    if old.owner != row.owner
        || old.sandbox != sandbox
        || old.source_generation != generation
        || old.workspace != row.workspace
        || old.manifest != manifest
        || old.stage != SnapshotStage::LocalSealed
    {
        return Err(WorkerReject::IdentityMismatch);
    }
    let mut next = world.clone();
    let mut old_plan = plan.clone();
    old_plan.capture = Some(
        plan.capture
            .and_then(crate::capture::CaptureHandoff::observe_bundle)
            .ok_or(WorkerReject::ResourceBusy)?,
    );
    next.operation_plans.put(plan.key, old_plan);
    let mut durable = *old;
    durable.stage = SnapshotStage::Durable;
    durable.immutable = true;
    put_snapshot(&mut next, snapshot, durable);
    // Do not enqueue an old Stop or alter the terminal session/fence.
    Ok(WorkerDecision {
        event: WorkerEvent::CommitSnapshot { snapshot },
        next,
    })
}
