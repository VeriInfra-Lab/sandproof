//! Controller-metadata retirement after terminal resource and transport closure.
//!
//! This module does not observe the host, gateway readers, storage pins or wall
//! clock. Its certificate is an explicit trusted-adapter precondition, not a
//! substitute for physical cleanup evidence. Production service admission of
//! these inputs remains disabled until that transactional adapter exists.

use crate::{
    Authority, Command, EffectId, EffectOrigin, EffectOutcome, EffectStatus, Epoch, FsId,
    LogicalEffectRecord, LogicalEffectResult, NetSlotId, OperationId, OperationInput, OperationKey,
    OperationKind, OperationLedgerRecord, OperationResult, OperationStatus, Phase, SandboxId,
    SandboxRecord, SnapshotId, TenantId, VmId, VmStatus, WorkerFence, World,
};

/// A named, explicitly trusted adapter attestation. The pure core cannot
/// establish this condition by observing the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum QuiescenceStatus {
    Unconfirmed,
    Confirmed,
}

/// The trusted adapter must establish all four guarantees for this exact
/// terminal identity, and keep them true through the atomic retirement commit.
/// Merely constructing this value does not establish any physical fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct QuiescenceCertificate {
    pub sandbox: SandboxId,
    pub owner: TenantId,
    pub epoch: Epoch,
    pub operation: OperationId,
    pub workspace: FsId,
    /// All external workers and deliveries for this sandbox have drained.
    pub worker_drained: QuiescenceStatus,
    /// Old data-channel connections and relays have finished or been revoked.
    pub data_connections_drained: QuiescenceStatus,
    /// Exact VM, network and workspace storage cleanup has been confirmed.
    pub host_resources_reclaimed: QuiescenceStatus,
    /// Gateway readers, snapshots and other external consumers hold no pins.
    pub external_pins_released: QuiescenceStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RetireRequest {
    pub sandbox: SandboxId,
    pub owner: TenantId,
    pub epoch: Epoch,
    pub operation: OperationId,
    pub retain_until: u64,
    pub quiescence: QuiescenceCertificate,
}

/// An exact, authorization-preserving query/replay result. These records grant
/// no worker authority and do not pin the old physical workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RetainedOperation {
    pub owner: TenantId,
    pub ledger: OperationLedgerRecord,
    pub expires_at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetirementReject {
    Disabled,
    ControllerDown,
    SandboxNotFound,
    CertificateMismatch,
    NotTerminal,
    LedgerMismatch,
    ResourcesNotReclaimed,
    UnresolvedEffects,
    SnapshotHistoryUnsupported,
    DependencyNotClosed,
    RetentionExpired,
    RetentionCapacity,
    IdentityBarrierMissing,
}

fn terminal_ledger(key: OperationKey, ledger: &OperationLedgerRecord) -> bool {
    if key.sandbox != ledger.sandbox || key.operation != ledger.operation.id {
        return false;
    }
    match (
        ledger.operation.kind,
        ledger.operation.status,
        ledger.result,
    ) {
        (_, OperationStatus::Failed(expected), Some(OperationResult::Failed(observed))) => {
            expected == observed
        }
        (
            OperationKind::Create | OperationKind::Resume,
            OperationStatus::Succeeded,
            Some(OperationResult::Running(_)),
        )
        | (OperationKind::Pause, OperationStatus::Succeeded, Some(OperationResult::Paused(_)))
        | (OperationKind::Kill, OperationStatus::Succeeded, Some(OperationResult::Destroyed)) => {
            true
        }
        _ => false,
    }
}

fn successful_exact(world: &World, id: &EffectId) -> bool {
    let Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(result))) =
        world.effects.get_copy(id)
    else {
        return false;
    };
    world
        .logical_effects
        .get(&id.logical)
        .is_some_and(|logical| {
            logical.current_attempt == id.attempt
                && logical.result
                    == Some(LogicalEffectResult::Succeeded {
                        attempt: id.attempt,
                        result,
                    })
        })
}

fn successful_command(world: &World, command: &Command) -> bool {
    world
        .effects
        .entries()
        .iter()
        .any(|(id, _)| id.command() == command && successful_exact(world, id))
}

fn reclaimed_vm_receipt(world: &World, sandbox: SandboxId, vm: VmId, generation: u64) -> bool {
    successful_command(
        world,
        &Command::CleanupVm {
            sandbox,
            generation,
            vm,
        },
    ) || world.effects.entries().iter().any(|(id, _)| {
        matches!(id.command(), Command::AbortPrepare {
                sandbox: found, generation: found_generation, vm: found_vm, ..
            } if *found == sandbox && *found_generation == generation && *found_vm == vm)
            && successful_exact(world, id)
    })
}

fn reclaimed_slot_receipt(
    world: &World,
    sandbox: SandboxId,
    slot: NetSlotId,
    generation: u64,
    vm: VmId,
) -> bool {
    successful_command(
        world,
        &Command::CleanupSlot {
            sandbox,
            generation,
            slot,
        },
    ) || world.effects.entries().iter().any(|(id, _)| {
        matches!(id.command(), Command::AbortPrepare {
                sandbox: found, generation: found_generation, vm: found_vm, slot: found_slot, ..
            } if *found == sandbox && *found_generation == generation
                && *found_vm == vm && *found_slot == slot)
            && successful_exact(world, id)
    })
}

#[derive(Default)]
struct OwnedIds {
    vms: Vec<VmId>,
    slots: Vec<NetSlotId>,
    snapshots: Vec<SnapshotId>,
}

