//! Terminal-only coverage of an original Freeze before any Full is dispatched.
//! Stop coverage is not a successful Freeze and never releases material itself.
use super::{CaptureCommit, RestoreEligibility};
use crate::effect::{EffectDecision, EffectReject};
use crate::{
    Authority, Command, EffectId, EffectOrigin, EffectOutcome, EffectStatus, Event,
    LogicalEffectResult, OperationInput, OperationKey, OperationKind, OperationStatus, Phase,
    StopCoverage, VmStatus, WorkerFence, World,
};

fn origin(world: &World, freeze: &EffectId) -> Option<OperationKey> {
    let Command::FreezeVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *freeze.command()
    else {
        return None;
    };
    let logical = world.logical_effect(&freeze.logical)?;
    let EffectOrigin::Operation(key) = logical.origin else {
        return None;
    };
    let ledger = world.operation_record(key)?;
    let plan = world.operation_plan(key)?;
    let capture = plan.capture?;
    if key.sandbox != sandbox
        || ledger.sandbox != sandbox
        || ledger.operation.id != key.operation
        || ledger.operation.kind != OperationKind::Pause
        || ledger.operation.epoch != epoch
        || ledger.input != (OperationInput::Pause { generation })
        || !matches!(ledger.operation.status, OperationStatus::Failed(_))
        || plan.key != key
        || plan.epoch != epoch
        || plan.generation != generation
        || if plan.sealed {
            plan.vm != Some(vm) || plan.target_snapshot.is_none()
        } else {
            plan.vm.is_some() || plan.target_snapshot.is_some()
        }
        || !plan.stop_vms.contains(&vm)
        || logical.current_attempt != freeze.attempt
        || capture.commit() != CaptureCommit::NotRegistered
        || capture.eligibility() != RestoreEligibility::Revoked
        || capture.has_unload_source()
        || capture.original_continuation().is_some()
    {
        return None;
    }
    // This first branch excludes even a queued Full, not merely a missing worker.
    if world.effect_ids().iter().any(|id| {
        world
            .logical_effect(&id.logical)
            .is_some_and(|l| l.origin == EffectOrigin::Operation(key))
            && matches!(
                id.command(),
                Command::SealSnapshot { .. } | Command::CommitSnapshot { .. }
            )
    }) {
        return None;
    }
    Some(key)
}

#[must_use]
pub fn stop_covered(freeze: &EffectId, coverage: StopCoverage) -> Option<Command> {
    let Command::FreezeVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *freeze.command()
    else {
        return None;
    };
    (coverage.operation.sandbox == sandbox && coverage.epoch > epoch).then_some(Command::StopVm {
        sandbox,
        epoch: coverage.epoch,
        generation,
        vm,
    })
}

#[must_use]
pub fn coverage_origin(world: &World, freeze: &EffectId, coverage: StopCoverage) -> bool {
    let Some(key) = origin(world, freeze) else {
        return false;
    };
    let Some(Command::StopVm {
        sandbox,
        generation,
        vm,
        ..
    }) = stop_covered(freeze, coverage)
    else {
        return false;
    };
    let Some(ledger) = world.operation_record(coverage.operation) else {
        return false;
    };
    let Some(plan) = world.operation_plan(coverage.operation) else {
        return false;
    };
    key != coverage.operation
        && ledger.sandbox == sandbox
        && ledger.operation.kind == OperationKind::Kill
        && ledger.operation.epoch == coverage.epoch
        && ledger.input == (OperationInput::Kill { generation })
        && plan.key == coverage.operation
        && plan.sealed
        && plan.epoch == coverage.epoch
        && plan.generation == generation
        && plan.stop_vms.contains(&vm)
}

#[must_use]
pub fn current_stop(world: &World, freeze: &EffectId) -> Option<Command> {
    let old = origin(world, freeze)?;
    let Command::FreezeVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *freeze.command()
    else {
        return None;
    };
    let row = world.sandbox_record(sandbox)?;
    let op = row.current_operation?;
    let key = OperationKey {
        sandbox,
        operation: op.id,
    };
    let plan = world.operation_plan(key)?;
    let old_plan = world.operation_plan(old)?;
    let machine = world.vm_record(vm)?;
    let slot = row.net_slot?;
    let network = world.slot_record(slot)?;
    let coverage = StopCoverage {
        operation: key,
        epoch: row.epoch,
        attempt: 0,
        result: crate::ResultDigest(0),
    };
    if !world.controller_up
        || !row.terminal
        || row.phase != Phase::Revoking
        || row.desired != Authority::Closed
        || row.effective != Authority::Closed
        || op.kind != OperationKind::Kill
        || op.epoch != row.epoch
        || row.epoch <= epoch
        || !matches!(
            op.status,
            OperationStatus::Pending | OperationStatus::OutcomeUnknown
        )
        || !coverage_origin(world, freeze, coverage)
        || row.generation != generation
        || row.current_vm != Some(vm)
        || !plan.close_slots.contains(&slot)
        || !old_plan.close_slots.contains(&slot)
        || !old_plan
            .capture
            .is_some_and(super::CaptureHandoff::has_capture_work)
        || machine.owner != row.owner
        || machine.sandbox != sandbox
        || machine.generation != generation
        || !matches!(
            machine.status,
            VmStatus::Executing | VmStatus::Frozen | VmStatus::Stopped
        )
        || network.owner != row.owner
        || network.sandbox != sandbox
        || network.generation != generation
        || network.vm != vm
        || network.policy != row.policy
        || !network.configured
        || network.active
        || network.reclaimed
    {
        return None;
    }
    let mut found = false;
    for id in world.effect_ids() {
        if id.command().sandbox() != sandbox || !crate::effect::outcome_is_unresolved(world, id) {
            continue;
        }
        if *id == *freeze {
            found = true;
            continue;
        }
        let logical = world.logical_effect(&id.logical)?;
        let terminal = logical.origin == EffectOrigin::Operation(key)
            && logical.current_attempt == id.attempt
            && logical.result.is_none()
            && (*id.command()
                == Command::InstallFence {
                    sandbox,
                    epoch: row.epoch,
                    terminal: true,
                }
                || *id.command()
                    == Command::StopVm {
                        sandbox,
                        epoch: row.epoch,
                        generation,
                        vm,
                    });
        if !terminal {
            return None;
        }
    }
    if !found || old == key {
        return None;
    }
    stop_covered(freeze, coverage)
}

