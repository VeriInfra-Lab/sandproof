//! One durable decision at a time. This module performs no host I/O and never
//! returns a replacement World for the caller to write after a slow operation.
use crate::effect::outcome_is_unresolved as effect_unresolved;
use crate::{
    Command, ControllerInput, EffectInput, EffectOrigin, EffectStatus, LogicalEffectResult,
    OperationKey, OperationKind, OperationStatus, Phase, PublicationInput, SandboxId,
    SchedulerInput, VmStatus, World,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowStep {
    Apply(Box<ControllerInput>),
    Claim,
    PublishRunning,
    Complete,
    Blocked,
}

fn apply(input: ControllerInput) -> WorkflowStep {
    WorkflowStep::Apply(Box::new(input))
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
        result = failed_create_effect(world, key, effect);
        index += 1;
    }
    result
}

// A single real production decision, shared by the scan and its extraction.
// Keeping the first matching effect remains the scan's only ordering rule.
fn failed_create_effect(
    world: &World,
    key: OperationKey,
    effect: &crate::EffectId,
) -> Option<ControllerInput> {
    let logical = world.logical_effect(&effect.logical)?;
    if logical.origin != EffectOrigin::Operation(key) || logical.current_attempt != effect.attempt {
        return None;
    }
    let Some(EffectStatus::Acknowledged(crate::EffectOutcome::Failed(failure))) =
        world.effect_status(effect)
    else {
        return None;
    };
    if logical.result.is_none() {
        let logical_id = effect.logical.clone();
        if failure.retryable() && logical.remaining_retries > 0 {
            return Some(ControllerInput::Effect(EffectInput::Retry(
                logical_id, failure,
            )));
        }
        return Some(ControllerInput::Effect(EffectInput::FinalizeFailure(
            logical_id, failure,
        )));
    }
    if logical.result
        == Some(LogicalEffectResult::Failed {
            attempt: effect.attempt,
            failure,
        })
    {
        return Some(ControllerInput::Request(crate::Request::Compensate(
            crate::CompensationRequest {
                sandbox: key.sandbox,
                failure,
                evidence: Some(effect.clone()),
            },
        )));
    }
    None
}