fn push_unique<T: PartialEq>(values: &mut Vec<T>, value: T) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn owned_ids(world: &World, sandbox: SandboxId) -> OwnedIds {
    let mut ids = OwnedIds::default();
    for (id, vm) in world.vms.entries() {
        if vm.sandbox == sandbox {
            push_unique(&mut ids.vms, *id);
        }
    }
    for (id, slot) in world.net_slots.entries() {
        if slot.sandbox == sandbox {
            push_unique(&mut ids.slots, *id);
        }
    }
    for (id, snapshot) in world.snapshots.entries() {
        if snapshot.sandbox == sandbox {
            push_unique(&mut ids.snapshots, *id);
        }
    }
    for (key, plan) in world.operation_plans.entries() {
        if key.sandbox == sandbox {
            if let Some(vm) = plan.vm {
                push_unique(&mut ids.vms, vm);
            }
            if let Some(slot) = plan.slot {
                push_unique(&mut ids.slots, slot);
            }
            for vm in &plan.stop_vms {
                push_unique(&mut ids.vms, *vm);
            }
            for slot in &plan.close_slots {
                push_unique(&mut ids.slots, *slot);
            }
            if let Some(source) = plan.restore_from {
                push_unique(&mut ids.snapshots, source.snapshot);
            }
            if let Some(source) = plan.target_snapshot {
                push_unique(&mut ids.snapshots, source.snapshot);
            }
        }
    }
    ids
}

fn authority_references(
    authority: &Authority,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    match authority {
        Authority::Closed => false,
        Authority::Running {
            sandbox: found,
            vm,
            workspace: found_workspace,
            slot,
            ..
        } => {
            *found == sandbox
                || *found_workspace == workspace
                || ids.vms.contains(vm)
                || ids.slots.contains(slot)
        }
        Authority::Paused {
            sandbox: found,
            source,
        } => *found == sandbox || ids.snapshots.contains(&source.snapshot),
    }
}

fn command_references(command: &Command, workspace: FsId, ids: &OwnedIds) -> bool {
    match command {
        Command::InstallFence { .. } => false,
        Command::PrepareVm {
            vm, slot, source, ..
        } => {
            ids.vms.contains(vm)
                || ids.slots.contains(slot)
                || source.is_some_and(|source| ids.snapshots.contains(&source.snapshot))
        }
        Command::ConfigureNetwork { vm, slot, .. } | Command::AbortPrepare { vm, slot, .. } => {
            ids.vms.contains(vm) || ids.slots.contains(slot)
        }
        Command::ActivateVm { vm, .. }
        | Command::FreezeVm { vm, .. }
        | Command::StopVm { vm, .. }
        | Command::CleanupVm { vm, .. } => ids.vms.contains(vm),
        Command::ApplyFilePolicy {
            workspace: found, ..
        } => *found == workspace,
        Command::SealSnapshot {
            vm,
            workspace: found,
            snapshot,
            ..
        } => ids.vms.contains(vm) || *found == workspace || ids.snapshots.contains(snapshot),
        Command::CommitSnapshot { snapshot, .. } => ids.snapshots.contains(snapshot),
        Command::CleanupSlot { slot, .. } => ids.slots.contains(slot),
    }
}

fn ledger_references(ledger: &OperationLedgerRecord, workspace: FsId, ids: &OwnedIds) -> bool {
    match &ledger.input {
        OperationInput::Create {
            workspace: found, ..
        } if *found == workspace => return true,
        OperationInput::Resume { source } if ids.snapshots.contains(&source.snapshot) => {
            return true;
        }
        _ => {}
    }
    matches!(ledger.result, Some(OperationResult::Paused(source))
        if ids.snapshots.contains(&source.snapshot))
}

fn origin_sandbox(logical: &LogicalEffectRecord) -> SandboxId {
    match logical.origin {
        EffectOrigin::Operation(key) => key.sandbox,
        EffectOrigin::Maintenance(sandbox) => sandbox,
    }
}

fn external_dependencies_closed(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    ids: &OwnedIds,
) -> bool {
    let workspace = record.workspace;
    // Preserve the original table order and stop at its first failed check.
    // Each traversal has one loop, avoiding the fixed translator's movement
    // of later loops into an earlier loop's exit branch.
    foreign_sandbox_dependencies_closed(world, sandbox, workspace, ids)
        && foreign_ledger_dependencies_closed(world, sandbox, workspace, ids)
        && foreign_plan_dependencies_closed(world, sandbox, ids)
        && foreign_resource_indexes_closed(world, sandbox, ids)
        && foreign_resource_links_closed(world, sandbox, workspace, ids)
        && foreign_logical_dependencies_closed(world, sandbox, workspace, ids)
        && foreign_stop_coverages_closed(world, sandbox)
        && foreign_capture_ends_closed(world, sandbox, workspace, ids)
        && foreign_capture_drains_closed(world, sandbox, workspace, ids)
        && foreign_effect_dependencies_closed(world, sandbox, workspace, ids)
        && foreign_effect_keys_closed(world, sandbox, workspace, ids)
        && foreign_inflight_dependencies_closed(world, sandbox, workspace, ids)
}

fn foreign_sandbox_dependencies_closed(
    world: &World,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    for (other, stored) in world.sandboxes.entries() {
        if *other != sandbox
            && (stored.workspace == workspace
                || stored.current_vm.is_some_and(|vm| ids.vms.contains(&vm))
                || stored
                    .net_slot
                    .is_some_and(|slot| ids.slots.contains(&slot))
                || stored
                    .durable_snapshot
                    .is_some_and(|source| ids.snapshots.contains(&source.snapshot))
                || stored
                    .restore_from
                    .is_some_and(|source| ids.snapshots.contains(&source.snapshot))
                || authority_references(&stored.desired, sandbox, workspace, ids)
                || authority_references(&stored.effective, sandbox, workspace, ids))
        {
            return false;
        }
    }
    true
}

