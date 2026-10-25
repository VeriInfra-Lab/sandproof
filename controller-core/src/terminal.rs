//! Narrow terminal coverage of an unknown startup activation. No host I/O or
//! timers live here: the caller must first persist exact stop evidence.

use crate::effect::{EffectDecision, EffectReject, effect_succeeded_check};
use crate::{
    Command, EffectId, EffectOrigin, EffectOutcome, EffectStatus, Event, LogicalEffectResult,
    OperationKey, OperationKind, OperationStatus, Phase, SandboxId, StopCoverage, VmStatus,
    WorkerFence, World,
};

/// An already dispatched startup effect can finish after cancellation. It may
/// add physical resource facts to the original compensation, never reopen
/// access, replace its operation, or authorize another startup command.
#[must_use]
pub fn startup_effect_in_current_abort(world: &World, effect: &EffectId) -> bool {
    let command = effect.command();
    if crate::capture::continuation::activation_in_current_abort(world, effect) {
        return true;
    }
    let (sandbox, epoch, generation) = match *command {
        Command::InstallFence {
            sandbox,
            epoch,
            terminal: false,
        } => {
            let Some(record) = world.sandbox_record(sandbox) else {
                return false;
            };
            (sandbox, epoch, record.generation)
        }
        Command::PrepareVm {
            sandbox,
            epoch,
            generation,
            ..
        }
        | Command::ConfigureNetwork {
            sandbox,
            epoch,
            generation,
            ..
        }
        | Command::ActivateVm {
            sandbox,
            epoch,
            generation,
            ..
        }
        | Command::ApplyFilePolicy {
            sandbox,
            epoch,
            generation,
            ..
        } => (sandbox, epoch, generation),
        _ => return false,
    };
    let Some(record) = world.sandbox_record(sandbox) else {
        return false;
    };
    let Some(operation) = record.current_operation else {
        return false;
    };
    let Some(logical) = world.logical_effect(&effect.logical) else {
        return false;
    };
    let EffectOrigin::Operation(original_key) = logical.origin else {
        return false;
    };
    let Some(original) = world.operation_record(original_key) else {
        return false;
    };
    let Some(plan) = world.operation_plan(original_key) else {
        return false;
    };
    let same_compensation = operation.id == original_key.operation
        && operation.kind == OperationKind::Create
        && matches!(operation.status, OperationStatus::Compensating(_))
        && record.epoch == epoch;
    let later_kill = operation.kind == OperationKind::Kill
        && operation.id != original_key.operation
        && record.epoch > epoch;
    let command_matches = startup_command_matches(world, command, plan, record);
    record.terminal
        && record.phase == Phase::Revoking
        && (same_compensation || later_kill)
        && original.operation.kind == OperationKind::Create
        && original_key.sandbox == sandbox
        && plan.key == original_key
        && record.generation == generation
        && plan.generation == generation
        && plan.epoch == epoch
        && plan.sealed
        && command_matches
        && logical.current_attempt == effect.attempt
        && logical.result.is_none()
        && matches!(
            world.effect_status(effect),
            Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown)
        )
}

fn startup_command_matches(
    world: &World,
    command: &Command,
    plan: &crate::OperationPlan,
    record: &crate::SandboxRecord,
) -> bool {
    match *command {
        Command::PrepareVm {
            vm, slot, source, ..
        } => plan.vm == Some(vm) && plan.slot == Some(slot) && plan.restore_from == source,
        Command::ConfigureNetwork {
            vm,
            slot,
            ref policy,
            ..
        } => plan.vm == Some(vm) && plan.slot == Some(slot) && record.policy == *policy,
        Command::ActivateVm { vm, .. } => {
            plan.vm == Some(vm)
                && world.vm_record(vm).is_some_and(|machine| {
                    matches!(machine.status, VmStatus::Prepared | VmStatus::Executing)
                })
        }
        Command::ApplyFilePolicy {
            workspace, policy, ..
        } => record.workspace == workspace && record.file_policy == policy,
        Command::InstallFence {
            terminal: false, ..
        } => true,
        _ => false,
    }
}

