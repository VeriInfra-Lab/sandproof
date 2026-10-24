//! Every capture belongs to its original generation and operation plan.
//! These are structural checks, not host authenticity or reachability proofs.
use super::{Reject, history, record, resources::observed};
use crate::capture::{CaptureCommit, RestoreEligibility};
use crate::{
    Authority, Command, Operation, OperationInput, OperationKey, OperationKind,
    OperationLedgerRecord, OperationPlan, OperationResult, OperationStatus, Phase, RestoreSource,
    SandboxId, SandboxRecord, SnapshotStage, VmId, VmStatus, WorkerFence, World, effect,
};

pub(super) fn ledger(key: OperationKey, row: &OperationLedgerRecord) -> Result<(), Reject> {
    let status = match (row.operation.status, row.result) {
        (
            OperationStatus::Pending
            | OperationStatus::OutcomeUnknown
            | OperationStatus::Compensating(_),
            None,
        )
        | (OperationStatus::Succeeded, Some(OperationResult::Paused(_))) => true,
        (OperationStatus::Failed(a), Some(OperationResult::Failed(b))) => a == b,
        _ => false,
    };
    if row.sandbox != key.sandbox
        || row.operation.id != key.operation
        || row.operation.kind != OperationKind::Pause
        || row.operation.epoch < 2
        || !matches!(row.input, OperationInput::Pause {generation} if generation>0)
        || !status
    {
        return Err(Reject::InvalidLedger);
    }
    Ok(())
}

pub(super) fn operation_history(world: &World, sandbox: SandboxId) -> Result<(), Reject> {
    history::validate(world, sandbox)
}

fn pause_plan(world: &World, sandbox: SandboxId) -> Result<&OperationPlan, Reject> {
    let row = record(world, sandbox)?;
    let operation = row.current_operation.ok_or(Reject::InvalidReference)?;
    let key = OperationKey {
        sandbox,
        operation: operation.id,
    };
    if operation.kind != OperationKind::Pause {
        return Err(Reject::InvalidReference);
    }
    world.operation_plan(key).ok_or(Reject::InvalidReference)
}
fn target_plan(
    world: &World,
    sandbox: SandboxId,
    target: RestoreSource,
) -> Result<&OperationPlan, Reject> {
    world
        .operation_plans
        .entries()
        .iter()
        .find_map(|(key, plan)| {
            (key.sandbox == sandbox
                && plan.target_snapshot == Some(target)
                && world
                    .operation_record(*key)
                    .is_some_and(|r| r.operation.kind == OperationKind::Pause))
            .then_some(plan)
        })
        .ok_or(Reject::InvalidReference)
}

pub(super) fn source(
    world: &World,
    sandbox: SandboxId,
    target: RestoreSource,
) -> Result<(), Reject> {
    let row = record(world, sandbox)?;
    let snapshot = world
        .snapshots
        .get(&target.snapshot)
        .ok_or(Reject::InvalidReference)?;
    if snapshot.owner != row.owner
        || snapshot.sandbox != sandbox
        || snapshot.source_generation == 0
        || snapshot.source_generation > row.generation
        || snapshot.workspace != row.workspace
        || snapshot.manifest != target.manifest
        || snapshot.stage != SnapshotStage::Durable
        || !snapshot.immutable
    {
        return Err(Reject::InvalidResource);
    }
    Ok(())
}