fn foreign_ledger_dependencies_closed(
    world: &World,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    for (key, ledger) in world.operations.entries() {
        if key.sandbox != sandbox
            && (ledger.sandbox == sandbox || ledger_references(ledger, workspace, ids))
        {
            return false;
        }
    }
    true
}

fn foreign_plan_dependencies_closed(world: &World, sandbox: SandboxId, ids: &OwnedIds) -> bool {
    for (key, plan) in world.operation_plans.entries() {
        if key.sandbox != sandbox
            && (plan.key.sandbox == sandbox
                || plan.vm.is_some_and(|vm| ids.vms.contains(&vm))
                || plan.slot.is_some_and(|slot| ids.slots.contains(&slot))
                || plan.stop_vms.iter().any(|vm| ids.vms.contains(vm))
                || plan.close_slots.iter().any(|slot| ids.slots.contains(slot))
                || plan
                    .restore_from
                    .is_some_and(|source| ids.snapshots.contains(&source.snapshot))
                || plan
                    .target_snapshot
                    .is_some_and(|source| ids.snapshots.contains(&source.snapshot)))
        {
            return false;
        }
    }
    true
}

fn foreign_resource_indexes_closed(world: &World, sandbox: SandboxId, ids: &OwnedIds) -> bool {
    for (other, index) in world.resource_indexes.entries() {
        if *other != sandbox
            && (index.vms.iter().any(|vm| ids.vms.contains(vm))
                || index.slots.iter().any(|slot| ids.slots.contains(slot))
                || index
                    .snapshots
                    .iter()
                    .any(|snapshot| ids.snapshots.contains(snapshot)))
        {
            return false;
        }
    }
    true
}

fn foreign_resource_links_closed(
    world: &World,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    if world
        .net_slots
        .entries()
        .iter()
        .any(|(_, slot)| slot.sandbox != sandbox && ids.vms.contains(&slot.vm))
        || world
            .snapshots
            .entries()
            .iter()
            .any(|(_, snapshot)| snapshot.sandbox != sandbox && snapshot.workspace == workspace)
        || world
            .applied_file_policies
            .entries()
            .iter()
            .any(|(other, policy)| {
                *other != sandbox && (policy.sandbox == sandbox || policy.workspace == workspace)
            })
    {
        return false;
    }
    true
}

fn foreign_logical_dependencies_closed(
    world: &World,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    for (id, logical) in world.logical_effects.entries() {
        if id.command.sandbox() != sandbox
            && (origin_sandbox(logical) == sandbox
                || matches!(logical.result, Some(LogicalEffectResult::RetiredByStop { coverage, .. })
                    if coverage.operation.sandbox == sandbox)
                || command_references(&id.command, workspace, ids))
        {
            return false;
        }
    }
    true
}

fn end_references(
    receipt: crate::capture::CaptureWorkEndReceipt,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    receipt
        .end
        .seal()
        .is_some_and(|seal| seal.workspace == workspace || ids.snapshots.contains(&seal.snapshot))
}
fn foreign_capture_ends_closed(
    world: &World,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    !world.logical_effects.entries().iter().any(|(id,logical)| {
        id.command.sandbox() != sandbox && matches!(logical.result,
            Some(LogicalEffectResult::RetiredByCaptureEnd {end_receipt,..}) if end_references(end_receipt,workspace,ids))
    }) && !world.effects.entries().iter().any(|(id,status)| {
        id.command().sandbox() != sandbox && matches!(status,
            EffectStatus::RetiredByCaptureEnd(receipt) if end_references(*receipt,workspace,ids))
    })
}

fn drain_references(
    coverage: crate::capture::drain::CaptureDrainCoverage,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    coverage.identity.key.sandbox == sandbox
        || coverage.stop.operation.sandbox == sandbox
        || ids.vms.contains(&coverage.identity.vm)
        || coverage.end_receipt.seal.workspace == workspace
        || ids.snapshots.contains(&coverage.end_receipt.seal.snapshot)
}
fn foreign_capture_drains_closed(
    world: &World,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    !world.logical_effects.entries().iter().any(|(id,logical)|id.command.sandbox() != sandbox
        && matches!(logical.result,Some(LogicalEffectResult::RetiredByCaptureDrain {coverage,..}) if drain_references(coverage,sandbox,workspace,ids)))
        && !world.effects.entries().iter().any(|(id,status)|id.command().sandbox() != sandbox
            && matches!(status,EffectStatus::RetiredByCaptureDrain(coverage) if drain_references(*coverage,sandbox,workspace,ids)))
}

fn foreign_stop_coverages_closed(world: &World, sandbox: SandboxId) -> bool {
    if world.effects.entries().iter().any(|(id, status)| id.command().sandbox() != sandbox
        && matches!(status, EffectStatus::RetiredByStop(coverage) if coverage.operation.sandbox == sandbox)) {
        return false;
    }
    true
}

fn foreign_effect_dependencies_closed(
    world: &World,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    for (id, _) in world.effects.entries() {
        if id.command().sandbox() != sandbox && command_references(id.command(), workspace, ids) {
            return false;
        }
    }
    true
}

fn foreign_effect_keys_closed(
    world: &World,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    for id in &world.effect_keys {
        if id.command().sandbox() != sandbox && command_references(id.command(), workspace, ids) {
            return false;
        }
    }
    true
}