/// Account for an original physical startup result after termination. This
/// transition preserves the current operation and closed authorization.
pub(crate) fn record_late_startup(
    world: &World,
    effect: &EffectId,
) -> Result<crate::WorkerDecision, crate::WorkerReject> {
    use crate::{AppliedFilePolicy, WorkerDecision, WorkerEvent, WorkerReject};
    if !startup_effect_in_current_abort(world, effect) {
        return Err(WorkerReject::IdentityMismatch);
    }
    let sandbox = effect.command().sandbox();
    let record = world
        .sandbox_record(sandbox)
        .ok_or(WorkerReject::SandboxNotFound)?;
    let mut next = world.clone();
    let event = match *effect.command() {
        Command::InstallFence {
            epoch,
            terminal: false,
            ..
        } => {
            let replace = match world.worker_fences.get(&sandbox) {
                Some(prior) => prior.epoch < epoch,
                None => true,
            };
            if replace {
                next.worker_fences.put(
                    sandbox,
                    WorkerFence {
                        epoch,
                        terminal: false,
                    },
                );
            }
            WorkerEvent::InstallFence { sandbox }
        }
        Command::PrepareVm {
            vm, slot, source, ..
        } => {
            next = late_prepare(world, sandbox, record, vm, slot)?;
            WorkerEvent::PrepareVm {
                sandbox,
                vm,
                slot,
                source,
            }
        }
        Command::ConfigureNetwork {
            vm,
            slot,
            generation,
            ref policy,
            ..
        } => {
            let stored = world
                .slot_record(slot)
                .ok_or(WorkerReject::ResourceNotFound)?;
            if stored.owner != record.owner
                || stored.sandbox != sandbox
                || stored.generation != generation
                || stored.vm != vm
                || stored.policy != *policy
            {
                return Err(WorkerReject::IdentityMismatch);
            }
            let mut updated = stored.clone();
            updated.configured = true;
            next.net_slots.put(slot, updated);
            WorkerEvent::ConfigureNetwork { sandbox, slot }
        }
        Command::ActivateVm { vm, generation, .. } => {
            let updated = late_activated_vm(world, effect, record, vm, generation)?;
            next.vms.put(vm, updated);
            WorkerEvent::ActivateVm { sandbox, vm }
        }
        Command::ApplyFilePolicy {
            generation,
            workspace,
            policy,
            ..
        } => {
            let applied = AppliedFilePolicy {
                sandbox,
                generation,
                workspace,
                policy,
            };
            next.applied_file_policies.put(sandbox, applied);
            WorkerEvent::ApplyFilePolicy {
                sandbox,
                generation,
            }
        }
        _ => return Err(WorkerReject::IdentityMismatch),
    };
    Ok(WorkerDecision { event, next })
}

fn late_activated_vm(
    world: &World,
    effect: &EffectId,
    record: &crate::SandboxRecord,
    vm: crate::VmId,
    generation: crate::Generation,
) -> Result<crate::Vm, crate::WorkerReject> {
    use crate::WorkerReject;
    let stored = world.vm_record(vm).ok_or(WorkerReject::ResourceNotFound)?;
    if stored.owner != record.owner
        || stored.sandbox != effect.command().sandbox()
        || stored.generation != generation
        || (stored.status != VmStatus::Prepared
            && !(crate::capture::continuation::activation_in_current_abort(world, effect)
                && matches!(stored.status, VmStatus::Frozen | VmStatus::Executing)))
    {
        return Err(WorkerReject::IdentityMismatch);
    }
    let mut updated = *stored;
    updated.status = VmStatus::Executing;
    Ok(updated)
}

fn late_prepare(
    world: &World,
    sandbox: SandboxId,
    record: &crate::SandboxRecord,
    vm: crate::VmId,
    slot: crate::NetSlotId,
) -> Result<World, crate::WorkerReject> {
    use crate::WorkerReject;
    if world.vm_record(vm).is_some()
        || world.slot_record(slot).is_some()
        || record.current_vm.is_some_and(|current| current != vm)
        || record.net_slot.is_some_and(|current| current != slot)
    {
        return Err(WorkerReject::IdentityMismatch);
    }
    let mut next = crate::worker::prepare_resources(world, sandbox, record, vm, slot);
    let current = record
        .current_operation
        .ok_or(WorkerReject::OperationNotFound)?;
    let key = OperationKey {
        sandbox,
        operation: current.id,
    };
    let mut plan = next
        .operation_plan(key)
        .ok_or(WorkerReject::PlanNotFound)?
        .clone();
    if !plan.stop_vms.contains(&vm) {
        plan.stop_vms.push(vm);
    }
    if !plan.close_slots.contains(&slot) {
        plan.close_slots.push(slot);
    }
    next.operation_plans.put(key, plan);
    Ok(next)
}