fn unresolved(world: &World, sandbox: SandboxId) -> bool {
    let mut index = 0;
    let mut found = false;
    while index < world.effect_ids().len() && !found {
        let effect = &world.effect_ids()[index];
        found = effect.command().sandbox() == sandbox && effect_unresolved(world, effect);
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
        if !vm_reclaimed(world, vm) {
            if scheduled(
                world,
                &Command::CleanupVm {
                    sandbox,
                    generation,
                    vm,
                },
            ) {
                result = Some(WorkflowStep::Blocked);
            } else {
                result = Some(apply(ControllerInput::Scheduler(
                    SchedulerInput::CleanupVm(sandbox, generation, vm),
                )));
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
        if !slot_reclaimed(world, slot) {
            found = true;
            if scheduled(
                world,
                &Command::CleanupSlot {
                    sandbox,
                    generation,
                    slot,
                },
            ) {
                result = WorkflowStep::Blocked;
            } else {
                result = apply(ControllerInput::Scheduler(SchedulerInput::CleanupSlot(
                    sandbox, generation, slot,
                )));
            }
        }
        index += 1;
    }
    result
}

fn vm_reclaimed(world: &World, vm: crate::VmId) -> bool {
    let Some(record) = world.vm_record(vm) else {
        return false;
    };
    record.status == VmStatus::Reclaimed
}

fn slot_reclaimed(world: &World, slot: crate::NetSlotId) -> bool {
    let Some(record) = world.slot_record(slot) else {
        return false;
    };
    record.reclaimed
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
        return apply(input);
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
        return apply(ControllerInput::Scheduler(SchedulerInput::ReserveStartup(
            key,
        )));
    }
    if claimable(world, sandbox) {
        return WorkflowStep::Claim;
    }
    if record.phase == Phase::Starting && operation.kind == OperationKind::Create {
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
            return apply(ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
                sandbox, vm, slot,
            )));
        }
        if unresolved(world, sandbox) {
            return WorkflowStep::Blocked;
        }
        return WorkflowStep::PublishRunning;
    }
    if let Some(step) = paused_workflow_step(world, sandbox, record, plan) {
        return step;
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
            return apply(ControllerInput::Scheduler(SchedulerInput::AbortPrepare(
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
    apply(ControllerInput::Publication(publication))
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
        if vm_needs_stop(world, vm) {
            let command = Command::StopVm {
                sandbox,
                epoch: record.epoch,
                generation: record.generation,
                vm,
            };
            if scheduled(world, &command) {
                result = Some(WorkflowStep::Blocked);
            } else {
                result = Some(apply(ControllerInput::Scheduler(SchedulerInput::StopVm(
                    sandbox,
                    record.generation,
                    vm,
                ))));
            }
        }
        index += 1;
    }
    result
}

fn vm_needs_stop(world: &World, vm: crate::VmId) -> bool {
    let Some(record) = world.vm_record(vm) else {
        return false;
    };
    let status = record.status;
    status != VmStatus::Stopped && status != VmStatus::Reclaimed
}

fn retire_next(world: &World, sandbox: SandboxId) -> Option<WorkflowStep> {
    let mut index = 0;
    let mut result = None;
    while index < world.effect_ids().len() && result.is_none() {
        let effect = &world.effect_ids()[index];
        if effect.command().sandbox() == sandbox && effect_unresolved(world, effect) {
            result = Some(WorkflowStep::Blocked);
            if let Some(command) = crate::terminal::activation_stop_current(world, effect) {
                let logical = crate::LogicalEffectId { command };
                if let Some(row) = world.logical_effect(&logical) {
                    let stop = crate::EffectId {
                        logical,
                        attempt: row.current_attempt,
                    };
                    result = Some(apply(ControllerInput::Effect(
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

fn pause_step(
    world: &World,
    sandbox: SandboxId,
    record: &crate::SandboxRecord,
    plan: &crate::OperationPlan,
) -> WorkflowStep {
    let key = plan.key;
    let Some(source) = plan.target_snapshot else {
        return apply(ControllerInput::Scheduler(
            SchedulerInput::ReservePauseSnapshot(key),
        ));
    };
    let Some(vm) = plan.vm else {
        return WorkflowStep::Blocked;
    };
    let Some(physical) = world.vm_record(vm) else {
        return WorkflowStep::Blocked;
    };
    let seal = Command::SealSnapshot {
        sandbox,
        epoch: plan.epoch,
        generation: plan.generation,
        vm,
        workspace: record.workspace,
        snapshot: source.snapshot,
        manifest: source.manifest,
    };
    if !scheduled(world, &seal) {
        if physical.status != VmStatus::Frozen || unresolved(world, sandbox) {
            return WorkflowStep::Blocked;
        }
        return apply(ControllerInput::Scheduler(SchedulerInput::SealSnapshot(
            sandbox,
            source.snapshot,
            source.manifest,
        )));
    }
    if !crate::effect::effect_succeeded_check(world, &seal) {
        return WorkflowStep::Blocked;
    }
    let commit = Command::CommitSnapshot {
        sandbox,
        epoch: plan.epoch,
        generation: plan.generation,
        snapshot: source.snapshot,
        manifest: source.manifest,
    };
    if !scheduled(world, &commit) {
        return apply(ControllerInput::Scheduler(SchedulerInput::CommitSnapshot(
            sandbox,
            source.snapshot,
        )));
    }
    if !crate::effect::effect_succeeded_check(world, &commit) {
        return WorkflowStep::Blocked;
    }
    let stop = Command::StopVm {
        sandbox,
        epoch: plan.epoch,
        generation: plan.generation,
        vm,
    };
    if !crate::effect::effect_succeeded_check(world, &stop) {
        return WorkflowStep::Blocked;
    }
    apply(ControllerInput::Publication(PublicationInput::Paused(
        sandbox, source,
    )))
}

fn paused_workflow_step(
    world: &World,
    sandbox: SandboxId,
    record: &crate::SandboxRecord,
    plan: &crate::OperationPlan,
) -> Option<WorkflowStep> {
    if record.phase == Phase::Pausing
        && record
            .current_operation
            .is_some_and(|operation| operation.kind == OperationKind::Pause)
    {
        return Some(pause_step(world, sandbox, record, plan));
    }
    if record.phase == Phase::Paused {
        return Some(cleanup(world, sandbox, record.generation));
    }
    None
}
