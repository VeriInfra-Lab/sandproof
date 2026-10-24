use crate::effect::effect_succeeded_check;
use crate::model::{
    Command, Event, OperationKey, OperationKind, OperationStatus, Phase, RestoreSource,
    SandboxRecord, SnapshotStage, WorkerFence, World,
};
use crate::request::Decision;
use crate::transition::{
    enqueue, operation_key, reserve_pause_snapshot, reserve_startup_resources,
};
use crate::{
    Generation, ManifestDigest, NetSlotId, SandboxId, SnapshotId, SnapshotManifestRef, VmId,
};

/// Rejections produced by the executable scheduler reducer.
///
/// `CounterExhausted` is the finite-machine refinement boundary for Lean's
/// unbounded natural-number allocation cursors.  It is checked only after all
/// guards present in the Lean handler have accepted the input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SchedulerReject {
    ControllerDown,
    SandboxNotFound,
    OperationUnavailable,
    PlanUnavailable,
    VmNotFound,
    SlotNotFound,
    SnapshotNotFound,
    WrongPhase,
    Terminal,
    PlanMismatch,
    PlanAlreadySealed,
    PlanNotOpen,
    FenceNotInstalled,
    ResourceAlreadyExists,
    RestoreSourceUnavailable,
    PrerequisiteNotSucceeded,
    ResourceMismatch,
    ResourceNotPlanned,
    ResourceNotReady,
    ResourceStillCurrent,
    AlreadyReclaimed,
    CounterExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum SchedulerInput {
    ReserveStartup(OperationKey),
    ReservePauseSnapshot(OperationKey),
    DispatchPrepare(SandboxId, VmId, NetSlotId),
    SealSnapshot(SandboxId, SnapshotId, SnapshotManifestRef),
    CommitSnapshot(SandboxId, SnapshotId),
    StopVm(SandboxId, Generation, VmId),
    AbortPrepare(SandboxId, Generation, VmId, NetSlotId),
    CleanupVm(SandboxId, Generation, VmId),
    CleanupSlot(SandboxId, Generation, NetSlotId),
}

const fn stoppable_vm_status(status: crate::VmStatus) -> bool {
    match status {
        crate::VmStatus::Prepared | crate::VmStatus::Executing | crate::VmStatus::Frozen => true,
        crate::VmStatus::Stopped | crate::VmStatus::Reclaimed => false,
    }
}

/// The only uncertainty permitted to coexist with terminal startup cleanup:
/// an exact `PrepareVm` from the same sealed create plan. Other unknown host
/// actions still block every new dispatch.
#[must_use]
pub fn only_uncertain_prepare_for_abort(world: &World, sandbox: SandboxId) -> bool {
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return false;
    };
    let Some(operation) = record.current_operation else {
        return false;
    };
    if record.phase != Phase::Revoking
        || !record.terminal
        || operation.kind != OperationKind::Create
        || !matches!(operation.status, OperationStatus::Compensating(_))
    {
        return false;
    }
    let Some(plan) = world
        .operation_plans
        .get(&operation_key(sandbox, operation.id))
    else {
        return false;
    };
    let (Some(vm), Some(slot)) = (plan.vm, plan.slot) else {
        return false;
    };
    let expected = Command::PrepareVm {
        sandbox,
        epoch: plan.epoch,
        generation: plan.generation,
        vm,
        slot,
        source: plan.restore_from,
    };
    let mut seen = false;
    let mut valid = true;
    let mut index = 0;
    while index < world.effect_ids().len() && valid {
        let id = &world.effect_ids()[index];
        if id.command().sandbox() == sandbox && crate::effect::outcome_is_unresolved(world, id) {
            seen = true;
            valid = *id.command() == expected;
        }
        index += 1;
    }
    seen && valid
}

fn current_terminal_fence(world: &World, command: &Command) -> bool {
    let Command::InstallFence {
        sandbox,
        epoch,
        terminal: true,
    } = command
    else {
        return false;
    };
    let Some(record) = world.sandboxes.get(sandbox) else {
        return false;
    };
    record.epoch == *epoch
}

#[must_use]
pub fn dispatch_allowed_during_abort(world: &World, command: &Command) -> bool {
    let sandbox = command.sandbox();
    if crate::capture::drain::terminal_capture_drain_allowed(world, command) {
        return true;
    }
    if crate::terminal::only_uncertain_activation_for_abort(world, sandbox) {
        let terminal_fence = current_terminal_fence(world, command);
        let mut index = 0;
        let mut found = false;
        while index < world.effect_ids().len() && !found {
            let id = &world.effect_ids()[index];
            found = crate::terminal::activation_in_current_abort(world, id)
                && (crate::terminal::activation_stop_current(world, id).as_ref() == Some(command)
                    || terminal_fence);
            index += 1;
        }
        return found;
    }
    if !only_uncertain_prepare_for_abort(world, sandbox) {
        return false;
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return false;
    };
    match command {
        Command::InstallFence {
            epoch,
            terminal: true,
            ..
        } => *epoch == record.epoch,
        Command::AbortPrepare {
            epoch,
            generation,
            vm,
            slot,
            ..
        } => {
            let Some(operation) = record.current_operation else {
                return false;
            };
            let Some(plan) = world
                .operation_plans
                .get(&operation_key(sandbox, operation.id))
            else {
                return false;
            };
            *epoch == plan.epoch
                && *generation == plan.generation
                && plan.vm == Some(*vm)
                && plan.slot == Some(*slot)
        }
        _ => false,
    }
}

