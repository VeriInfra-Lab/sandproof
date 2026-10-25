//! Narrow terminal permission to stop an original source with an unknown Full.
//! Stopping is not capture/API completion, worker drain or material retirement.
use super::{CaptureCommit, RestoreEligibility};
use crate::{
    Authority, Command, EffectId, EffectOrigin, EffectStatus, NetSlotId, OperationInput,
    OperationKey, OperationKind, OperationStatus, Phase, SandboxId, SandboxRecord, VmId, VmStatus,
    WorkerFence, World,
};

struct TerminalSource<'a> {
    row: &'a SandboxRecord,
    key: OperationKey,
    vm: VmId,
    slot: NetSlotId,
}
fn terminal_source(world: &World, sandbox: SandboxId) -> Option<TerminalSource<'_>> {
    let row = world.sandbox_record(sandbox)?;
    let operation = row.current_operation?;
    let key = OperationKey {
        sandbox,
        operation: operation.id,
    };
    let ledger = world.operation_record(key)?;
    let plan = world.operation_plan(key)?;
    let vm = row.current_vm?;
    let slot = row.net_slot?;
    let machine = world.vm_record(vm)?;
    let network = world.slot_record(slot)?;
    if !world.controller_up
        || !row.terminal
        || row.phase != Phase::Revoking
        || row.desired != Authority::Closed
        || row.effective != Authority::Closed
        || operation.kind != OperationKind::Kill
        || operation.epoch != row.epoch
        || !matches!(
            operation.status,
            OperationStatus::Pending | OperationStatus::OutcomeUnknown
        )
        || ledger.sandbox != sandbox
        || ledger.operation != operation
        || ledger.result.is_some()
        || ledger.input
            != (OperationInput::Kill {
                generation: row.generation,
            })
        || plan.key != key
        || plan.epoch != row.epoch
        || plan.generation != row.generation
        || !plan.sealed
        || !plan.stop_vms.contains(&vm)
        || !plan.close_slots.contains(&slot)
        || machine.owner != row.owner
        || machine.sandbox != sandbox
        || machine.generation != row.generation
        || !matches!(machine.status, VmStatus::Frozen | VmStatus::Stopped)
        || network.owner != row.owner
        || network.sandbox != sandbox
        || network.generation != row.generation
        || network.vm != vm
        || network.policy != row.policy
        || !network.configured
        || network.active
        || network.reclaimed
    {
        return None;
    }
    Some(TerminalSource { row, key, vm, slot })
}
fn terminal_fence(source: &TerminalSource<'_>) -> Command {
    Command::InstallFence {
        sandbox: source.key.sandbox,
        epoch: source.row.epoch,
        terminal: true,
    }
}
fn terminal_stop(source: &TerminalSource<'_>) -> Command {
    Command::StopVm {
        sandbox: source.key.sandbox,
        epoch: source.row.epoch,
        generation: source.row.generation,
        vm: source.vm,
    }
}
fn original_seal(
    world: &World,
    source: &TerminalSource<'_>,
    effect: &EffectId,
) -> Option<OperationKey> {
    let Command::SealSnapshot {
        sandbox,
        epoch,
        generation,
        vm,
        workspace,
        snapshot,
        manifest,
    } = *effect.command()
    else {
        return None;
    };
    let logical = world.logical_effect(&effect.logical)?;
    let EffectOrigin::Operation(key) = logical.origin else {
        return None;
    };
    let ledger = world.operation_record(key)?;
    let plan = world.operation_plan(key)?;
    let capture = plan.capture?;
    if sandbox != source.key.sandbox
        || key.sandbox != sandbox
        || key == source.key
        || epoch >= source.row.epoch
        || generation != source.row.generation
        || vm != source.vm
        || workspace != source.row.workspace
        || logical.current_attempt != effect.attempt
        || logical.result.is_some()
        || plan.key != key
        || plan.epoch != epoch
        || plan.generation != generation
        || plan.vm != Some(vm)
        || !plan.sealed
        || plan.target_snapshot != Some(crate::RestoreSource { snapshot, manifest })
        || !plan.stop_vms.contains(&vm)
        || !plan.close_slots.contains(&source.slot)
        || ledger.sandbox != sandbox
        || ledger.operation.id != key.operation
        || ledger.operation.kind != OperationKind::Pause
        || ledger.operation.epoch != epoch
        || ledger.input != (OperationInput::Pause { generation })
        || !matches!(ledger.operation.status, OperationStatus::Failed(_))
        || capture.commit() != CaptureCommit::NotRegistered
        || capture.eligibility() != RestoreEligibility::Revoked
        || !capture.has_capture_work()
        || capture.has_unload_source()
        || capture.original_continuation().is_some()
    {
        return None;
    }
    let freeze = Command::FreezeVm {
        sandbox,
        epoch,
        generation,
        vm,
    };
    if !crate::effect::effect_succeeded_check(world, &freeze) {
        return None;
    }
    Some(key)
}
fn drain_action_origin(world: &World, source: &TerminalSource<'_>, effect: &EffectId) -> bool {
    world
        .logical_effect(&effect.logical)
        .is_some_and(|logical| {
            logical.origin == EffectOrigin::Operation(source.key)
                && logical.current_attempt == effect.attempt
                && logical.result.is_none()
        })
        && (*effect.command() == terminal_fence(source)
            || *effect.command() == terminal_stop(source))
}
fn only_original_capture_and_drain(world: &World, source: &TerminalSource<'_>) -> bool {
    let mut position = 0;
    let mut original = None;
    let mut valid = true;
    while position < world.effect_ids().len() && valid {
        let effect = &world.effect_ids()[position];
        if effect.command().sandbox() == source.key.sandbox
            && crate::effect::outcome_is_unresolved(world, effect)
        {
            if let Some(key) = original_seal(world, source, effect) {
                match original {
                    None => original = Some(key),
                    Some(previous) => valid = previous == key,
                }
            } else {
                valid = drain_action_origin(world, source, effect);
            }
        }
        position += 1;
    }
    valid && original.is_some()
}
/// One shared scheduling/claim guard, never an execution or cleanup permit.
/// Only an authorized current terminal plan may fence/stop its same original
/// Full source. Every other unknown effect continues to block this exception.
#[must_use]
pub fn terminal_capture_drain_allowed(world: &World, command: &Command) -> bool {
    let Some(source) = terminal_source(world, command.sandbox()) else {
        return false;
    };
    let fence = terminal_fence(&source);
    let action_allowed = if *command == fence {
        true
    } else if *command == terminal_stop(&source) {
        world.worker_fences.get_copy(&source.key.sandbox)
            == Some(WorkerFence {
                epoch: source.row.epoch,
                terminal: true,
            })
            && crate::effect::effect_succeeded_check(world, &fence)
    } else {
        false
    };
    action_allowed && only_original_capture_and_drain(world, &source)
}

