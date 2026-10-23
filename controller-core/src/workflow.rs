//! One durable decision at a time. This module performs no host I/O and never
//! returns a replacement World for the caller to write after a slow operation.
use crate::{
    Command, ControllerInput, EffectInput, EffectOrigin, EffectStatus, LogicalEffectResult,
    OperationKey, OperationKind, OperationStatus, Phase, PublicationInput, SandboxId,
    SchedulerInput, VmStatus, World,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowStep {
    Apply(ControllerInput),
    Claim,
    PublishRunning,
    Complete,
    Blocked,
}

fn scheduled(world: &World, command: &Command) -> bool {
    let mut index = 0;
    let mut found = false;
    while index < world.effect_ids().len() && !found {
        found = world.effect_ids()[index].command() == command;
        index += 1;
    }
    found
}

fn failed_create(world: &World, sandbox: SandboxId) -> Option<ControllerInput> {
    let record = world.sandbox_record(sandbox)?;
    let operation = record.current_operation?;
    if record.phase != Phase::Starting || operation.kind != OperationKind::Create {
        return None;
    }
    let key = OperationKey {
        sandbox,
        operation: operation.id,
    };
    let mut index = 0;
    let mut result = None;
    while index < world.effect_ids().len() && result.is_none() {
        let effect = &world.effect_ids()[index];
        if let Some(logical) = world.logical_effect(&effect.logical)
            && logical.origin == EffectOrigin::Operation(key)
            && logical.current_attempt == effect.attempt
            && let Some(EffectStatus::Acknowledged(crate::EffectOutcome::Failed(failure))) =
                world.effect_status(effect)
        {
            if logical.result.is_none() {
                let input = if failure.retryable() && logical.remaining_retries > 0 {
                    EffectInput::Retry(effect.logical.clone(), failure)
                } else {
                    EffectInput::FinalizeFailure(effect.logical.clone(), failure)
                };
                result = Some(ControllerInput::Effect(input));
            } else if logical.result
                == Some(LogicalEffectResult::Failed {
                    attempt: effect.attempt,
                    failure: failure,
                })
            {
                result = Some(ControllerInput::Request(crate::Request::Compensate(
                    crate::CompensationRequest {
                        sandbox,
                        failure: failure,
                        evidence: Some(effect.clone()),
                    },
                )));
            }
        }
        index += 1;
    }
    result
}

fn unresolved(world: &World, sandbox: SandboxId) -> bool {
    let mut index = 0;
    let mut found = false;
    while index < world.effect_ids().len() && !found {
        let effect = &world.effect_ids()[index];
        found = effect.command().sandbox() == sandbox
            && matches!(
                world.effect_status(effect),
                Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown)
            );
        index += 1;
    }
    found
}

fn claimable(world: &World, sandbox: SandboxId) -> bool {
    let unresolved = unresolved(world, sandbox);
    let mut index = 0;
    let mut found = false;
    while index < world.effect_ids().len() && !found {
        let effect = &world.effect_ids()[index];
        if effect.command().sandbox() == sandbox
            && world.effect_status(effect) == Some(EffectStatus::Committed)
            && !crate::effect::startup_dispatch_revoked(world, effect.command())
            && (!unresolved
                || crate::scheduler::dispatch_allowed_during_abort(world, effect.command()))
        {
            // A scheduling hint, not an execution permit. The journal previews
            // dispatch + worker eligibility in its authoritative transaction.
            found = true;
        }
        index += 1;
    }
    found
}

fn cleanup_vm(
    world: &World,
    sandbox: SandboxId,
    generation: crate::Generation,
) -> Option<WorkflowStep> {
    let resources = world.resource_index(sandbox);
    let mut index = 0;
    let mut result = None;
    while index < resources.vms.len() && result.is_none() {
        let vm = resources.vms[index];
        if !matches!(world.vm_record(vm), Some(record) if record.status == VmStatus::Reclaimed) {
            if !scheduled(
                world,
                &Command::CleanupVm {
                    sandbox,
                    generation,
                    vm,
                },
            ) {
                result = Some(WorkflowStep::Apply(ControllerInput::Scheduler(
                    SchedulerInput::CleanupVm(sandbox, generation, vm),
                )));
            } else {
                result = Some(WorkflowStep::Blocked);
            }
        }
        index += 1;
    }
    result
}

fn cleanup(world: &World, sandbox: SandboxId, generation: crate::Generation) -> WorkflowStep {
    if let Some(step) = cleanup_vm(world, sandbox, generation) {
        return step;
    }
    let resources = world.resource_index(sandbox);
    let mut index = 0;
    let mut result = WorkflowStep::Complete;
    let mut found = false;
    while index < resources.slots.len() && !found {
        let slot = resources.slots[index];
        if !matches!(world.slot_record(slot), Some(record) if record.reclaimed) {
            found = true;
            if !scheduled(
                world,
                &Command::CleanupSlot {
                    sandbox,
                    generation,
                    slot,
                },
            ) {
                result = WorkflowStep::Apply(ControllerInput::Scheduler(
                    SchedulerInput::CleanupSlot(sandbox, generation, slot),
                ));
            } else {
                result = WorkflowStep::Blocked;
            }
        }
        index += 1;
    }
    result
}