/// Rust implementation of Lean's `Controller.handleReserveStartup`.
///
/// # Errors
///
/// Returns the first rejection from Lean's ordered guards.  Once all Lean
/// guards pass, finite `u64` cursor exhaustion is reported without mutation.
pub fn handle_reserve_startup(
    world: &World,
    key: OperationKey,
) -> Result<Decision, SchedulerReject> {
    if !world.controller_up {
        return Err(SchedulerReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&key.sandbox) else {
        return Err(SchedulerReject::SandboxNotFound);
    };
    if record.phase != Phase::Starting {
        return Err(SchedulerReject::WrongPhase);
    }
    if record.terminal {
        return Err(SchedulerReject::Terminal);
    }
    let Some(operation) = record.current_operation else {
        return Err(SchedulerReject::OperationUnavailable);
    };
    if operation.id != key.operation {
        return Err(SchedulerReject::PlanMismatch);
    }
    let Some(plan) = world.operation_plans.get(&key) else {
        return Err(SchedulerReject::PlanUnavailable);
    };
    if plan.key != key {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.epoch != record.epoch {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.generation != record.generation {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.sealed {
        return Err(SchedulerReject::PlanAlreadySealed);
    }
    if plan.vm.is_some() {
        return Err(SchedulerReject::PlanNotOpen);
    }
    if plan.slot.is_some() {
        return Err(SchedulerReject::PlanNotOpen);
    }
    if plan.restore_from != record.restore_from {
        return Err(SchedulerReject::PlanMismatch);
    }

    let vm = VmId(world.next_vm_id);
    let slot = NetSlotId(world.next_slot_id);
    if world.vms.contains_key(&vm) {
        return Err(SchedulerReject::ResourceAlreadyExists);
    }
    if world.net_slots.contains_key(&slot) {
        return Err(SchedulerReject::ResourceAlreadyExists);
    }

    // Lean's guards, including both freshness checks, deliberately precede
    // this finite-representation check.
    let Some(next) = reserve_startup_resources(world, key, plan) else {
        return Err(SchedulerReject::CounterExhausted);
    };
    Ok(Decision {
        event: Event::ReserveStartupResources { key, vm, slot },
        next,
    })
}

/// Reserve a pause snapshot only for the current eligible capture plan.
/// Capture eligibility extends the historical Lean correspondence.
///
/// # Errors
///
/// Returns the first rejection from Lean's ordered guards.  Once all Lean
/// guards pass, finite `u64` cursor exhaustion is reported without mutation.
pub fn handle_reserve_pause_snapshot(
    world: &World,
    key: OperationKey,
) -> Result<Decision, SchedulerReject> {
    if !world.controller_up {
        return Err(SchedulerReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&key.sandbox) else {
        return Err(SchedulerReject::SandboxNotFound);
    };
    if record.phase != Phase::Pausing {
        return Err(SchedulerReject::WrongPhase);
    }
    if record.terminal {
        return Err(SchedulerReject::Terminal);
    }
    let Some(operation) = record.current_operation else {
        return Err(SchedulerReject::OperationUnavailable);
    };
    if operation.id != key.operation {
        return Err(SchedulerReject::PlanMismatch);
    }
    let Some(plan) = world.operation_plans.get(&key) else {
        return Err(SchedulerReject::PlanUnavailable);
    };
    if plan.key != key {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.epoch != record.epoch {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.generation != record.generation {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.sealed {
        return Err(SchedulerReject::PlanAlreadySealed);
    }
    if plan.vm.is_some() {
        return Err(SchedulerReject::PlanNotOpen);
    }
    if plan.target_snapshot.is_some() {
        return Err(SchedulerReject::PlanNotOpen);
    }
    if !plan
        .capture
        .is_some_and(crate::capture::CaptureHandoff::accepts_registration)
    {
        return Err(SchedulerReject::PlanNotOpen);
    }
    let Some(vm) = record.current_vm else {
        return Err(SchedulerReject::VmNotFound);
    };

    let snapshot = SnapshotId(world.next_snapshot_id);
    let manifest = SnapshotManifestRef {
        digest: ManifestDigest(world.next_manifest_digest),
    };
    if world.snapshots.contains_key(&snapshot) {
        return Err(SchedulerReject::ResourceAlreadyExists);
    }

    // The snapshot freshness guard is part of Lean; only then do we refine
    // the unbounded cursors to finite machine integers.
    let Some(next) = reserve_pause_snapshot(world, key, plan, vm) else {
        return Err(SchedulerReject::CounterExhausted);
    };
    Ok(Decision {
        event: Event::ReservePauseSnapshot {
            key,
            snapshot,
            manifest,
        },
        next,
    })
}

fn usable_snapshot_check(
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

fn restore_source_ready_check(world: &World, sandbox: SandboxId, record: &SandboxRecord) -> bool {
    match record.restore_from {
        None => true,
        Some(source) => {
            record.durable_snapshot == Some(source)
                && usable_snapshot_check(world, sandbox, record, source)
        }
    }
}

/// Rust implementation of Lean's `Controller.handleDispatchPrepare`.
///
/// # Errors
///
/// Returns the first rejection from Lean's ordered plan, fence, resource, and
/// restore-source checks.  An accepted decision only journals the prepare
/// command; worker-side resource creation remains a later protocol step.
pub fn handle_dispatch_prepare(
    world: &World,
    sandbox: SandboxId,
    vm: VmId,
    slot: NetSlotId,
) -> Result<Decision, SchedulerReject> {
    if !world.controller_up {
        return Err(SchedulerReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(SchedulerReject::SandboxNotFound);
    };
    if record.phase != Phase::Starting {
        return Err(SchedulerReject::WrongPhase);
    }
    if record.terminal {
        return Err(SchedulerReject::Terminal);
    }
    let Some(operation) = record.current_operation else {
        return Err(SchedulerReject::OperationUnavailable);
    };
    let key = operation_key(sandbox, operation.id);
    let Some(plan) = world.operation_plans.get(&key) else {
        return Err(SchedulerReject::PlanUnavailable);
    };
    if plan.key != key {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.epoch != record.epoch {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.generation != record.generation {
        return Err(SchedulerReject::PlanMismatch);
    }
    if !plan.sealed {
        return Err(SchedulerReject::PlanNotOpen);
    }
    if plan.vm != Some(vm) {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.slot != Some(slot) {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.restore_from != record.restore_from {
        return Err(SchedulerReject::PlanMismatch);
    }
    if world.worker_fences.get(&sandbox)
        != Some(&WorkerFence {
            epoch: record.epoch,
            terminal: false,
        })
    {
        return Err(SchedulerReject::FenceNotInstalled);
    }
    if world.vms.contains_key(&vm) {
        return Err(SchedulerReject::ResourceAlreadyExists);
    }
    if world.net_slots.contains_key(&slot) {
        return Err(SchedulerReject::ResourceAlreadyExists);
    }
    if !restore_source_ready_check(world, sandbox, record) {
        return Err(SchedulerReject::RestoreSourceUnavailable);
    }

    let source = record.restore_from;
    let command = Command::PrepareVm {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        vm,
        slot,
        source,
    };
    let mut next = world.clone();
    enqueue(&mut next, command);
    Ok(Decision {
        event: Event::DispatchPrepare {
            sandbox,
            vm,
            slot,
            source,
        },
        next,
    })
}

/// Schedule sealing for the current eligible capture plan. The added capture
/// guard is outside the historical Lean correspondence.
///
/// # Errors
///
/// Returns the first rejection from Lean's ordered plan, VM, fence, freeze
/// receipt, and snapshot-freshness checks.
pub fn handle_schedule_seal_snapshot(
    world: &World,
    sandbox: SandboxId,
    snapshot: SnapshotId,
    manifest: SnapshotManifestRef,
) -> Result<Decision, SchedulerReject> {
    if !world.controller_up {
        return Err(SchedulerReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(SchedulerReject::SandboxNotFound);
    };
    if record.phase != Phase::Pausing {
        return Err(SchedulerReject::WrongPhase);
    }
    let Some(operation) = record.current_operation else {
        return Err(SchedulerReject::OperationUnavailable);
    };
    let key = operation_key(sandbox, operation.id);
    let Some(plan) = world.operation_plans.get(&key) else {
        return Err(SchedulerReject::PlanUnavailable);
    };
    if plan.key != key {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.epoch != record.epoch {
        return Err(SchedulerReject::PlanMismatch);
    }
    if plan.generation != record.generation {
        return Err(SchedulerReject::PlanMismatch);
    }
    if !plan.sealed {
        return Err(SchedulerReject::PlanNotOpen);
    }
    if plan.target_snapshot != Some(RestoreSource { snapshot, manifest }) {
        return Err(SchedulerReject::PlanMismatch);
    }
    if !plan
        .capture
        .is_some_and(crate::capture::CaptureHandoff::accepts_registration)
    {
        return Err(SchedulerReject::PlanNotOpen);
    }
    let Some(vm_id) = record.current_vm else {
        return Err(SchedulerReject::VmNotFound);
    };
    if plan.vm != Some(vm_id) {
        return Err(SchedulerReject::PlanMismatch);
    }
    let Some(vm) = world.vms.get(&vm_id) else {
        return Err(SchedulerReject::VmNotFound);
    };
    if vm.owner != record.owner || vm.sandbox != sandbox || vm.generation != record.generation {
        return Err(SchedulerReject::ResourceMismatch);
    }
    if vm.status != crate::VmStatus::Frozen {
        return Err(SchedulerReject::ResourceNotReady);
    }
    if world.worker_fences.get(&sandbox)
        != Some(&WorkerFence {
            epoch: record.epoch,
            terminal: false,
        })
    {
        return Err(SchedulerReject::FenceNotInstalled);
    }
    let freeze = Command::FreezeVm {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        vm: vm_id,
    };
    if !effect_succeeded_check(world, &freeze) {
        return Err(SchedulerReject::PrerequisiteNotSucceeded);
    }
    if world.snapshots.contains_key(&snapshot) {
        return Err(SchedulerReject::ResourceAlreadyExists);
    }

    let command = Command::SealSnapshot {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        vm: vm_id,
        workspace: record.workspace,
        snapshot,
        manifest,
    };
    let mut next = world.clone();
    enqueue(&mut next, command);
    Ok(Decision {
        event: Event::ScheduleSealSnapshot {
            sandbox,
            snapshot,
            manifest,
        },
        next,
    })
}

/// Schedule registration for the current eligible capture plan. The added
/// capture guard is outside the historical Lean correspondence.
///
/// # Errors
///
/// Returns the first rejection from Lean's snapshot-first lookup order,
/// identity and stage checks, and seal-success prerequisite.
pub fn handle_schedule_commit_snapshot(
    world: &World,
    sandbox: SandboxId,
    snapshot_id: SnapshotId,
) -> Result<Decision, SchedulerReject> {
    if !world.controller_up {
        return Err(SchedulerReject::ControllerDown);
    }
    let Some(snapshot) = world.snapshots.get(&snapshot_id) else {
        return Err(SchedulerReject::SnapshotNotFound);
    };
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(SchedulerReject::SandboxNotFound);
    };
    if snapshot.sandbox != sandbox {
        return Err(SchedulerReject::ResourceMismatch);
    }
    if record.phase != Phase::Pausing {
        return Err(SchedulerReject::WrongPhase);
    }
    if !crate::transition::current_capture(world, sandbox, record)
        .is_some_and(crate::capture::CaptureHandoff::accepts_registration)
    {
        return Err(SchedulerReject::PlanNotOpen);
    }
    let Some(vm) = record.current_vm else {
        return Err(SchedulerReject::VmNotFound);
    };
    if snapshot.source_generation != record.generation || snapshot.workspace != record.workspace {
        return Err(SchedulerReject::ResourceMismatch);
    }
    if snapshot.stage != SnapshotStage::LocalSealed {
        return Err(SchedulerReject::ResourceNotReady);
    }
    let seal = Command::SealSnapshot {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        vm,
        workspace: record.workspace,
        snapshot: snapshot_id,
        manifest: snapshot.manifest,
    };
    if !effect_succeeded_check(world, &seal) {
        return Err(SchedulerReject::PrerequisiteNotSucceeded);
    }

    let command = Command::CommitSnapshot {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        snapshot: snapshot_id,
        manifest: snapshot.manifest,
    };
    let mut next = world.clone();
    enqueue(&mut next, command);
    Ok(Decision {
        event: Event::ScheduleCommitSnapshot {
            sandbox,
            snapshot: snapshot_id,
        },
        next,
    })
}

/// Rust implementation of Lean's `Controller.handleScheduleStopVm`.
///
/// # Errors
///
/// Returns the first rejection from Lean's revocation, terminal-fence,
/// current fence-receipt, VM identity, and stoppable-state checks.
pub fn handle_schedule_stop_vm(
    world: &World,
    sandbox: SandboxId,
    generation: Generation,
    vm_id: VmId,
) -> Result<Decision, SchedulerReject> {
    if !world.controller_up {
        return Err(SchedulerReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(SchedulerReject::SandboxNotFound);
    };
    if record.phase != Phase::Revoking {
        return Err(SchedulerReject::WrongPhase);
    }
    if !record.terminal {
        return Err(SchedulerReject::Terminal);
    }
    let Some(operation) = record.current_operation else {
        return Err(SchedulerReject::OperationUnavailable);
    };
    let key = operation_key(sandbox, operation.id);
    let Some(plan) = world.operation_plans.get(&key) else {
        return Err(SchedulerReject::PlanUnavailable);
    };
    if plan.key != key || plan.epoch != record.epoch || plan.generation != record.generation {
        return Err(SchedulerReject::PlanMismatch);
    }
    if !plan.stop_vms.contains(&vm_id) {
        return Err(SchedulerReject::ResourceNotPlanned);
    }
    if world.worker_fences.get(&sandbox)
        != Some(&WorkerFence {
            epoch: record.epoch,
            terminal: true,
        })
    {
        return Err(SchedulerReject::FenceNotInstalled);
    }
    let fence = Command::InstallFence {
        sandbox,
        epoch: record.epoch,
        terminal: true,
    };
    if !effect_succeeded_check(world, &fence) {
        return Err(SchedulerReject::PrerequisiteNotSucceeded);
    }
    let Some(vm) = world.vms.get(&vm_id) else {
        return Err(SchedulerReject::VmNotFound);
    };
    if vm.sandbox != sandbox || vm.generation != generation {
        return Err(SchedulerReject::ResourceMismatch);
    }
    if !stoppable_vm_status(vm.status) {
        return Err(SchedulerReject::ResourceNotReady);
    }

    let command = Command::StopVm {
        sandbox,
        epoch: record.epoch,
        generation,
        vm: vm_id,
    };
    let mut next = world.clone();
    enqueue(&mut next, command);
    Ok(Decision {
        event: Event::ScheduleStopVm {
            sandbox,
            generation,
            vm: vm_id,
        },
        next,
    })
}

/// Schedule exact reclamation of a reserved startup resource whose `PrepareVm`
/// never produced a durable VM record. The terminal fence must already have
/// been physically acknowledged. No other generation or operation qualifies.
///
/// # Errors
/// Rejects an unplanned, already registered, or unfenced startup resource.
pub fn handle_schedule_abort_prepare(
    world: &World,
    sandbox: SandboxId,
    generation: Generation,
    vm: VmId,
    slot: NetSlotId,
) -> Result<Decision, SchedulerReject> {
    if !world.controller_up {
        return Err(SchedulerReject::ControllerDown);
    }
    let record = world
        .sandboxes
        .get(&sandbox)
        .ok_or(SchedulerReject::SandboxNotFound)?;
    if record.phase != Phase::Revoking || !record.terminal {
        return Err(SchedulerReject::WrongPhase);
    }
    let operation = record
        .current_operation
        .ok_or(SchedulerReject::OperationUnavailable)?;
    if operation.kind != OperationKind::Create
        || !matches!(operation.status, OperationStatus::Compensating(_))
    {
        return Err(SchedulerReject::WrongPhase);
    }
    let key = operation_key(sandbox, operation.id);
    let plan = world
        .operation_plans
        .get(&key)
        .ok_or(SchedulerReject::PlanUnavailable)?;
    if plan.key != key
        || plan.epoch != record.epoch
        || plan.generation != generation
        || record.generation != generation
        || !plan.sealed
        || plan.vm != Some(vm)
        || plan.slot != Some(slot)
        || !plan.stop_vms.contains(&vm)
        || !plan.close_slots.contains(&slot)
    {
        return Err(SchedulerReject::PlanMismatch);
    }
    if world.vms.contains_key(&vm) || world.net_slots.contains_key(&slot) {
        return Err(SchedulerReject::ResourceAlreadyExists);
    }
    if world.worker_fences.get(&sandbox)
        != Some(&WorkerFence {
            epoch: record.epoch,
            terminal: true,
        })
        || !effect_succeeded_check(
            world,
            &Command::InstallFence {
                sandbox,
                epoch: record.epoch,
                terminal: true,
            },
        )
    {
        return Err(SchedulerReject::FenceNotInstalled);
    }
    let mut next = world.clone();
    enqueue(
        &mut next,
        Command::AbortPrepare {
            sandbox,
            epoch: record.epoch,
            generation,
            vm,
            slot,
        },
    );
    Ok(Decision {
        event: Event::ScheduleAbortPrepare {
            sandbox,
            generation,
            vm,
            slot,
        },
        next,
    })
}

fn not_current_vm_check(world: &World, sandbox: SandboxId, vm: VmId) -> bool {
    match world.sandboxes.get(&sandbox) {
        Some(record) => record.current_vm != Some(vm),
        None => true,
    }
}

/// Rust implementation of Lean's `Controller.handleScheduleCleanupVm`.
///
/// # Errors
///
/// Returns the first rejection from Lean's current-plan membership, VM
/// identity, stopped-state, and no-longer-current checks.
pub fn handle_schedule_cleanup_vm(
    world: &World,
    sandbox: SandboxId,
    generation: Generation,
    vm_id: VmId,
) -> Result<Decision, SchedulerReject> {
    if !world.controller_up {
        return Err(SchedulerReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(SchedulerReject::SandboxNotFound);
    };
    let Some(operation) = record.current_operation else {
        return Err(SchedulerReject::OperationUnavailable);
    };
    let key = operation_key(sandbox, operation.id);
    let Some(plan) = world.operation_plans.get(&key) else {
        return Err(SchedulerReject::PlanUnavailable);
    };
    if plan.key != key || plan.epoch != record.epoch || plan.generation != record.generation {
        return Err(SchedulerReject::PlanMismatch);
    }
    if !plan.stop_vms.contains(&vm_id) {
        return Err(SchedulerReject::ResourceNotPlanned);
    }
    let Some(vm) = world.vms.get(&vm_id) else {
        return Err(SchedulerReject::VmNotFound);
    };
    if vm.sandbox != sandbox || vm.generation != generation {
        return Err(SchedulerReject::ResourceMismatch);
    }
    if vm.status != crate::VmStatus::Stopped {
        return Err(SchedulerReject::ResourceNotReady);
    }
    if !not_current_vm_check(world, sandbox, vm_id) {
        return Err(SchedulerReject::ResourceStillCurrent);
    }

    let command = Command::CleanupVm {
        sandbox,
        generation,
        vm: vm_id,
    };
    let mut next = world.clone();
    enqueue(&mut next, command);
    Ok(Decision {
        event: Event::ScheduleCleanupVm {
            sandbox,
            generation,
            vm: vm_id,
        },
        next,
    })
}

fn not_current_slot_check(world: &World, sandbox: SandboxId, slot: NetSlotId) -> bool {
    match world.sandboxes.get(&sandbox) {
        Some(record) => record.net_slot != Some(slot),
        None => true,
    }
}

/// Rust implementation of Lean's `Controller.handleScheduleCleanupSlot`.
///
/// # Errors
///
/// Returns the first rejection from Lean's current-plan membership, slot
/// identity, inactive, unreclaimed, and no-longer-current checks.
pub fn handle_schedule_cleanup_slot(
    world: &World,
    sandbox: SandboxId,
    generation: Generation,
    slot_id: NetSlotId,
) -> Result<Decision, SchedulerReject> {
    if !world.controller_up {
        return Err(SchedulerReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(SchedulerReject::SandboxNotFound);
    };
    let Some(operation) = record.current_operation else {
        return Err(SchedulerReject::OperationUnavailable);
    };
    let key = operation_key(sandbox, operation.id);
    let Some(plan) = world.operation_plans.get(&key) else {
        return Err(SchedulerReject::PlanUnavailable);
    };
    if plan.key != key || plan.epoch != record.epoch || plan.generation != record.generation {
        return Err(SchedulerReject::PlanMismatch);
    }
    if !plan.close_slots.contains(&slot_id) {
        return Err(SchedulerReject::ResourceNotPlanned);
    }
    let Some(slot) = world.net_slots.get(&slot_id) else {
        return Err(SchedulerReject::SlotNotFound);
    };
    if slot.sandbox != sandbox || slot.generation != generation {
        return Err(SchedulerReject::ResourceMismatch);
    }
    if slot.active {
        return Err(SchedulerReject::ResourceNotReady);
    }
    if slot.reclaimed {
        return Err(SchedulerReject::AlreadyReclaimed);
    }
    if !not_current_slot_check(world, sandbox, slot_id) {
        return Err(SchedulerReject::ResourceStillCurrent);
    }

    let command = Command::CleanupSlot {
        sandbox,
        generation,
        slot: slot_id,
    };
    let mut next = world.clone();
    enqueue(&mut next, command);
    Ok(Decision {
        event: Event::ScheduleCleanupSlot {
            sandbox,
            generation,
            slot: slot_id,
        },
        next,
    })
}

/// Execute one scheduler request through the same eight-way split as Lean's
/// `handleScheduler`.
///
/// # Errors
/// Returns the selected scheduler handler's first failed guard.
pub fn handle_scheduler(world: &World, input: SchedulerInput) -> Result<Decision, SchedulerReject> {
    match input {
        SchedulerInput::ReserveStartup(key) => handle_reserve_startup(world, key),
        SchedulerInput::ReservePauseSnapshot(key) => handle_reserve_pause_snapshot(world, key),
        SchedulerInput::DispatchPrepare(sandbox, vm, slot) => {
            handle_dispatch_prepare(world, sandbox, vm, slot)
        }
        SchedulerInput::SealSnapshot(sandbox, snapshot, manifest) => {
            handle_schedule_seal_snapshot(world, sandbox, snapshot, manifest)
        }
        SchedulerInput::CommitSnapshot(sandbox, snapshot) => {
            handle_schedule_commit_snapshot(world, sandbox, snapshot)
        }
        SchedulerInput::StopVm(sandbox, generation, vm) => {
            handle_schedule_stop_vm(world, sandbox, generation, vm)
        }
        SchedulerInput::AbortPrepare(sandbox, generation, vm, slot) => {
            handle_schedule_abort_prepare(world, sandbox, generation, vm, slot)
        }
        SchedulerInput::CleanupVm(sandbox, generation, vm) => {
            handle_schedule_cleanup_vm(world, sandbox, generation, vm)
        }
        SchedulerInput::CleanupSlot(sandbox, generation, slot) => {
            handle_schedule_cleanup_slot(world, sandbox, generation, slot)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SchedulerReject, handle_dispatch_prepare, handle_reserve_pause_snapshot,
        handle_reserve_startup, handle_schedule_cleanup_slot, handle_schedule_cleanup_vm,
        handle_schedule_commit_snapshot, handle_schedule_seal_snapshot, handle_schedule_stop_vm,
    };
    use crate::model::{
        Authority, Command, EffectOrigin, EffectStatus, Event, FilePolicyRef, LogicalEffectId,
        NetworkPolicy, Operation, OperationKind, OperationPlan, OperationStatus, Phase,
        RestoreSource, SandboxRecord, Snapshot, SnapshotStage, WorkerFence, WorkspaceApiAccess,
        World,
    };
    use crate::transition::operation_key;
    use crate::{
        FsId, Generation, ManifestDigest, NetSlot, NetSlotId, OperationId, PolicyDigest,
        ResultDigest, SandboxId, SnapshotId, SnapshotManifestRef, TenantId, Vm, VmId, VmStatus,
    };

    const OWNER: TenantId = TenantId(7);
    const SID: SandboxId = SandboxId(11);
    const VM: VmId = VmId(23);
    const OPERATION: OperationId = OperationId(29);

    fn record(phase: Phase, kind: OperationKind) -> SandboxRecord {
        SandboxRecord {
            owner: OWNER,
            phase,
            terminal: false,
            epoch: 5,
            generation: 3,
            current_vm: Some(VM),
            workspace: FsId(13),
            workspace_access: WorkspaceApiAccess::ManagedReadWrite,
            file_policy: FilePolicyRef {
                digest: PolicyDigest(17),
            },
            durable_snapshot: None,
            restore_from: None,
            net_slot: None,
            policy: NetworkPolicy::DenyAll,
            desired: Authority::Closed,
            effective: Authority::Closed,
            current_operation: Some(Operation {
                id: OPERATION,
                epoch: 5,
                kind,
                status: OperationStatus::Pending,
            }),
        }
    }

    fn plan() -> OperationPlan {
        OperationPlan {
            key: operation_key(SID, OPERATION),
            epoch: 5,
            generation: 3,
            vm: None,
            slot: None,
            restore_from: None,
            target_snapshot: None,
            capture: None,
            stop_vms: vec![VmId(8)],
            close_slots: vec![NetSlotId(9)],
            retry_limit: 3,
            sealed: false,
        }
    }

    fn startup_world() -> World {
        let mut world = World {
            next_vm_id: 21,
            next_slot_id: 34,
            ..World::default()
        };
        let key = operation_key(SID, OPERATION);
        world
            .sandboxes
            .put(SID, record(Phase::Starting, OperationKind::Create));
        world.operation_plans.put(key, plan());
        world
    }

    fn pause_world() -> World {
        let mut world = World {
            next_snapshot_id: 55,
            next_manifest_digest: 89,
            ..World::default()
        };
        let key = operation_key(SID, OPERATION);
        world
            .sandboxes
            .put(SID, record(Phase::Pausing, OperationKind::Pause));
        let mut capture_plan = plan();
        capture_plan.capture = Some(crate::capture::CaptureHandoff::accepted());
        world.operation_plans.put(key, capture_plan);
        world
    }

    fn dispatch_world(source: Option<RestoreSource>) -> World {
        let mut world = startup_world();
        let key = operation_key(SID, OPERATION);
        let mut sandbox = world.sandboxes.get(&SID).expect("sandbox").clone();
        sandbox.restore_from = source;
        sandbox.durable_snapshot = source;
        world.sandboxes.put(SID, sandbox);
        let mut reserved = world.operation_plans.get(&key).expect("plan").clone();
        reserved.vm = Some(VmId(21));
        reserved.slot = Some(NetSlotId(34));
        reserved.restore_from = source;
        reserved.sealed = true;
        world.operation_plans.put(key, reserved);
        world.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 5,
                terminal: false,
            },
        );
        world
    }

    fn acknowledge_success(world: &mut World, command: Command) {
        let id = crate::EffectId::initial(command);
        world.effects.put(
            id,
            EffectStatus::Acknowledged(crate::EffectOutcome::Succeeded(ResultDigest(97))),
        );
    }

    fn seal_scheduler_world() -> (World, SnapshotManifestRef) {
        let mut world = pause_world();
        let key = operation_key(SID, OPERATION);
        let manifest = SnapshotManifestRef {
            digest: ManifestDigest(89),
        };
        let mut plan = world.operation_plans.get(&key).expect("plan").clone();
        plan.vm = Some(VM);
        plan.target_snapshot = Some(RestoreSource {
            snapshot: SnapshotId(55),
            manifest,
        });
        plan.sealed = true;
        world.operation_plans.put(key, plan);
        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                status: VmStatus::Frozen,
            },
        );
        world.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 5,
                terminal: false,
            },
        );
        acknowledge_success(
            &mut world,
            Command::FreezeVm {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                vm: VM,
            },
        );
        (world, manifest)
    }

    fn commit_scheduler_world() -> (World, SnapshotManifestRef) {
        let (mut world, manifest) = seal_scheduler_world();
        world.snapshots.put(
            SnapshotId(55),
            Snapshot {
                owner: OWNER,
                sandbox: SID,
                source_generation: 3,
                workspace: FsId(13),
                manifest,
                stage: SnapshotStage::LocalSealed,
                immutable: false,
            },
        );
        acknowledge_success(
            &mut world,
            Command::SealSnapshot {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                vm: VM,
                workspace: FsId(13),
                snapshot: SnapshotId(55),
                manifest,
            },
        );
        (world, manifest)
    }

    fn stop_scheduler_world() -> World {
        let mut sandbox = record(Phase::Revoking, OperationKind::Kill);
        sandbox.terminal = true;
        let mut world = World::default();
        world.sandboxes.put(SID, sandbox);
        let mut stop_plan = plan();
        stop_plan.stop_vms.insert(0, VM);
        stop_plan.sealed = true;
        world
            .operation_plans
            .put(operation_key(SID, OPERATION), stop_plan);
        world.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 5,
                terminal: true,
            },
        );
        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                status: VmStatus::Prepared,
            },
        );
        acknowledge_success(
            &mut world,
            Command::InstallFence {
                sandbox: SID,
                epoch: 5,
                terminal: true,
            },
        );
        world
    }

    fn cleanup_vm_world() -> World {
        let mut world = World::default();
        let mut sandbox = record(Phase::Revoking, OperationKind::Kill);
        sandbox.current_vm = None;
        world.sandboxes.put(SID, sandbox);
        let mut cleanup_plan = plan();
        cleanup_plan.stop_vms.insert(0, VM);
        cleanup_plan.sealed = true;
        world
            .operation_plans
            .put(operation_key(SID, OPERATION), cleanup_plan);
        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                status: VmStatus::Stopped,
            },
        );
        world
    }

    fn cleanup_slot_world() -> World {
        let mut world = World::default();
        let mut sandbox = record(Phase::Revoking, OperationKind::Kill);
        sandbox.current_vm = None;
        sandbox.net_slot = None;
        world.sandboxes.put(SID, sandbox);
        let mut cleanup_plan = plan();
        cleanup_plan.close_slots.insert(0, NetSlotId(34));
        cleanup_plan.sealed = true;
        world
            .operation_plans
            .put(operation_key(SID, OPERATION), cleanup_plan);
        world.net_slots.put(
            NetSlotId(34),
            NetSlot {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                vm: VM,
                policy: NetworkPolicy::DenyAll,
                configured: true,
                active: false,
                reclaimed: false,
            },
        );
        world
    }

    #[test]
    fn startup_accepts_and_atomically_seals_the_reserved_plan() {
        let world = startup_world();
        let before = world.clone();
        let key = operation_key(SID, OPERATION);
        let decision = handle_reserve_startup(&world, key).expect("valid reservation");

        assert_eq!(
            decision.event,
            Event::ReserveStartupResources {
                key,
                vm: VmId(21),
                slot: NetSlotId(34),
            }
        );
        let reserved = decision
            .next
            .operation_plans
            .get(&key)
            .expect("reserved plan");
        assert_eq!(decision.next.next_vm_id, 22);
        assert_eq!(decision.next.next_slot_id, 35);
        assert_eq!(reserved.vm, Some(VmId(21)));
        assert_eq!(reserved.slot, Some(NetSlotId(34)));
        assert_eq!(reserved.stop_vms, vec![VmId(21), VmId(8)]);
        assert_eq!(reserved.close_slots, vec![NetSlotId(34), NetSlotId(9)]);
        assert!(reserved.sealed);
        assert_eq!(world, before);
    }

    #[test]
    fn pause_accepts_and_atomically_seals_the_snapshot_pair() {
        let world = pause_world();
        let before = world.clone();
        let key = operation_key(SID, OPERATION);
        let decision = handle_reserve_pause_snapshot(&world, key).expect("valid reservation");
        let manifest = SnapshotManifestRef {
            digest: ManifestDigest(89),
        };

        assert_eq!(
            decision.event,
            Event::ReservePauseSnapshot {
                key,
                snapshot: SnapshotId(55),
                manifest,
            }
        );
        let reserved = decision
            .next
            .operation_plans
            .get(&key)
            .expect("reserved plan");
        assert_eq!(decision.next.next_snapshot_id, 56);
        assert_eq!(decision.next.next_manifest_digest, 90);
        assert_eq!(reserved.vm, Some(VM));
        assert_eq!(
            reserved.target_snapshot,
            Some(crate::RestoreSource {
                snapshot: SnapshotId(55),
                manifest,
            })
        );
        assert!(reserved.sealed);
        assert_eq!(world, before);
    }

    #[test]
    fn startup_rejections_follow_the_lean_guard_order() {
        let key = operation_key(SID, OPERATION);
        let mut world = startup_world();
        world.controller_up = false;
        world.sandboxes = crate::Store::default();
        assert_eq!(
            handle_reserve_startup(&world, key),
            Err(SchedulerReject::ControllerDown)
        );

        let mut world = startup_world();
        let mut sandbox = world.sandboxes.get(&SID).expect("sandbox").clone();
        sandbox.phase = Phase::Running;
        sandbox.terminal = true;
        sandbox.current_operation = None;
        world.sandboxes.put(SID, sandbox);
        world.operation_plans = crate::Store::default();
        assert_eq!(
            handle_reserve_startup(&world, key),
            Err(SchedulerReject::WrongPhase)
        );

        let mut world = startup_world();
        let mut sandbox = world.sandboxes.get(&SID).expect("sandbox").clone();
        sandbox.terminal = true;
        sandbox.current_operation = None;
        world.sandboxes.put(SID, sandbox);
        assert_eq!(
            handle_reserve_startup(&world, key),
            Err(SchedulerReject::Terminal)
        );

        let mut world = startup_world();
        let mut sandbox = world.sandboxes.get(&SID).expect("sandbox").clone();
        sandbox.current_operation = None;
        world.sandboxes.put(SID, sandbox);
        assert_eq!(
            handle_reserve_startup(&world, key),
            Err(SchedulerReject::OperationUnavailable)
        );

        let mut world = startup_world();
        world.operation_plans = crate::Store::default();
        assert_eq!(
            handle_reserve_startup(&world, key),
            Err(SchedulerReject::PlanUnavailable)
        );

        let mut world = startup_world();
        let mut reserved = plan();
        reserved.sealed = true;
        reserved.vm = Some(VmId(99));
        world.operation_plans.put(key, reserved);
        assert_eq!(
            handle_reserve_startup(&world, key),
            Err(SchedulerReject::PlanAlreadySealed)
        );
    }

    #[test]
    fn pause_rejections_follow_the_lean_guard_order() {
        let key = operation_key(SID, OPERATION);
        let mut world = pause_world();
        let mut sandbox = world.sandboxes.get(&SID).expect("sandbox").clone();
        sandbox.phase = Phase::Starting;
        sandbox.terminal = true;
        sandbox.current_vm = None;
        world.sandboxes.put(SID, sandbox);
        assert_eq!(
            handle_reserve_pause_snapshot(&world, key),
            Err(SchedulerReject::WrongPhase)
        );

        let mut world = pause_world();
        let mut sandbox = world.sandboxes.get(&SID).expect("sandbox").clone();
        sandbox.current_vm = None;
        world.sandboxes.put(SID, sandbox);
        assert_eq!(
            handle_reserve_pause_snapshot(&world, key),
            Err(SchedulerReject::VmNotFound)
        );

        let mut world = pause_world();
        let mut reserved = plan();
        reserved.vm = Some(VM);
        reserved.target_snapshot = Some(crate::RestoreSource {
            snapshot: SnapshotId(77),
            manifest: SnapshotManifestRef {
                digest: ManifestDigest(78),
            },
        });
        world.operation_plans.put(key, reserved);
        assert_eq!(
            handle_reserve_pause_snapshot(&world, key),
            Err(SchedulerReject::PlanNotOpen)
        );
    }

    #[test]
    fn startup_never_wraps_and_freshness_still_precedes_capacity() {
        let key = operation_key(SID, OPERATION);
        let mut world = startup_world();
        world.next_vm_id = u64::MAX;
        let before = world.clone();
        assert_eq!(
            handle_reserve_startup(&world, key),
            Err(SchedulerReject::CounterExhausted)
        );
        assert_eq!(world, before);
        assert_eq!(world.next_vm_id, u64::MAX);

        world.vms.put(
            VmId(u64::MAX),
            Vm {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                status: VmStatus::Prepared,
            },
        );
        assert_eq!(
            handle_reserve_startup(&world, key),
            Err(SchedulerReject::ResourceAlreadyExists)
        );

        let mut slot_world = startup_world();
        slot_world.next_slot_id = u64::MAX;
        slot_world.net_slots.put(
            NetSlotId(u64::MAX),
            NetSlot {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                vm: VM,
                policy: NetworkPolicy::DenyAll,
                configured: false,
                active: false,
                reclaimed: false,
            },
        );
        assert_eq!(
            handle_reserve_startup(&slot_world, key),
            Err(SchedulerReject::ResourceAlreadyExists)
        );
    }

    #[test]
    fn pause_never_wraps_and_freshness_still_precedes_capacity() {
        let key = operation_key(SID, OPERATION);
        let mut world = pause_world();
        world.next_snapshot_id = u64::MAX;
        let before = world.clone();
        assert_eq!(
            handle_reserve_pause_snapshot(&world, key),
            Err(SchedulerReject::CounterExhausted)
        );
        assert_eq!(world, before);
        assert_eq!(world.next_snapshot_id, u64::MAX);

        world.snapshots.put(
            SnapshotId(u64::MAX),
            Snapshot {
                owner: OWNER,
                sandbox: SID,
                source_generation: Generation::from(3_u64),
                workspace: FsId(13),
                manifest: SnapshotManifestRef {
                    digest: ManifestDigest(80),
                },
                stage: SnapshotStage::LocalSealed,
                immutable: false,
            },
        );
        assert_eq!(
            handle_reserve_pause_snapshot(&world, key),
            Err(SchedulerReject::ResourceAlreadyExists)
        );

        let mut manifest_world = pause_world();
        manifest_world.next_manifest_digest = u64::MAX;
        assert_eq!(
            handle_reserve_pause_snapshot(&manifest_world, key),
            Err(SchedulerReject::CounterExhausted)
        );
        assert_eq!(manifest_world.next_manifest_digest, u64::MAX);
    }

    #[test]
    fn dispatch_prepare_accepts_create_and_journals_the_exact_command() {
        let world = dispatch_world(None);
        let before = world.clone();
        let decision = handle_dispatch_prepare(&world, SID, VmId(21), NetSlotId(34))
            .expect("valid prepare dispatch");

        assert_eq!(
            decision.event,
            Event::DispatchPrepare {
                sandbox: SID,
                vm: VmId(21),
                slot: NetSlotId(34),
                source: None,
            }
        );
        let command = Command::PrepareVm {
            sandbox: SID,
            epoch: 5,
            generation: 3,
            vm: VmId(21),
            slot: NetSlotId(34),
            source: None,
        };
        let effect = crate::EffectId::initial(command.clone());
        assert_eq!(
            decision.next.effects.get(&effect),
            Some(&EffectStatus::Committed)
        );
        assert_eq!(decision.next.effect_keys, vec![effect.clone()]);
        assert_eq!(decision.next.inflight, vec![effect]);
        assert_eq!(
            decision
                .next
                .logical_effects
                .get(&LogicalEffectId { command })
                .expect("logical effect")
                .origin,
            EffectOrigin::Operation(operation_key(SID, OPERATION))
        );
        assert_eq!(world, before);
    }

    #[test]
    fn dispatch_prepare_accepts_only_the_selected_usable_restore_source() {
        let source = RestoreSource {
            snapshot: SnapshotId(55),
            manifest: SnapshotManifestRef {
                digest: ManifestDigest(89),
            },
        };
        let mut world = dispatch_world(Some(source));
        world.snapshots.put(
            source.snapshot,
            Snapshot {
                owner: OWNER,
                sandbox: SID,
                source_generation: 2,
                workspace: FsId(13),
                manifest: source.manifest,
                stage: SnapshotStage::Durable,
                immutable: true,
            },
        );

        let decision = handle_dispatch_prepare(&world, SID, VmId(21), NetSlotId(34))
            .expect("usable selected source");
        assert_eq!(
            decision.event,
            Event::DispatchPrepare {
                sandbox: SID,
                vm: VmId(21),
                slot: NetSlotId(34),
                source: Some(source),
            }
        );
        assert!(
            decision
                .next
                .effects
                .contains_key(&crate::EffectId::initial(Command::PrepareVm {
                    sandbox: SID,
                    epoch: 5,
                    generation: 3,
                    vm: VmId(21),
                    slot: NetSlotId(34),
                    source: Some(source),
                }))
        );
    }

    #[test]
    fn dispatch_prepare_rejections_follow_fence_resource_and_source_order() {
        let source = RestoreSource {
            snapshot: SnapshotId(55),
            manifest: SnapshotManifestRef {
                digest: ManifestDigest(89),
            },
        };
        let mut world = dispatch_world(Some(source));
        world.worker_fences = crate::Store::default();
        world.vms.put(
            VmId(21),
            Vm {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                status: VmStatus::Prepared,
            },
        );
        assert_eq!(
            handle_dispatch_prepare(&world, SID, VmId(21), NetSlotId(34)),
            Err(SchedulerReject::FenceNotInstalled)
        );

        world.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 5,
                terminal: false,
            },
        );
        assert_eq!(
            handle_dispatch_prepare(&world, SID, VmId(21), NetSlotId(34)),
            Err(SchedulerReject::ResourceAlreadyExists)
        );

        world.vms = crate::Store::default();
        assert_eq!(
            handle_dispatch_prepare(&world, SID, VmId(21), NetSlotId(34)),
            Err(SchedulerReject::RestoreSourceUnavailable)
        );
    }

    #[test]
    fn dispatch_prepare_checks_the_sealed_plan_before_the_fence() {
        let mut world = dispatch_world(None);
        let key = operation_key(SID, OPERATION);
        let mut plan = world.operation_plans.get(&key).expect("plan").clone();
        plan.sealed = false;
        plan.vm = Some(VmId(99));
        world.operation_plans.put(key, plan);
        world.worker_fences = crate::Store::default();

        assert_eq!(
            handle_dispatch_prepare(&world, SID, VmId(21), NetSlotId(34)),
            Err(SchedulerReject::PlanNotOpen)
        );
    }

    #[test]
    fn schedule_seal_accepts_and_journals_the_exact_command() {
        let (world, manifest) = seal_scheduler_world();
        let before = world.clone();
        let decision = handle_schedule_seal_snapshot(&world, SID, SnapshotId(55), manifest)
            .expect("valid seal scheduling");

        assert_eq!(
            decision.event,
            Event::ScheduleSealSnapshot {
                sandbox: SID,
                snapshot: SnapshotId(55),
                manifest,
            }
        );
        let command = Command::SealSnapshot {
            sandbox: SID,
            epoch: 5,
            generation: 3,
            vm: VM,
            workspace: FsId(13),
            snapshot: SnapshotId(55),
            manifest,
        };
        let effect = crate::EffectId::initial(command);
        assert_eq!(
            decision.next.effects.get(&effect),
            Some(&EffectStatus::Committed)
        );
        assert_eq!(decision.next.inflight.first(), Some(&effect));
        assert_eq!(world, before);
    }

    #[test]
    fn schedule_seal_rejections_follow_vm_fence_receipt_and_freshness_order() {
        let (mut world, manifest) = seal_scheduler_world();
        let mut vm = *world.vms.get(&VM).expect("vm");
        vm.owner = TenantId(99);
        vm.status = VmStatus::Prepared;
        world.vms.put(VM, vm);
        world.worker_fences = crate::Store::default();
        world.effects = crate::Store::default();
        world.snapshots.put(
            SnapshotId(55),
            Snapshot {
                owner: OWNER,
                sandbox: SID,
                source_generation: 3,
                workspace: FsId(13),
                manifest,
                stage: SnapshotStage::LocalSealed,
                immutable: false,
            },
        );
        assert_eq!(
            handle_schedule_seal_snapshot(&world, SID, SnapshotId(55), manifest),
            Err(SchedulerReject::ResourceMismatch)
        );

        vm.owner = OWNER;
        world.vms.put(VM, vm);
        assert_eq!(
            handle_schedule_seal_snapshot(&world, SID, SnapshotId(55), manifest),
            Err(SchedulerReject::ResourceNotReady)
        );

        vm.status = VmStatus::Frozen;
        world.vms.put(VM, vm);
        assert_eq!(
            handle_schedule_seal_snapshot(&world, SID, SnapshotId(55), manifest),
            Err(SchedulerReject::FenceNotInstalled)
        );

        world.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 5,
                terminal: false,
            },
        );
        assert_eq!(
            handle_schedule_seal_snapshot(&world, SID, SnapshotId(55), manifest),
            Err(SchedulerReject::PrerequisiteNotSucceeded)
        );

        acknowledge_success(
            &mut world,
            Command::FreezeVm {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                vm: VM,
            },
        );
        assert_eq!(
            handle_schedule_seal_snapshot(&world, SID, SnapshotId(55), manifest),
            Err(SchedulerReject::ResourceAlreadyExists)
        );
    }

    #[test]
    fn schedule_seal_checks_the_plan_before_vm_and_fence_state() {
        let (mut world, manifest) = seal_scheduler_world();
        let key = operation_key(SID, OPERATION);
        let mut plan = world.operation_plans.get(&key).expect("plan").clone();
        plan.sealed = false;
        plan.target_snapshot = None;
        world.operation_plans.put(key, plan);
        world.vms = crate::Store::default();
        world.worker_fences = crate::Store::default();

        assert_eq!(
            handle_schedule_seal_snapshot(&world, SID, SnapshotId(55), manifest),
            Err(SchedulerReject::PlanNotOpen)
        );
    }

    #[test]
    fn schedule_commit_accepts_and_journals_the_exact_command() {
        let (world, manifest) = commit_scheduler_world();
        let before = world.clone();
        let decision = handle_schedule_commit_snapshot(&world, SID, SnapshotId(55))
            .expect("valid commit scheduling");

        assert_eq!(
            decision.event,
            Event::ScheduleCommitSnapshot {
                sandbox: SID,
                snapshot: SnapshotId(55),
            }
        );
        let command = Command::CommitSnapshot {
            sandbox: SID,
            epoch: 5,
            generation: 3,
            snapshot: SnapshotId(55),
            manifest,
        };
        let effect = crate::EffectId::initial(command);
        assert_eq!(
            decision.next.effects.get(&effect),
            Some(&EffectStatus::Committed)
        );
        assert_eq!(decision.next.inflight.first(), Some(&effect));
        assert_eq!(world, before);
    }

    #[test]
    fn schedule_commit_rejections_follow_snapshot_first_identity_stage_and_receipt_order() {
        let empty = World::default();
        assert_eq!(
            handle_schedule_commit_snapshot(&empty, SID, SnapshotId(55)),
            Err(SchedulerReject::SnapshotNotFound)
        );

        let (mut world, manifest) = commit_scheduler_world();
        let mut snapshot = *world.snapshots.get(&SnapshotId(55)).expect("snapshot");
        snapshot.sandbox = SandboxId(99);
        snapshot.source_generation = 99;
        snapshot.stage = SnapshotStage::Durable;
        world.snapshots.put(SnapshotId(55), snapshot);
        let mut record = world.sandboxes.get(&SID).expect("sandbox").clone();
        record.phase = Phase::Starting;
        record.current_vm = None;
        world.sandboxes.put(SID, record);
        world.effects = crate::Store::default();
        assert_eq!(
            handle_schedule_commit_snapshot(&world, SID, SnapshotId(55)),
            Err(SchedulerReject::ResourceMismatch)
        );

        snapshot.sandbox = SID;
        world.snapshots.put(SnapshotId(55), snapshot);
        assert_eq!(
            handle_schedule_commit_snapshot(&world, SID, SnapshotId(55)),
            Err(SchedulerReject::WrongPhase)
        );

        let (mut world, _) = commit_scheduler_world();
        let mut snapshot = *world.snapshots.get(&SnapshotId(55)).expect("snapshot");
        snapshot.source_generation = 99;
        snapshot.stage = SnapshotStage::Durable;
        world.snapshots.put(SnapshotId(55), snapshot);
        assert_eq!(
            handle_schedule_commit_snapshot(&world, SID, SnapshotId(55)),
            Err(SchedulerReject::ResourceMismatch)
        );

        snapshot.source_generation = 3;
        world.snapshots.put(SnapshotId(55), snapshot);
        assert_eq!(
            handle_schedule_commit_snapshot(&world, SID, SnapshotId(55)),
            Err(SchedulerReject::ResourceNotReady)
        );

        snapshot.stage = SnapshotStage::LocalSealed;
        world.snapshots.put(SnapshotId(55), snapshot);
        world.effects = crate::Store::default();
        assert_eq!(
            handle_schedule_commit_snapshot(&world, SID, SnapshotId(55)),
            Err(SchedulerReject::PrerequisiteNotSucceeded)
        );

        assert_eq!(manifest, snapshot.manifest);
    }

    #[test]
    fn schedule_stop_accepts_exact_command_and_repeated_scheduling_is_idempotent() {
        let world = stop_scheduler_world();
        let before = world.clone();
        let decision = handle_schedule_stop_vm(&world, SID, 3, VM).expect("valid stop scheduling");
        assert_eq!(
            decision.event,
            Event::ScheduleStopVm {
                sandbox: SID,
                generation: 3,
                vm: VM,
            }
        );
        let command = Command::StopVm {
            sandbox: SID,
            epoch: 5,
            generation: 3,
            vm: VM,
        };
        let effect = crate::EffectId::initial(command);
        assert_eq!(
            decision.next.effects.get(&effect),
            Some(&EffectStatus::Committed)
        );
        assert_eq!(decision.next.inflight.first(), Some(&effect));
        assert_eq!(world, before);

        let repeated = handle_schedule_stop_vm(&decision.next, SID, 3, VM)
            .expect("same stop remains schedulable");
        assert_eq!(repeated.next, decision.next);
    }

    #[test]
    fn schedule_stop_rejections_follow_terminal_fence_receipt_and_vm_order() {
        let mut world = stop_scheduler_world();
        let mut sandbox = world.sandboxes.get(&SID).expect("sandbox").clone();
        sandbox.phase = Phase::Running;
        sandbox.terminal = false;
        world.sandboxes.put(SID, sandbox.clone());
        world.worker_fences = crate::Store::default();
        world.effects = crate::Store::default();
        world.vms = crate::Store::default();
        assert_eq!(
            handle_schedule_stop_vm(&world, SID, 3, VM),
            Err(SchedulerReject::WrongPhase)
        );

        sandbox.phase = Phase::Revoking;
        world.sandboxes.put(SID, sandbox.clone());
        assert_eq!(
            handle_schedule_stop_vm(&world, SID, 3, VM),
            Err(SchedulerReject::Terminal)
        );

        sandbox.terminal = true;
        world.sandboxes.put(SID, sandbox);
        assert_eq!(
            handle_schedule_stop_vm(&world, SID, 3, VM),
            Err(SchedulerReject::FenceNotInstalled)
        );

        world.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 5,
                terminal: true,
            },
        );
        assert_eq!(
            handle_schedule_stop_vm(&world, SID, 3, VM),
            Err(SchedulerReject::PrerequisiteNotSucceeded)
        );

        acknowledge_success(
            &mut world,
            Command::InstallFence {
                sandbox: SID,
                epoch: 5,
                terminal: true,
            },
        );
        assert_eq!(
            handle_schedule_stop_vm(&world, SID, 3, VM),
            Err(SchedulerReject::VmNotFound)
        );

        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SandboxId(99),
                generation: 9,
                status: VmStatus::Stopped,
            },
        );
        assert_eq!(
            handle_schedule_stop_vm(&world, SID, 3, VM),
            Err(SchedulerReject::ResourceMismatch)
        );

        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                status: VmStatus::Stopped,
            },
        );
        assert_eq!(
            handle_schedule_stop_vm(&world, SID, 3, VM),
            Err(SchedulerReject::ResourceNotReady)
        );
    }

    #[test]
    fn schedule_cleanup_vm_accepts_exact_command_and_is_idempotent() {
        let world = cleanup_vm_world();
        let decision =
            handle_schedule_cleanup_vm(&world, SID, 3, VM).expect("valid VM cleanup scheduling");
        assert_eq!(
            decision.event,
            Event::ScheduleCleanupVm {
                sandbox: SID,
                generation: 3,
                vm: VM,
            }
        );
        let effect = crate::EffectId::initial(Command::CleanupVm {
            sandbox: SID,
            generation: 3,
            vm: VM,
        });
        assert_eq!(
            decision.next.effects.get(&effect),
            Some(&EffectStatus::Committed)
        );
        let repeated = handle_schedule_cleanup_vm(&decision.next, SID, 3, VM)
            .expect("same cleanup remains schedulable");
        assert_eq!(repeated.next, decision.next);
    }

    #[test]
    fn schedule_cleanup_vm_rejections_follow_identity_status_and_current_order() {
        let mut world = cleanup_vm_world();
        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SandboxId(99),
                generation: 9,
                status: VmStatus::Prepared,
            },
        );
        let mut sandbox = record(Phase::Running, OperationKind::Create);
        sandbox.current_vm = Some(VM);
        world.sandboxes.put(SID, sandbox);
        assert_eq!(
            handle_schedule_cleanup_vm(&world, SID, 3, VM),
            Err(SchedulerReject::ResourceMismatch)
        );

        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                status: VmStatus::Prepared,
            },
        );
        assert_eq!(
            handle_schedule_cleanup_vm(&world, SID, 3, VM),
            Err(SchedulerReject::ResourceNotReady)
        );

        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                status: VmStatus::Stopped,
            },
        );
        assert_eq!(
            handle_schedule_cleanup_vm(&world, SID, 3, VM),
            Err(SchedulerReject::ResourceStillCurrent)
        );
    }

    #[test]
    fn schedule_cleanup_slot_accepts_exact_command_and_is_idempotent() {
        let world = cleanup_slot_world();
        let slot = NetSlotId(34);
        let decision = handle_schedule_cleanup_slot(&world, SID, 3, slot)
            .expect("valid slot cleanup scheduling");
        assert_eq!(
            decision.event,
            Event::ScheduleCleanupSlot {
                sandbox: SID,
                generation: 3,
                slot,
            }
        );
        let effect = crate::EffectId::initial(Command::CleanupSlot {
            sandbox: SID,
            generation: 3,
            slot,
        });
        assert_eq!(
            decision.next.effects.get(&effect),
            Some(&EffectStatus::Committed)
        );
        let repeated = handle_schedule_cleanup_slot(&decision.next, SID, 3, slot)
            .expect("same cleanup remains schedulable");
        assert_eq!(repeated.next, decision.next);
    }

    #[test]
    fn schedule_cleanup_slot_rejections_follow_identity_activity_reclaimed_and_current_order() {
        let slot_id = NetSlotId(34);
        let mut world = cleanup_slot_world();
        let mut slot = world.net_slots.get(&slot_id).expect("slot").clone();
        slot.sandbox = SandboxId(99);
        slot.generation = 9;
        slot.active = true;
        slot.reclaimed = true;
        world.net_slots.put(slot_id, slot.clone());
        let mut sandbox = record(Phase::Running, OperationKind::Create);
        sandbox.net_slot = Some(slot_id);
        world.sandboxes.put(SID, sandbox);
        assert_eq!(
            handle_schedule_cleanup_slot(&world, SID, 3, slot_id),
            Err(SchedulerReject::ResourceMismatch)
        );

        slot.sandbox = SID;
        slot.generation = 3;
        world.net_slots.put(slot_id, slot.clone());
        assert_eq!(
            handle_schedule_cleanup_slot(&world, SID, 3, slot_id),
            Err(SchedulerReject::ResourceNotReady)
        );

        slot.active = false;
        world.net_slots.put(slot_id, slot.clone());
        assert_eq!(
            handle_schedule_cleanup_slot(&world, SID, 3, slot_id),
            Err(SchedulerReject::AlreadyReclaimed)
        );

        slot.reclaimed = false;
        world.net_slots.put(slot_id, slot);
        assert_eq!(
            handle_schedule_cleanup_slot(&world, SID, 3, slot_id),
            Err(SchedulerReject::ResourceStillCurrent)
        );
    }

    #[test]
    fn remaining_scheduler_handlers_check_controller_before_resource_lookups() {
        let mut world = World {
            controller_up: false,
            ..World::default()
        };
        assert_eq!(
            handle_schedule_stop_vm(&world, SID, 3, VM),
            Err(SchedulerReject::ControllerDown)
        );
        assert_eq!(
            handle_schedule_cleanup_vm(&world, SID, 3, VM),
            Err(SchedulerReject::ControllerDown)
        );
        assert_eq!(
            handle_schedule_cleanup_slot(&world, SID, 3, NetSlotId(34)),
            Err(SchedulerReject::ControllerDown)
        );

        world.controller_up = true;
        assert_eq!(
            handle_schedule_stop_vm(&world, SID, 3, VM),
            Err(SchedulerReject::SandboxNotFound)
        );
        assert_eq!(
            handle_schedule_cleanup_vm(&world, SID, 3, VM),
            Err(SchedulerReject::SandboxNotFound)
        );
        assert_eq!(
            handle_schedule_cleanup_slot(&world, SID, 3, NetSlotId(34)),
            Err(SchedulerReject::SandboxNotFound)
        );
    }

    #[test]
    fn remaining_scheduler_handlers_reject_resources_outside_the_current_plan() {
        let mut stop_world = stop_scheduler_world();
        let key = operation_key(SID, OPERATION);
        let mut plan = stop_world.operation_plans.get(&key).expect("plan").clone();
        plan.stop_vms.clear();
        stop_world.operation_plans.put(key, plan);
        stop_world.worker_fences = crate::Store::default();
        assert_eq!(
            handle_schedule_stop_vm(&stop_world, SID, 3, VM),
            Err(SchedulerReject::ResourceNotPlanned)
        );

        let mut vm_world = cleanup_vm_world();
        let mut plan = vm_world.operation_plans.get(&key).expect("plan").clone();
        plan.stop_vms.clear();
        vm_world.operation_plans.put(key, plan);
        vm_world.vms = crate::Store::default();
        assert_eq!(
            handle_schedule_cleanup_vm(&vm_world, SID, 3, VM),
            Err(SchedulerReject::ResourceNotPlanned)
        );

        let mut slot_world = cleanup_slot_world();
        let mut plan = slot_world.operation_plans.get(&key).expect("plan").clone();
        plan.close_slots.clear();
        slot_world.operation_plans.put(key, plan);
        slot_world.net_slots = crate::Store::default();
        assert_eq!(
            handle_schedule_cleanup_slot(&slot_world, SID, 3, NetSlotId(34)),
            Err(SchedulerReject::ResourceNotPlanned)
        );
    }
}
