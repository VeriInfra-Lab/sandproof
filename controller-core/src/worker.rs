mod late_capture;
use crate::model::{
    AppliedFilePolicy, Authority, Command, EffectId, EffectStatus, LogicalEffectId, NetSlot,
    OperationKey, OperationKind, OperationStatus, Phase, RestoreSource, SandboxRecord, Snapshot,
    SnapshotStage, Vm, VmStatus, WorkerFence, World,
};
use crate::transition::{enqueue, operation_key};
use crate::{Generation, NetSlotId, SandboxId, SnapshotId, SnapshotManifestRef, VmId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkerEvent {
    InstallFence {
        sandbox: SandboxId,
    },
    PrepareVm {
        sandbox: SandboxId,
        vm: VmId,
        slot: NetSlotId,
        source: Option<RestoreSource>,
    },
    ConfigureNetwork {
        sandbox: SandboxId,
        slot: NetSlotId,
    },
    ActivateVm {
        sandbox: SandboxId,
        vm: VmId,
    },
    ApplyFilePolicy {
        sandbox: SandboxId,
        generation: Generation,
    },
    FreezeVm {
        sandbox: SandboxId,
        vm: VmId,
    },
    SealSnapshot {
        sandbox: SandboxId,
        snapshot: SnapshotId,
        manifest: SnapshotManifestRef,
    },
    CommitSnapshot {
        snapshot: SnapshotId,
    },
    StopVm {
        sandbox: SandboxId,
        vm: VmId,
    },
    AbortPrepare {
        sandbox: SandboxId,
        vm: VmId,
        slot: NetSlotId,
    },
    CleanupVm {
        sandbox: SandboxId,
        generation: Generation,
        vm: VmId,
    },
    CleanupSlot {
        sandbox: SandboxId,
        generation: Generation,
        slot: NetSlotId,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerReject {
    StaleAttempt,
    NotQueued,
    NotAwaiting,
    SandboxNotFound,
    ResourceNotFound,
    OperationNotFound,
    PlanNotFound,
    WrongPhase,
    Terminal,
    IdentityMismatch,
    PlanMismatch,
    FenceNotReady,
    ResourceBusy,
    ResultAlreadyPresent,
    SourceUnavailable,
    SnapshotNotReady,
    ResourceStillCurrent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerDecision {
    pub event: WorkerEvent,
    pub next: World,
}

fn insert_unique<T: Clone + PartialEq>(values: &mut Vec<T>, value: T) {
    let mut index = 0;
    while index < values.len() {
        if values[index] == value {
            return;
        }
        index += 1;
    }
    let mut next = Vec::new();
    next.push(value);
    index = 0;
    while index < values.len() {
        next.push(values[index].clone());
        index += 1;
    }
    *values = next;
}

fn contains<T: PartialEq>(values: &[T], value: &T) -> bool {
    let mut index = 0;
    while index < values.len() {
        if &values[index] == value {
            return true;
        }
        index += 1;
    }
    false
}

fn current_effect_id(world: &World, command: &Command) -> EffectId {
    let logical = LogicalEffectId {
        command: command.clone(),
    };
    let attempt = match world.logical_effects.get_copy(&logical) {
        Some(record) => record.current_attempt,
        None => 0,
    };
    EffectId { logical, attempt }
}

fn fence_can_advance(old: Option<&WorkerFence>, epoch: u64, terminal: bool) -> bool {
    match old {
        None => true,
        Some(fence) => fence.epoch <= epoch && (!fence.terminal || terminal),
    }
}

fn effect_awaiting(status: Option<&EffectStatus>) -> bool {
    match status {
        Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown) => true,
        Some(
            EffectStatus::Committed
            | EffectStatus::Acknowledged(_)
            | EffectStatus::RetiredByStop(_),
        )
        | None => false,
    }
}

const fn revocation_phase(phase: Phase) -> bool {
    match phase {
        Phase::Pausing | Phase::Revoking => true,
        Phase::Starting | Phase::Running | Phase::Paused | Phase::Destroyed => false,
    }
}

const fn stoppable_vm_status(status: VmStatus) -> bool {
    match status {
        VmStatus::Prepared | VmStatus::Executing | VmStatus::Frozen => true,
        VmStatus::Stopped | VmStatus::Reclaimed => false,
    }
}

// Keep this value-only check separate from the worker's live plan/record
// borrows. It is the same guard at the same short-circuit position.
const fn operation_is_compensating(status: OperationStatus) -> bool {
    matches!(status, OperationStatus::Compensating(_))
}

fn register_vm(world: &mut World, sandbox: SandboxId, vm: VmId) {
    let mut index = world.resource_index(sandbox);
    insert_unique(&mut index.vms, vm);
    world.resource_indexes.put(sandbox, index);
}

fn register_slot(world: &mut World, sandbox: SandboxId, slot: NetSlotId) {
    let mut index = world.resource_index(sandbox);
    insert_unique(&mut index.slots, slot);
    world.resource_indexes.put(sandbox, index);
}

fn register_snapshot(world: &mut World, sandbox: SandboxId, snapshot: SnapshotId) {
    let mut index = world.resource_index(sandbox);
    insert_unique(&mut index.snapshots, snapshot);
    world.resource_indexes.put(sandbox, index);
}

fn put_vm(world: &mut World, id: VmId, value: Vm) {
    register_vm(world, value.sandbox, id);
    world.vms.put(id, value);
}

fn put_slot(world: &mut World, id: NetSlotId, value: NetSlot) {
    register_slot(world, value.sandbox, id);
    world.net_slots.put(id, value);
}

fn put_snapshot(world: &mut World, id: SnapshotId, value: Snapshot) {
    register_snapshot(world, value.sandbox, id);
    world.snapshots.put(id, value);
}

fn usable_snapshot(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    source: RestoreSource,
) -> bool {
    match world.snapshots.get(&source.snapshot) {
        Some(snapshot) => {
            snapshot.owner == record.owner
                && snapshot.sandbox == sandbox
                && snapshot.workspace == record.workspace
                && snapshot.manifest == source.manifest
                && snapshot.stage == SnapshotStage::Durable
                && snapshot.immutable
        }
        None => false,
    }
}

fn indexed_no_executing_vm(world: &World, sandbox: SandboxId) -> bool {
    let index = world.resource_index(sandbox);
    let mut cursor = 0;
    while cursor < index.vms.len() {
        let Some(vm) = world.vms.get(&index.vms[cursor]) else {
            return false;
        };
        if vm.status == VmStatus::Executing {
            return false;
        }
        cursor += 1;
    }
    true
}

fn apply_start_fence(world: &World, sandbox: SandboxId, record: &SandboxRecord) -> World {
    let mut next = world.clone();
    next.worker_fences.put(
        sandbox,
        WorkerFence {
            epoch: record.epoch,
            terminal: false,
        },
    );
    next
}

fn apply_revocation_fence(world: &World, sandbox: SandboxId, record: &SandboxRecord) -> World {
    let mut next = world.clone();
    let mut updated = record.clone();
    updated.effective = Authority::Closed;
    next.sandboxes.put(sandbox, updated);
    next.worker_fences.put(
        sandbox,
        WorkerFence {
            epoch: record.epoch,
            terminal: record.terminal,
        },
    );
    if let Some(slot_id) = record.net_slot
        && let Some(slot) = world.net_slots.get(&slot_id)
    {
        let mut closed = slot.clone();
        closed.active = false;
        put_slot(&mut next, slot_id, closed);
    }
    next
}

pub(crate) fn prepare_resources(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    vm_id: VmId,
    slot_id: NetSlotId,
) -> World {
    let mut next = world.clone();
    put_vm(
        &mut next,
        vm_id,
        Vm {
            owner: record.owner,
            sandbox,
            generation: record.generation,
            status: VmStatus::Prepared,
        },
    );
    put_slot(
        &mut next,
        slot_id,
        NetSlot {
            owner: record.owner,
            sandbox,
            generation: record.generation,
            vm: vm_id,
            policy: record.policy.clone(),
            configured: false,
            active: false,
            reclaimed: false,
        },
    );
    let mut updated = record.clone();
    updated.current_vm = Some(vm_id);
    updated.net_slot = Some(slot_id);
    next.sandboxes.put(sandbox, updated);
    if record.terminal {
        return next;
    }
    enqueue(
        &mut next,
        Command::ConfigureNetwork {
            sandbox,
            epoch: record.epoch,
            generation: record.generation,
            vm: vm_id,
            slot: slot_id,
            policy: record.policy.clone(),
        },
    );
    enqueue(
        &mut next,
        Command::ActivateVm {
            sandbox,
            epoch: record.epoch,
            generation: record.generation,
            vm: vm_id,
        },
    );
    enqueue(
        &mut next,
        Command::ApplyFilePolicy {
            sandbox,
            epoch: record.epoch,
            generation: record.generation,
            workspace: record.workspace,
            policy: record.file_policy,
        },
    );
    next
}

fn current_plan<'a>(
    world: &'a World,
    sandbox: SandboxId,
    record: &SandboxRecord,
) -> Result<(OperationKey, &'a crate::OperationPlan), WorkerReject> {
    let Some(operation) = record.current_operation else {
        return Err(WorkerReject::OperationNotFound);
    };
    let key = operation_key(sandbox, operation.id);
    let Some(plan) = world.operation_plans.get(&key) else {
        return Err(WorkerReject::PlanNotFound);
    };
    Ok((key, plan))
}

/// Execute one exact current worker attempt against durable state.
///
/// # Errors
/// Returns the first failed guard. Rejection leaves `world` unchanged.
/// Capture registration, late-result accounting and stop permission extend
/// the historical worker reducer and require their own correspondence proof.
#[allow(clippy::too_many_lines)]
pub fn handle_worker(world: &World, id: &EffectId) -> Result<WorkerDecision, WorkerReject> {
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

    if crate::terminal::startup_effect_in_current_abort(world, id) {
        return crate::terminal::record_late_startup(world, id);
    }
    if let Some(result) = late_capture::record(world, id) {
        return result;
    }
    match command {
        Command::InstallFence {
            sandbox,
            epoch,
            terminal,
        } => {
            let Some(record) = world.sandboxes.get(&sandbox) else {
                return Err(WorkerReject::SandboxNotFound);
            };
            if epoch != record.epoch || terminal != record.terminal {
                return Err(WorkerReject::IdentityMismatch);
            }
            if !fence_can_advance(world.worker_fences.get(&sandbox), epoch, terminal) {
                return Err(WorkerReject::FenceNotReady);
            }
            let next = if record.phase == Phase::Starting {
                if record.terminal {
                    return Err(WorkerReject::Terminal);
                }
                apply_start_fence(world, sandbox, record)
            } else if revocation_phase(record.phase) {
                if let Some(slot_id) = record.net_slot {
                    let Some(slot) = world.net_slots.get(&slot_id) else {
                        return Err(WorkerReject::ResourceNotFound);
                    };
                    if slot.sandbox != sandbox {
                        return Err(WorkerReject::IdentityMismatch);
                    }
                }
                apply_revocation_fence(world, sandbox, record)
            } else {
                return Err(WorkerReject::WrongPhase);
            };
            Ok(WorkerDecision {
                event: WorkerEvent::InstallFence { sandbox },
                next,
            })
        }
        Command::PrepareVm {
            sandbox,
            epoch,
            generation,
            vm,
            slot,
            source,
        } => {
            let Some(record) = world.sandboxes.get(&sandbox) else {
                return Err(WorkerReject::SandboxNotFound);
            };
            if record.phase != Phase::Starting {
                return Err(WorkerReject::WrongPhase);
            }
            if record.terminal {
                return Err(WorkerReject::Terminal);
            }
            if epoch != record.epoch || generation != record.generation {
                return Err(WorkerReject::IdentityMismatch);
            }
            let (key, plan) = current_plan(world, sandbox, record)?;
            if plan.key != key
                || plan.epoch != record.epoch
                || plan.generation != record.generation
                || !plan.sealed
                || plan.vm != Some(vm)
                || plan.slot != Some(slot)
                || plan.restore_from != source
            {
                return Err(WorkerReject::PlanMismatch);
            }
            if world.worker_fences.get(&sandbox)
                != Some(&WorkerFence {
                    epoch: record.epoch,
                    terminal: false,
                })
            {
                return Err(WorkerReject::FenceNotReady);
            }
            if world.vms.contains_key(&vm) || world.net_slots.contains_key(&slot) {
                return Err(WorkerReject::ResultAlreadyPresent);
            }
            if source != record.restore_from {
                return Err(WorkerReject::SourceUnavailable);
            }
            if let Some(selected) = source
                && (record.durable_snapshot != Some(selected)
                    || !usable_snapshot(world, sandbox, record, selected))
            {
                return Err(WorkerReject::SourceUnavailable);
            }
            Ok(WorkerDecision {
                event: WorkerEvent::PrepareVm {
                    sandbox,
                    vm,
                    slot,
                    source,
                },
                next: prepare_resources(world, sandbox, record, vm, slot),
            })
        }
        Command::ConfigureNetwork {
            sandbox,
            epoch,
            generation,
            vm,
            slot,
            policy,
        } => {
            let Some(record) = world.sandboxes.get(&sandbox) else {
                return Err(WorkerReject::SandboxNotFound);
            };
            if record.phase != Phase::Starting {
                return Err(WorkerReject::WrongPhase);
            }
            if record.terminal {
                return Err(WorkerReject::Terminal);
            }
            if epoch != record.epoch || generation != record.generation || policy != record.policy {
                return Err(WorkerReject::IdentityMismatch);
            }
            let Some(stored) = world.net_slots.get(&slot) else {
                return Err(WorkerReject::ResourceNotFound);
            };
            if record.net_slot != Some(slot)
                || record.current_vm != Some(vm)
                || stored.owner != record.owner
                || stored.sandbox != sandbox
                || stored.generation != record.generation
                || stored.vm != vm
                || stored.policy != record.policy
            {
                return Err(WorkerReject::IdentityMismatch);
            }
            if world.worker_fences.get(&sandbox)
                != Some(&WorkerFence {
                    epoch: record.epoch,
                    terminal: false,
                })
            {
                return Err(WorkerReject::FenceNotReady);
            }
            let mut next = world.clone();
            let mut configured = stored.clone();
            configured.configured = true;
            put_slot(&mut next, slot, configured);
            Ok(WorkerDecision {
                event: WorkerEvent::ConfigureNetwork { sandbox, slot },
                next,
            })
        }
        Command::ApplyFilePolicy {
            sandbox,
            epoch,
            generation,
            workspace,
            policy,
        } => {
            let Some(record) = world.sandboxes.get(&sandbox) else {
                return Err(WorkerReject::SandboxNotFound);
            };
            if record.phase != Phase::Starting {
                return Err(WorkerReject::WrongPhase);
            }
            if record.terminal {
                return Err(WorkerReject::Terminal);
            }
            if epoch != record.epoch
                || generation != record.generation
                || workspace != record.workspace
                || policy != record.file_policy
            {
                return Err(WorkerReject::IdentityMismatch);
            }
            if world.worker_fences.get(&sandbox)
                != Some(&WorkerFence {
                    epoch: record.epoch,
                    terminal: false,
                })
            {
                return Err(WorkerReject::FenceNotReady);
            }
            let mut next = world.clone();
            next.applied_file_policies.put(
                sandbox,
                AppliedFilePolicy {
                    sandbox,
                    generation: record.generation,
                    workspace: record.workspace,
                    policy: record.file_policy,
                },
            );
            Ok(WorkerDecision {
                event: WorkerEvent::ApplyFilePolicy {
                    sandbox,
                    generation: record.generation,
                },
                next,
            })
        }
        Command::ActivateVm {
            sandbox,
            epoch,
            generation,
            vm,
        } => {
            let Some(record) = world.sandboxes.get(&sandbox) else {
                return Err(WorkerReject::SandboxNotFound);
            };
            let continuing =
                crate::capture::continuation::continuation_activation_pending(world, &command);
            if record.phase != Phase::Starting && !continuing {
                return Err(WorkerReject::WrongPhase);
            }
            if continuing
                && !crate::capture::continuation::continuation_resources_ready(world, &command)
            {
                return Err(WorkerReject::ResourceBusy);
            }
            if record.terminal {
                return Err(WorkerReject::Terminal);
            }
            if epoch != record.epoch
                || generation != record.generation
                || record.current_vm != Some(vm)
            {
                return Err(WorkerReject::IdentityMismatch);
            }
            let Some(stored_vm) = world.vms.get(&vm) else {
                return Err(WorkerReject::ResourceNotFound);
            };
            if stored_vm.owner != record.owner
                || stored_vm.sandbox != sandbox
                || stored_vm.generation != record.generation
            {
                return Err(WorkerReject::IdentityMismatch);
            }
            if (!continuing && stored_vm.status != VmStatus::Prepared)
                || (continuing
                    && !matches!(stored_vm.status, VmStatus::Frozen | VmStatus::Executing))
            {
                return Err(WorkerReject::WrongPhase);
            }
            let Some(slot_id) = record.net_slot else {
                return Err(WorkerReject::ResourceNotFound);
            };
            let Some(slot) = world.net_slots.get(&slot_id) else {
                return Err(WorkerReject::ResourceNotFound);
            };
            if slot.vm != vm || !slot.configured {
                return Err(WorkerReject::IdentityMismatch);
            }
            if world.worker_fences.get(&sandbox)
                != Some(&WorkerFence {
                    epoch: record.epoch,
                    terminal: false,
                })
            {
                return Err(WorkerReject::FenceNotReady);
            }
            if !indexed_no_executing_vm(world, sandbox) && !continuing {
                return Err(WorkerReject::ResourceBusy);
            }
            let mut next = world.clone();
            let mut executing = *stored_vm;
            executing.status = VmStatus::Executing;
            put_vm(&mut next, vm, executing);
            Ok(WorkerDecision {
                event: WorkerEvent::ActivateVm { sandbox, vm },
                next,
            })
        }
        Command::FreezeVm {
            sandbox,
            epoch,
            generation,
            vm,
        } => {
            let Some(record) = world.sandboxes.get(&sandbox) else {
                return Err(WorkerReject::SandboxNotFound);
            };
            if record.phase != Phase::Pausing {
                return Err(WorkerReject::WrongPhase);
            }
            if !crate::transition::current_capture(world, sandbox, record)
                .is_some_and(crate::capture::CaptureHandoff::has_capture_work)
            {
                return Err(WorkerReject::ResourceBusy);
            }
            if epoch != record.epoch
                || generation != record.generation
                || record.current_vm != Some(vm)
            {
                return Err(WorkerReject::IdentityMismatch);
            }
            let Some(stored_vm) = world.vms.get(&vm) else {
                return Err(WorkerReject::ResourceNotFound);
            };
            if stored_vm.owner != record.owner
                || stored_vm.sandbox != sandbox
                || stored_vm.generation != record.generation
            {
                return Err(WorkerReject::IdentityMismatch);
            }
            if stored_vm.status != VmStatus::Executing {
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
            let mut next = world.clone();
            let mut frozen = *stored_vm;
            frozen.status = VmStatus::Frozen;
            put_vm(&mut next, vm, frozen);
            Ok(WorkerDecision {
                event: WorkerEvent::FreezeVm { sandbox, vm },
                next,
            })
        }
        Command::SealSnapshot {
            sandbox,
            epoch,
            generation,
            vm,
            workspace,
            snapshot,
            manifest,
        } => {
            let Some(record) = world.sandboxes.get(&sandbox) else {
                return Err(WorkerReject::SandboxNotFound);
            };
            if record.phase != Phase::Pausing {
                return Err(WorkerReject::WrongPhase);
            }
            if epoch != record.epoch
                || generation != record.generation
                || workspace != record.workspace
            {
                return Err(WorkerReject::IdentityMismatch);
            }
            let (key, plan) = current_plan(world, sandbox, record)?;
            if plan.key != key
                || plan.epoch != record.epoch
                || plan.generation != record.generation
                || !plan.sealed
                || plan.vm != Some(vm)
                || plan.target_snapshot != Some(RestoreSource { snapshot, manifest })
            {
                return Err(WorkerReject::PlanMismatch);
            }
            if !plan
                .capture
                .is_some_and(crate::capture::CaptureHandoff::has_capture_work)
            {
                return Err(WorkerReject::ResourceBusy);
            }
            let Some(stored_vm) = world.vms.get(&vm) else {
                return Err(WorkerReject::ResourceNotFound);
            };
            if record.current_vm != Some(vm)
                || stored_vm.owner != record.owner
                || stored_vm.sandbox != sandbox
                || stored_vm.generation != record.generation
            {
                return Err(WorkerReject::IdentityMismatch);
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
            if world.snapshots.contains_key(&snapshot) {
                return Err(WorkerReject::ResultAlreadyPresent);
            }
            let mut next = world.clone();
            put_snapshot(
                &mut next,
                snapshot,
                Snapshot {
                    owner: record.owner,
                    sandbox,
                    source_generation: record.generation,
                    workspace: record.workspace,
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
        Command::CommitSnapshot {
            sandbox,
            epoch,
            generation,
            snapshot,
            manifest,
        } => {
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
            if stored.source_generation != record.generation || stored.workspace != record.workspace
            {
                return Err(WorkerReject::IdentityMismatch);
            }
            if stored.stage != SnapshotStage::LocalSealed {
                return Err(WorkerReject::SnapshotNotReady);
            }
            let (key, plan) = current_plan(world, sandbox, record)?;
            if plan.vm != Some(vm)
                || plan.target_snapshot != Some(RestoreSource { snapshot, manifest })
            {
                return Err(WorkerReject::PlanMismatch);
            }
            let Some(capture) = crate::transition::current_capture(world, sandbox, record)
                .and_then(crate::capture::CaptureHandoff::observe_bundle)
            else {
                return Err(WorkerReject::ResourceBusy);
            };
            let mut next = world.clone();
            let mut registered = plan.clone();
            registered.capture = Some(capture);
            next.operation_plans.put(key, registered);
            let mut durable = *stored;
            durable.stage = SnapshotStage::Durable;
            durable.immutable = true;
            put_snapshot(&mut next, snapshot, durable);
            if capture.may_stop_original() {
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
        Command::StopVm {
            sandbox,
            epoch,
            generation,
            vm,
        } => {
            let Some(record) = world.sandboxes.get(&sandbox) else {
                return Err(WorkerReject::SandboxNotFound);
            };
            let Some(stored_vm) = world.vms.get(&vm) else {
                return Err(WorkerReject::ResourceNotFound);
            };
            if epoch != record.epoch
                || generation != stored_vm.generation
                || stored_vm.sandbox != sandbox
            {
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
        Command::AbortPrepare {
            sandbox,
            epoch,
            generation,
            vm,
            slot,
        } => {
            let record = world
                .sandboxes
                .get(&sandbox)
                .ok_or(WorkerReject::SandboxNotFound)?;
            let (key, plan) = current_plan(world, sandbox, record)?;
            let operation = record
                .current_operation
                .ok_or(WorkerReject::OperationNotFound)?;
            if record.phase != Phase::Revoking
                || !record.terminal
                || operation.kind != OperationKind::Create
                || !operation_is_compensating(operation.status)
            {
                return Err(WorkerReject::WrongPhase);
            }
            if epoch != record.epoch
                || generation != record.generation
                || plan.key != key
                || plan.epoch != epoch
                || plan.generation != generation
                || !plan.sealed
                || plan.vm != Some(vm)
                || plan.slot != Some(slot)
                || !plan.stop_vms.contains(&vm)
                || !plan.close_slots.contains(&slot)
            {
                return Err(WorkerReject::PlanMismatch);
            }
            if world.worker_fences.get(&sandbox)
                != Some(&WorkerFence {
                    epoch,
                    terminal: true,
                })
            {
                return Err(WorkerReject::FenceNotReady);
            }
            if world.vms.contains_key(&vm) || world.net_slots.contains_key(&slot) {
                return Err(WorkerReject::ResultAlreadyPresent);
            }
            let mut next = world.clone();
            put_vm(
                &mut next,
                vm,
                Vm {
                    owner: record.owner,
                    sandbox,
                    generation,
                    status: VmStatus::Reclaimed,
                },
            );
            put_slot(
                &mut next,
                slot,
                NetSlot {
                    owner: record.owner,
                    sandbox,
                    generation,
                    vm,
                    policy: record.policy.clone(),
                    configured: false,
                    active: false,
                    reclaimed: true,
                },
            );
            Ok(WorkerDecision {
                event: WorkerEvent::AbortPrepare { sandbox, vm, slot },
                next,
            })
        }
        Command::CleanupVm {
            sandbox,
            generation,
            vm,
        } => {
            let Some(stored) = world.vms.get(&vm) else {
                return Err(WorkerReject::ResourceNotFound);
            };
            if stored.sandbox != sandbox || stored.generation != generation {
                return Err(WorkerReject::IdentityMismatch);
            }
            if stored.status != VmStatus::Stopped {
                return Err(WorkerReject::WrongPhase);
            }
            if let Some(record) = world.sandboxes.get(&sandbox)
                && record.current_vm == Some(vm)
            {
                return Err(WorkerReject::ResourceStillCurrent);
            }
            let mut next = world.clone();
            let mut reclaimed = *stored;
            reclaimed.status = VmStatus::Reclaimed;
            put_vm(&mut next, vm, reclaimed);
            Ok(WorkerDecision {
                event: WorkerEvent::CleanupVm {
                    sandbox,
                    generation,
                    vm,
                },
                next,
            })
        }
        Command::CleanupSlot {
            sandbox,
            generation,
            slot,
        } => {
            let Some(stored) = world.net_slots.get(&slot) else {
                return Err(WorkerReject::ResourceNotFound);
            };
            if stored.sandbox != sandbox || stored.generation != generation {
                return Err(WorkerReject::IdentityMismatch);
            }
            if stored.active {
                return Err(WorkerReject::ResourceBusy);
            }
            if stored.reclaimed {
                return Err(WorkerReject::ResultAlreadyPresent);
            }
            if let Some(record) = world.sandboxes.get(&sandbox)
                && record.net_slot == Some(slot)
            {
                return Err(WorkerReject::ResourceStillCurrent);
            }
            let mut next = world.clone();
            let mut reclaimed = stored.clone();
            reclaimed.active = false;
            reclaimed.reclaimed = true;
            put_slot(&mut next, slot, reclaimed);
            Ok(WorkerDecision {
                event: WorkerEvent::CleanupSlot {
                    sandbox,
                    generation,
                    slot,
                },
                next,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkerEvent, WorkerReject, handle_worker};
    use crate::effect::{handle_effect_dispatch, handle_effect_receipt};
    use crate::model::{
        Actor, Command, EffectId, EffectOutcome, FilePolicyRef, LogicalEffectId, NetworkPolicy,
        VmStatus, WorkerFence, World,
    };
    use crate::request::{CreateRequest, handle_create};
    use crate::scheduler::{handle_dispatch_prepare, handle_reserve_startup};
    use crate::transition::operation_key;
    use crate::{
        FsId, NetSlotId, OperationId, PolicyDigest, ResultDigest, SandboxId, TenantId, VmId,
    };

    const SID: SandboxId = SandboxId(11);

    #[test]
    fn compensation_guard_matches_original_for_every_status_and_failure() {
        use crate::{FailureKind, OperationStatus};
        let mut statuses = vec![
            OperationStatus::Pending,
            OperationStatus::OutcomeUnknown,
            OperationStatus::Succeeded,
        ];
        for failure in [
            FailureKind::Cancelled,
            FailureKind::DeadlineExceeded,
            FailureKind::ResourceFailure,
            FailureKind::PolicyFailure,
            FailureKind::StorageFailure,
            FailureKind::InternalFailure,
        ] {
            statuses.push(OperationStatus::Compensating(failure));
            statuses.push(OperationStatus::Failed(failure));
        }
        for status in statuses {
            assert_eq!(
                super::operation_is_compensating(status),
                matches!(status, OperationStatus::Compensating(_))
            );
        }
    }
    const OPERATION: OperationId = OperationId(19);

    fn created() -> World {
        handle_create(
            &World::default(),
            CreateRequest {
                actor: Actor::Tenant(TenantId(7)),
                owner: TenantId(7),
                sandbox: SID,
                workspace: FsId(13),
                file_policy: FilePolicyRef {
                    digest: PolicyDigest(17),
                },
                policy: NetworkPolicy::DenyAll,
                operation: OPERATION,
            },
        )
        .expect("create")
        .next
    }

    fn dispatch(world: &World, command: Command) -> (World, EffectId) {
        let logical = LogicalEffectId { command };
        let attempt = world
            .logical_effects
            .get(&logical)
            .map_or(0, |record| record.current_attempt);
        let id = EffectId { logical, attempt };
        let next = handle_effect_dispatch(world, id.clone())
            .expect("dispatch")
            .next;
        (next, id)
    }

    fn execute_and_acknowledge(world: &World, command: Command, result: u64) -> World {
        let (dispatched, id) = dispatch(world, command);
        let executed = handle_worker(&dispatched, &id)
            .expect("worker execution")
            .next;
        handle_effect_receipt(
            &executed,
            id,
            EffectOutcome::Succeeded(ResultDigest(result)),
        )
        .expect("success receipt")
        .next
    }

    #[test]
    fn stale_and_unqueued_attempts_are_rejected_before_command_guards() {
        let world = created();
        let command = Command::InstallFence {
            sandbox: SID,
            epoch: 1,
            terminal: false,
        };
        let stale = EffectId {
            logical: LogicalEffectId {
                command: command.clone(),
            },
            attempt: 1,
        };
        assert_eq!(
            handle_worker(&world, &stale),
            Err(WorkerReject::StaleAttempt)
        );

        let initial = EffectId::initial(command);
        let mut unqueued = world;
        unqueued.inflight.clear();
        assert_eq!(
            handle_worker(&unqueued, &initial),
            Err(WorkerReject::NotQueued)
        );
    }

    #[test]
    fn startup_worker_commands_reach_an_executing_prepared_environment() {
        let world = execute_and_acknowledge(
            &created(),
            Command::InstallFence {
                sandbox: SID,
                epoch: 1,
                terminal: false,
            },
            1,
        );
        assert_eq!(
            world.worker_fences.get(&SID),
            Some(&WorkerFence {
                epoch: 1,
                terminal: false,
            })
        );

        let key = operation_key(SID, OPERATION);
        let reserved = handle_reserve_startup(&world, key)
            .expect("reserve startup")
            .next;
        let plan = reserved.operation_plans.get(&key).expect("plan");
        let vm = plan.vm.expect("planned VM");
        let slot = plan.slot.expect("planned slot");
        let scheduled = handle_dispatch_prepare(&reserved, SID, vm, slot)
            .expect("schedule prepare")
            .next;
        let prepared = execute_and_acknowledge(
            &scheduled,
            Command::PrepareVm {
                sandbox: SID,
                epoch: 1,
                generation: 1,
                vm,
                slot,
                source: None,
            },
            2,
        );
        assert_eq!(
            prepared.vms.get(&vm).map(|value| value.status),
            Some(VmStatus::Prepared)
        );
        assert!(prepared.resource_index(SID).vms.contains(&vm));
        assert!(prepared.resource_index(SID).slots.contains(&slot));

        let configured = execute_and_acknowledge(
            &prepared,
            Command::ConfigureNetwork {
                sandbox: SID,
                epoch: 1,
                generation: 1,
                vm,
                slot,
                policy: NetworkPolicy::DenyAll,
            },
            3,
        );
        let policy_applied = execute_and_acknowledge(
            &configured,
            Command::ApplyFilePolicy {
                sandbox: SID,
                epoch: 1,
                generation: 1,
                workspace: FsId(13),
                policy: FilePolicyRef {
                    digest: PolicyDigest(17),
                },
            },
            4,
        );
        let active = execute_and_acknowledge(
            &policy_applied,
            Command::ActivateVm {
                sandbox: SID,
                epoch: 1,
                generation: 1,
                vm,
            },
            5,
        );
        assert_eq!(
            active.vms.get(&vm).map(|value| value.status),
            Some(VmStatus::Executing)
        );
        assert!(
            active
                .net_slots
                .get(&slot)
                .is_some_and(|value| value.configured)
        );
        assert!(active.applied_file_policies.contains_key(&SID));
    }

    #[test]
    fn activation_distinguishes_identity_mismatch_from_unprepared_vm() {
        let mut world = created();
        let vm = VmId(88);
        let mut record = world.sandboxes.get(&SID).expect("sandbox").clone();
        record.current_vm = Some(vm);
        world.sandboxes.put(SID, record.clone());
        let machine = crate::model::Vm {
            owner: record.owner,
            sandbox: SID,
            generation: record.generation,
            status: VmStatus::Frozen,
        };
        super::put_vm(&mut world, vm, machine);
        let command = Command::ActivateVm {
            sandbox: SID,
            epoch: record.epoch,
            generation: record.generation,
            vm,
        };
        crate::transition::enqueue(&mut world, command.clone());
        let (world, id) = dispatch(&world, command);
        assert_eq!(handle_worker(&world, &id), Err(WorkerReject::WrongPhase));

        // Identity is checked first, even when the VM is also not prepared.
        for mismatched in [
            crate::model::Vm {
                owner: TenantId(99),
                ..machine
            },
            crate::model::Vm {
                sandbox: SandboxId(99),
                ..machine
            },
            crate::model::Vm {
                generation: record.generation + 1,
                ..machine
            },
        ] {
            let mut invalid = world.clone();
            invalid.vms.put(vm, mismatched);
            assert_eq!(
                handle_worker(&invalid, &id),
                Err(WorkerReject::IdentityMismatch)
            );
        }
    }

    #[test]
    fn prepare_rejects_resources_outside_the_sealed_plan() {
        let mut world = created();
        world.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 1,
                terminal: false,
            },
        );
        let command = Command::PrepareVm {
            sandbox: SID,
            epoch: 1,
            generation: 1,
            vm: VmId(88),
            slot: NetSlotId(89),
            source: None,
        };
        crate::transition::enqueue(&mut world, command.clone());
        let (world, id) = dispatch(&world, command);
        assert_eq!(handle_worker(&world, &id), Err(WorkerReject::PlanMismatch));
    }

    #[test]
    fn worker_execution_survives_controller_unavailability() {
        let command = Command::InstallFence {
            sandbox: SID,
            epoch: 1,
            terminal: false,
        };
        let (mut world, id) = dispatch(&created(), command);
        world.controller_up = false;
        assert_eq!(
            handle_worker(&world, &id).map(|decision| decision.event),
            Ok(WorkerEvent::InstallFence { sandbox: SID })
        );
    }
}