pub(super) fn plan(world: &World, key: OperationKey, plan: &OperationPlan) -> Result<(), Reject> {
    let startup =
        history::startup(world, key.sandbox, plan.generation).ok_or(Reject::InvalidReference)?;
    let capture = plan.capture.ok_or(Reject::InvalidLedger)?;
    if plan.key != key
        || world.operation_record(key).is_none_or(|r| {
            r.operation.epoch != plan.epoch
                || r.input
                    != (OperationInput::Pause {
                        generation: plan.generation,
                    })
        })
        || plan.generation == 0
        || plan.retry_limit != crate::transition::DEFAULT_RETRY_LIMIT
        || plan.restore_from.is_some()
        || plan.slot.is_some()
        || plan.stop_vms != startup.stop_vms
        || plan.close_slots != startup.close_slots
        || plan.sealed != plan.target_snapshot.is_some()
        || plan.vm.is_some() != plan.target_snapshot.is_some()
        || plan.vm.is_some_and(|vm| startup.vm != Some(vm))
        || capture.has_capture_work() != capture.original_continuation().is_none()
    {
        return Err(Reject::InvalidLedger);
    }
    if let Some(continued) = capture.original_continuation() {
        if !capture.may_reactivate_original() || startup.vm != Some(continued.vm) {
            return Err(Reject::InvalidLedger);
        }
        let command = Command::ActivateVm {
            sandbox: key.sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            vm: continued.vm,
        };
        if !world
            .logical_effect(&crate::LogicalEffectId { command })
            .is_some_and(|logical| logical.origin == crate::EffectOrigin::Operation(key))
        {
            return Err(Reject::InvalidEffect);
        }
    }
    if let Some(target) = plan.target_snapshot {
        if target.snapshot.0 >= world.next_snapshot_id
            || target.manifest.digest.0 >= world.next_manifest_digest
        {
            return Err(Reject::InvalidFrontier);
        }
        if world.operation_plans.entries().iter().any(|(other, p)| {
            *other != key
                && p.target_snapshot
                    .is_some_and(|s| s.snapshot == target.snapshot || s.manifest == target.manifest)
        }) {
            return Err(Reject::DuplicateKey);
        }
    }
    match capture.commit() {
        CaptureCommit::NotRegistered | CaptureCommit::Unknown => {
            if capture.has_unload_source()
                || capture.eligibility() == RestoreEligibility::Recoverable
            {
                return Err(Reject::InvalidLedger);
            }
            if capture.commit() == CaptureCommit::Unknown {
                let target = plan.target_snapshot.ok_or(Reject::InvalidReference)?;
                if world
                    .snapshots
                    .get(&target.snapshot)
                    .is_none_or(|s| s.stage != SnapshotStage::LocalSealed)
                {
                    return Err(Reject::InvalidResource);
                }
            }
        }
        CaptureCommit::Registered => {
            if !capture.has_unload_source()
                || capture.eligibility() == RestoreEligibility::Capturing
            {
                return Err(Reject::InvalidLedger);
            }
            source(
                world,
                key.sandbox,
                plan.target_snapshot.ok_or(Reject::InvalidReference)?,
            )?;
        }
    }
    if let Some(OperationResult::Paused(found)) = world.operation_record(key).and_then(|r| r.result)
        && plan.target_snapshot != Some(found)
    {
        return Err(Reject::InvalidLedger);
    }
    Ok(())
}

pub(super) fn snapshots(world: &World) -> Result<(), Reject> {
    for (id, snapshot) in world.snapshots.entries() {
        let row = record(world, snapshot.sandbox)?;
        let plan = target_plan(
            world,
            snapshot.sandbox,
            RestoreSource {
                snapshot: *id,
                manifest: snapshot.manifest,
            },
        )?;
        let vm = plan.vm.ok_or(Reject::InvalidReference)?;
        if snapshot.owner != row.owner
            || snapshot.source_generation != plan.generation
            || snapshot.workspace != row.workspace
            || plan.target_snapshot
                != Some(RestoreSource {
                    snapshot: *id,
                    manifest: snapshot.manifest,
                })
            || !world
                .resource_index(snapshot.sandbox)
                .snapshots
                .contains(id)
            || snapshot.immutable != (snapshot.stage == SnapshotStage::Durable)
        {
            return Err(Reject::InvalidResource);
        }
        let seal = Command::SealSnapshot {
            sandbox: snapshot.sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            vm,
            workspace: snapshot.workspace,
            snapshot: *id,
            manifest: snapshot.manifest,
        };
        if !observed(world, |c| *c == seal) {
            return Err(Reject::InvalidResource);
        }
        if snapshot.stage == SnapshotStage::Durable {
            let commit = Command::CommitSnapshot {
                sandbox: snapshot.sandbox,
                epoch: plan.epoch,
                generation: plan.generation,
                snapshot: *id,
                manifest: snapshot.manifest,
            };
            if !observed(world, |c| *c == commit)
                || plan
                    .capture
                    .is_none_or(|c| c.commit() != CaptureCommit::Registered)
            {
                return Err(Reject::InvalidResource);
            }
        }
    }
    Ok(())
}

