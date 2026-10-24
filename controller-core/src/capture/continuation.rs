//! Original-VM continuation inputs. These receipts name facts already validated
//! and durably recorded by the service; digest equality is not an OS/API proof.
//! No type here authorizes physical work before the original dispatch commits.
use crate::{Command, Epoch, Generation, OperationKey, VmId};

/// Exact original Pause identity. Native identifiers (including VM zero) retain
/// their native domains; equality with the saved plan is the authority check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct OriginalContinueIdentity {
    pub key: OperationKey,
    pub epoch: Epoch,
    pub generation: Generation,
    pub vm: VmId,
}
impl OriginalContinueIdentity {
    #[must_use]
    pub const fn activation_command(self) -> Command {
        Command::ActivateVm {
            sandbox: self.key.sandbox,
            epoch: self.epoch,
            generation: self.generation,
            vm: self.vm,
        }
    }
}

/// SHA-256 bindings to the complete original Full request and its confirmed
/// durable end-fact record. The service checks original action/source/worker,
/// API completion or definite non-release, and actual writer termination.
/// Neither a worker exit, missing marker nor a current Running GET suffices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct CaptureWorkEndReceipt {
    pub work_binding: [u8; 32],
    pub end_binding: [u8; 32],
    pub end: CaptureWorkEnd,
}

/// The original SQL end record distinguishes a never-granted Full permit from
/// a returned API plus confirmed writer exit. Both retain any material debt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub enum CaptureWorkEnd {
    NeverReleased { seal: Option<CaptureSealEnd> },
    Returned { seal: CaptureSealEnd },
}
impl CaptureWorkEnd {
    #[must_use]
    pub const fn seal(self) -> Option<CaptureSealEnd> {
        match self {
            Self::NeverReleased { seal } => seal,
            Self::Returned { seal } => Some(seal),
        }
    }
}

/// Complete remaining native Seal coordinates; key/epoch/generation/VM are
/// supplied by the enclosing original identity and checked against its plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct CaptureSealEnd {
    pub attempt: crate::AttemptNo,
    pub workspace: crate::FsId,
    pub snapshot: crate::SnapshotId,
    pub manifest: crate::SnapshotManifestRef,
}
impl CaptureSealEnd {
    #[must_use]
    pub const fn command(self, identity: OriginalContinueIdentity) -> Command {
        Command::SealSnapshot {
            sandbox: identity.key.sandbox,
            epoch: identity.epoch,
            generation: identity.generation,
            vm: identity.vm,
            workspace: self.workspace,
            snapshot: self.snapshot,
            manifest: self.manifest,
        }
    }
}

use super::{CaptureCommit, CaptureHandoff, RestoreEligibility};
use crate::{
    AppliedFilePolicy, Authority, EffectId, EffectOrigin, EffectOutcome, EffectStatus, FailureKind,
    LogicalEffectId, OperationInput, OperationKind, OperationPlan, OperationResult,
    OperationStatus, Phase, VmStatus, WorkerFence, World,
};

