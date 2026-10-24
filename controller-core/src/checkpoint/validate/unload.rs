//! Schema 2 adds one first-generation pause to the original create/kill image.
//! These are structural checks, not host authenticity or reachability proofs.
use super::{Reject, record, resources::observed, startup_plan};
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
        || row.operation.epoch != 2
        || row.input != (OperationInput::Pause { generation: 1 })
        || !status
    {
        return Err(Reject::InvalidLedger);
    }
    Ok(())
}

pub(super) fn operation_history(world: &World, sandbox: SandboxId) -> Result<(), Reject> {
    let mut pauses = 0;
    let mut latest = 1;
    for (key, row) in world.operations.entries() {
        if key.sandbox != sandbox {
            continue;
        }
        if row.operation.kind == OperationKind::Pause {
            pauses += 1;
            if pauses > 1 {
                return Err(Reject::UnsupportedSnapshotHistory);
            }
        }
        latest = latest.max(row.operation.epoch);
    }
    for (key, row) in world.operations.entries() {
        if key.sandbox != sandbox {
            continue;
        }
        if row.operation.kind == OperationKind::Kill && row.operation.epoch != 2 + pauses {
            return Err(Reject::InvalidLedger);
        }
    }
    if record(world, sandbox)?.epoch != latest {
        return Err(Reject::InvalidLedger);
    }
    if pauses != 0
        && startup_plan(world, sandbox)
            .and_then(|p| world.operation_record(p.key))
            .is_none_or(|r| r.operation.status != OperationStatus::Succeeded)
    {
        return Err(Reject::InvalidLedger);
    }
    Ok(())
}

fn pause_plan(world: &World, sandbox: SandboxId) -> Result<&OperationPlan, Reject> {
    world
        .operation_plans
        .entries()
        .iter()
        .find_map(|(key, plan)| {
            (key.sandbox == sandbox
                && world
                    .operation_record(*key)
                    .is_some_and(|r| r.operation.kind == OperationKind::Pause))
            .then_some(plan)
        })
        .ok_or(Reject::InvalidReference)
}

fn source(world: &World, sandbox: SandboxId, target: RestoreSource) -> Result<(), Reject> {
    let row = record(world, sandbox)?;
    let snapshot = world
        .snapshots
        .get(&target.snapshot)
        .ok_or(Reject::InvalidReference)?;
    if snapshot.owner != row.owner
        || snapshot.sandbox != sandbox
        || snapshot.source_generation != 1
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
    let startup = startup_plan(world, key.sandbox).ok_or(Reject::InvalidReference)?;
    let capture = plan.capture.ok_or(Reject::InvalidLedger)?;
    if plan.key != key
        || plan.epoch != 2
        || plan.generation != 1
        || plan.retry_limit != crate::transition::DEFAULT_RETRY_LIMIT
        || plan.restore_from.is_some()
        || plan.slot.is_some()
        || plan.stop_vms != startup.stop_vms
        || plan.close_slots != startup.close_slots
        || plan.sealed != plan.target_snapshot.is_some()
        || plan.vm.is_some() != plan.target_snapshot.is_some()
        || plan.vm.is_some_and(|vm| startup.vm != Some(vm))
        || !capture.has_capture_work()
    {
        return Err(Reject::InvalidLedger);
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
        let plan = pause_plan(world, snapshot.sandbox)?;
        let vm = plan.vm.ok_or(Reject::InvalidReference)?;
        if snapshot.owner != row.owner
            || snapshot.source_generation != 1
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
            epoch: 2,
            generation: 1,
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
                epoch: 2,
                generation: 1,
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
        if pause_plan(world, sandbox)?.target_snapshot != Some(target) {
            return Err(Reject::InvalidReference);
        }
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
    if row.terminal || operation.kind != OperationKind::Pause || row.epoch != 2 {
        return Err(Reject::InvalidAuthority);
    }
    if row.phase == Phase::Pausing {
        if row.desired != Authority::Closed
            || row.current_vm.is_none()
            || row.durable_snapshot.is_some()
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
                epoch: 2,
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
            epoch: 2,
            terminal: false,
        },
        Command::FreezeVm {
            sandbox,
            epoch: 2,
            generation: 1,
            vm,
        },
        Command::SealSnapshot {
            sandbox,
            epoch: 2,
            generation: 1,
            vm,
            workspace: row.workspace,
            snapshot: target.snapshot,
            manifest: target.manifest,
        },
        Command::CommitSnapshot {
            sandbox,
            epoch: 2,
            generation: 1,
            snapshot: target.snapshot,
            manifest: target.manifest,
        },
        Command::StopVm {
            sandbox,
            epoch: 2,
            generation: 1,
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
    let original = startup_plan(world, sandbox)
        .and_then(|p| p.vm)
        .ok_or(Reject::InvalidReference)?;
    if plan.key.sandbox != sandbox
        || plan.epoch != 2
        || plan.generation != 1
        || world
            .operation_record(plan.key)
            .is_none_or(|r| r.operation.kind != OperationKind::Pause)
    {
        return Err(Reject::InvalidEffect);
    }
    match command {
        Command::FreezeVm { generation, vm, .. } if *generation == 1 && *vm == original => Ok(()),
        Command::SealSnapshot {
            generation,
            vm,
            workspace,
            snapshot,
            manifest,
            ..
        } if *generation == 1
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
        } if *generation == 1
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