pub(super) fn record_sources(
    world: &World,
    sandbox: SandboxId,
    row: &SandboxRecord,
) -> Result<(), Reject> {
    if let Some(target) = row.durable_snapshot {
        source(world, sandbox, target)?;
        target_plan(world, sandbox, target)?;
        let latest = world
            .operations
            .entries()
            .iter()
            .filter(|(key, r)| {
                key.sandbox == sandbox
                    && r.operation.kind == OperationKind::Pause
                    && r.operation.status == OperationStatus::Succeeded
            })
            .max_by_key(|(_, r)| r.operation.epoch);
        if latest.is_none_or(|(_, r)| r.result != Some(OperationResult::Paused(target))) {
            return Err(Reject::InvalidReference);
        }
    }
    if let Some(target) = row.restore_from {
        source(world, sandbox, target)?;
        if history::startup(world, sandbox, row.generation)
            .is_none_or(|p| p.restore_from != Some(target))
            || world
                .snapshots
                .get(&target.snapshot)
                .is_none_or(|s| s.source_generation.checked_add(1) != Some(row.generation))
            || matches!(row.phase, Phase::Running | Phase::Pausing | Phase::Paused)
        {
            return Err(Reject::InvalidReference);
        }
    } else if row.phase == Phase::Starting
        && row
            .current_operation
            .is_some_and(|o| o.kind == OperationKind::Resume)
    {
        return Err(Reject::InvalidReference);
    }
    Ok(())
}

pub(super) fn paused_authority(
    world: &World,
    sandbox: SandboxId,
    row: &SandboxRecord,
    found: SandboxId,
    target: RestoreSource,
) -> Result<(), Reject> {
    if found != sandbox
        || row.durable_snapshot != Some(target)
        || !matches!(row.phase, Phase::Paused | Phase::Revoking)
    {
        return Err(Reject::InvalidAuthority);
    }
    source(world, sandbox, target)
}