#[must_use]
pub fn allowed(world: &World, command: &Command) -> bool {
    for freeze in world.effect_ids() {
        let Some(stop) = current_stop(world, freeze) else {
            continue;
        };
        let Command::StopVm { sandbox, epoch, .. } = stop else {
            continue;
        };
        let fence = Command::InstallFence {
            sandbox,
            epoch,
            terminal: true,
        };
        if *command == fence {
            return true;
        }
        if *command == stop
            && world.worker_fences.get_copy(&sandbox)
                == Some(WorkerFence {
                    epoch,
                    terminal: true,
                })
            && crate::effect::effect_succeeded_check(world, &fence)
        {
            return true;
        }
    }
    false
}

/// End only the original unknown Freeze using an acknowledged same-source Stop.
/// This never fabricates Pause success or releases a material responsibility.
/// # Errors
/// Rejects nonterminal, foreign, unfenced, unresolved or mismatched Stop evidence.
pub fn retire(
    world: &World,
    freeze: EffectId,
    stop: EffectId,
) -> Result<EffectDecision, EffectReject> {
    if !world.controller_up {
        return Err(EffectReject::ControllerDown);
    }
    let Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(result))) =
        world.effect_status(&stop)
    else {
        return Err(EffectReject::InvalidRetirementEvidence);
    };
    let logical = world
        .logical_effect(&stop.logical)
        .ok_or(EffectReject::InvalidRetirementEvidence)?;
    let EffectOrigin::Operation(key) = logical.origin else {
        return Err(EffectReject::InvalidRetirementEvidence);
    };
    let Command::StopVm {
        sandbox, epoch, vm, ..
    } = *stop.command()
    else {
        return Err(EffectReject::InvalidRetirementEvidence);
    };
    let coverage = StopCoverage {
        operation: key,
        epoch,
        attempt: stop.attempt,
        result,
    };
    if logical.current_attempt != stop.attempt
        || !coverage_origin(world, &freeze, coverage)
        || stop_covered(&freeze, coverage).as_ref() != Some(stop.command())
        || !crate::effect::effect_succeeded_check(world, stop.command())
        || !crate::effect::effect_succeeded_check(
            world,
            &Command::InstallFence {
                sandbox,
                epoch,
                terminal: true,
            },
        )
    {
        return Err(EffectReject::InvalidRetirementEvidence);
    }
    let row = world
        .sandbox_record(sandbox)
        .ok_or(EffectReject::InvalidRetirementEvidence)?;
    if !row.terminal
        || row.epoch != epoch
        || world.worker_fences.get_copy(&sandbox)
            != Some(WorkerFence {
                epoch,
                terminal: true,
            })
    {
        return Err(EffectReject::InvalidRetirementEvidence);
    }
    if world.effect_status(&freeze) == Some(EffectStatus::RetiredByStop(coverage)) {
        return Ok(EffectDecision {
            event: Event::RetireFreeze(freeze, stop),
            next: world.clone(),
        });
    }
    if current_stop(world, &freeze).as_ref() != Some(stop.command())
        || world
            .vm_record(vm)
            .is_none_or(|m| m.status != VmStatus::Stopped)
    {
        return Err(EffectReject::InvalidRetirementEvidence);
    }
    let old = world
        .logical_effect(&freeze.logical)
        .ok_or(EffectReject::LogicalEffectNotFound)?;
    let mut next = world.clone();
    next.effects
        .put(freeze.clone(), EffectStatus::RetiredByStop(coverage));
    let mut retired = old;
    retired.result = Some(LogicalEffectResult::RetiredByStop {
        attempt: freeze.attempt,
        coverage,
    });
    next.logical_effects.put(freeze.logical.clone(), retired);
    Ok(EffectDecision {
        event: Event::RetireFreeze(freeze, stop),
        next,
    })
}
