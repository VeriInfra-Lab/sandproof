//! Real reducer/checkpoint terminal paths with explicit synthetic physical
//! conditions. No VMM, worker/FD observation or release is proved by this test.
use super::*;
use crate::capture::drain::{
    CaptureDrainCoverage, CaptureDrainEndReceipt, terminal_capture_drain_allowed,
};
use crate::capture::{CaptureInput, OriginalContinueIdentity};
use crate::maintenance::retirement::QuiescenceStatus;

fn kill(world: &World, actor: Actor) -> World {
    step(
        world,
        ControllerInput::Request(Request::Kill(KillRequest {
            actor,
            sandbox: SandboxId(10),
            operation: OperationId(9),
        })),
    )
}
fn fence(world: &World) -> Command {
    Command::InstallFence {
        sandbox: SandboxId(10),
        epoch: world.sandbox_record(SandboxId(10)).unwrap().epoch,
        terminal: true,
    }
}
fn stop(world: &World) -> Command {
    let row = world.sandbox_record(SandboxId(10)).unwrap();
    Command::StopVm {
        sandbox: SandboxId(10),
        epoch: row.epoch,
        generation: row.generation,
        vm: row.current_vm.unwrap(),
    }
}
fn source_identity(world: &World, seal: &EffectId) -> OriginalContinueIdentity {
    let EffectOrigin::Operation(key) = world.logical_effect(&seal.logical).unwrap().origin else {
        unreachable!()
    };
    let Command::SealSnapshot {
        epoch,
        generation,
        vm,
        ..
    } = *seal.command()
    else {
        unreachable!()
    };
    OriginalContinueIdentity {
        key,
        epoch,
        generation,
        vm,
    }
}
fn stopped(unknown: bool, actor: Actor) -> (World, EffectId, CaptureDrainCoverage) {
    let (world, seal, coordinates) = continuation::pending_capture(unknown);
    let identity = source_identity(&world, &seal);
    let world = kill(&world, actor);
    assert!(terminal_capture_drain_allowed(&world, &fence(&world)));
    assert!(!terminal_capture_drain_allowed(&world, &stop(&world)));
    assert_eq!(
        crate::workflow::next_step(&world, identity.key.sandbox),
        crate::workflow::WorkflowStep::Claim
    );
    let terminal_fence = effect(&world, identity.key.sandbox, |command| {
        *command == fence(&world)
    });
    let world = execute_id(&world, terminal_fence);
    let command = stop(&world);
    assert!(terminal_capture_drain_allowed(&world, &command));
    let Command::StopVm {
        sandbox,
        generation,
        vm,
        ..
    } = command
    else {
        unreachable!()
    };
    assert!(
        matches!(crate::workflow::next_step(&world,sandbox),crate::workflow::WorkflowStep::Apply(input) if *input == ControllerInput::Scheduler(SchedulerInput::StopVm(sandbox,generation,vm)))
    );
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::StopVm(sandbox, generation, vm)),
    );
    let stop_id = effect(&world, sandbox, |command| *command == stop(&world));
    let world = execute_id(&world, stop_id.clone());
    assert_eq!(
        crate::workflow::next_step(&world, sandbox),
        crate::workflow::WorkflowStep::Blocked
    );
    let row = world.sandbox_record(sandbox).unwrap();
    let Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(result))) =
        world.effect_status(&stop_id)
    else {
        unreachable!()
    };
    let coverage = CaptureDrainCoverage {
        identity,
        stop: StopCoverage {
            operation: OperationKey {
                sandbox,
                operation: row.current_operation.unwrap().id,
            },
            epoch: row.epoch,
            attempt: stop_id.attempt,
            result,
        },
        end_receipt: CaptureDrainEndReceipt {
            work_binding: [19; 32],
            end_binding: [23; 32],
            seal: coordinates,
            worker_drained: QuiescenceStatus::Confirmed,
            descriptors_closed: QuiescenceStatus::Confirmed,
        },
    };
    (world, seal, coverage)
}
#[test]
fn authorized_destroy_or_expiry_drains_original_unknown_full_without_api_end_or_continue() {
    for unknown in [false, true] {
        for actor in [Actor::Tenant(OWNER), Actor::SystemController] {
            let (_, binding) = live();
            let (world, seal, coverage) = stopped(unknown, actor);
            let world = step(
                &world,
                ControllerInput::Capture(CaptureInput::RetireTerminatedCapture(coverage)),
            );
            assert_eq!(
                world.effect_status(&seal),
                Some(EffectStatus::RetiredByCaptureDrain(coverage))
            );
            assert!(!crate::effect::effect_succeeded_check(
                &world,
                seal.command()
            ));
            assert!(
                !world
                    .operation_plan(coverage.identity.key)
                    .unwrap()
                    .capture
                    .unwrap()
                    .has_capture_work()
            );
            assert_eq!(
                handle_effect_receipt(
                    &world,
                    seal.clone(),
                    EffectOutcome::Failed(FailureKind::Cancelled)
                ),
                Err(EffectReject::EffectRetired)
            );
            assert_eq!(
                handle_effect_dispatch(&world, seal.clone()),
                Err(EffectReject::NotDispatchable)
            );
            assert!(crate::worker::handle_worker(&world, &seal).is_err());
            let world = step(&world, ControllerInput::Runtime(RuntimeInput::Crash));
            let world = step(&world, ControllerInput::Runtime(RuntimeInput::Recover));
            let same = step(
                &world,
                ControllerInput::Capture(CaptureInput::RetireTerminatedCapture(coverage)),
            );
            assert_eq!(same, world);
            assert!(
                handle_controller(
                    &world,
                    ControllerInput::Capture(CaptureInput::PublishOriginalContinued {
                        identity: coverage.identity
                    })
                )
                .is_err()
            );
            assert_eq!(
                world.sandbox_record(binding.sandbox).unwrap().effective,
                Authority::Closed
            );
            let world = step(
                &world,
                ControllerInput::Publication(PublicationInput::Destroyed(binding.sandbox)),
            );
            let world = cleanup(&world, binding);
            let world = release(&world, 0, binding);
            let world = retire(&world, binding.sandbox);
            assert!(world.sandbox_record(binding.sandbox).is_none());
        }
    }
}
#[test]
fn terminal_drain_rejects_unconfirmed_physical_conditions_and_changed_original_coordinates() {
    let (world, seal, original) = stopped(true, Actor::Tenant(OWNER));
    for field in [
        "worker",
        "fd",
        "key",
        "epoch",
        "generation",
        "vm",
        "attempt",
        "workspace",
        "snapshot",
        "manifest",
        "stop-key",
        "stop-epoch",
        "stop-attempt",
        "stop-result",
    ] {
        let mut changed = original;
        match field {
            "worker" => changed.end_receipt.worker_drained = QuiescenceStatus::Unconfirmed,
            "fd" => changed.end_receipt.descriptors_closed = QuiescenceStatus::Unconfirmed,
            "key" => changed.identity.key.operation.0 += 1,
            "epoch" => changed.identity.epoch += 1,
            "generation" => changed.identity.generation += 1,
            "vm" => changed.identity.vm.0 += 1,
            "attempt" => changed.end_receipt.seal.attempt += 1,
            "workspace" => changed.end_receipt.seal.workspace.0 += 1,
            "snapshot" => changed.end_receipt.seal.snapshot.0 += 1,
            "manifest" => changed.end_receipt.seal.manifest.digest.0 += 1,
            "stop-key" => changed.stop.operation.operation.0 += 1,
            "stop-epoch" => changed.stop.epoch += 1,
            "stop-attempt" => changed.stop.attempt += 1,
            "stop-result" => changed.stop.result.0 += 1,
            _ => unreachable!(),
        }
        assert!(
            handle_controller(
                &world,
                ControllerInput::Capture(CaptureInput::RetireTerminatedCapture(changed))
            )
            .is_err(),
            "accepted {field}"
        );
        assert_eq!(
            world.effect_status(&seal),
            Some(EffectStatus::OutcomeUnknown)
        );
        assert!(
            world
                .operation_plan(original.identity.key)
                .unwrap()
                .capture
                .unwrap()
                .has_capture_work()
        );
    }
}
#[test]
fn terminal_exception_never_allows_ordinary_pause_other_commands_or_unknown_freeze_commit() {
    let (world, seal, _) = continuation::pending_capture(true);
    let id = source_identity(&world, &seal);
    let future_stop = Command::StopVm {
        sandbox: id.key.sandbox,
        epoch: id.epoch + 1,
        generation: id.generation,
        vm: id.vm,
    };
    assert!(!terminal_capture_drain_allowed(&world, &future_stop));
    let world = kill(&world, Actor::Tenant(OWNER));
    for command in [
        seal.command().clone(),
        id.activation_command(),
        Command::CleanupVm {
            sandbox: id.key.sandbox,
            generation: id.generation,
            vm: id.vm,
        },
        Command::InstallFence {
            sandbox: id.key.sandbox,
            epoch: id.epoch,
            terminal: true,
        },
    ] {
        assert!(!terminal_capture_drain_allowed(&world, &command));
    }
    let world = continuation::pause_source(false);
    let freeze = effect(&world, SandboxId(10), |command| {
        matches!(command, Command::FreezeVm { .. })
    });
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(freeze.clone())),
    );
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(freeze)),
    );
    let world = kill(&world, Actor::SystemController);
    assert!(!terminal_capture_drain_allowed(&world, &fence(&world)));
    let world = unload::commit_planned();
    let commit = effect(&world, SandboxId(10), |command| {
        matches!(command, Command::CommitSnapshot { .. })
    });
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(commit.clone())),
    );
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(commit)),
    );
    let world = kill(&world, Actor::Tenant(OWNER));
    assert!(!terminal_capture_drain_allowed(&world, &fence(&world)));
}
