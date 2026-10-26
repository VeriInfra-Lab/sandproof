use crate::SandboxId;
use crate::effect::effect_succeeded_check;
use crate::model::{
    Authority, Command, EffectId, FailureKind, OperationKind, OperationResult, OperationStatus,
    Phase, RestoreSource, SandboxRecord, SnapshotStage, VmStatus, WorkerFence, World,
};
use crate::transition::operation_key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PublicationInput {
    Running(SandboxId),
    Paused(SandboxId, RestoreSource),
    Destroyed(SandboxId),
    Failed(SandboxId, FailureKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationEvent {
    PublishRunning(SandboxId),
    PublishPaused(SandboxId, crate::SnapshotId),
    PublishDestroyed(SandboxId),
    PublishFailure(SandboxId, FailureKind),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationReject {
    ControllerDown,
    SandboxNotFound,
    VmNotFound,
    SlotNotFound,
    SnapshotNotFound,
    OperationUnavailable,
    WrongPhase,
    TerminalMismatch,
    FenceNotInstalled,
    ResourceMismatch,
    ResourceNotReady,
    PolicyNotApplied,
    PrerequisiteNotSucceeded,
    ResourcesNotClosed,
    StopReceiptsMissing,
    WrongOperationKind,
    WrongOperationStatus,
    LedgerMismatch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicationDecision {
    pub event: PublicationEvent,
    pub next: World,
}

fn indexed_vms_closed(world: &World, sandbox: SandboxId) -> bool {
    let index = world.resource_index(sandbox);
    let mut cursor = 0;
    while cursor < index.vms.len() {
        let Some(vm) = world.vms.get(&index.vms[cursor]) else {
            return false;
        };
        if vm.status != VmStatus::Stopped && vm.status != VmStatus::Reclaimed {
            return false;
        }
        cursor += 1;
    }
    true
}

fn indexed_slots_closed(world: &World, sandbox: SandboxId) -> bool {
    let index = world.resource_index(sandbox);
    let mut cursor = 0;
    while cursor < index.slots.len() {
        let Some(slot) = world.net_slots.get(&index.slots[cursor]) else {
            return false;
        };
        if slot.active {
            return false;
        }
        cursor += 1;
    }
    true
}

fn stop_receipt_candidate(
    world: &World,
    sandbox: SandboxId,
    generation: u64,
    vm: crate::VmId,
    candidate: &EffectId,
) -> bool {
    match candidate.command() {
        Command::StopVm {
            sandbox: command_sandbox,
            generation: command_generation,
            vm: command_vm,
            ..
        }
        | Command::AbortPrepare {
            sandbox: command_sandbox,
            generation: command_generation,
            vm: command_vm,
            ..
        } => {
            *command_sandbox == sandbox
                && *command_generation == generation
                && *command_vm == vm
                && effect_succeeded_check(world, candidate.command())
        }
        Command::InstallFence { .. }
        | Command::PrepareVm { .. }
        | Command::ConfigureNetwork { .. }
        | Command::ActivateVm { .. }
        | Command::FreezeVm { .. }
        | Command::ApplyFilePolicy { .. }
        | Command::SealSnapshot { .. }
        | Command::CommitSnapshot { .. }
        | Command::CleanupVm { .. }
        | Command::CleanupSlot { .. } => false,
    }
}

fn vm_has_stop_receipt(
    world: &World,
    sandbox: SandboxId,
    generation: u64,
    vm: crate::VmId,
) -> bool {
    let mut cursor = 0;
    while cursor < world.effect_keys.len() {
        if stop_receipt_candidate(world, sandbox, generation, vm, &world.effect_keys[cursor]) {
            return true;
        }
        cursor += 1;
    }
    false
}

pub(crate) fn indexed_stops_succeeded(world: &World, sandbox: SandboxId) -> bool {
    let index = world.resource_index(sandbox);
    let mut cursor = 0;
    while cursor < index.vms.len() {
        let vm_id = index.vms[cursor];
        let Some(vm) = world.vms.get(&vm_id) else {
            return false;
        };
        if !vm_has_stop_receipt(world, sandbox, vm.generation, vm_id) {
            return false;
        }
        cursor += 1;
    }
    true
}

fn planned_vms_stopped(world: &World, sandbox: SandboxId, plan: &crate::OperationPlan) -> bool {
    for vm_id in &plan.stop_vms {
        let Some(vm) = world.vms.get(vm_id) else {
            return false;
        };
        if vm.sandbox != sandbox
            || vm.generation != plan.generation
            || !matches!(vm.status, VmStatus::Stopped | VmStatus::Reclaimed)
            || !vm_has_stop_receipt(world, sandbox, plan.generation, *vm_id)
        {
            return false;
        }
    }
    true
}

fn planned_slots_inactive(world: &World, sandbox: SandboxId, plan: &crate::OperationPlan) -> bool {
    for slot_id in &plan.close_slots {
        let Some(slot) = world.net_slots.get(slot_id) else {
            return false;
        };
        if slot.sandbox != sandbox || slot.generation != plan.generation || slot.active {
            return false;
        }
    }
    true
}

// Reservations are recorded before `PrepareVm` reaches the host. An empty
// resource index therefore cannot prove that the reserved VM was stopped.
// Keep the original VM-first, short-circuit order. Separate loop functions
// avoid the fixed extractor's normalization of sequential early-return loops.
fn planned_resources_closed(
    world: &World,
    sandbox: SandboxId,
    plan: &crate::OperationPlan,
) -> bool {
    planned_vms_stopped(world, sandbox, plan) && planned_slots_inactive(world, sandbox, plan)
}

fn update_operation_result(
    world: &mut World,
    sandbox: SandboxId,
    operation: crate::Operation,
    result: OperationResult,
) {
    let key = operation_key(sandbox, operation.id);
    if let Some(stored) = world.operations.get(&key) {
        let mut ledger = stored.clone();
        ledger.operation = operation;
        ledger.result = Some(result);
        world.operations.put(key, ledger);
    }
}

fn publish_running(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    slot_id: crate::NetSlotId,
    slot: &crate::NetSlot,
) -> World {
    let mut next = world.clone();
    let mut running = record.clone();
    running.phase = Phase::Running;
    running.terminal = false;
    running.restore_from = None;
    if let Some(mut operation) = running.current_operation {
        operation.status = OperationStatus::Succeeded;
        running.current_operation = Some(operation);
    }
    let authority = Authority::Running {
        sandbox,
        generation: running.generation,
        vm: slot.vm,
        workspace: running.workspace,
        access: running.workspace_access,
        slot: slot_id,
        policy: running.policy.clone(),
    };
    running.desired = authority.clone();
    running.effective = authority;
    let mut active_slot = slot.clone();
    active_slot.active = true;
    next.net_slots.put(slot_id, active_slot);
    next.sandboxes.put(sandbox, running.clone());
    if let Some(operation) = running.current_operation {
        update_operation_result(
            &mut next,
            sandbox,
            operation,
            OperationResult::Running(running.generation),
        );
    }
    next
}

fn publish_paused(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    source: RestoreSource,
) -> World {
    let mut next = world.clone();
    let mut paused = record.clone();
    paused.phase = Phase::Paused;
    paused.current_vm = None;
    paused.durable_snapshot = Some(source);
    paused.restore_from = None;
    paused.net_slot = None;
    if let Some(mut operation) = paused.current_operation {
        operation.status = OperationStatus::Succeeded;
        paused.current_operation = Some(operation);
    }
    let authority = Authority::Paused { sandbox, source };
    paused.desired = authority.clone();
    paused.effective = authority;
    next.sandboxes.put(sandbox, paused.clone());
    if let Some(operation) = paused.current_operation {
        update_operation_result(
            &mut next,
            sandbox,
            operation,
            OperationResult::Paused(source),
        );
    }
    next
}

fn publish_destroyed(world: &World, sandbox: SandboxId, record: &SandboxRecord) -> World {
    let mut next = world.clone();
    let mut destroyed = record.clone();
    destroyed.phase = Phase::Destroyed;
    destroyed.terminal = true;
    destroyed.current_vm = None;
    destroyed.net_slot = None;
    destroyed.desired = Authority::Closed;
    destroyed.effective = Authority::Closed;
    if let Some(mut operation) = destroyed.current_operation {
        operation.status = OperationStatus::Succeeded;
        destroyed.current_operation = Some(operation);
    }
    next.sandboxes.put(sandbox, destroyed.clone());
    if let Some(operation) = destroyed.current_operation {
        update_operation_result(&mut next, sandbox, operation, OperationResult::Destroyed);
    }
    next
}

fn publish_failure(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    failure: FailureKind,
) -> World {
    let mut next = world.clone();
    let mut destroyed = record.clone();
    destroyed.phase = Phase::Destroyed;
    destroyed.terminal = true;
    destroyed.current_vm = None;
    destroyed.net_slot = None;
    destroyed.desired = Authority::Closed;
    destroyed.effective = Authority::Closed;
    if let Some(mut operation) = destroyed.current_operation {
        operation.status = OperationStatus::Failed(failure);
        destroyed.current_operation = Some(operation);
    }
    next.sandboxes.put(sandbox, destroyed.clone());
    if let Some(operation) = destroyed.current_operation {
        update_operation_result(
            &mut next,
            sandbox,
            operation,
            OperationResult::Failed(failure),
        );
    }
    next
}

/// Publish a fully prepared startup or resume as the only running authority.
///
/// # Errors
/// Returns the first failed guard in Lean's publication order.
#[allow(clippy::too_many_lines)]
pub fn handle_publish_running(
    world: &World,
    sandbox: SandboxId,
) -> Result<PublicationDecision, PublicationReject> {
    if !world.controller_up {
        return Err(PublicationReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(PublicationReject::SandboxNotFound);
    };
    if record.phase != Phase::Starting {
        return Err(PublicationReject::WrongPhase);
    }
    if record.terminal {
        return Err(PublicationReject::TerminalMismatch);
    }
    let Some(vm_id) = record.current_vm else {
        return Err(PublicationReject::VmNotFound);
    };
    let Some(vm) = world.vms.get(&vm_id) else {
        return Err(PublicationReject::VmNotFound);
    };
    if vm.owner != record.owner || vm.sandbox != sandbox || vm.generation != record.generation {
        return Err(PublicationReject::ResourceMismatch);
    }
    if vm.status != VmStatus::Executing {
        return Err(PublicationReject::ResourceNotReady);
    }
    let Some(slot_id) = record.net_slot else {
        return Err(PublicationReject::SlotNotFound);
    };
    let Some(slot) = world.net_slots.get(&slot_id) else {
        return Err(PublicationReject::SlotNotFound);
    };
    if slot.owner != record.owner
        || slot.sandbox != sandbox
        || slot.generation != record.generation
        || slot.vm != vm_id
        || slot.policy != record.policy
        || !slot.configured
        || slot.active
    {
        return Err(PublicationReject::ResourceNotReady);
    }
    if !indexed_slots_closed(world, sandbox) {
        return Err(PublicationReject::ResourcesNotClosed);
    }
    if world.applied_file_policies.get(&sandbox)
        != Some(&crate::AppliedFilePolicy {
            sandbox,
            generation: record.generation,
            workspace: record.workspace,
            policy: record.file_policy,
        })
    {
        return Err(PublicationReject::PolicyNotApplied);
    }
    let fence = Command::InstallFence {
        sandbox,
        epoch: record.epoch,
        terminal: false,
    };
    if !effect_succeeded_check(world, &fence) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    let prepare = Command::PrepareVm {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        vm: vm_id,
        slot: slot_id,
        source: record.restore_from,
    };
    if !effect_succeeded_check(world, &prepare) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    let configure = Command::ConfigureNetwork {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        vm: vm_id,
        slot: slot_id,
        policy: record.policy.clone(),
    };
    if !effect_succeeded_check(world, &configure) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    let activate = Command::ActivateVm {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        vm: vm_id,
    };
    if !effect_succeeded_check(world, &activate) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    let apply_policy = Command::ApplyFilePolicy {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        workspace: record.workspace,
        policy: record.file_policy,
    };
    if !effect_succeeded_check(world, &apply_policy) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    let Some(operation) = record.current_operation else {
        return Err(PublicationReject::OperationUnavailable);
    };
    let key = operation_key(sandbox, operation.id);
    let Some(ledger) = world.operations.get(&key) else {
        return Err(PublicationReject::OperationUnavailable);
    };
    if ledger.sandbox != sandbox || ledger.operation != operation || ledger.result.is_some() {
        return Err(PublicationReject::LedgerMismatch);
    }
    if world.worker_fences.get(&sandbox)
        != Some(&WorkerFence {
            epoch: record.epoch,
            terminal: false,
        })
    {
        return Err(PublicationReject::FenceNotInstalled);
    }
    Ok(PublicationDecision {
        event: PublicationEvent::PublishRunning(sandbox),
        next: publish_running(world, sandbox, record, slot_id, slot),
    })
}

/// Publish a durable pause after every exact pause command has succeeded.
/// Capture identity and stop-permit checks extend the historical reducer.
///
/// # Errors
/// Returns the first failed guard in publication order.
#[allow(clippy::too_many_lines)]
pub fn handle_publish_paused(
    world: &World,
    sandbox: SandboxId,
    source: RestoreSource,
) -> Result<PublicationDecision, PublicationReject> {
    if !world.controller_up {
        return Err(PublicationReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(PublicationReject::SandboxNotFound);
    };
    if record.phase != Phase::Pausing {
        return Err(PublicationReject::WrongPhase);
    }
    if record.terminal {
        return Err(PublicationReject::TerminalMismatch);
    }
    let Some(snapshot) = world.snapshots.get(&source.snapshot) else {
        return Err(PublicationReject::SnapshotNotFound);
    };
    if snapshot.owner != record.owner
        || snapshot.sandbox != sandbox
        || snapshot.source_generation != record.generation
        || snapshot.workspace != record.workspace
        || snapshot.manifest != source.manifest
        || snapshot.stage != SnapshotStage::Durable
        || !snapshot.immutable
    {
        return Err(PublicationReject::ResourceNotReady);
    }
    if world.worker_fences.get(&sandbox)
        != Some(&WorkerFence {
            epoch: record.epoch,
            terminal: false,
        })
    {
        return Err(PublicationReject::FenceNotInstalled);
    }
    let Some(vm_id) = record.current_vm else {
        return Err(PublicationReject::VmNotFound);
    };
    let fence = Command::InstallFence {
        sandbox,
        epoch: record.epoch,
        terminal: false,
    };
    if !effect_succeeded_check(world, &fence) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    let freeze = Command::FreezeVm {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        vm: vm_id,
    };
    if !effect_succeeded_check(world, &freeze) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    let seal = Command::SealSnapshot {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        vm: vm_id,
        workspace: record.workspace,
        snapshot: source.snapshot,
        manifest: source.manifest,
    };
    if !effect_succeeded_check(world, &seal) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    let commit = Command::CommitSnapshot {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        snapshot: source.snapshot,
        manifest: source.manifest,
    };
    if !effect_succeeded_check(world, &commit) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    let stop = Command::StopVm {
        sandbox,
        epoch: record.epoch,
        generation: record.generation,
        vm: vm_id,
    };
    if !effect_succeeded_check(world, &stop) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    if !indexed_vms_closed(world, sandbox) || !indexed_slots_closed(world, sandbox) {
        return Err(PublicationReject::ResourcesNotClosed);
    }
    let Some(operation) = record.current_operation else {
        return Err(PublicationReject::OperationUnavailable);
    };
    let key = operation_key(sandbox, operation.id);
    let Some(ledger) = world.operations.get(&key) else {
        return Err(PublicationReject::OperationUnavailable);
    };
    if ledger.sandbox != sandbox || ledger.operation != operation || ledger.result.is_some() {
        return Err(PublicationReject::LedgerMismatch);
    }
    let Some(plan) = world.operation_plans.get(&key) else {
        return Err(PublicationReject::LedgerMismatch);
    };
    if plan.target_snapshot != Some(source)
        || plan.vm != Some(vm_id)
        || !crate::transition::current_capture(world, sandbox, record)
            .is_some_and(crate::capture::CaptureHandoff::may_stop_original)
    {
        return Err(PublicationReject::ResourceNotReady);
    }
    Ok(PublicationDecision {
        event: PublicationEvent::PublishPaused(sandbox, source.snapshot),
        next: publish_paused(world, sandbox, record, source),
    })
}

const fn operation_pending_or_unknown(status: OperationStatus) -> bool {
    match status {
        OperationStatus::Pending | OperationStatus::OutcomeUnknown => true,
        OperationStatus::Compensating(_)
        | OperationStatus::Succeeded
        | OperationStatus::Failed(_) => false,
    }
}

const fn recoverable_operation_kind(kind: OperationKind) -> bool {
    match kind {
        OperationKind::Create | OperationKind::Pause | OperationKind::Resume => true,
        OperationKind::Kill => false,
    }
}

/// Publish terminal success only after the terminal fence, every planned stop,
/// and every indexed resource closure are durably visible.
///
/// # Errors
/// Returns the first failed guard in Lean's publication order.
#[allow(clippy::too_many_lines)]
pub fn handle_publish_destroyed(
    world: &World,
    sandbox: SandboxId,
) -> Result<PublicationDecision, PublicationReject> {
    if !world.controller_up {
        return Err(PublicationReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(PublicationReject::SandboxNotFound);
    };
    if record.phase != Phase::Revoking {
        return Err(PublicationReject::WrongPhase);
    }
    if !record.terminal {
        return Err(PublicationReject::TerminalMismatch);
    }
    if !crate::maintenance::released_execution::terminal_closed(world, sandbox) {
    if world.worker_fences.get(&sandbox)
        != Some(&WorkerFence {
            epoch: record.epoch,
            terminal: true,
        })
    {
        return Err(PublicationReject::FenceNotInstalled);
    }
    let fence = Command::InstallFence {
        sandbox,
        epoch: record.epoch,
        terminal: true,
    };
    if !effect_succeeded_check(world, &fence) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    }
    if !indexed_vms_closed(world, sandbox) {
        return Err(PublicationReject::ResourcesNotClosed);
    }
    if !indexed_stops_succeeded(world, sandbox) {
        return Err(PublicationReject::StopReceiptsMissing);
    }
    if !indexed_slots_closed(world, sandbox) {
        return Err(PublicationReject::ResourcesNotClosed);
    }
    let Some(operation) = record.current_operation else {
        return Err(PublicationReject::OperationUnavailable);
    };
    if operation.kind != OperationKind::Kill {
        return Err(PublicationReject::WrongOperationKind);
    }
    if !operation_pending_or_unknown(operation.status) {
        return Err(PublicationReject::WrongOperationStatus);
    }
    let key = operation_key(sandbox, operation.id);
    let Some(ledger) = world.operations.get(&key) else {
        return Err(PublicationReject::OperationUnavailable);
    };
    if ledger.sandbox != sandbox || ledger.operation != operation || ledger.result.is_some() {
        return Err(PublicationReject::LedgerMismatch);
    }
    Ok(PublicationDecision {
        event: PublicationEvent::PublishDestroyed(sandbox),
        next: publish_destroyed(world, sandbox, record),
    })
}

/// Publish terminal compensated failure after revocation is durably complete.
///
/// # Errors
/// Returns the first failed guard in Lean's publication order.
#[allow(clippy::too_many_lines)]
pub fn handle_publish_failure(
    world: &World,
    sandbox: SandboxId,
    failure: FailureKind,
) -> Result<PublicationDecision, PublicationReject> {
    if !world.controller_up {
        return Err(PublicationReject::ControllerDown);
    }
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return Err(PublicationReject::SandboxNotFound);
    };
    if record.phase != Phase::Revoking {
        return Err(PublicationReject::WrongPhase);
    }
    if !record.terminal {
        return Err(PublicationReject::TerminalMismatch);
    }
    if world.worker_fences.get(&sandbox)
        != Some(&WorkerFence {
            epoch: record.epoch,
            terminal: true,
        })
    {
        return Err(PublicationReject::FenceNotInstalled);
    }
    let fence = Command::InstallFence {
        sandbox,
        epoch: record.epoch,
        terminal: true,
    };
    if !effect_succeeded_check(world, &fence) {
        return Err(PublicationReject::PrerequisiteNotSucceeded);
    }
    if !indexed_vms_closed(world, sandbox) {
        return Err(PublicationReject::ResourcesNotClosed);
    }
    if !indexed_stops_succeeded(world, sandbox) {
        return Err(PublicationReject::StopReceiptsMissing);
    }
    if !indexed_slots_closed(world, sandbox) {
        return Err(PublicationReject::ResourcesNotClosed);
    }
    let Some(operation) = record.current_operation else {
        return Err(PublicationReject::OperationUnavailable);
    };
    if operation.status != OperationStatus::Compensating(failure) {
        return Err(PublicationReject::WrongOperationStatus);
    }
    if !recoverable_operation_kind(operation.kind) {
        return Err(PublicationReject::WrongOperationKind);
    }
    let key = operation_key(sandbox, operation.id);
    let Some(plan) = world.operation_plans.get(&key) else {
        return Err(PublicationReject::OperationUnavailable);
    };
    if !planned_resources_closed(world, sandbox, plan) {
        return Err(PublicationReject::ResourcesNotClosed);
    }
    let Some(ledger) = world.operations.get(&key) else {
        return Err(PublicationReject::OperationUnavailable);
    };
    if ledger.sandbox != sandbox || ledger.operation != operation || ledger.result.is_some() {
        return Err(PublicationReject::LedgerMismatch);
    }
    Ok(PublicationDecision {
        event: PublicationEvent::PublishFailure(sandbox, failure),
        next: publish_failure(world, sandbox, record, failure),
    })
}

/// Execute one publication request through the same four-way split as Lean's
/// `handlePublication`.
///
/// # Errors
/// Returns the selected handler's first failed guard.
pub fn handle_publication(
    world: &World,
    input: PublicationInput,
) -> Result<PublicationDecision, PublicationReject> {
    match input {
        PublicationInput::Running(sandbox) => handle_publish_running(world, sandbox),
        PublicationInput::Paused(sandbox, source) => handle_publish_paused(world, sandbox, source),
        PublicationInput::Destroyed(sandbox) => handle_publish_destroyed(world, sandbox),
        PublicationInput::Failed(sandbox, failure) => {
            handle_publish_failure(world, sandbox, failure)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PublicationEvent, PublicationInput, PublicationReject, handle_publication,
        handle_publish_destroyed, handle_publish_failure, handle_publish_paused,
        handle_publish_running,
    };
    use crate::model::{
        AppliedFilePolicy, Authority, Command, EffectId, EffectOutcome, EffectStatus,
        FilePolicyRef, LogicalEffectId, NetworkPolicy, Operation, OperationInput, OperationKind,
        OperationLedgerRecord, OperationPlan, OperationResult, OperationStatus, Phase,
        ResourceIndex, RestoreSource, SandboxRecord, Snapshot, SnapshotStage, Vm, VmStatus,
        WorkerFence, WorkspaceApiAccess, World,
    };
    use crate::transition::operation_key;
    use crate::{
        FsId, ManifestDigest, NetSlot, NetSlotId, OperationId, PolicyDigest, ResultDigest,
        SandboxId, SnapshotId, SnapshotManifestRef, TenantId, VmId,
    };

    const OWNER: TenantId = TenantId(7);
    const SID: SandboxId = SandboxId(11);
    const VM: VmId = VmId(13);
    const SLOT: NetSlotId = NetSlotId(17);
    const OPERATION: OperationId = OperationId(19);
    const WORKSPACE: FsId = FsId(23);
    const FILE_POLICY: FilePolicyRef = FilePolicyRef {
        digest: PolicyDigest(29),
    };

    fn operation(kind: OperationKind, status: OperationStatus) -> Operation {
        Operation {
            id: OPERATION,
            epoch: 5,
            kind,
            status,
        }
    }

    fn record(phase: Phase, terminal: bool, operation: Operation) -> SandboxRecord {
        SandboxRecord {
            owner: OWNER,
            phase,
            terminal,
            epoch: 5,
            generation: 3,
            current_vm: Some(VM),
            workspace: WORKSPACE,
            workspace_access: WorkspaceApiAccess::ManagedReadWrite,
            file_policy: FILE_POLICY,
            durable_snapshot: None,
            restore_from: None,
            net_slot: Some(SLOT),
            policy: NetworkPolicy::DenyAll,
            desired: Authority::Closed,
            effective: Authority::Closed,
            current_operation: Some(operation),
        }
    }

    fn put_ledger(world: &mut World, operation: Operation, input: OperationInput) {
        world.operations.put(
            operation_key(SID, OPERATION),
            OperationLedgerRecord {
                sandbox: SID,
                operation,
                input,
                result: None,
            },
        );
    }

    fn acknowledge(world: &mut World, command: Command, result: u64) {
        let logical = LogicalEffectId { command };
        let id = EffectId {
            logical,
            attempt: 0,
        };
        world.effects.put(
            id.clone(),
            EffectStatus::Acknowledged(EffectOutcome::Succeeded(ResultDigest(result))),
        );
        world.effect_keys.push(id);
    }

    #[allow(clippy::too_many_lines)]
    fn running_ready_world() -> World {
        let operation = operation(OperationKind::Create, OperationStatus::Pending);
        let mut world = World::default();
        world
            .sandboxes
            .put(SID, record(Phase::Starting, false, operation));
        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                status: VmStatus::Executing,
            },
        );
        world.net_slots.put(
            SLOT,
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
        world.resource_indexes.put(
            SID,
            ResourceIndex {
                vms: vec![VM],
                slots: vec![SLOT],
                snapshots: Vec::new(),
            },
        );
        world.applied_file_policies.put(
            SID,
            AppliedFilePolicy {
                sandbox: SID,
                generation: 3,
                workspace: WORKSPACE,
                policy: FILE_POLICY,
            },
        );
        world.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 5,
                terminal: false,
            },
        );
        put_ledger(
            &mut world,
            operation,
            OperationInput::Create {
                workspace: WORKSPACE,
                network: NetworkPolicy::DenyAll,
                file_policy: FILE_POLICY,
            },
        );
        acknowledge(
            &mut world,
            Command::InstallFence {
                sandbox: SID,
                epoch: 5,
                terminal: false,
            },
            1,
        );
        acknowledge(
            &mut world,
            Command::PrepareVm {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                vm: VM,
                slot: SLOT,
                source: None,
            },
            2,
        );
        acknowledge(
            &mut world,
            Command::ConfigureNetwork {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                vm: VM,
                slot: SLOT,
                policy: NetworkPolicy::DenyAll,
            },
            3,
        );
        acknowledge(
            &mut world,
            Command::ActivateVm {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                vm: VM,
            },
            4,
        );
        acknowledge(
            &mut world,
            Command::ApplyFilePolicy {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                workspace: WORKSPACE,
                policy: FILE_POLICY,
            },
            5,
        );
        world
    }

    fn terminal_ready_world(kind: OperationKind, status: OperationStatus) -> World {
        let operation = operation(kind, status);
        let mut world = World::default();
        world
            .sandboxes
            .put(SID, record(Phase::Revoking, true, operation));
        world.vms.put(
            VM,
            Vm {
                owner: OWNER,
                sandbox: SID,
                generation: 3,
                status: VmStatus::Stopped,
            },
        );
        world.net_slots.put(
            SLOT,
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
        world.resource_indexes.put(
            SID,
            ResourceIndex {
                vms: vec![VM],
                slots: vec![SLOT],
                snapshots: Vec::new(),
            },
        );
        world.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 5,
                terminal: true,
            },
        );
        let input = match kind {
            OperationKind::Create => OperationInput::Create {
                workspace: WORKSPACE,
                network: NetworkPolicy::DenyAll,
                file_policy: FILE_POLICY,
            },
            OperationKind::Pause => OperationInput::Pause { generation: 3 },
            OperationKind::Resume => OperationInput::Resume {
                source: RestoreSource {
                    snapshot: SnapshotId(31),
                    manifest: SnapshotManifestRef {
                        digest: ManifestDigest(37),
                    },
                },
            },
            OperationKind::Kill => OperationInput::Kill { generation: 3 },
        };
        put_ledger(&mut world, operation, input);
        world.operation_plans.put(
            operation_key(SID, OPERATION),
            OperationPlan {
                key: operation_key(SID, OPERATION),
                epoch: 5,
                generation: 3,
                vm: Some(VM),
                slot: Some(SLOT),
                restore_from: None,
                target_snapshot: None,
                capture: None,
                stop_vms: vec![VM],
                close_slots: vec![SLOT],
                retry_limit: 3,
                sealed: true,
            },
        );
        acknowledge(
            &mut world,
            Command::InstallFence {
                sandbox: SID,
                epoch: 5,
                terminal: true,
            },
            1,
        );
        acknowledge(
            &mut world,
            Command::StopVm {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                vm: VM,
            },
            2,
        );
        world
    }

    #[test]
    fn running_publication_atomically_installs_authority_and_ledger_result() {
        let world = running_ready_world();
        let decision = handle_publish_running(&world, SID).expect("publish running");
        assert_eq!(decision.event, PublicationEvent::PublishRunning(SID));
        let record = decision.next.sandboxes.get(&SID).expect("sandbox");
        assert_eq!(record.phase, Phase::Running);
        assert_eq!(record.desired, record.effective);
        assert!(decision.next.net_slots.get(&SLOT).expect("slot").active);
        assert_eq!(
            decision
                .next
                .operations
                .get(&operation_key(SID, OPERATION))
                .and_then(|ledger| ledger.result),
            Some(OperationResult::Running(3))
        );
    }

    #[test]
    fn terminal_success_requires_stop_receipts_and_closes_authority() {
        let world = terminal_ready_world(OperationKind::Kill, OperationStatus::Pending);
        let decision = handle_publish_destroyed(&world, SID).expect("publish destroyed");
        let record = decision.next.sandboxes.get(&SID).expect("sandbox");
        assert_eq!(record.phase, Phase::Destroyed);
        assert_eq!(record.desired, Authority::Closed);
        assert_eq!(record.effective, Authority::Closed);
        assert_eq!(
            decision
                .next
                .operations
                .get(&operation_key(SID, OPERATION))
                .and_then(|ledger| ledger.result),
            Some(OperationResult::Destroyed)
        );

        let mut missing = world;
        missing
            .effect_keys
            .retain(|id| !matches!(id.command(), Command::StopVm { .. }));
        assert_eq!(
            handle_publish_destroyed(&missing, SID),
            Err(PublicationReject::StopReceiptsMissing)
        );
    }

    #[test]
    fn compensated_failure_publishes_only_the_matching_final_failure() {
        let failure = crate::FailureKind::StorageFailure;
        let world = terminal_ready_world(
            OperationKind::Create,
            OperationStatus::Compensating(failure),
        );
        let decision =
            handle_publish_failure(&world, SID, failure).expect("publish compensated failure");
        assert_eq!(
            decision.event,
            PublicationEvent::PublishFailure(SID, failure)
        );
        assert_eq!(
            decision
                .next
                .operations
                .get(&operation_key(SID, OPERATION))
                .and_then(|ledger| ledger.result),
            Some(OperationResult::Failed(failure))
        );
        assert_eq!(
            handle_publish_failure(&world, SID, crate::FailureKind::InternalFailure),
            Err(PublicationReject::WrongOperationStatus)
        );
    }

    #[test]
    fn reserved_but_unregistered_startup_vm_cannot_vacuously_publish_failure() {
        let failure = crate::FailureKind::DeadlineExceeded;
        let mut world = terminal_ready_world(
            OperationKind::Create,
            OperationStatus::Compensating(failure),
        );
        world.vms = crate::Store::default();
        world.net_slots = crate::Store::default();
        world.resource_indexes.put(SID, ResourceIndex::default());
        assert_eq!(
            handle_publish_failure(&world, SID, failure),
            Err(PublicationReject::ResourcesNotClosed)
        );
    }

    // Frozen pre-refactor predicate used only as a regression oracle. This is
    // not a production implementation or a substitute for Lean refinement.
    fn original_planned_resources_closed(
        world: &World,
        sandbox: SandboxId,
        plan: &OperationPlan,
    ) -> bool {
        for vm_id in &plan.stop_vms {
            let Some(vm) = world.vms.get(vm_id) else {
                return false;
            };
            if vm.sandbox != sandbox
                || vm.generation != plan.generation
                || !matches!(vm.status, VmStatus::Stopped | VmStatus::Reclaimed)
                || !super::vm_has_stop_receipt(world, sandbox, plan.generation, *vm_id)
            {
                return false;
            }
        }
        for slot_id in &plan.close_slots {
            let Some(slot) = world.net_slots.get(slot_id) else {
                return false;
            };
            if slot.sandbox != sandbox || slot.generation != plan.generation || slot.active {
                return false;
            }
        }
        true
    }

    #[test]
    fn planned_resource_split_matches_original_guards_and_empty_lists() {
        let failure = crate::FailureKind::DeadlineExceeded;
        let original = terminal_ready_world(
            OperationKind::Create,
            OperationStatus::Compensating(failure),
        );
        let key = operation_key(SID, OPERATION);
        let original_plan = original.operation_plans.get(&key).expect("plan");
        // Every VM guard, both accepted statuses, both receipt sources and every
        // slot guard. Include simultaneous faults and faults at either position.
        for vm_case in 0..10 {
            for slot_case in 0..5 {
                let mut world = original.clone();
                let mut vm = *world.vms.get(&VM).expect("vm");
                match vm_case {
                    1 => vm.sandbox = SandboxId(99),
                    2 => vm.generation = 4,
                    3 => vm.status = VmStatus::Prepared,
                    4 => vm.status = VmStatus::Executing,
                    5 => vm.status = VmStatus::Frozen,
                    6 => vm.status = VmStatus::Reclaimed,
                    7 => world.effect_keys.clear(),
                    8 => {
                        world.effect_keys.clear();
                        acknowledge(
                            &mut world,
                            Command::AbortPrepare {
                                sandbox: SID,
                                epoch: 5,
                                generation: 3,
                                vm: VM,
                                slot: SLOT,
                            },
                            3,
                        );
                    }
                    _ => {}
                }
                world.vms.put(VM, vm);
                if vm_case == 9 {
                    world.vms = crate::Store::default();
                }
                let mut slot = world.net_slots.get(&SLOT).expect("slot").clone();
                match slot_case {
                    1 => slot.sandbox = SandboxId(99),
                    2 => slot.generation = 4,
                    3 => slot.active = true,
                    _ => {}
                }
                world.net_slots.put(SLOT, slot);
                if slot_case == 4 {
                    world.net_slots = crate::Store::default();
                }
                for vms in [vec![], vec![VM], vec![VM, VmId(99)], vec![VmId(99), VM]] {
                    for slots in [
                        vec![],
                        vec![SLOT],
                        vec![SLOT, NetSlotId(99)],
                        vec![NetSlotId(99), SLOT],
                    ] {
                        let mut plan = original_plan.clone();
                        plan.stop_vms = vms.clone();
                        plan.close_slots = slots;
                        let before = world.clone();
                        assert_eq!(
                            super::planned_resources_closed(&world, SID, &plan),
                            original_planned_resources_closed(&world, SID, &plan),
                            "vm={vm_case}, slot={slot_case}, plan={plan:?}"
                        );
                        assert_eq!(world, before);
                    }
                }
            }
        }
    }

    #[test]
    fn planned_resource_rejection_still_precedes_ledger_check() {
        let failure = crate::FailureKind::DeadlineExceeded;
        let mut world = terminal_ready_world(
            OperationKind::Create,
            OperationStatus::Compensating(failure),
        );
        let key = operation_key(SID, OPERATION);
        // No registered resources to fail the earlier index checks. The plan
        // still owns reservations, so its guard must precede a missing ledger.
        world.resource_indexes.put(SID, ResourceIndex::default());
        world.operations = crate::Store::default();
        let before = world.clone();
        assert_eq!(
            handle_publish_failure(&world, SID, failure),
            Err(PublicationReject::OperationUnavailable)
        );
        assert_eq!(world, before);
        world.vms = crate::Store::default();
        assert_eq!(
            handle_publish_failure(&world, SID, failure),
            Err(PublicationReject::ResourcesNotClosed)
        );
        let mut plan = world.operation_plans.get(&key).expect("plan").clone();
        plan.stop_vms.clear();
        world.operation_plans.put(key, plan);
        world.net_slots = crate::Store::default();
        assert_eq!(
            handle_publish_failure(&world, SID, failure),
            Err(PublicationReject::ResourcesNotClosed)
        );
        assert_eq!(
            handle_publish_failure(&world, SID, crate::FailureKind::InternalFailure),
            Err(PublicationReject::WrongOperationStatus)
        );
    }

    #[test]
    fn unified_publication_dispatch_preserves_guard_priority() {
        let mut world = running_ready_world();
        world.controller_up = false;
        assert_eq!(
            handle_publication(&world, PublicationInput::Running(SID)),
            Err(PublicationReject::ControllerDown)
        );
    }

    #[test]
    fn paused_publication_checks_the_exact_durable_source() {
        let source = RestoreSource {
            snapshot: SnapshotId(31),
            manifest: SnapshotManifestRef {
                digest: ManifestDigest(37),
            },
        };
        let operation = operation(OperationKind::Pause, OperationStatus::Pending);
        let mut world = terminal_ready_world(OperationKind::Pause, OperationStatus::Pending);
        let key = operation_key(SID, OPERATION);
        let mut plan = world.operation_plans.get(&key).expect("plan").clone();
        plan.target_snapshot = Some(source);
        plan.capture = crate::capture::CaptureHandoff::accepted().register_bundle();
        world.operation_plans.put(key, plan);
        let mut paused = record(Phase::Pausing, false, operation);
        paused.terminal = false;
        world.sandboxes.put(SID, paused);
        world.worker_fences.put(
            SID,
            WorkerFence {
                epoch: 5,
                terminal: false,
            },
        );
        world.snapshots.put(
            source.snapshot,
            Snapshot {
                owner: OWNER,
                sandbox: SID,
                source_generation: 3,
                workspace: WORKSPACE,
                manifest: source.manifest,
                stage: SnapshotStage::Durable,
                immutable: true,
            },
        );
        acknowledge(
            &mut world,
            Command::InstallFence {
                sandbox: SID,
                epoch: 5,
                terminal: false,
            },
            3,
        );
        acknowledge(
            &mut world,
            Command::FreezeVm {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                vm: VM,
            },
            4,
        );
        acknowledge(
            &mut world,
            Command::SealSnapshot {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                vm: VM,
                workspace: WORKSPACE,
                snapshot: source.snapshot,
                manifest: source.manifest,
            },
            5,
        );
        acknowledge(
            &mut world,
            Command::CommitSnapshot {
                sandbox: SID,
                epoch: 5,
                generation: 3,
                snapshot: source.snapshot,
                manifest: source.manifest,
            },
            6,
        );
        let decision = handle_publish_paused(&world, SID, source).expect("publish paused");
        let record = decision.next.sandboxes.get(&SID).expect("sandbox");
        assert_eq!(record.phase, Phase::Paused);
        assert_eq!(record.durable_snapshot, Some(source));
        assert_eq!(record.desired, record.effective);
    }
}