fn foreign_inflight_dependencies_closed(
    world: &World,
    sandbox: SandboxId,
    workspace: FsId,
    ids: &OwnedIds,
) -> bool {
    for id in &world.inflight {
        if id.command().sandbox() != sandbox && command_references(id.command(), workspace, ids) {
            return false;
        }
    }
    true
}

fn check_ledgers(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
) -> Result<usize, RetirementReject> {
    let operation = record
        .current_operation
        .ok_or(RetirementReject::LedgerMismatch)?;
    let key = OperationKey {
        sandbox,
        operation: operation.id,
    };
    let current = world
        .operations
        .get(&key)
        .ok_or(RetirementReject::LedgerMismatch)?;
    if current.operation != operation
        || operation.epoch != record.epoch
        || world
            .operation_plans
            .get(&key)
            .is_none_or(|plan| plan.generation != record.generation)
        || !terminal_ledger(key, current)
        || !matches!(
            (operation.kind, operation.status, current.result),
            (
                OperationKind::Kill,
                OperationStatus::Succeeded,
                Some(OperationResult::Destroyed)
            ) | (
                OperationKind::Create | OperationKind::Pause | OperationKind::Resume,
                OperationStatus::Failed(_),
                Some(OperationResult::Failed(_))
            )
        )
    {
        return Err(RetirementReject::LedgerMismatch);
    }
    let count = check_historical_ledgers(world, sandbox, record)?;
    check_plan_ledgers(world, sandbox)?;
    Ok(count)
}

// Preserve the original ledger-before-plan order and each first rejection.
// Keep sequential early-return traversals in separate extraction functions.
fn check_historical_ledgers(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
) -> Result<usize, RetirementReject> {
    let mut count = 0;
    for (key, ledger) in world.operations.entries() {
        if key.sandbox != sandbox {
            continue;
        }
        if !terminal_ledger(*key, ledger) || ledger.operation.epoch > record.epoch {
            return Err(RetirementReject::LedgerMismatch);
        }
        if (matches!(ledger.input, OperationInput::Resume { .. })
            || matches!(ledger.result, Some(OperationResult::Paused(_))))
            && !crate::checkpoint::valid_snapshot_retirement_history(world, sandbox)
        {
            return Err(RetirementReject::SnapshotHistoryUnsupported);
        }
        match (&ledger.operation.kind, &ledger.input) {
            (
                OperationKind::Create,
                OperationInput::Create {
                    workspace,
                    network,
                    file_policy,
                },
            ) if *workspace == record.workspace
                && *network == record.policy
                && *file_policy == record.file_policy => {}
            (OperationKind::Pause, OperationInput::Pause { .. })
            | (OperationKind::Kill, OperationInput::Kill { .. })
            | (OperationKind::Resume, OperationInput::Resume { .. }) => {}
            _ => return Err(RetirementReject::LedgerMismatch),
        }
        let plan = world
            .operation_plans
            .get(key)
            .ok_or(RetirementReject::LedgerMismatch)?;
        if plan.key != *key || plan.epoch != ledger.operation.epoch {
            return Err(RetirementReject::LedgerMismatch);
        }
        count += 1;
    }
    Ok(count)
}

fn check_plan_ledgers(world: &World, sandbox: SandboxId) -> Result<(), RetirementReject> {
    for (key, plan) in world.operation_plans.entries() {
        if key.sandbox == sandbox {
            if plan.key != *key || world.operations.get(key).is_none() {
                return Err(RetirementReject::LedgerMismatch);
            }
            if (plan.restore_from.is_some() || plan.target_snapshot.is_some())
                && !crate::checkpoint::valid_snapshot_retirement_history(world, sandbox)
            {
                return Err(RetirementReject::SnapshotHistoryUnsupported);
            }
        }
    }
    Ok(())
}

fn check_resources(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    ids: &OwnedIds,
) -> Result<(), RetirementReject> {
    if (record.durable_snapshot.is_some()
        || record.restore_from.is_some()
        || !ids.snapshots.is_empty()
        || !world.resource_index(sandbox).snapshots.is_empty())
        && (world.sandbox_record(sandbox) != Some(record)
            || !crate::checkpoint::valid_snapshot_retirement_history(world, sandbox))
    {
        return Err(RetirementReject::SnapshotHistoryUnsupported);
    }
    let filesystem = world
        .filesystems
        .get(&record.workspace)
        .ok_or(RetirementReject::LedgerMismatch)?;
    if filesystem.sandbox != sandbox
        || filesystem.owner != record.owner
        || world
            .filesystems
            .entries()
            .iter()
            .any(|(id, fs)| fs.sandbox == sandbox && *id != record.workspace)
    {
        return Err(RetirementReject::LedgerMismatch);
    }
    if world
        .applied_file_policies
        .get(&sandbox)
        .is_some_and(|policy| {
            policy.sandbox != sandbox
                || policy.workspace != record.workspace
                || policy.policy != record.file_policy
        })
    {
        return Err(RetirementReject::LedgerMismatch);
    }
    if ids.vms.iter().any(|id| id.0 >= world.next_vm_id)
        || ids.slots.iter().any(|id| id.0 >= world.next_slot_id)
    {
        return Err(RetirementReject::IdentityBarrierMissing);
    }
    let index = world.resource_index(sandbox);
    check_reclaimed_vms(world, sandbox, record, &index)?;
    check_vm_index_membership(world, sandbox, &index)?;
    check_reclaimed_slots(world, sandbox, record, &index)?;
    check_slot_index_membership(world, sandbox, &index)?;
    check_reserved_vms(world, sandbox, record, ids)?;
    check_reserved_slots(world, sandbox, record, ids)
}