/// Select one synchronous input from the current committed state. The service
/// must still run the ordinary reducer/transaction guards before committing it.
#[must_use]
pub fn next_step(world: &World, sandbox: SandboxId) -> WorkflowStep {
    let Some(record) = world.sandbox_record(sandbox) else {
        return WorkflowStep::Blocked;
    };
    let Some(operation) = record.current_operation else {
        return WorkflowStep::Blocked;
    };
    if let Some(input) = failed_create(world, sandbox) {
        return WorkflowStep::Apply(input);
    }
    let key = OperationKey {
        sandbox,
        operation: operation.id,
    };
    let Some(plan) = world.operation_plan(key) else {
        return WorkflowStep::Blocked;
    };
    if record.phase == Phase::Starting
        && operation.kind == OperationKind::Create
        && plan.vm.is_none()
    {
        return WorkflowStep::Apply(ControllerInput::Scheduler(SchedulerInput::ReserveStartup(
            key,
        )));
    }
    if claimable(world, sandbox) {
        return WorkflowStep::Claim;
    }
    if record.phase == Phase::Starting && operation.kind == OperationKind::Create {
        if plan.vm.is_none() {
            return WorkflowStep::Apply(ControllerInput::Scheduler(
                SchedulerInput::ReserveStartup(key),
            ));
        }
        let (Some(vm), Some(slot)) = (plan.vm, plan.slot) else {
            return WorkflowStep::Blocked;
        };
        let prepare = Command::PrepareVm {
            sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            vm,
            slot,
            source: plan.restore_from,
        };
        if !scheduled(world, &prepare) {
            return WorkflowStep::Apply(ControllerInput::Scheduler(
                SchedulerInput::DispatchPrepare(sandbox, vm, slot),
            ));
        }
        if unresolved(world, sandbox) {
            return WorkflowStep::Blocked;
        }
        return WorkflowStep::PublishRunning;
    }
    if record.phase == Phase::Running {
        return WorkflowStep::Complete;
    }
    if record.phase == Phase::Destroyed {
        return cleanup(world, sandbox, record.generation);
    }
    if record.phase != Phase::Revoking {
        return WorkflowStep::Blocked;
    }
    let failure = match (operation.kind, operation.status) {
        (OperationKind::Kill, _) => None,
        (
            OperationKind::Create,
            OperationStatus::Compensating(failure) | OperationStatus::Failed(failure),
        ) => Some(failure),
        _ => return WorkflowStep::Blocked,
    };
    if failure.is_some()
        && let (Some(vm), Some(slot)) = (plan.vm, plan.slot)
        && world.vm_record(vm).is_none()
    {
        let command = Command::AbortPrepare {
            sandbox,
            epoch: record.epoch,
            generation: record.generation,
            vm,
            slot,
        };
        if !scheduled(world, &command) {
            return WorkflowStep::Apply(ControllerInput::Scheduler(SchedulerInput::AbortPrepare(
                sandbox,
                record.generation,
                vm,
                slot,
            )));
        }
    }
    if let Some(step) = stop_next(world, sandbox, record, plan) {
        return step;
    }
    if let Some(step) = retire_next(world, sandbox) {
        return step;
    }
    let publication = match failure {
        Some(failure) => PublicationInput::Failed(sandbox, failure),
        None => PublicationInput::Destroyed(sandbox),
    };
    WorkflowStep::Apply(ControllerInput::Publication(publication))
}

fn stop_next(
    world: &World,
    sandbox: SandboxId,
    record: &crate::SandboxRecord,
    plan: &crate::OperationPlan,
) -> Option<WorkflowStep> {
    let mut index = 0;
    let mut result = None;
    while index < plan.stop_vms.len() && result.is_none() {
        let vm = plan.stop_vms[index];
        if let Some(machine) = world.vm_record(vm)
            && !matches!(machine.status, VmStatus::Stopped | VmStatus::Reclaimed)
        {
            let command = Command::StopVm {
                sandbox,
                epoch: record.epoch,
                generation: record.generation,
                vm,
            };
            if !scheduled(world, &command) {
                result = Some(WorkflowStep::Apply(ControllerInput::Scheduler(
                    SchedulerInput::StopVm(sandbox, record.generation, vm),
                )));
            } else {
                result = Some(WorkflowStep::Blocked);
            }
        }
        index += 1;
    }
    result
}

fn retire_next(world: &World, sandbox: SandboxId) -> Option<WorkflowStep> {
    let mut index = 0;
    let mut result = None;
    while index < world.effect_ids().len() && result.is_none() {
        let effect = &world.effect_ids()[index];
        if effect.command().sandbox() == sandbox
            && matches!(
                world.effect_status(effect),
                Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown)
            )
        {
            result = Some(WorkflowStep::Blocked);
            if let Some(command) = crate::terminal::activation_stop_current(world, effect) {
                let logical = crate::LogicalEffectId { command };
                if let Some(row) = world.logical_effect(&logical) {
                    let stop = crate::EffectId {
                        logical,
                        attempt: row.current_attempt,
                    };
                    result = Some(WorkflowStep::Apply(ControllerInput::Effect(
                        EffectInput::RetireActivation {
                            activation: effect.clone(),
                            stop,
                        },
                    )));
                }
            }
        }
        index += 1;
    }
    result
}