use super::continuation::{
    CaptureDecision, CaptureEvent, CaptureReject, CaptureSealEnd, OriginalContinueIdentity,
};
use crate::maintenance::retirement::QuiescenceStatus;
use crate::{EffectOutcome, LogicalEffectId, LogicalEffectResult, StopCoverage};

/// Bind the canonical original terminal observation. These fields are not OS
/// proof: the service must validate and save the full original physical facts.
/// Both Confirmed values require actual original source/worker exit and all
/// relevant descriptor owners drained under the exact protected slot gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct CaptureDrainEndReceipt {
    pub work_binding: [u8; 32],
    pub end_binding: [u8; 32],
    pub seal: CaptureSealEnd,
    pub worker_drained: QuiescenceStatus,
    pub descriptors_closed: QuiescenceStatus,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct CaptureDrainCoverage {
    /// Original Pause coordinates only. This terminal path never continues it.
    pub identity: OriginalContinueIdentity,
    pub stop: StopCoverage,
    pub end_receipt: CaptureDrainEndReceipt,
}
impl CaptureDrainCoverage {
    fn seal_effect(self) -> EffectId {
        EffectId {
            logical: LogicalEffectId {
                command: self.end_receipt.seal.command(self.identity),
            },
            attempt: self.end_receipt.seal.attempt,
        }
    }
    fn stop_effect(self) -> EffectId {
        EffectId {
            logical: LogicalEffectId {
                command: Command::StopVm {
                    sandbox: self.identity.key.sandbox,
                    epoch: self.stop.epoch,
                    generation: self.identity.generation,
                    vm: self.identity.vm,
                },
            },
            attempt: self.stop.attempt,
        }
    }
}
fn stop_coverage_matches(world: &World, coverage: CaptureDrainCoverage) -> bool {
    let stop = coverage.stop_effect();
    let fence = Command::InstallFence {
        sandbox: coverage.stop.operation.sandbox,
        epoch: coverage.stop.epoch,
        terminal: true,
    };
    coverage.stop.operation.sandbox == coverage.identity.key.sandbox
        && world
            .worker_fences
            .get_copy(&coverage.stop.operation.sandbox)
            == Some(WorkerFence {
                epoch: coverage.stop.epoch,
                terminal: true,
            })
        && crate::effect::effect_succeeded_check(world, &fence)
        && world
            .operation_plan(coverage.stop.operation)
            .is_some_and(|plan| {
                plan.key == coverage.stop.operation
                    && plan.epoch == coverage.stop.epoch
                    && plan.generation == coverage.identity.generation
                    && plan.stop_vms.contains(&coverage.identity.vm)
            })
        && world
            .operation_record(coverage.stop.operation)
            .is_some_and(|ledger| {
                ledger.sandbox == coverage.stop.operation.sandbox
                    && ledger.operation.id == coverage.stop.operation.operation
                    && ledger.operation.kind == OperationKind::Kill
                    && ledger.operation.epoch == coverage.stop.epoch
                    && ledger.input
                        == (OperationInput::Kill {
                            generation: coverage.identity.generation,
                        })
            })
        && world.logical_effect(&stop.logical).is_some_and(|logical| {
            logical.origin == EffectOrigin::Operation(coverage.stop.operation)
                && logical.current_attempt == stop.attempt
                && logical.result
                    == Some(LogicalEffectResult::Succeeded {
                        attempt: stop.attempt,
                        result: coverage.stop.result,
                    })
        })
        && world.effect_status(&stop)
            == Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(
                coverage.stop.result,
            )))
}
fn confirmed_end(coverage: CaptureDrainCoverage) -> bool {
    coverage.end_receipt.worker_drained == QuiescenceStatus::Confirmed
        && coverage.end_receipt.descriptors_closed == QuiescenceStatus::Confirmed
}
/// Historical check remains valid after Destroy publication and exact cleanup.
/// It cannot turn a stopped source into a usable snapshot or running authority.
#[must_use]
pub fn retired_capture_drain_valid(
    world: &World,
    effect: &EffectId,
    coverage: CaptureDrainCoverage,
) -> bool {
    let id = coverage.identity;
    let Some(row) = world.sandbox_record(id.key.sandbox) else {
        return false;
    };
    let Some(operation) = row.current_operation else {
        return false;
    };
    let Some(plan) = world.operation_plan(id.key) else {
        return false;
    };
    let Some(capture) = plan.capture else {
        return false;
    };
    let Some(logical) = world.logical_effect(&effect.logical) else {
        return false;
    };
    confirmed_end(coverage)
        && *effect == coverage.seal_effect()
        && row.terminal
        && matches!(row.phase, Phase::Revoking | Phase::Destroyed)
        && row.desired == Authority::Closed
        && row.effective == Authority::Closed
        && row.generation == id.generation
        && row.epoch == coverage.stop.epoch
        && (OperationKey {
            sandbox: id.key.sandbox,
            operation: operation.id,
        }) == coverage.stop.operation
        && operation.kind == OperationKind::Kill
        && coverage.stop.epoch > id.epoch
        && world.operation_record(id.key).is_some_and(|ledger| {
            ledger.sandbox == id.key.sandbox
                && ledger.operation.id == id.key.operation
                && ledger.operation.kind == OperationKind::Pause
                && ledger.operation.epoch == id.epoch
                && ledger.input
                    == (OperationInput::Pause {
                        generation: id.generation,
                    })
        })
        && plan.key == id.key
        && plan.epoch == id.epoch
        && plan.generation == id.generation
        && plan.vm == Some(id.vm)
        && plan.target_snapshot
            == Some(crate::RestoreSource {
                snapshot: coverage.end_receipt.seal.snapshot,
                manifest: coverage.end_receipt.seal.manifest,
            })
        && coverage.end_receipt.seal.workspace == row.workspace
        && capture.eligibility() == RestoreEligibility::Revoked
        && capture.commit() == CaptureCommit::NotRegistered
        && !capture.has_capture_work()
        && !capture.has_unload_source()
        && capture.original_continuation().is_none()
        && capture.terminal_drain() == Some(coverage)
        && logical.origin == EffectOrigin::Operation(id.key)
        && logical.current_attempt == effect.attempt
        && logical.result
            == Some(LogicalEffectResult::RetiredByCaptureDrain {
                attempt: effect.attempt,
                coverage,
            })
        && world.vm_record(id.vm).is_some_and(|vm| {
            vm.owner == row.owner
                && vm.sandbox == id.key.sandbox
                && vm.generation == id.generation
                && matches!(vm.status, VmStatus::Stopped | VmStatus::Reclaimed)
        })
        && stop_coverage_matches(world, coverage)
}
/// Retire only the unknown original Full after authorized terminal Stop and
/// the original saved physical drain observation. No material is deleted here.
/// # Errors
/// Missing/changed facts, other unknown effects and nonterminal work refuse.
pub fn retire_terminated_capture(
    world: &World,
    coverage: CaptureDrainCoverage,
) -> Result<CaptureDecision, CaptureReject> {
    if !world.controller_up {
        return Err(CaptureReject::ControllerDown);
    }
    let seal = coverage.seal_effect();
    let event =
        CaptureEvent::RetireTerminatedCapture(coverage.identity.key, coverage.stop.operation);
    if let Some(EffectStatus::RetiredByCaptureDrain(stored)) = world.effect_status(&seal) {
        if stored == coverage && retired_capture_drain_valid(world, &seal, coverage) {
            return Ok(CaptureDecision {
                event,
                next: world.clone(),
            });
        }
        return Err(CaptureReject::ReceiptConflict);
    }
    if !confirmed_end(coverage) {
        return Err(CaptureReject::DrainNotConfirmed);
    }
    let source =
        terminal_source(world, coverage.identity.key.sandbox).ok_or(CaptureReject::WrongPhase)?;
    if coverage.stop.operation != source.key
        || coverage.stop.epoch != source.row.epoch
        || coverage.identity.vm != source.vm
        || coverage.identity.generation != source.row.generation
        || original_seal(world, &source, &seal) != Some(coverage.identity.key)
        || !matches!(
            world.effect_status(&seal),
            Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown)
        )
        || !only_original_capture_and_drain(world, &source)
    {
        return Err(CaptureReject::IdentityMismatch);
    }
    if !stop_coverage_matches(world, coverage)
        || world
            .vm_record(source.vm)
            .is_none_or(|vm| vm.status != VmStatus::Stopped)
        || !terminal_capture_drain_allowed(world, &terminal_stop(&source))
    {
        return Err(CaptureReject::StopNotObserved);
    }
    let mut next = world.clone();
    let mut plan = world
        .operation_plan(coverage.identity.key)
        .ok_or(CaptureReject::LedgerMismatch)?
        .clone();
    let capture = plan.capture.ok_or(CaptureReject::LedgerMismatch)?;
    plan.capture = capture.finish_terminal_drain(coverage);
    if plan.capture.is_none() {
        return Err(CaptureReject::ReceiptConflict);
    }
    next.operation_plans.put(coverage.identity.key, plan);
    let mut logical = world
        .logical_effect(&seal.logical)
        .ok_or(CaptureReject::LedgerMismatch)?;
    logical.result = Some(LogicalEffectResult::RetiredByCaptureDrain {
        attempt: seal.attempt,
        coverage,
    });
    next.logical_effects.put(seal.logical.clone(), logical);
    next.effects
        .put(seal, EffectStatus::RetiredByCaptureDrain(coverage));
    Ok(CaptureDecision { event, next })
}