fn check_reclaimed_vms(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    index: &crate::ResourceIndex,
) -> Result<(), RetirementReject> {
    for (id, vm) in world.vms.entries() {
        if vm.sandbox == sandbox
            && (vm.owner != record.owner
                || vm.status != VmStatus::Reclaimed
                || !index.vms.contains(id)
                || !reclaimed_vm_receipt(world, sandbox, *id, vm.generation))
        {
            return Err(RetirementReject::ResourcesNotReclaimed);
        }
    }
    Ok(())
}

fn check_vm_index_membership(
    world: &World,
    sandbox: SandboxId,
    index: &crate::ResourceIndex,
) -> Result<(), RetirementReject> {
    for id in &index.vms {
        if world.vms.get(id).is_none_or(|vm| vm.sandbox != sandbox) {
            return Err(RetirementReject::LedgerMismatch);
        }
    }
    Ok(())
}

fn check_reclaimed_slots(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    index: &crate::ResourceIndex,
) -> Result<(), RetirementReject> {
    for (id, slot) in world.net_slots.entries() {
        if slot.sandbox == sandbox
            && (slot.owner != record.owner
                || slot.active
                || !slot.reclaimed
                || !index.slots.contains(id)
                || world
                    .vms
                    .get(&slot.vm)
                    .is_none_or(|vm| vm.sandbox != sandbox || vm.generation != slot.generation)
                || !reclaimed_slot_receipt(world, sandbox, *id, slot.generation, slot.vm))
        {
            return Err(RetirementReject::ResourcesNotReclaimed);
        }
    }
    Ok(())
}

fn check_slot_index_membership(
    world: &World,
    sandbox: SandboxId,
    index: &crate::ResourceIndex,
) -> Result<(), RetirementReject> {
    for id in &index.slots {
        if world
            .net_slots
            .get(id)
            .is_none_or(|slot| slot.sandbox != sandbox)
        {
            return Err(RetirementReject::LedgerMismatch);
        }
    }
    Ok(())
}

fn check_reserved_vms(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    ids: &OwnedIds,
) -> Result<(), RetirementReject> {
    // Plans may reserve IDs without ever issuing a worker command. Absent rows
    // are safe only because all effect attempts are separately checked below.
    for vm in &ids.vms {
        if world
            .vms
            .get(vm)
            .is_some_and(|vm| vm.sandbox != sandbox || vm.owner != record.owner)
        {
            return Err(RetirementReject::DependencyNotClosed);
        }
    }
    Ok(())
}

fn check_reserved_slots(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
    ids: &OwnedIds,
) -> Result<(), RetirementReject> {
    for slot in &ids.slots {
        if world
            .net_slots
            .get(slot)
            .is_some_and(|slot| slot.sandbox != sandbox || slot.owner != record.owner)
        {
            return Err(RetirementReject::DependencyNotClosed);
        }
    }
    Ok(())
}

fn origin_bound(world: &World, command: &Command, logical: &LogicalEffectRecord) -> bool {
    let sandbox = command.sandbox();
    match logical.origin {
        EffectOrigin::Maintenance(found) => {
            if found != sandbox
                || !matches!(
                    command,
                    Command::CleanupVm { .. } | Command::CleanupSlot { .. }
                )
            {
                return false;
            }
        }
        EffectOrigin::Operation(key) => {
            let Some(plan) = world.operation_plans.get(&key) else {
                return false;
            };
            if key.sandbox != sandbox || plan.key != key || command.epoch() != Some(plan.epoch) {
                return false;
            }
            let matches_start = |generation, vm, slot: Option<NetSlotId>| {
                plan.generation == generation
                    && plan.vm == Some(vm)
                    && slot.is_none_or(|slot| plan.slot == Some(slot))
            };
            match command {
                Command::PrepareVm {
                    generation,
                    vm,
                    slot,
                    source,
                    ..
                } => {
                    if !matches_start(*generation, *vm, Some(*slot)) || *source != plan.restore_from
                    {
                        return false;
                    }
                }
                Command::ConfigureNetwork {
                    generation,
                    vm,
                    slot,
                    ..
                }
                | Command::AbortPrepare {
                    generation,
                    vm,
                    slot,
                    ..
                } => {
                    if !matches_start(*generation, *vm, Some(*slot)) {
                        return false;
                    }
                }
                Command::ActivateVm { generation, vm, .. } => {
                    if !matches_start(*generation, *vm, None)
                        && !crate::capture::continuation::activation_belongs_to_continuation(
                            plan, command,
                        )
                    {
                        return false;
                    }
                }
                Command::ApplyFilePolicy { generation, .. } => {
                    if plan.generation != *generation {
                        return false;
                    }
                }
                Command::StopVm { vm, .. } => {
                    if !plan.stop_vms.contains(vm) {
                        return false;
                    }
                }
                Command::InstallFence { .. } => {}
                Command::FreezeVm { .. }
                | Command::SealSnapshot { .. }
                | Command::CommitSnapshot { .. } => {
                    if !crate::checkpoint::valid_snapshot_retirement_history(world, sandbox) {
                        return false;
                    }
                }
                Command::CleanupVm { .. } | Command::CleanupSlot { .. } => return false,
            }
        }
    }
    true
}

