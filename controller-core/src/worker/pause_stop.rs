//! The actual Commit/Stop worker route, isolated from unrelated continuation code.
//!
//! For these commands `released_execution::discharged` and the startup-abort
//! selector are structurally false: their commands are terminal InstallFence
//! and ActivateVm/startup commands respectively. Keep those predicates intact.
//! The live worker delegates here before its general dispatch. Other command
//! tags return None without reading World or cloning values.
use super::{
    Command, EffectId, Generation, Phase, RestoreSource, SandboxId, SnapshotId,
    SnapshotManifestRef, SnapshotStage, VmId, VmStatus, WorkerDecision, WorkerEvent, WorkerFence,
    WorkerReject, World, contains, current_effect_id, current_plan, effect_awaiting, enqueue,
    late_capture, put_snapshot, put_vm, stoppable_vm_status,
};

pub(super) fn record(world: &World, id: &EffectId) -> Option<Result<WorkerDecision, WorkerReject>> {
    match id.command() {
        Command::CommitSnapshot { .. } | Command::StopVm { .. } => Some(execute(world, id)),
        _ => None,
    }
}

fn execute(world: &World, id: &EffectId) -> Result<WorkerDecision, WorkerReject> {
    let command = id.logical.command.clone();
    if current_effect_id(world, &command) != *id {
        return Err(WorkerReject::StaleAttempt);
    }
    if !contains(&world.inflight, id) {
        return Err(WorkerReject::NotQueued);
    }
    if !effect_awaiting(world.effects.get(id)) {
        return Err(WorkerReject::NotAwaiting);
    }
    if let Some(result) = late_capture::record(world, id) {
        return result;
    }
    match command {
        Command::CommitSnapshot {
            sandbox,
            epoch,
            generation,
            snapshot,
            manifest,
        } => commit_snapshot(world, sandbox, epoch, generation, snapshot, manifest),
        Command::StopVm {
            sandbox,
            epoch,
            generation,
            vm,
        } => stop_vm(world, sandbox, epoch, generation, vm),
        _ => unreachable!("only CommitSnapshot and StopVm enter this route"),
    }
}

fn commit_snapshot(
    world: &World,
    sandbox: SandboxId,
    epoch: u64,
    generation: Generation,
    snapshot: SnapshotId,
    manifest: SnapshotManifestRef,
) -> Result<WorkerDecision, WorkerReject> {
    let Some(stored) = world.snapshots.get(&snapshot) else {
        return Err(WorkerReject::ResourceNotFound);
    };
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(WorkerReject::SandboxNotFound);
    };
    if epoch != record.epoch
        || generation != record.generation
        || manifest != stored.manifest
        || stored.sandbox != sandbox
    {
        return Err(WorkerReject::IdentityMismatch);
    }
    if record.phase != Phase::Pausing {
        return Err(WorkerReject::WrongPhase);
    }
    let Some(vm) = record.current_vm else {
        return Err(WorkerReject::ResourceNotFound);
    };
    if stored.source_generation != record.generation || stored.workspace != record.workspace {
        return Err(WorkerReject::IdentityMismatch);
    }
    if stored.stage != SnapshotStage::LocalSealed {
        return Err(WorkerReject::SnapshotNotReady);
    }
    let (key, plan) = current_plan(world, sandbox, record)?;
    if plan.vm != Some(vm) || plan.target_snapshot != Some(RestoreSource { snapshot, manifest }) {
        return Err(WorkerReject::PlanMismatch);
    }
    let Some(handoff) = crate::transition::current_capture(world, sandbox, record)
        .and_then(crate::capture::CaptureHandoff::observe_bundle)
    else {
        return Err(WorkerReject::ResourceBusy);
    };
    let mut next = world.clone();
    let mut registered = plan.clone();
    registered.capture = Some(handoff);
    next.operation_plans.put(key, registered);
    let mut durable = *stored;
    durable.stage = SnapshotStage::Durable;
    durable.immutable = true;
    put_snapshot(&mut next, snapshot, durable);
    if handoff.may_stop_original() {
        enqueue(
            &mut next,
            Command::StopVm {
                sandbox,
                epoch: record.epoch,
                generation: record.generation,
                vm,
            },
        );
    }
    Ok(WorkerDecision {
        event: WorkerEvent::CommitSnapshot { snapshot },
        next,
    })
}

fn stop_vm(
    world: &World,
    sandbox: SandboxId,
    epoch: u64,
    generation: Generation,
    vm: VmId,
) -> Result<WorkerDecision, WorkerReject> {
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(WorkerReject::SandboxNotFound);
    };
    let Some(stored_vm) = world.vms.get(&vm) else {
        return Err(WorkerReject::ResourceNotFound);
    };
    if epoch != record.epoch || generation != stored_vm.generation || stored_vm.sandbox != sandbox {
        return Err(WorkerReject::IdentityMismatch);
    }
    if !stoppable_vm_status(stored_vm.status) {
        return Err(WorkerReject::WrongPhase);
    }
    if record.phase == Phase::Pausing {
        if record.current_vm != Some(vm) {
            return Err(WorkerReject::IdentityMismatch);
        }
        if record.terminal {
            return Err(WorkerReject::Terminal);
        }
        if stored_vm.status != VmStatus::Frozen {
            return Err(WorkerReject::WrongPhase);
        }
        if world.worker_fences.get(&sandbox)
            != Some(&WorkerFence {
                epoch: record.epoch,
                terminal: false,
            })
        {
            return Err(WorkerReject::FenceNotReady);
        }
        let (_, plan) = current_plan(world, sandbox, record)?;
        if plan.vm != Some(vm)
            || !crate::transition::current_capture(world, sandbox, record)
                .is_some_and(crate::capture::CaptureHandoff::may_stop_original)
        {
            return Err(WorkerReject::SnapshotNotReady);
        }
        let Some(target) = plan.target_snapshot else {
            return Err(WorkerReject::SnapshotNotReady);
        };
        let Some(stored_snapshot) = world.snapshots.get(&target.snapshot) else {
            return Err(WorkerReject::SnapshotNotReady);
        };
        if stored_snapshot.owner != record.owner
            || stored_snapshot.sandbox != sandbox
            || stored_snapshot.source_generation != record.generation
            || stored_snapshot.workspace != record.workspace
            || stored_snapshot.manifest != target.manifest
            || stored_snapshot.stage != SnapshotStage::Durable
            || !stored_snapshot.immutable
        {
            return Err(WorkerReject::SnapshotNotReady);
        }
    } else if record.phase == Phase::Revoking {
        if !record.terminal {
            return Err(WorkerReject::Terminal);
        }
        if world.worker_fences.get(&sandbox)
            != Some(&WorkerFence {
                epoch: record.epoch,
                terminal: true,
            })
        {
            return Err(WorkerReject::FenceNotReady);
        }
    } else {
        return Err(WorkerReject::WrongPhase);
    }
    let mut next = world.clone();
    let mut stopped = *stored_vm;
    stopped.status = VmStatus::Stopped;
    put_vm(&mut next, vm, stopped);
    Ok(WorkerDecision {
        event: WorkerEvent::StopVm { sandbox, vm },
        next,
    })
}