/// Stored inside the original capture plan. No second VM, generation, token or
/// lifetime is allocated. The enclosing plan supplies its key/epoch/generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct OriginalContinuation {
    pub vm: VmId,
    pub end_receipt: CaptureWorkEndReceipt,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CaptureInput {
    PrepareOriginalContinue {
        identity: OriginalContinueIdentity,
        end_receipt: CaptureWorkEndReceipt,
    },
    PublishOriginalContinued {
        identity: OriginalContinueIdentity,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureEvent {
    PrepareOriginalContinue(OriginalContinueIdentity),
    PublishOriginalContinued(OriginalContinueIdentity),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureReject {
    ControllerDown,
    SandboxNotFound,
    Terminal,
    WrongPhase,
    IdentityMismatch,
    LedgerMismatch,
    CaptureNotRevoked,
    CommitUnresolved,
    EffectsUnresolved,
    FreezeUnresolved,
    FenceNotInstalled,
    ResourcesNotReady,
    ReceiptConflict,
    ContinuationNotPrepared,
    ActivationNotObserved,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureDecision {
    pub event: CaptureEvent,
    pub next: World,
}

fn original_plan(
    world: &World,
    id: OriginalContinueIdentity,
) -> Result<&OperationPlan, CaptureReject> {
    if !world.controller_up {
        return Err(CaptureReject::ControllerDown);
    }
    let row = world
        .sandbox_record(id.key.sandbox)
        .ok_or(CaptureReject::SandboxNotFound)?;
    if row.terminal {
        return Err(CaptureReject::Terminal);
    }
    let operation = row.current_operation.ok_or(CaptureReject::LedgerMismatch)?;
    if operation.id != id.key.operation
        || operation.kind != OperationKind::Pause
        || operation.epoch != id.epoch
        || row.epoch != id.epoch
        || row.generation != id.generation
        || row.current_vm != Some(id.vm)
    {
        return Err(CaptureReject::IdentityMismatch);
    }
    let ledger = world
        .operation_record(id.key)
        .ok_or(CaptureReject::LedgerMismatch)?;
    let plan = world
        .operation_plan(id.key)
        .ok_or(CaptureReject::LedgerMismatch)?;
    if ledger.operation != operation
        || ledger.sandbox != id.key.sandbox
        || ledger.input
            != (OperationInput::Pause {
                generation: id.generation,
            })
        || plan.key != id.key
        || plan.epoch != id.epoch
        || plan.generation != id.generation
        || plan.vm.is_some_and(|vm| vm != id.vm)
    {
        return Err(CaptureReject::LedgerMismatch);
    }
    Ok(plan)
}
fn compensating(world: &World, id: OriginalContinueIdentity) -> Result<FailureKind, CaptureReject> {
    let row = world
        .sandbox_record(id.key.sandbox)
        .ok_or(CaptureReject::SandboxNotFound)?;
    if row.phase != Phase::Pausing {
        return Err(CaptureReject::WrongPhase);
    }
    let operation = row.current_operation.ok_or(CaptureReject::LedgerMismatch)?;
    let OperationStatus::Compensating(failure) = operation.status else {
        return Err(CaptureReject::WrongPhase);
    };
    if world
        .operation_record(id.key)
        .is_none_or(|ledger| ledger.result.is_some())
    {
        return Err(CaptureReject::LedgerMismatch);
    }
    Ok(failure)
}
fn source_resources(world: &World, id: OriginalContinueIdentity) -> Result<(), CaptureReject> {
    let row = world
        .sandbox_record(id.key.sandbox)
        .ok_or(CaptureReject::SandboxNotFound)?;
    let vm = world
        .vm_record(id.vm)
        .ok_or(CaptureReject::ResourcesNotReady)?;
    let slot = world
        .slot_record(row.net_slot.ok_or(CaptureReject::ResourcesNotReady)?)
        .ok_or(CaptureReject::ResourcesNotReady)?;
    if vm.owner != row.owner
        || vm.sandbox != id.key.sandbox
        || vm.generation != id.generation
        || slot.owner != row.owner
        || slot.sandbox != id.key.sandbox
        || slot.generation != id.generation
        || slot.vm != id.vm
        || !slot.configured
        || slot.reclaimed
        || slot.policy != row.policy
        || world.applied_file_policies.get(&id.key.sandbox)
            != Some(&AppliedFilePolicy {
                sandbox: id.key.sandbox,
                generation: id.generation,
                workspace: row.workspace,
                policy: row.file_policy,
            })
    {
        return Err(CaptureReject::ResourcesNotReady);
    }
    let index = world.resource_index(id.key.sandbox);
    let mut position = 0;
    let mut resources_ready = true;
    while position < index.vms.len() && resources_ready {
        let other = index.vms[position];
        resources_ready = match world.vm_record(other) {
            None => false,
            Some(machine) => {
                other == id.vm
                    || !matches!(
                        machine.status,
                        VmStatus::Prepared | VmStatus::Executing | VmStatus::Frozen
                    )
            }
        };
        position += 1;
    }
    if !resources_ready {
        return Err(CaptureReject::ResourcesNotReady);
    }
    if world.worker_fences.get(&id.key.sandbox)
        != Some(&WorkerFence {
            epoch: id.epoch,
            terminal: false,
        })
        || !crate::effect::effect_succeeded_check(
            world,
            &crate::Command::InstallFence {
                sandbox: id.key.sandbox,
                epoch: id.epoch,
                terminal: false,
            },
        )
    {
        return Err(CaptureReject::FenceNotInstalled);
    }
    Ok(())
}
fn capture_command(command: &Command) -> bool {
    matches!(
        command,
        Command::FreezeVm { .. } | Command::SealSnapshot { .. } | Command::CommitSnapshot { .. }
    )
}
fn seal_end_matches(
    world: &World,
    effect: &EffectId,
    id: OriginalContinueIdentity,
    receipt: CaptureWorkEndReceipt,
) -> bool {
    let Some(seal) = receipt.end.seal() else {
        return false;
    };
    let Some(plan) = world.operation_plan(id.key) else {
        return false;
    };
    let Some(row) = world.sandbox_record(id.key.sandbox) else {
        return false;
    };
    effect.attempt == seal.attempt
        && *effect.command() == seal.command(id)
        && plan.key == id.key
        && plan.epoch == id.epoch
        && plan.generation == id.generation
        && plan.vm == Some(id.vm)
        && plan.target_snapshot
            == Some(crate::RestoreSource {
                snapshot: seal.snapshot,
                manifest: seal.manifest,
            })
        && row.workspace == seal.workspace
        && world
            .logical_effect(&effect.logical)
            .is_some_and(|logical| {
                logical.origin == EffectOrigin::Operation(id.key)
                    && logical.current_attempt == effect.attempt
            })
}

/// Checkpoint and metadata retirement recheck the same original saved end.
/// A retired Seal never counts as a successful snapshot or a failed host call.
#[must_use]
pub fn retired_capture_end_valid(
    world: &World,
    effect: &EffectId,
    receipt: CaptureWorkEndReceipt,
) -> bool {
    let Some(logical) = world.logical_effect(&effect.logical) else {
        return false;
    };
    let EffectOrigin::Operation(key) = logical.origin else {
        return false;
    };
    let Some(plan) = world.operation_plan(key) else {
        return false;
    };
    let Some(capture) = plan.capture else {
        return false;
    };
    let Some(saved) = capture.original_continuation() else {
        return false;
    };
    capture.eligibility() == RestoreEligibility::Revoked
        && capture.commit() == CaptureCommit::NotRegistered
        && !capture.has_unload_source()
        && saved.end_receipt == receipt
        && seal_end_matches(
            world,
            effect,
            OriginalContinueIdentity {
                key,
                epoch: plan.epoch,
                generation: plan.generation,
                vm: saved.vm,
            },
            receipt,
        )
        && logical.result
            == Some(crate::LogicalEffectResult::RetiredByCaptureEnd {
                attempt: effect.attempt,
                end_receipt: receipt,
            })
}

fn retire_ended_seal(
    world: &mut World,
    id: OriginalContinueIdentity,
    receipt: CaptureWorkEndReceipt,
) {
    let Some(seal) = receipt.end.seal() else {
        return;
    };
    let effect = EffectId {
        logical: LogicalEffectId {
            command: seal.command(id),
        },
        attempt: seal.attempt,
    };
    if !matches!(
        world.effect_status(&effect),
        Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown)
    ) {
        return;
    }
    let Some(mut logical) = world.logical_effect(&effect.logical) else {
        return;
    };
    logical.result = Some(crate::LogicalEffectResult::RetiredByCaptureEnd {
        attempt: effect.attempt,
        end_receipt: receipt,
    });
    world.logical_effects.put(effect.logical.clone(), logical);
    world
        .effects
        .put(effect, EffectStatus::RetiredByCaptureEnd(receipt));
}

fn ended_attempts(
    world: &World,
    id: OriginalContinueIdentity,
    receipt: CaptureWorkEndReceipt,
) -> Result<(), CaptureReject> {
    if let Some(seal) = receipt.end.seal() {
        let effect = EffectId {
            logical: LogicalEffectId {
                command: seal.command(id),
            },
            attempt: seal.attempt,
        };
        if !seal_end_matches(world, &effect, id, receipt) {
            return Err(CaptureReject::ReceiptConflict);
        }
    }
    let mut position = 0;
    let mut result = Ok(());
    while position < world.effect_ids().len() && matches!(result, Ok(())) {
        result = capture_attempt_ended(world, &world.effect_ids()[position], id, receipt);
        position += 1;
    }
    result
}

fn capture_attempt_ended(
    world: &World,
    effect: &EffectId,
    id: OriginalContinueIdentity,
    receipt: CaptureWorkEndReceipt,
) -> Result<(), CaptureReject> {
    if !capture_command(effect.command()) {
        return Ok(());
    }
    let logical = world
        .logical_effect(&effect.logical)
        .ok_or(CaptureReject::LedgerMismatch)?;
    if logical.origin != EffectOrigin::Operation(id.key) {
        return Ok(());
    }
    if effect.command().epoch() != Some(id.epoch) || effect.command().sandbox() != id.key.sandbox {
        return Err(CaptureReject::LedgerMismatch);
    }
    match world.effect_status(effect) {
        Some(EffectStatus::Committed | EffectStatus::Acknowledged(_)) => {}
        Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown)
            if seal_end_matches(world, effect, id, receipt) && logical.result.is_none() => {}
        Some(EffectStatus::RetiredByCaptureEnd(saved))
            if saved == receipt && retired_capture_end_valid(world, effect, saved) => {}
        _ => return Err(CaptureReject::EffectsUnresolved),
    }
    Ok(())
}

fn source_can_continue(world: &World, id: OriginalContinueIdentity) -> Result<(), CaptureReject> {
    let freeze = Command::FreezeVm {
        sandbox: id.key.sandbox,
        epoch: id.epoch,
        generation: id.generation,
        vm: id.vm,
    };
    let machine = world
        .vm_record(id.vm)
        .ok_or(CaptureReject::ResourcesNotReady)?;
    if machine.status == VmStatus::Frozen && crate::effect::effect_succeeded_check(world, &freeze) {
        return Ok(());
    }
    // An attempt-zero Committed record predates durable Dispatch, so no Freeze
    // request could have been sent under the original execution contract. Keep
    // it recorded, permanently withdrawn by the closed original capture plan.
    let initial = EffectId::initial(freeze);
    if machine.status == VmStatus::Executing
        && world.effect_status(&initial) == Some(EffectStatus::Committed)
        && world
            .logical_effect(&initial.logical)
            .is_some_and(|logical| {
                logical.origin == EffectOrigin::Operation(id.key)
                    && logical.current_attempt == 0
                    && logical.result.is_none()
            })
    {
        return Ok(());
    }
    Err(CaptureReject::FreezeUnresolved)
}
fn prepared(plan: &OperationPlan, id: OriginalContinueIdentity) -> bool {
    plan.key == id.key
        && plan.epoch == id.epoch
        && plan.generation == id.generation
        && plan.capture.is_some_and(|capture| {
            capture.may_reactivate_original()
                && capture
                    .original_continuation()
                    .is_some_and(|saved| saved.vm == id.vm)
        })
}
/// Structural identity retained after current state has advanced to a later
/// operation. It does not establish physical authorization or completion.
#[must_use]
pub fn activation_belongs_to_continuation(plan: &OperationPlan, command: &Command) -> bool {
    let Command::ActivateVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *command
    else {
        return false;
    };
    prepared(
        plan,
        OriginalContinueIdentity {
            key: plan.key,
            epoch,
            generation,
            vm,
        },
    ) && plan.key.sandbox == sandbox
}
#[must_use]
pub fn continuation_activation_pending(world: &World, command: &Command) -> bool {
    let Command::ActivateVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *command
    else {
        return false;
    };
    let Some(row) = world.sandbox_record(sandbox) else {
        return false;
    };
    let Some(operation) = row.current_operation else {
        return false;
    };
    let id = OriginalContinueIdentity {
        key: OperationKey {
            sandbox,
            operation: operation.id,
        },
        epoch,
        generation,
        vm,
    };
    !row.terminal
        && row.phase == Phase::Pausing
        && row.epoch == epoch
        && row.generation == generation
        && row.current_vm == Some(vm)
        && operation.kind == OperationKind::Pause
        && matches!(operation.status, OperationStatus::Compensating(_))
        && world
            .operation_plan(id.key)
            .is_some_and(|plan| prepared(plan, id))
        && world
            .logical_effect(&LogicalEffectId {
                command: command.clone(),
            })
            .is_some_and(|logical| logical.origin == EffectOrigin::Operation(id.key))
}
/// Exact original-plan guard. A later eligible Pause never re-enables an old
/// queued Freeze/Seal/Commit, even in the same native generation.
#[must_use]
pub fn capture_dispatch_allowed(world: &World, effect: &EffectId) -> bool {
    if !capture_command(effect.command()) {
        return true;
    }
    let sandbox = effect.command().sandbox();
    let Some(row) = world.sandbox_record(sandbox) else {
        return false;
    };
    let Some(operation) = row.current_operation else {
        return false;
    };
    let key = OperationKey {
        sandbox,
        operation: operation.id,
    };
    let generation_matches = match *effect.command() {
        Command::FreezeVm { generation, vm, .. } | Command::SealSnapshot { generation, vm, .. } => {
            generation == row.generation && row.current_vm == Some(vm)
        }
        Command::CommitSnapshot { generation, .. } => generation == row.generation,
        _ => false,
    };
    generation_matches
        && operation.kind == OperationKind::Pause
        && !row.terminal
        && row.phase == Phase::Pausing
        && effect.command().epoch() == Some(row.epoch)
        && world
            .logical_effect(&effect.logical)
            .is_some_and(|logical| logical.origin == EffectOrigin::Operation(key))
        && world.operation_plan(key).is_some_and(|plan| {
            plan.epoch == row.epoch
                && plan.generation == row.generation
                && plan
                    .capture
                    .is_some_and(CaptureHandoff::accepts_registration)
        })
}
/// Never-issued obsolete work stays in the ledger until ordinary retirement;
/// this is controller withdrawal, not a fabricated host failure receipt.
#[must_use]
pub fn capture_dispatch_permanently_revoked(world: &World, effect: &EffectId) -> bool {
    if !capture_command(effect.command()) {
        return false;
    }
    let Some(logical) = world.logical_effect(&effect.logical) else {
        return false;
    };
    let EffectOrigin::Operation(key) = logical.origin else {
        return false;
    };
    world.operation_plan(key).is_some_and(|plan| {
        key.sandbox == effect.command().sandbox()
            && plan.epoch == effect.command().epoch().unwrap_or(0)
            && plan
                .capture
                .is_some_and(|capture| capture.eligibility() == RestoreEligibility::Revoked)
    })
}
// The original journal records end facts and this native input atomically.
// Validate every unknown-effect barrier before using the existing Pause
// compensation reducer; illegal end input cannot even revoke the capture.
fn prepare(
    world: &World,
    id: OriginalContinueIdentity,
    receipt: CaptureWorkEndReceipt,
) -> Result<CaptureDecision, CaptureReject> {
    let plan = original_plan(world, id)?;
    let row = world
        .sandbox_record(id.key.sandbox)
        .ok_or(CaptureReject::SandboxNotFound)?;
    let operation = row.current_operation.ok_or(CaptureReject::LedgerMismatch)?;
    if matches!(
        operation.status,
        OperationStatus::Pending | OperationStatus::OutcomeUnknown
    ) {
        if row.phase != Phase::Pausing {
            return Err(CaptureReject::WrongPhase);
        }
        let capture = plan.capture.ok_or(CaptureReject::LedgerMismatch)?;
        if capture.commit() != CaptureCommit::NotRegistered {
            return Err(CaptureReject::CommitUnresolved);
        }
        if !capture.accepts_registration() {
            return Err(CaptureReject::CaptureNotRevoked);
        }
        source_resources(world, id)?;
        ended_attempts(world, id, receipt)?;
        source_can_continue(world, id)?;
        let compensated = crate::request::handle_compensation(
            world,
            crate::CompensationRequest {
                sandbox: id.key.sandbox,
                failure: FailureKind::Cancelled,
                evidence: None,
            },
        )
        .map_err(|_| CaptureReject::LedgerMismatch)?;
        return prepare_revoked(&compensated.next, id, receipt);
    }
    prepare_revoked(world, id, receipt)
}

fn prepare_revoked(
    world: &World,
    id: OriginalContinueIdentity,
    receipt: CaptureWorkEndReceipt,
) -> Result<CaptureDecision, CaptureReject> {
    let plan = original_plan(world, id)?;
    compensating(world, id)?;
    let capture = plan.capture.ok_or(CaptureReject::LedgerMismatch)?;
    if capture.commit() != CaptureCommit::NotRegistered {
        return Err(CaptureReject::CommitUnresolved);
    }
    if capture.eligibility() != RestoreEligibility::Revoked || capture.has_unload_source() {
        return Err(CaptureReject::CaptureNotRevoked);
    }
    if let Some(saved) = capture.original_continuation() {
        if saved
            != (OriginalContinuation {
                vm: id.vm,
                end_receipt: receipt,
            })
        {
            return Err(CaptureReject::ReceiptConflict);
        }
        return Ok(CaptureDecision {
            event: CaptureEvent::PrepareOriginalContinue(id),
            next: world.clone(),
        });
    }
    source_resources(world, id)?;
    ended_attempts(world, id, receipt)?;
    source_can_continue(world, id)?;
    let mut next = world.clone();
    let mut updated = plan.clone();
    updated.capture = capture.prepare_original_continuation(OriginalContinuation {
        vm: id.vm,
        end_receipt: receipt,
    });
    if updated.capture.is_none() {
        return Err(CaptureReject::CaptureNotRevoked);
    }
    next.operation_plans.put(id.key, updated);
    retire_ended_seal(&mut next, id, receipt);
    crate::transition::enqueue(&mut next, id.activation_command());
    Ok(CaptureDecision {
        event: CaptureEvent::PrepareOriginalContinue(id),
        next,
    })
}
fn publish(world: &World, id: OriginalContinueIdentity) -> Result<CaptureDecision, CaptureReject> {
    let plan = original_plan(world, id)?;
    if !prepared(plan, id) {
        return Err(CaptureReject::ContinuationNotPrepared);
    }
    let row = world
        .sandbox_record(id.key.sandbox)
        .ok_or(CaptureReject::SandboxNotFound)?;
    if row.phase == Phase::Running
        && matches!(
            row.current_operation.map(|operation| operation.status),
            Some(OperationStatus::Failed(_))
        )
    {
        return Ok(CaptureDecision {
            event: CaptureEvent::PublishOriginalContinued(id),
            next: world.clone(),
        });
    }
    let failure = compensating(world, id)?;
    source_resources(world, id)?;
    let saved = plan
        .capture
        .and_then(CaptureHandoff::original_continuation)
        .ok_or(CaptureReject::ContinuationNotPrepared)?;
    ended_attempts(world, id, saved.end_receipt)?;
    if row.desired != Authority::Closed
        || row.effective != Authority::Closed
        || world
            .vm_record(id.vm)
            .is_none_or(|machine| machine.status != VmStatus::Executing)
        || !crate::effect::effect_succeeded_check(world, &id.activation_command())
    {
        return Err(CaptureReject::ActivationNotObserved);
    }
    let mut next = world.clone();
    let mut updated = row.clone();
    let slot_id = updated.net_slot.ok_or(CaptureReject::ResourcesNotReady)?;
    let mut slot = world
        .slot_record(slot_id)
        .ok_or(CaptureReject::ResourcesNotReady)?
        .clone();
    let mut ledger = world
        .operation_record(id.key)
        .ok_or(CaptureReject::LedgerMismatch)?
        .clone();
    ledger.operation.status = OperationStatus::Failed(failure);
    ledger.result = Some(OperationResult::Failed(failure));
    updated.phase = Phase::Running;
    updated.current_operation = Some(ledger.operation);
    updated.desired = Authority::Running {
        sandbox: id.key.sandbox,
        generation: id.generation,
        vm: id.vm,
        workspace: row.workspace,
        access: row.workspace_access,
        slot: slot_id,
        policy: row.policy.clone(),
    };
    updated.effective = updated.desired.clone();
    slot.active = true;
    next.net_slots.put(slot_id, slot);
    next.operations.put(id.key, ledger);
    next.sandboxes.put(id.key.sandbox, updated);
    Ok(CaptureDecision {
        event: CaptureEvent::PublishOriginalContinued(id),
        next,
    })
}
/// Caller first checks and persists full original end facts. These inputs do
/// not prove writer/API termination, reclaim material, or open center routes.
/// # Errors
/// Rejects mismatched identities, pending effects, unknown commit and terminal intent.
pub fn handle_capture(
    world: &World,
    input: CaptureInput,
) -> Result<CaptureDecision, CaptureReject> {
    match input {
        CaptureInput::PrepareOriginalContinue {
            identity,
            end_receipt,
        } => prepare(world, identity, end_receipt),
        CaptureInput::PublishOriginalContinued { identity } => publish(world, identity),
    }
}

pub(crate) fn continuation_resources_ready(world: &World, command: &Command) -> bool {
    let Command::ActivateVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *command
    else {
        return false;
    };
    let Some(operation) = world
        .sandbox_record(sandbox)
        .and_then(|row| row.current_operation)
    else {
        return false;
    };
    let id = OriginalContinueIdentity {
        key: OperationKey {
            sandbox,
            operation: operation.id,
        },
        epoch,
        generation,
        vm,
    };
    continuation_activation_pending(world, command)
        && source_resources(world, id).is_ok()
        && world
            .operation_plan(id.key)
            .and_then(|plan| plan.capture)
            .and_then(CaptureHandoff::original_continuation)
            .is_some_and(|saved| ended_attempts(world, id, saved.end_receipt).is_ok())
}
/// A historic successful Freeze remains true after the same plan's explicitly
/// recorded reactivation. This is not evidence for a new/unknown Freeze receipt.
pub(crate) fn historical_freeze_after_continue(world: &World, command: &Command) -> bool {
    let Command::FreezeVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *command
    else {
        return false;
    };
    world.operation_plans.entries().iter().any(|(key, plan)| {
        key.sandbox == sandbox
            && plan.epoch == epoch
            && activation_belongs_to_continuation(
                plan,
                &Command::ActivateVm {
                    sandbox,
                    epoch,
                    generation,
                    vm,
                },
            )
            && world.effects.entries().iter().any(|(effect, status)| {
                effect.command()
                    == &Command::ActivateVm {
                        sandbox,
                        epoch,
                        generation,
                        vm,
                    }
                    && matches!(
                        status,
                        EffectStatus::Dispatched
                            | EffectStatus::OutcomeUnknown
                            | EffectStatus::Acknowledged(EffectOutcome::Succeeded(_))
                    )
            })
    })
}
/// Exact possible execution after Destroy remains accountable against the old
/// plan; it never authorizes a late continuation publication or a new dispatch.
pub(crate) fn activation_bound_in_current_abort(world: &World, effect: &EffectId) -> bool {
    let command = effect.command();
    let Command::ActivateVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *command
    else {
        return false;
    };
    let Some(row) = world.sandbox_record(sandbox) else {
        return false;
    };
    let Some(operation) = row.current_operation else {
        return false;
    };
    let Some(logical) = world.logical_effect(&effect.logical) else {
        return false;
    };
    let EffectOrigin::Operation(key) = logical.origin else {
        return false;
    };
    row.terminal
        && row.phase == Phase::Revoking
        && operation.kind == OperationKind::Kill
        && operation.id != key.operation
        && row.epoch > epoch
        && row.generation == generation
        && row.current_vm == Some(vm)
        && world
            .operation_plan(key)
            .is_some_and(|plan| activation_belongs_to_continuation(plan, command))
        && logical.current_attempt == effect.attempt
        && logical.result.is_none()
        && matches!(
            world.effect_status(effect),
            Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown)
        )
        && world.vm_record(vm).is_some_and(|machine| {
            machine.owner == row.owner
                && machine.sandbox == sandbox
                && machine.generation == generation
                && matches!(
                    machine.status,
                    VmStatus::Frozen | VmStatus::Executing | VmStatus::Stopped
                )
        })
        && row
            .net_slot
            .and_then(|slot| world.slot_record(slot))
            .is_some_and(|network| {
                network.owner == row.owner
                    && network.sandbox == sandbox
                    && network.generation == generation
                    && network.vm == vm
                    && network.policy == row.policy
                    && network.configured
                    && !network.reclaimed
            })
        && world
            .operation_plan(OperationKey {
                sandbox,
                operation: operation.id,
            })
            .is_some_and(|plan| {
                plan.stop_vms.contains(&vm)
                    && row
                        .net_slot
                        .is_some_and(|slot| plan.close_slots.contains(&slot))
            })
}

pub(crate) fn activation_in_current_abort(world: &World, effect: &EffectId) -> bool {
    activation_bound_in_current_abort(world, effect)
        && match *effect.command() {
            Command::ActivateVm { vm, .. } => world.vm_record(vm).is_some_and(|machine| {
                matches!(machine.status, VmStatus::Frozen | VmStatus::Executing)
            }),
            _ => false,
        }
}