fn command_bound(
    world: &World,
    command: &Command,
    logical: &LogicalEffectRecord,
    record: &SandboxRecord,
) -> bool {
    if !origin_bound(world, command, logical) {
        return false;
    }
    let sandbox = command.sandbox();
    let vm_matches = |id: &VmId, generation| {
        world.vms.get(id).is_some_and(|vm| {
            vm.sandbox == sandbox && vm.owner == record.owner && vm.generation == generation
        })
    };
    let slot_matches = |id: &NetSlotId, generation, vm: Option<VmId>| {
        world.net_slots.get(id).is_some_and(|slot| {
            slot.sandbox == sandbox
                && slot.owner == record.owner
                && slot.generation == generation
                && vm.is_none_or(|vm| slot.vm == vm)
        })
    };
    match command {
        Command::InstallFence { epoch, .. } => *epoch <= record.epoch,
        Command::PrepareVm {
            generation,
            vm,
            slot,
            ..
        }
        | Command::AbortPrepare {
            generation,
            vm,
            slot,
            ..
        } => {
            // The plan above binds never-materialized reservations. No unknown
            // or succeeded prepare may be hidden by absent resource rows.
            (world.vms.get(vm).is_none() || vm_matches(vm, *generation))
                && (world.net_slots.get(slot).is_none()
                    || slot_matches(slot, *generation, Some(*vm)))
                && (!matches!(
                    command,
                    Command::PrepareVm {
                        source: Some(_),
                        ..
                    }
                ) || crate::checkpoint::valid_snapshot_retirement_history(world, sandbox))
        }
        Command::ConfigureNetwork {
            generation,
            vm,
            slot,
            policy,
            ..
        } => {
            vm_matches(vm, *generation)
                && slot_matches(slot, *generation, Some(*vm))
                && *policy == record.policy
        }
        Command::ActivateVm { generation, vm, .. }
        | Command::FreezeVm { generation, vm, .. }
        | Command::StopVm { generation, vm, .. }
        | Command::CleanupVm { generation, vm, .. } => vm_matches(vm, *generation),
        Command::ApplyFilePolicy {
            workspace, policy, ..
        } => *workspace == record.workspace && *policy == record.file_policy,
        Command::CleanupSlot {
            generation, slot, ..
        } => slot_matches(slot, *generation, None),
        Command::SealSnapshot { .. } | Command::CommitSnapshot { .. } => {
            crate::checkpoint::valid_snapshot_retirement_history(world, sandbox)
        }
    }
}

fn retired_stop_valid(world: &World, id: &EffectId, coverage: crate::StopCoverage) -> bool {
    let Some(command) = crate::terminal::activation_stop_covered(id, coverage) else {
        return false;
    };
    if command.epoch() != Some(coverage.epoch) || coverage.operation.sandbox != command.sandbox() {
        return false;
    }
    let stop = EffectId {
        logical: crate::LogicalEffectId { command },
        attempt: coverage.attempt,
    };
    world.effects.get_copy(&stop)
        == Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(
            coverage.result,
        )))
        && successful_exact(world, &stop)
        && world
            .logical_effects
            .get(&stop.logical)
            .is_some_and(|logical| logical.origin == EffectOrigin::Operation(coverage.operation))
        && world
            .logical_effects
            .get(&id.logical)
            .is_some_and(|logical| {
                crate::terminal::activation_coverage_origin(world, id, coverage)
                    && logical.current_attempt == id.attempt
                    && logical.result
                        == Some(LogicalEffectResult::RetiredByStop {
                            attempt: id.attempt,
                            coverage,
                        })
            })
}

fn obsolete_committed(world: &World, id: &EffectId, logical: &LogicalEffectRecord) -> bool {
    if logical.current_attempt != id.attempt
        || logical.result.is_some()
        || !world.inflight.contains(id)
    {
        return false;
    }
    let candidate = match crate::effect::handle_effect_dispatch(world, id.clone()) {
        Ok(candidate) => candidate,
        Err(crate::EffectReject::NotDispatchable) => {
            // Current attempt, no result and queue membership were checked
            // above. The same terminal guard forbids all future issuance;
            // unlike Dispatched work this committed action has no host effect.
            return crate::effect::startup_dispatch_revoked(world, id.command())
                || crate::capture::continuation::capture_dispatch_permanently_revoked(world, id);
        }
        Err(_) => return false,
    };
    // Semantic ineligibility is checked after all ledger/queue/resource binding
    // checks. A missing queue entry or corrupt plan is not quiescence evidence.
    matches!(
        crate::worker::handle_worker(&candidate.next, id),
        Err(crate::WorkerReject::WrongPhase
            | crate::WorkerReject::Terminal
            | crate::WorkerReject::IdentityMismatch
            | crate::WorkerReject::ResourceNotFound
            | crate::WorkerReject::ResultAlreadyPresent)
    )
}

fn check_effects(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
) -> Result<(), RetirementReject> {
    check_effect_membership(world, sandbox, &world.effect_keys)?;
    check_effect_membership(world, sandbox, &world.inflight)?;
    check_logical_effects(world, sandbox, record)?;
    check_effect_attempts(world, sandbox)
}

fn check_effect_membership(
    world: &World,
    sandbox: SandboxId,
    ids: &[EffectId],
) -> Result<(), RetirementReject> {
    for id in ids {
        if id.command().sandbox() == sandbox && !world.effects.contains_key(id) {
            return Err(RetirementReject::LedgerMismatch);
        }
    }
    Ok(())
}