/// Bind an activation to the current terminal create and its sealed resources.
/// This is not permission to execute it again; only fence/stop may proceed.
#[must_use]
pub fn activation_in_current_abort(world: &World, effect: &EffectId) -> bool {
    if crate::capture::continuation::activation_bound_in_current_abort(world, effect) {
        return true;
    }
    let Command::ActivateVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *effect.command()
    else {
        return false;
    };
    let Some(record) = world.sandboxes.get(&sandbox) else {
        return false;
    };
    let Some(operation) = record.current_operation else {
        return false;
    };
    let key = OperationKey {
        sandbox,
        operation: operation.id,
    };
    let Some(logical) = world.logical_effects.get(&effect.logical) else {
        return false;
    };
    let EffectOrigin::Operation(original_key) = logical.origin else {
        return false;
    };
    let Some(plan) = world.operation_plans.get(&original_key) else {
        return false;
    };
    let Some(current_plan) = world.operation_plans.get(&key) else {
        return false;
    };
    let Some(original) = world.operation_record(original_key) else {
        return false;
    };
    let resume = resume_activation_origin(world, original, plan, effect);
    if original.operation.kind != OperationKind::Create && !resume {
        return false;
    }
    let same_compensation = original_key == key
        && operation.kind == OperationKind::Create
        && matches!(operation.status, OperationStatus::Compensating(_))
        && record.epoch == epoch;
    let later_kill =
        original_key != key && operation.kind == OperationKind::Kill && record.epoch > epoch;
    if resume
        && (!later_kill
            || current_plan.key != key
            || !current_plan.sealed
            || current_plan.epoch != record.epoch
            || current_plan.generation != generation)
    {
        return false;
    }
    let Some(slot) = plan.slot else {
        return false;
    };
    record.phase == Phase::Revoking
        && record.terminal
        && (same_compensation || later_kill)
        && operation.epoch == record.epoch
        && record.generation == generation
        && record.current_vm == Some(vm)
        && record.net_slot == Some(slot)
        && plan.key == original_key
        && plan.sealed
        && plan.epoch == epoch
        && plan.generation == generation
        && plan.vm == Some(vm)
        && plan.slot == Some(slot)
        && plan.stop_vms.contains(&vm)
        && plan.close_slots.contains(&slot)
        && current_plan.stop_vms.contains(&vm)
        && current_plan.close_slots.contains(&slot)
        && logical.current_attempt == effect.attempt
        && logical.result.is_none()
        && activation_resources_match(world, record, sandbox, generation, vm, slot)
}

// A restored candidate can be stopped without learning whether activation
// succeeded. Its load must already be acknowledged for this exact sealed
// source/VM/slot; an unknown Prepare is never covered by this exception.
fn resume_activation_origin(
    world: &World,
    original: &crate::OperationLedgerRecord,
    plan: &crate::OperationPlan,
    effect: &EffectId,
) -> bool {
    let crate::OperationInput::Resume { source } = original.input else {
        return false;
    };
    let Command::ActivateVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *effect.command()
    else {
        return false;
    };
    let Some(slot) = plan.slot else {
        return false;
    };
    original.operation.kind == OperationKind::Resume
        && original.sandbox == sandbox
        && original.operation.id == plan.key.operation
        && original.operation.epoch == epoch
        && plan.key.sandbox == sandbox
        && plan.sealed
        && plan.epoch == epoch
        && plan.generation == generation
        && plan.vm == Some(vm)
        && plan.restore_from == Some(source)
        && effect_succeeded_check(
            world,
            &Command::PrepareVm {
                sandbox,
                epoch,
                generation,
                vm,
                slot,
                source: Some(source),
            },
        )
}

fn activation_resources_match(
    world: &World,
    record: &crate::SandboxRecord,
    sandbox: SandboxId,
    generation: crate::Generation,
    vm: crate::VmId,
    slot: crate::NetSlotId,
) -> bool {
    let Some(machine) = world.vms.get(&vm) else {
        return false;
    };
    let Some(network) = world.net_slots.get(&slot) else {
        return false;
    };
    machine.owner == record.owner
        && machine.sandbox == sandbox
        && machine.generation == generation
        && machine.status != VmStatus::Reclaimed
        && network.owner == record.owner
        && network.sandbox == sandbox
        && network.generation == generation
        && network.vm == vm
        && network.policy == record.policy
        && network.configured
        && !network.reclaimed
}

/// Only the original activation may be unknown in this exception. An unknown
/// fence, stop, network mutation or unrelated action must first be reconciled.
#[must_use]
pub fn only_uncertain_activation_for_abort(world: &World, sandbox: SandboxId) -> bool {
    let mut seen = false;
    let mut valid = true;
    let mut index = 0;
    while index < world.effect_ids().len() && valid {
        let id = &world.effect_ids()[index];
        if id.command().sandbox() == sandbox && crate::effect::outcome_is_unresolved(world, id) {
            valid = activation_in_current_abort(world, id);
            seen = true;
        }
        index += 1;
    }
    seen && valid
}

/// The exact stop that can cover this activation, derived rather than chosen
/// by a caller. Compensation preserves the original create epoch/generation.
#[must_use]
pub fn activation_stop(effect: &EffectId) -> Option<Command> {
    match *effect.command() {
        Command::ActivateVm {
            sandbox,
            epoch,
            generation,
            vm,
            ..
        } => Some(Command::StopVm {
            sandbox,
            epoch,
            generation,
            vm,
        }),
        _ => None,
    }
}

/// Current termination uses its own fence epoch, including when it supersedes
/// the activation's Create. This derives identity only, not proof of stopping.
#[must_use]
pub fn activation_stop_current(world: &World, effect: &EffectId) -> Option<Command> {
    let Command::ActivateVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *effect.command()
    else {
        return None;
    };
    let record = world.sandbox_record(sandbox)?;
    if !record.terminal || record.epoch < epoch || record.generation != generation {
        return None;
    }
    Some(Command::StopVm {
        sandbox,
        epoch: record.epoch,
        generation,
        vm,
    })
}

