//! Narrow terminal coverage of an unknown V1 activation. No host I/O or
//! timers live here: the caller must first persist exact stop evidence.

use crate::effect::{EffectDecision, EffectReject, effect_succeeded_check};
use crate::{
    Command, EffectId, EffectOrigin, EffectOutcome, EffectStatus, Event, LogicalEffectResult,
    OperationKey, OperationKind, OperationStatus, Phase, SandboxId, StopCoverage, VmStatus,
    WorkerFence, World,
};

/// Bind an activation to the current terminal create and its sealed resources.
/// This is not permission to execute it again; only fence/stop may proceed.
#[must_use]
pub fn activation_in_current_abort(world: &World, effect: &EffectId) -> bool {
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
    let Some(plan) = world.operation_plans.get(&key) else {
        return false;
    };
    let Some(slot) = plan.slot else {
        return false;
    };
    let Some(logical) = world.logical_effects.get(&effect.logical) else {
        return false;
    };
    let Some(machine) = world.vms.get(&vm) else {
        return false;
    };
    let Some(network) = world.net_slots.get(&slot) else {
        return false;
    };
    record.phase == Phase::Revoking
        && record.terminal
        && operation.kind == OperationKind::Create
        && matches!(operation.status, OperationStatus::Compensating(_))
        && operation.epoch == epoch
        && record.epoch == epoch
        && record.generation == generation
        && record.current_vm == Some(vm)
        && record.net_slot == Some(slot)
        && plan.key == key
        && plan.sealed
        && plan.epoch == epoch
        && plan.generation == generation
        && plan.vm == Some(vm)
        && plan.slot == Some(slot)
        && plan.stop_vms.contains(&vm)
        && plan.close_slots.contains(&slot)
        && logical.origin == EffectOrigin::Operation(key)
        && logical.current_attempt == effect.attempt
        && logical.result.is_none()
        && machine.owner == record.owner
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
    for id in world.effect_ids() {
        if id.command().sandbox() == sandbox
            && matches!(
                world.effect_status(id),
                Some(EffectStatus::Dispatched | EffectStatus::OutcomeUnknown)
            )
        {
            if !activation_in_current_abort(world, id) {
                return false;
            }
            seen = true;
        }
    }
    seen
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
    if activation_stop(&activation).as_ref() != Some(stop.command()) {
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
    if logical.origin != EffectOrigin::Operation(key)
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

    fn pending_stop() -> (World, EffectId, EffectId) {
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