pub(super) fn phase(
    world: &World,
    sandbox: SandboxId,
    row: &SandboxRecord,
    operation: Operation,
) -> Result<(), Reject> {
    if row.terminal || operation.kind != OperationKind::Pause || row.epoch != operation.epoch {
        return Err(Reject::InvalidAuthority);
    }
    if row.phase == Phase::Pausing {
        if row.desired != Authority::Closed
            || row.current_vm.is_none()
            || row.durable_snapshot.is_some_and(|target| {
                world
                    .snapshots
                    .get(&target.snapshot)
                    .is_none_or(|s| s.source_generation >= row.generation)
            })
            || !matches!(
                operation.status,
                OperationStatus::Pending
                    | OperationStatus::OutcomeUnknown
                    | OperationStatus::Compensating(_)
            )
            || !matches!(row.effective, Authority::Closed | Authority::Running { .. })
        {
            return Err(Reject::InvalidAuthority);
        }
        return Ok(());
    }
    let plan = pause_plan(world, sandbox)?;
    let target = plan.target_snapshot.ok_or(Reject::InvalidReference)?;
    let vm = plan.vm.ok_or(Reject::InvalidReference)?;
    stop_plan(world, plan, vm)?;
    if operation.status != OperationStatus::Succeeded
        || row.current_vm.is_some()
        || row.net_slot.is_some()
        || row.durable_snapshot != Some(target)
        || row.desired != row.effective
        || row.effective
            != (Authority::Paused {
                sandbox,
                source: target,
            })
        || world.worker_fences.get(&sandbox)
            != Some(&WorkerFence {
                epoch: plan.epoch,
                terminal: false,
            })
        || world.resource_index(sandbox).vms.iter().any(|id| {
            world
                .vm_record(*id)
                .is_none_or(|v| !matches!(v.status, VmStatus::Stopped | VmStatus::Reclaimed))
        })
        || world
            .resource_index(sandbox)
            .slots
            .iter()
            .any(|id| world.slot_record(*id).is_none_or(|s| s.active))
    {
        return Err(Reject::InvalidAuthority);
    }
    let commands = [
        Command::InstallFence {
            sandbox,
            epoch: plan.epoch,
            terminal: false,
        },
        Command::FreezeVm {
            sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            vm,
        },
        Command::SealSnapshot {
            sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            vm,
            workspace: row.workspace,
            snapshot: target.snapshot,
            manifest: target.manifest,
        },
        Command::CommitSnapshot {
            sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            snapshot: target.snapshot,
            manifest: target.manifest,
        },
        Command::StopVm {
            sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            vm,
        },
    ];
    if commands
        .iter()
        .any(|c| !effect::effect_succeeded_check(world, c))
    {
        return Err(Reject::InvalidEffect);
    }
    Ok(())
}

pub(super) fn stop_plan(world: &World, plan: &OperationPlan, vm: VmId) -> Result<(), Reject> {
    if world
        .operation_record(plan.key)
        .is_none_or(|r| r.operation.kind != OperationKind::Pause)
        || plan.vm != Some(vm)
        || !plan
            .capture
            .is_some_and(crate::capture::CaptureHandoff::may_stop_original)
    {
        return Err(Reject::InvalidEffect);
    }
    source(
        world,
        plan.key.sandbox,
        plan.target_snapshot.ok_or(Reject::InvalidReference)?,
    )
}

pub(super) fn command(
    world: &World,
    command: &Command,
    plan: &OperationPlan,
) -> Result<(), Reject> {
    let sandbox = command.sandbox();
    let row = record(world, sandbox)?;
    let original = history::startup(world, sandbox, plan.generation)
        .and_then(|p| p.vm)
        .ok_or(Reject::InvalidReference)?;
    if plan.key.sandbox != sandbox
        || command.epoch() != Some(plan.epoch)
        || plan.generation == 0
        || world
            .operation_record(plan.key)
            .is_none_or(|r| r.operation.kind != OperationKind::Pause)
    {
        return Err(Reject::InvalidEffect);
    }
    match command {
        Command::FreezeVm { generation, vm, .. }
            if *generation == plan.generation && *vm == original =>
        {
            Ok(())
        }
        Command::SealSnapshot {
            generation,
            vm,
            workspace,
            snapshot,
            manifest,
            ..
        } if *generation == plan.generation
            && *vm == original
            && *workspace == row.workspace
            && plan.sealed
            && plan.target_snapshot
                == Some(RestoreSource {
                    snapshot: *snapshot,
                    manifest: *manifest,
                }) =>
        {
            Ok(())
        }
        Command::CommitSnapshot {
            generation,
            snapshot,
            manifest,
            ..
        } if *generation == plan.generation
            && plan.sealed
            && plan.target_snapshot
                == Some(RestoreSource {
                    snapshot: *snapshot,
                    manifest: *manifest,
                })
            && world
                .snapshots
                .get(snapshot)
                .is_some_and(|s| s.manifest == *manifest) =>
        {
            Ok(())
        }
        _ => Err(Reject::InvalidEffect),
    }
}