/// The exact durable `StopCoverage` relation. It survives publication and
/// cleanup; an older Create origin remains distinct from a newer Kill origin.
#[must_use]
pub fn activation_stop_covered(effect: &EffectId, coverage: StopCoverage) -> Option<Command> {
    let Command::ActivateVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *effect.command()
    else {
        return None;
    };
    if coverage.operation.sandbox != sandbox || coverage.epoch < epoch {
        return None;
    }
    Some(Command::StopVm {
        sandbox,
        epoch: coverage.epoch,
        generation,
        vm,
    })
}

#[must_use]
pub fn activation_coverage_origin(
    world: &World,
    effect: &EffectId,
    coverage: StopCoverage,
) -> bool {
    let Some(command) = activation_stop_covered(effect, coverage) else {
        return false;
    };
    let Some(logical) = world.logical_effect(&effect.logical) else {
        return false;
    };
    let EffectOrigin::Operation(original_key) = logical.origin else {
        return false;
    };
    let Some(original) = world.operation_record(original_key) else {
        return false;
    };
    let Some(original_plan) = world.operation_plan(original_key) else {
        return false;
    };
    let Some(stop_ledger) = world.operation_record(coverage.operation) else {
        return false;
    };
    let Some(stop_plan) = world.operation_plan(coverage.operation) else {
        return false;
    };
    let Command::ActivateVm {
        sandbox,
        epoch,
        generation,
        vm,
    } = *effect.command()
    else {
        return false;
    };
    let same = original_key == coverage.operation
        && coverage.epoch == epoch
        && stop_ledger.operation.kind == OperationKind::Create;
    let later = original_key != coverage.operation
        && coverage.epoch > epoch
        && stop_ledger.operation.kind == OperationKind::Kill;
    original_key.sandbox == sandbox
        && (original.operation.kind == OperationKind::Create
            && original_plan.sealed
            && original_plan.vm == Some(vm)
            || resume_activation_origin(world, original, original_plan, effect)
            || original.operation.kind == OperationKind::Pause
                && crate::capture::continuation::activation_belongs_to_continuation(
                    original_plan,
                    effect.command(),
                ))
        && original_plan.epoch == epoch
        && original_plan.generation == generation
        && (same || later)
        && stop_ledger.operation.epoch == coverage.epoch
        && stop_plan.epoch == coverage.epoch
        && stop_plan.generation == generation
        && stop_plan.stop_vms.contains(&vm)
        && command.sandbox() == sandbox
}