fn check_logical_effects(
    world: &World,
    sandbox: SandboxId,
    record: &SandboxRecord,
) -> Result<(), RetirementReject> {
    for (logical_id, logical) in world.logical_effects.entries() {
        if logical_id.command.sandbox() != sandbox {
            continue;
        }
        let id = EffectId {
            logical: logical_id.clone(),
            attempt: logical.current_attempt,
        };
        let status = world
            .effects
            .get_copy(&id)
            .ok_or(RetirementReject::LedgerMismatch)?;
        if !command_bound(world, &logical_id.command, logical, record) {
            return Err(RetirementReject::LedgerMismatch);
        }
        let result_matches = match (logical.result, status) {
            (
                None,
                EffectStatus::Committed | EffectStatus::Acknowledged(EffectOutcome::Failed(_)),
            ) => true,
            (
                Some(LogicalEffectResult::Succeeded { attempt, result }),
                EffectStatus::Acknowledged(EffectOutcome::Succeeded(observed)),
            ) => attempt == id.attempt && result == observed,
            (
                Some(LogicalEffectResult::Failed { attempt, failure }),
                EffectStatus::Acknowledged(EffectOutcome::Failed(observed)),
            ) => attempt == id.attempt && failure == observed,
            (
                Some(LogicalEffectResult::RetiredByStop { attempt, coverage }),
                EffectStatus::RetiredByStop(stored),
            ) => {
                attempt == id.attempt
                    && coverage == stored
                    && retired_stop_valid(world, &id, coverage)
            }
            (
                Some(LogicalEffectResult::RetiredByCaptureEnd {
                    attempt,
                    end_receipt,
                }),
                EffectStatus::RetiredByCaptureEnd(stored),
            ) => {
                attempt == id.attempt
                    && end_receipt == stored
                    && crate::capture::continuation::retired_capture_end_valid(world, &id, stored)
            }
            (
                Some(LogicalEffectResult::RetiredByCaptureDrain { attempt, coverage }),
                EffectStatus::RetiredByCaptureDrain(stored),
            ) => {
                attempt == id.attempt
                    && coverage == stored
                    && crate::capture::drain::retired_capture_drain_valid(world, &id, stored)
            }
            (_, EffectStatus::Dispatched | EffectStatus::OutcomeUnknown) => {
                return Err(RetirementReject::UnresolvedEffects);
            }
            _ => false,
        };
        if !result_matches {
            return Err(RetirementReject::LedgerMismatch);
        }
    }
    Ok(())
}

fn is_acknowledged_success(status: EffectStatus) -> bool {
    matches!(
        status,
        EffectStatus::Acknowledged(EffectOutcome::Succeeded(_))
    )
}

fn check_effect_attempts(world: &World, sandbox: SandboxId) -> Result<(), RetirementReject> {
    for (id, status) in world.effects.entries() {
        if id.command().sandbox() != sandbox {
            continue;
        }
        let logical = world
            .logical_effects
            .get(&id.logical)
            .ok_or(RetirementReject::LedgerMismatch)?;
        if !world.effect_keys.contains(id) || id.attempt > logical.current_attempt {
            return Err(RetirementReject::LedgerMismatch);
        }
        match status {
            EffectStatus::Acknowledged(_) => {
                // A successful prepare/abort must have materialized its exact
                // reserved resources. Retired metadata cannot hide a lost row.
                if is_acknowledged_success(*status)
                    && let Command::PrepareVm { vm, slot, .. }
                    | Command::AbortPrepare { vm, slot, .. } = id.command()
                    && (!world.vms.contains_key(vm) || !world.net_slots.contains_key(slot))
                {
                    return Err(RetirementReject::LedgerMismatch);
                }
            }
            EffectStatus::RetiredByStop(coverage) if retired_stop_valid(world, id, *coverage) => {}
            EffectStatus::RetiredByCaptureEnd(receipt)
                if crate::capture::continuation::retired_capture_end_valid(world, id, *receipt) => {
            }
            EffectStatus::RetiredByCaptureDrain(coverage)
                if crate::capture::drain::retired_capture_drain_valid(world, id, *coverage) => {}
            EffectStatus::Committed if obsolete_committed(world, id, logical) => {}
            EffectStatus::Dispatched | EffectStatus::OutcomeUnknown => {
                return Err(RetirementReject::UnresolvedEffects);
            }
            _ => return Err(RetirementReject::LedgerMismatch),
        }
    }
    Ok(())
}

fn check_certificate(
    record: &SandboxRecord,
    request: &RetireRequest,
) -> Result<(), RetirementReject> {
    let sandbox = request.sandbox;
    let certificate = &request.quiescence;
    if request.owner != record.owner
        || request.epoch != record.epoch
        || record
            .current_operation
            .is_none_or(|operation| operation.id != request.operation)
        || certificate.sandbox != sandbox
        || certificate.owner != record.owner
        || certificate.epoch != record.epoch
        || certificate.operation != request.operation
        || certificate.workspace != record.workspace
        || certificate.worker_drained != QuiescenceStatus::Confirmed
        || certificate.data_connections_drained != QuiescenceStatus::Confirmed
        || certificate.host_resources_reclaimed != QuiescenceStatus::Confirmed
        || certificate.external_pins_released != QuiescenceStatus::Confirmed
    {
        return Err(RetirementReject::CertificateMismatch);
    }
    if record.phase != Phase::Destroyed
        || !record.terminal
        || record.desired != Authority::Closed
        || record.effective != Authority::Closed
        || record.current_vm.is_some()
        || record.net_slot.is_some()
    {
        return Err(RetirementReject::NotTerminal);
    }
    Ok(())
}

fn retain_operation_results(
    next: &mut World,
    world: &World,
    sandbox: SandboxId,
    owner: TenantId,
    expires_at: u64,
) -> Result<(), RetirementReject> {
    let retained = &mut next
        .maintenance
        .as_mut()
        .ok_or(RetirementReject::Disabled)?
        .retained;
    for (key, ledger) in world.operations.entries() {
        if key.sandbox == sandbox {
            retained.put(
                *key,
                RetainedOperation {
                    owner,
                    ledger: ledger.clone(),
                    expires_at,
                },
            );
        }
    }
    Ok(())
}