/// Retire an unknown activation without inventing its outcome. The reference
/// to the acknowledged stop is persisted alongside the original identity.
///
/// # Errors
/// Rejects any nonterminal, stale, mismatched, unstopped or unfenced resource,
/// or any stop that lacks a successful current-attempt receipt.
pub fn handle_retire_activation(
    world: &World,
    activation: EffectId,
    stop: EffectId,
) -> Result<EffectDecision, EffectReject> {
    if !world.controller_up {
        return Err(EffectReject::ControllerDown);
    }
    if activation_stop_current(world, &activation).as_ref() != Some(stop.command()) {
        return Err(EffectReject::InvalidRetirementEvidence);
    }
    let Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(result))) =
        world.effect_status(&stop)
    else {
        return Err(EffectReject::InvalidRetirementEvidence);
    };
    let Some(stop_logical) = world.logical_effects.get(&stop.logical) else {
        return Err(EffectReject::LogicalEffectNotFound);
    };
    let EffectOrigin::Operation(key) = stop_logical.origin else {
        return Err(EffectReject::InvalidRetirementEvidence);
    };
    if stop_logical.current_attempt != stop.attempt {
        return Err(EffectReject::InvalidRetirementEvidence);
    }
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
    match world.effect_status(&activation) {
        Some(EffectStatus::RetiredByStop(stored)) if stored == coverage => {
            return Ok(EffectDecision {
                event: Event::RetireActivation(activation, stop),
                next: world.clone(),
            });
        }
        Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown) => {}
        _ => return Err(EffectReject::NotAwaitingResult),
    }
    if !activation_in_current_abort(world, &activation) {
        return Err(EffectReject::InvalidRetirementEvidence);
    }
    let logical = *world
        .logical_effects
        .get(&activation.logical)
        .ok_or(EffectReject::LogicalEffectNotFound)?;
    let fence = Command::InstallFence {
        sandbox,
        epoch,
        terminal: true,
    };
    let record = world
        .sandboxes
        .get(&sandbox)
        .ok_or(EffectReject::InvalidRetirementEvidence)?;
    let slot = record
        .net_slot
        .ok_or(EffectReject::InvalidRetirementEvidence)?;
    if !activation_coverage_origin(world, &activation, coverage)
        || world.worker_fences.get(&sandbox)
            != Some(&WorkerFence {
                epoch,
                terminal: true,
            })
        || !effect_succeeded_check(world, &fence)
        || world
            .vms
            .get(&vm)
            .is_none_or(|machine| machine.status != VmStatus::Stopped)
        || world
            .net_slots
            .get(&slot)
            .is_none_or(|network| network.active)
    {
        return Err(EffectReject::InvalidRetirementEvidence);
    }
    let mut next = world.clone();
    next.effects
        .put(activation.clone(), EffectStatus::RetiredByStop(coverage));
    let mut retired = logical;
    retired.result = Some(LogicalEffectResult::RetiredByStop {
        attempt: activation.attempt,
        coverage,
    });
    next.logical_effects
        .put(activation.logical.clone(), retired);
    Ok(EffectDecision {
        event: Event::RetireActivation(activation, stop),
        next,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    fn step(world: &World, input: ControllerInput) -> World {
        handle_controller(world, input).unwrap().next
    }

    fn execute(world: &World, predicate: impl Fn(&Command) -> bool) -> World {
        let effect = world
            .effect_ids()
            .iter()
            .find(|id| predicate(id.command()))
            .unwrap()
            .clone();
        let dispatched = step(
            world,
            ControllerInput::Effect(EffectInput::Dispatch(effect.clone())),
        );
        let updated = step(&dispatched, ControllerInput::Worker(effect.clone()));
        step(
            &updated,
            ControllerInput::Effect(EffectInput::Receipt(
                effect,
                EffectOutcome::Succeeded(ResultDigest(1)),
            )),
        )
    }

    fn pending_activation() -> (World, EffectId) {
        let sandbox = SandboxId(1);
        let key = OperationKey {
            sandbox,
            operation: OperationId(1),
        };
        let world = step(
            &initial_world(),
            ControllerInput::Request(Request::Create(CreateRequest {
                actor: Actor::Tenant(TenantId(1)),
                owner: TenantId(1),
                sandbox,
                workspace: FsId(1),
                file_policy: FilePolicyRef {
                    digest: PolicyDigest(1),
                },
                policy: NetworkPolicy::DenyAll,
                operation: key.operation,
            })),
        );
        let world = execute(&world, |c| {
            matches!(
                c,
                Command::InstallFence {
                    terminal: false,
                    ..
                }
            )
        });
        let world = step(
            &world,
            ControllerInput::Scheduler(SchedulerInput::ReserveStartup(key)),
        );
        let plan = world.operation_plan(key).unwrap();
        let world = step(
            &world,
            ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
                sandbox,
                plan.vm.unwrap(),
                plan.slot.unwrap(),
            )),
        );
        let world = execute(&world, |c| matches!(c, Command::PrepareVm { .. }));
        let world = execute(&world, |c| matches!(c, Command::ConfigureNetwork { .. }));
        let world = execute(&world, |c| matches!(c, Command::ApplyFilePolicy { .. }));
        let activation = world
            .effect_ids()
            .iter()
            .find(|id| matches!(id.command(), Command::ActivateVm { .. }))
            .unwrap()
            .clone();
        let world = step(
            &world,
            ControllerInput::Effect(EffectInput::Dispatch(activation.clone())),
        );
        let world = step(
            &world,
            ControllerInput::Effect(EffectInput::Timeout(activation.clone())),
        );
        (world, activation)
    }

    fn pending_stop() -> (World, EffectId, EffectId) {
        let sandbox = SandboxId(1);
        let (world, activation) = pending_activation();
        let world = step(
            &world,
            ControllerInput::Request(Request::Compensate(CompensationRequest {
                sandbox,
                failure: FailureKind::DeadlineExceeded,
                evidence: None,
            })),
        );
        let world = execute(&world, |c| {
            matches!(c, Command::InstallFence { terminal: true, .. })
        });
        let generation = world.sandbox_record(sandbox).unwrap().generation;
        let world = step(
            &world,
            ControllerInput::Scheduler(SchedulerInput::StopVm(sandbox, generation, VmId(0))),
        );
        let stop = world
            .effect_ids()
            .iter()
            .find(|id| matches!(id.command(), Command::StopVm { .. }))
            .unwrap()
            .clone();
        (world, activation, stop)
    }

    // Controlled predicate inputs. The service regression separately reaches
    // the Resume/Unknown/Destroy state through the original SQLite journal.
    fn restore_abort() -> (World, EffectId, OperationKey) {
        let (mut world, activation) = pending_activation();
        let key = OperationKey {
            sandbox: SandboxId(1),
            operation: OperationId(1),
        };
        let source = RestoreSource {
            snapshot: SnapshotId(31),
            manifest: SnapshotManifestRef {
                digest: ManifestDigest(37),
            },
        };
        let mut original = world.operation_record(key).unwrap().clone();
        original.operation.kind = OperationKind::Resume;
        original.input = OperationInput::Resume { source };
        world.operations.put(key, original.clone());
        let mut record = world.sandbox_record(key.sandbox).unwrap().clone();
        record.current_operation = Some(original.operation);
        record.restore_from = Some(source);
        world.sandboxes.put(key.sandbox, record);
        let mut plan = world.operation_plan(key).unwrap().clone();
        plan.restore_from = Some(source);
        world.operation_plans.put(key, plan.clone());
        let prepare = EffectId::initial(Command::PrepareVm {
            sandbox: key.sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            vm: plan.vm.unwrap(),
            slot: plan.slot.unwrap(),
            source: Some(source),
        });
        world.effects.put(
            prepare.clone(),
            EffectStatus::Acknowledged(EffectOutcome::Succeeded(ResultDigest(11))),
        );
        world.logical_effects.put(
            prepare.logical.clone(),
            LogicalEffectRecord {
                origin: EffectOrigin::Operation(key),
                current_attempt: 0,
                remaining_retries: 0,
                result: Some(LogicalEffectResult::Succeeded {
                    attempt: 0,
                    result: ResultDigest(11),
                }),
            },
        );
        world.effect_keys.push(prepare);
        world = step(
            &world,
            ControllerInput::Request(Request::Kill(KillRequest {
                actor: Actor::Tenant(TenantId(1)),
                sandbox: key.sandbox,
                operation: OperationId(2),
            })),
        );
        (world, activation, key)
    }

    #[test]
    fn restored_activation_requires_exact_sealed_source_and_scope_for_terminal_takeover() {
        let (world, activation, key) = restore_abort();
        assert!(activation_in_current_abort(&world, &activation));
        assert!(!startup_effect_in_current_abort(&world, &activation));
        assert!(only_uncertain_activation_for_abort(&world, key.sandbox));
        let plan = world.operation_plan(key).unwrap();
        let mut variants = Vec::new();
        let mut changed = plan.clone();
        changed.vm = Some(VmId(99));
        variants.push(changed);
        let mut changed = plan.clone();
        changed.slot = Some(NetSlotId(99));
        variants.push(changed);
        let mut changed = plan.clone();
        changed.epoch += 1;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.generation += 1;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.sealed = false;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.restore_from = None;
        variants.push(changed);
        let mut changed = plan.clone();
        changed.restore_from.as_mut().unwrap().manifest.digest = ManifestDigest(99);
        variants.push(changed);
        for changed in variants {
            let mut invalid = world.clone();
            invalid.operation_plans.put(key, changed);
            assert!(!activation_in_current_abort(&invalid, &activation));
            assert!(!only_uncertain_activation_for_abort(&invalid, key.sandbox));
        }
        let prepare = world
            .effect_ids()
            .iter()
            .find(|effect| {
                matches!(
                    effect.command(),
                    Command::PrepareVm {
                        source: Some(_),
                        ..
                    }
                )
            })
            .unwrap()
            .clone();
        let mut load_unknown = world.clone();
        load_unknown
            .effects
            .put(prepare, EffectStatus::OutcomeUnknown);
        assert!(!activation_in_current_abort(&load_unknown, &activation));
        assert!(!only_uncertain_activation_for_abort(
            &load_unknown,
            key.sandbox
        ));
        assert!(effect::handle_effect_dispatch(&world, activation.clone()).is_err());
        assert!(
            handle_controller(
                &world,
                ControllerInput::Effect(EffectInput::Receipt(
                    activation,
                    EffectOutcome::Succeeded(ResultDigest(99))
                ))
            )
            .is_err()
        );
    }

    #[test]
    fn restored_activation_retirement_requires_confirmed_current_stop_and_fence() {
        let (world, activation, _) = restore_abort();
        let world = execute(&world, |c| {
            matches!(c, Command::InstallFence { terminal: true, .. })
        });
        let row = world.sandbox_record(SandboxId(1)).unwrap();
        let world = step(
            &world,
            ControllerInput::Scheduler(SchedulerInput::StopVm(
                SandboxId(1),
                row.generation,
                row.current_vm.unwrap(),
            )),
        );
        let stop = world
            .effect_ids()
            .iter()
            .find(|effect| matches!(effect.command(), Command::StopVm { .. }))
            .unwrap()
            .clone();
        assert!(handle_retire_activation(&world, activation.clone(), stop.clone()).is_err());
        assert!(!checkpoint::retired_activation_valid(&world, &activation));
        let stopped = execute(&world, |c| c == stop.command());
        let retired = handle_retire_activation(&stopped, activation.clone(), stop.clone())
            .unwrap()
            .next;
        let Some(EffectStatus::RetiredByStop(coverage)) = retired.effect_status(&activation) else {
            panic!("original unknown restore activation must retire by its exact Stop");
        };
        assert!(activation_coverage_origin(&retired, &activation, coverage));
        assert!(checkpoint::retired_activation_valid(&retired, &activation));
        assert!(!effect_succeeded_check(&retired, activation.command()));
        assert!(
            handle_controller(
                &retired,
                ControllerInput::Effect(EffectInput::Receipt(
                    activation.clone(),
                    EffectOutcome::Succeeded(ResultDigest(99))
                ))
            )
            .is_err()
        );
        let mut unfenced = stopped.clone();
        unfenced.worker_fences.put(
            SandboxId(1),
            WorkerFence {
                epoch: coverage.epoch,
                terminal: false,
            },
        );
        assert!(handle_retire_activation(&unfenced, activation.clone(), stop.clone()).is_err());
        let mut missing_stop = retired.clone();
        missing_stop
            .effects
            .put(stop.clone(), EffectStatus::OutcomeUnknown);
        assert!(!checkpoint::retired_activation_valid(
            &missing_stop,
            &activation
        ));
        let mut mismatched_coverage = retired.clone();
        mismatched_coverage.effects.put(
            activation.clone(),
            EffectStatus::RetiredByStop(StopCoverage {
                result: ResultDigest(99),
                ..coverage
            }),
        );
        assert!(!checkpoint::retired_activation_valid(
            &mismatched_coverage,
            &activation
        ));
        let mut missing_fence = retired.clone();
        missing_fence.effects.put(
            EffectId::initial(Command::InstallFence {
                sandbox: SandboxId(1),
                epoch: coverage.epoch,
                terminal: true,
            }),
            EffectStatus::OutcomeUnknown,
        );
        assert!(!checkpoint::retired_activation_valid(
            &missing_fence,
            &activation
        ));
        let mut old_epoch = stop.clone();
        if let Command::StopVm { epoch, .. } = &mut old_epoch.logical.command {
            *epoch -= 1;
        }
        assert!(handle_retire_activation(&stopped, activation, old_epoch).is_err());
    }

    #[test]
    fn unknown_activation_is_covered_by_the_current_kill_epoch_without_reactivation() {
        let (world, activation) = pending_activation();
        let mut world = step(
            &world,
            ControllerInput::Request(Request::Kill(crate::KillRequest {
                actor: Actor::Tenant(TenantId(1)),
                sandbox: SandboxId(1),
                operation: OperationId(2),
            })),
        );
        let epoch = world.sandbox_record(SandboxId(1)).unwrap().epoch;
        assert!(activation_in_current_abort(&world, &activation));
        assert!(only_uncertain_activation_for_abort(&world, SandboxId(1)));
        assert!(crate::effect::handle_effect_dispatch(&world, activation.clone()).is_err());
        for _ in 0..32 {
            match crate::workflow::next_step(&world, SandboxId(1)) {
                crate::workflow::WorkflowStep::Apply(input) => world = step(&world, *input),
                crate::workflow::WorkflowStep::Claim => {
                    let selected = world
                        .effect_ids()
                        .iter()
                        .find(|effect| {
                            if world.effect_status(effect) != Some(EffectStatus::Committed) {
                                return false;
                            }
                            let Ok(dispatch) =
                                crate::effect::handle_effect_dispatch(&world, (*effect).clone())
                            else {
                                return false;
                            };
                            crate::worker::handle_worker(&dispatch.next, effect).is_ok()
                        })
                        .unwrap()
                        .clone();
                    assert!(!matches!(selected.command(), Command::ActivateVm { .. }));
                    world = execute(&world, |command| command == selected.command());
                }
                crate::workflow::WorkflowStep::Complete => break,
                other => panic!("termination must make progress: {other:?}"),
            }
        }
        assert_eq!(
            world.sandbox_record(SandboxId(1)).unwrap().phase,
            Phase::Destroyed
        );
        let Some(EffectStatus::RetiredByStop(coverage)) = world.effect_status(&activation) else {
            panic!("unknown activation not settled")
        };
        assert_eq!(coverage.operation.operation, OperationId(2));
        assert_eq!(coverage.epoch, epoch);
        assert!(activation_coverage_origin(&world, &activation, coverage));
        assert!(!crate::effect::effect_succeeded_check(
            &world,
            activation.command()
        ));
        assert!(
            world.resource_index(SandboxId(1)).vms.iter().all(|vm| world
                .vm_record(*vm)
                .unwrap()
                .status
                == VmStatus::Reclaimed)
        );
    }

    #[test]
    fn retirement_needs_acknowledged_stop_and_never_means_activation_success() {
        let (world, activation, stop) = pending_stop();
        assert!(only_uncertain_activation_for_abort(&world, SandboxId(1)));
        assert!(scheduler::dispatch_allowed_during_abort(
            &world,
            stop.command()
        ));
        assert!(!scheduler::dispatch_allowed_during_abort(
            &world,
            activation.command()
        ));
        assert!(handle_retire_activation(&world, activation.clone(), stop.clone()).is_err());
        let dispatched = step(
            &world,
            ControllerInput::Effect(EffectInput::Dispatch(stop.clone())),
        );
        assert!(!only_uncertain_activation_for_abort(
            &dispatched,
            SandboxId(1)
        ));
        let stopped = step(&dispatched, ControllerInput::Worker(stop.clone()));
        assert!(handle_retire_activation(&stopped, activation.clone(), stop.clone()).is_err());
        let acknowledged = step(
            &stopped,
            ControllerInput::Effect(EffectInput::Receipt(
                stop.clone(),
                EffectOutcome::Succeeded(ResultDigest(8)),
            )),
        );
        assert!(
            handle_controller(
                &acknowledged,
                ControllerInput::Effect(EffectInput::Receipt(
                    activation.clone(),
                    EffectOutcome::Succeeded(ResultDigest(9)),
                ))
            )
            .is_err(),
            "stopped does not prove successful activation"
        );
        let decision =
            handle_retire_activation(&acknowledged, activation.clone(), stop.clone()).unwrap();
        assert!(matches!(
            decision.next.effect_status(&activation),
            Some(EffectStatus::RetiredByStop(_))
        ));
        assert!(!effect_succeeded_check(
            &decision.next,
            activation.command()
        ));
        assert_eq!(decision.next.vms, acknowledged.vms);
        assert_eq!(decision.next.sandboxes, acknowledged.sandboxes);
        assert_eq!(decision.next.net_slots, acknowledged.net_slots);
        assert_eq!(
            handle_retire_activation(&decision.next, activation.clone(), stop)
                .unwrap()
                .next,
            decision.next
        );
        for input in [
            ControllerInput::Worker(activation.clone()),
            ControllerInput::Effect(EffectInput::Receipt(
                activation.clone(),
                EffectOutcome::Succeeded(ResultDigest(9)),
            )),
            ControllerInput::Effect(EffectInput::Dispatch(activation.clone())),
            ControllerInput::Effect(EffectInput::Retry(
                activation.logical.clone(),
                FailureKind::ResourceFailure,
            )),
        ] {
            assert!(handle_controller(&decision.next, input).is_err());
        }
    }

    #[test]
    fn retirement_rejects_wrong_identity_plan_fence_and_live_resources() {
        let (world, activation, stop) = pending_stop();
        let stopped = execute(&world, |c| matches!(c, Command::StopVm { .. }));
        let key = OperationKey {
            sandbox: SandboxId(1),
            operation: OperationId(1),
        };
        let mut cases = Vec::new();
        // Each mutation breaks one independent premise of terminal coverage.
        for kind in 0..10 {
            let mut changed = stopped.clone();
            match kind {
                0 => {
                    let mut r = *changed.vms.get(&VmId(0)).unwrap();
                    r.owner = TenantId(2);
                    changed.vms.put(VmId(0), r);
                }
                1 => {
                    let mut r = *changed.vms.get(&VmId(0)).unwrap();
                    r.status = VmStatus::Prepared;
                    changed.vms.put(VmId(0), r);
                }
                2 => {
                    let mut p = changed.operation_plans.get(&key).unwrap().clone();
                    p.sealed = false;
                    changed.operation_plans.put(key, p);
                }
                3 => {
                    let mut p = changed.operation_plans.get(&key).unwrap().clone();
                    p.stop_vms.clear();
                    changed.operation_plans.put(key, p);
                }
                4 => {
                    let mut r = changed.sandboxes.get(&SandboxId(1)).unwrap().clone();
                    r.terminal = false;
                    changed.sandboxes.put(SandboxId(1), r);
                }
                5 => {
                    changed.worker_fences.put(
                        SandboxId(1),
                        WorkerFence {
                            epoch: 0,
                            terminal: false,
                        },
                    );
                }
                6 => {
                    let mut r = changed.net_slots.get(&NetSlotId(0)).unwrap().clone();
                    r.active = true;
                    changed.net_slots.put(NetSlotId(0), r);
                }
                7 => {
                    let mut r = *changed.logical_effects.get(&activation.logical).unwrap();
                    r.current_attempt += 1;
                    changed.logical_effects.put(activation.logical.clone(), r);
                }
                8 => {
                    let mut r = *changed.logical_effects.get(&stop.logical).unwrap();
                    r.current_attempt += 1;
                    changed.logical_effects.put(stop.logical.clone(), r);
                }
                _ => {
                    let mut r = *changed.logical_effects.get(&stop.logical).unwrap();
                    r.origin = EffectOrigin::Operation(OperationKey {
                        sandbox: SandboxId(1),
                        operation: OperationId(2),
                    });
                    changed.logical_effects.put(stop.logical.clone(), r);
                }
            }
            cases.push(changed);
        }
        for changed in cases {
            assert!(handle_retire_activation(&changed, activation.clone(), stop.clone()).is_err());
        }
        let mut wrong_stop = stop;
        wrong_stop.attempt += 1;
        assert!(handle_retire_activation(&stopped, activation, wrong_stop).is_err());
    }
}