fn remove_details(world: &mut World, sandbox: SandboxId) {
    world.sandboxes.retain(|key, _| *key != sandbox);
    world.filesystems.retain(|_, fs| fs.sandbox != sandbox);
    world.vms.retain(|_, vm| vm.sandbox != sandbox);
    world.net_slots.retain(|_, slot| slot.sandbox != sandbox);
    world
        .snapshots
        .retain(|_, snapshot| snapshot.sandbox != sandbox);
    world.applied_file_policies.retain(|key, _| *key != sandbox);
    world.worker_fences.retain(|key, _| *key != sandbox);
    world.operations.retain(|key, _| key.sandbox != sandbox);
    world
        .operation_plans
        .retain(|key, _| key.sandbox != sandbox);
    world.resource_indexes.retain(|key, _| *key != sandbox);
    world
        .logical_effects
        .retain(|id, _| id.command.sandbox() != sandbox);
    world
        .effects
        .retain(|id, _| id.command().sandbox() != sandbox);
    world
        .effect_keys
        .retain(|id| id.command().sandbox() != sandbox);
    world
        .inflight
        .retain(|id| id.command().sandbox() != sandbox);
}

/// Verify exact terminal controller cleanup without changing state. This is
/// logical evidence only: it does not certify physical host/data quiescence,
/// release external pins, require physical positions Free, or retire history.
/// # Errors
/// Unknown actions, incomplete ledgers/resources, authority or dependencies fail closed.
pub fn validate_terminal_cleanup(
    world: &World,
    sandbox: SandboxId,
) -> Result<(), RetirementReject> {
    if !world.controller_up {
        return Err(RetirementReject::ControllerDown);
    }
    let record = world
        .sandboxes
        .get(&sandbox)
        .ok_or(RetirementReject::SandboxNotFound)?;
    if record.phase != Phase::Destroyed
        || !record.terminal
        || record.desired != Authority::Closed
        || record.effective != Authority::Closed
        || record.current_vm.is_some()
        || record.net_slot.is_some()
    {
        return Err(RetirementReject::NotTerminal);
    }
    check_ledgers(world, sandbox, record)?;
    let ids = owned_ids(world, sandbox);
    if !external_dependencies_closed(world, sandbox, record, &ids) {
        return Err(RetirementReject::DependencyNotClosed);
    }
    check_resources(world, sandbox, record, &ids)?;
    check_effects(world, sandbox, record)?;
    if world.worker_fences.get(&sandbox)
        != Some(&WorkerFence {
            epoch: record.epoch,
            terminal: true,
        })
        || !successful_command(
            world,
            &Command::InstallFence {
                sandbox,
                epoch: record.epoch,
                terminal: true,
            },
        )
    {
        return Err(RetirementReject::NotTerminal);
    }

    Ok(())
}

/// Atomically retain exact terminal results and remove the closed sandbox's
/// controller metadata. No physical deletion or receipt synthesis occurs.
///
/// # Errors
/// Returns an ordered guard failure without changing the supplied World. The
/// caller's trusted certificate must remain valid through durable commit.
pub fn retire(world: &World, request: RetireRequest) -> Result<World, RetirementReject> {
    let maintenance = world
        .maintenance
        .as_ref()
        .ok_or(RetirementReject::Disabled)?;
    if !world.controller_up {
        return Err(RetirementReject::ControllerDown);
    }
    let sandbox = request.sandbox;
    let record = world
        .sandboxes
        .get(&sandbox)
        .ok_or(RetirementReject::SandboxNotFound)?;
    check_certificate(record, &request)?;
    if request.retain_until <= maintenance.time_floor {
        return Err(RetirementReject::RetentionExpired);
    }
    if maintenance
        .sandbox_frontier
        .is_none_or(|frontier| sandbox.0 > frontier)
        || maintenance
            .workspace_frontier
            .is_none_or(|frontier| record.workspace.0 > frontier)
    {
        return Err(RetirementReject::IdentityBarrierMissing);
    }
    let count = check_ledgers(world, sandbox, record)?;
    let retained_count = u64::try_from(maintenance.retained.entries().len())
        .ok()
        .and_then(|old| {
            u64::try_from(count)
                .ok()
                .and_then(|new| old.checked_add(new))
        })
        .ok_or(RetirementReject::RetentionCapacity)?;
    if retained_count > maintenance.max_retained_results {
        return Err(RetirementReject::RetentionCapacity);
    }
    if world
        .operations
        .entries()
        .iter()
        .any(|(key, _)| key.sandbox == sandbox && maintenance.retained.contains_key(key))
    {
        return Err(RetirementReject::LedgerMismatch);
    }
    let ids = owned_ids(world, sandbox);
    if maintenance.slots.entries().iter().any(|(_, slot)| {
        slot.phase != super::slots::HostSlotPhase::Free
            && (slot.binding.sandbox == sandbox
                || ids.vms.contains(&slot.binding.vm)
                || ids.slots.contains(&slot.binding.network))
    }) {
        return Err(RetirementReject::ResourcesNotReclaimed);
    }
    validate_terminal_cleanup(world, sandbox)?;

    let mut next = world.clone();
    retain_operation_results(
        &mut next,
        world,
        sandbox,
        record.owner,
        request.retain_until,
    )?;
    remove_details(&mut next, sandbox);
    Ok(next)
}

#[cfg(test)]
#[path = "retirement_tests.rs"]
mod tests;
