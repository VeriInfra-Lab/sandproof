//! Real native reducers and checkpoint roundtrips. Host end/activation evidence
//! remains an explicit service input; these tests do not run a VMM or `SQLite`.
use super::*;
use crate::capture::continuation::CaptureReject;
use crate::capture::{CaptureInput, CaptureWorkEndReceipt, OriginalContinueIdentity};

fn advance(world: &World, input: ControllerInput) -> World {
    let label = format!("{input:?}");
    let next = handle_controller(world, input)
        .unwrap_or_else(|error| panic!("{label}: {error:?}"))
        .next;
    let image = capture(&next, LIMITS)
        .unwrap_or_else(|error| panic!("checkpoint after {label}: {error:?}"));
    assert_eq!(restore(image, LIMITS).unwrap(), next);
    next
}
fn finish(world: &World, id: EffectId) -> World {
    let world = advance(
        world,
        ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
    );
    let world = advance(&world, ControllerInput::Worker(id.clone()));
    advance(
        &world,
        ControllerInput::Effect(EffectInput::Receipt(
            id,
            EffectOutcome::Succeeded(ResultDigest(9)),
        )),
    )
}
fn receipt() -> CaptureWorkEndReceipt {
    CaptureWorkEndReceipt {
        work_binding: [13; 32],
        end_binding: [17; 32],
        end: crate::capture::CaptureWorkEnd::NeverReleased { seal: None },
    }
}
fn identity(world: &World) -> OriginalContinueIdentity {
    let row = world.sandbox_record(SandboxId(10)).unwrap();
    OriginalContinueIdentity {
        key: OperationKey {
            sandbox: SandboxId(10),
            operation: row.current_operation.unwrap().id,
        },
        epoch: row.epoch,
        generation: row.generation,
        vm: row.current_vm.unwrap(),
    }
}
fn compensate(world: &World) -> World {
    advance(
        world,
        ControllerInput::Request(Request::Compensate(CompensationRequest {
            sandbox: SandboxId(10),
            failure: FailureKind::Cancelled,
            evidence: None,
        })),
    )
}
fn pause_source(frozen: bool) -> World {
    let (world, _) = live();
    let world = advance(
        &world,
        ControllerInput::Request(Request::Pause(PauseRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SandboxId(10),
            operation: OperationId(2),
        })),
    );
    let id = identity(&world);
    let fence = effect(
        &world,
        id.key.sandbox,
        |command| matches!(command,Command::InstallFence {epoch,..} if *epoch==id.epoch),
    );
    let world = finish(&world, fence);
    if frozen {
        let world = advance(
            &world,
            ControllerInput::Scheduler(SchedulerInput::ReservePauseSnapshot(id.key)),
        );
        let freeze = effect(
            &world,
            id.key.sandbox,
            |command| matches!(command,Command::FreezeVm {epoch,..} if *epoch==id.epoch),
        );
        finish(&world, freeze)
    } else {
        world
    }
}
fn failed_pause(frozen: bool) -> World {
    compensate(&pause_source(frozen))
}
fn prepare(world: &World) -> World {
    advance(
        world,
        ControllerInput::Capture(CaptureInput::PrepareOriginalContinue {
            identity: identity(world),
            end_receipt: receipt(),
        }),
    )
}
fn continued(world: &World) -> World {
    let id = identity(world);
    let world = prepare(world);
    let activate = effect(&world, id.key.sandbox, |command| {
        *command == id.activation_command()
    });
    let world = finish(&world, activate);
    advance(
        &world,
        ControllerInput::Capture(CaptureInput::PublishOriginalContinued { identity: id }),
    )
}
#[test]
fn original_continue_from_frozen_or_never_dispatched_freeze_restores_same_execution_and_authority()
{
    for frozen in [false, true] {
        let world = failed_pause(frozen);
        let id = identity(&world);
        let next_vm = world.next_vm_id;
        let next_slot = world.next_slot_id;
        let workspace = world.sandbox_record(id.key.sandbox).unwrap().workspace;
        let world = prepare(&world);
        assert_eq!(
            world.sandbox_record(id.key.sandbox).unwrap().effective,
            Authority::Closed
        );
        assert!(
            !world
                .slot_record(
                    world
                        .sandbox_record(id.key.sandbox)
                        .unwrap()
                        .net_slot
                        .unwrap()
                )
                .unwrap()
                .active
        );
        assert!(matches!(
            handle_controller(
                &world,
                ControllerInput::Capture(CaptureInput::PublishOriginalContinued { identity: id })
            ),
            Err(ControllerReject::Capture(
                CaptureReject::ActivationNotObserved
            ))
        ));
        let activate = effect(&world, id.key.sandbox, |command| {
            *command == id.activation_command()
        });
        let world = finish(&world, activate);
        let world = advance(
            &world,
            ControllerInput::Capture(CaptureInput::PublishOriginalContinued { identity: id }),
        );
        let row = world.sandbox_record(id.key.sandbox).unwrap();
        assert_eq!(
            (row.phase, row.generation, row.current_vm, row.workspace),
            (Phase::Running, id.generation, Some(id.vm), workspace)
        );
        assert_eq!((world.next_vm_id, world.next_slot_id), (next_vm, next_slot));
        assert!(matches!(row.effective, Authority::Running { .. }));
        assert!(world.slot_record(row.net_slot.unwrap()).unwrap().active);
        assert_eq!(
            world.operation_record(id.key).unwrap().result,
            Some(OperationResult::Failed(FailureKind::Cancelled))
        );
        let same = advance(
            &world,
            ControllerInput::Capture(CaptureInput::PublishOriginalContinued { identity: id }),
        );
        assert_eq!(same, world);
    }
}
#[test]
fn original_continue_unknown_activation_reopens_same_plan_after_crash_without_new_allocation() {
    let world = prepare(&failed_pause(true));
    let id = identity(&world);
    let activate = effect(&world, id.key.sandbox, |command| {
        *command == id.activation_command()
    });
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(activate.clone())),
    );
    let world = advance(&world, ControllerInput::Worker(activate.clone()));
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(activate.clone())),
    );
    let before = world.clone();
    let world = advance(&world, ControllerInput::Runtime(RuntimeInput::Crash));
    let world = advance(&world, ControllerInput::Runtime(RuntimeInput::Recover));
    let world = advance(
        &world,
        ControllerInput::Capture(CaptureInput::PrepareOriginalContinue {
            identity: id,
            end_receipt: receipt(),
        }),
    );
    assert_eq!(
        (world.next_vm_id, world.next_slot_id, world.effect_ids()),
        (before.next_vm_id, before.next_slot_id, before.effect_ids())
    );
    let world = advance(&world, ControllerInput::Worker(activate.clone()));
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Receipt(
            activate,
            EffectOutcome::Succeeded(ResultDigest(9)),
        )),
    );
    let world = advance(
        &world,
        ControllerInput::Capture(CaptureInput::PublishOriginalContinued { identity: id }),
    );
    assert_eq!(
        world.sandbox_record(id.key.sandbox).unwrap().phase,
        Phase::Running
    );
}
#[test]
fn old_queued_freeze_is_permanently_withdrawn_and_cannot_revive_during_a_later_pause() {
    let world = failed_pause(false);
    let old = effect(&world, SandboxId(10), |command| {
        matches!(command, Command::FreezeVm { .. })
    });
    let world = continued(&world);
    let world = advance(
        &world,
        ControllerInput::Request(Request::Pause(PauseRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SandboxId(10),
            operation: OperationId(3),
        })),
    );
    assert_eq!(world.effect_status(&old), Some(EffectStatus::Committed));
    assert_eq!(
        handle_effect_dispatch(&world, old.clone()),
        Err(EffectReject::NotDispatchable)
    );
    let current = effect(&world, SandboxId(10), |command| {
        matches!(command, Command::FreezeVm { epoch: 3, .. })
    });
    assert!(handle_effect_dispatch(&world, current).is_ok());
}
#[test]
fn original_continue_rejects_changed_binding_digest_conflicts_unknown_freeze_and_unknown_commit() {
    let original = failed_pause(true);
    let id = identity(&original);
    for field in 0..4 {
        let mut wrong = id;
        match field {
            0 => wrong.key.operation = OperationId(999),
            1 => wrong.epoch += 1,
            2 => wrong.generation += 1,
            3 => wrong.vm = VmId(999),
            _ => unreachable!(),
        }
        assert!(
            handle_controller(
                &original,
                ControllerInput::Capture(CaptureInput::PrepareOriginalContinue {
                    identity: wrong,
                    end_receipt: receipt()
                })
            )
            .is_err()
        );
    }
    let prepared = prepare(&original);
    let mut wrong = receipt();
    wrong.end_binding[0] ^= 1;
    assert_eq!(
        handle_controller(
            &prepared,
            ControllerInput::Capture(CaptureInput::PrepareOriginalContinue {
                identity: id,
                end_receipt: wrong
            })
        ),
        Err(ControllerReject::Capture(CaptureReject::ReceiptConflict))
    );
    let world = failed_pause(false);
    let id = identity(&world);
    let freeze = effect(&world, id.key.sandbox, |command| {
        matches!(command, Command::FreezeVm { .. })
    });
    // Build the real dispatched/unknown prefix before revocation, rather than
    // changing an effect row behind the reducer.
    let (live_world, _) = live();
    let world = advance(
        &live_world,
        ControllerInput::Request(Request::Pause(PauseRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: id.key.sandbox,
            operation: id.key.operation,
        })),
    );
    let fence = effect(&world, id.key.sandbox, |command| {
        matches!(command, Command::InstallFence { epoch: 2, .. })
    });
    let world = finish(&world, fence);
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(freeze.clone())),
    );
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(freeze)),
    );
    let world = compensate(&world);
    assert_eq!(
        handle_controller(
            &world,
            ControllerInput::Capture(CaptureInput::PrepareOriginalContinue {
                identity: id,
                end_receipt: receipt()
            })
        ),
        Err(ControllerReject::Capture(CaptureReject::EffectsUnresolved))
    );
    let world = unload::commit_planned();
    let id = identity(&world);
    let commit = effect(&world, id.key.sandbox, |command| {
        matches!(command, Command::CommitSnapshot { .. })
    });
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(commit.clone())),
    );
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(commit)),
    );
    let world = compensate(&world);
    assert_eq!(
        handle_controller(
            &world,
            ControllerInput::Capture(CaptureInput::PrepareOriginalContinue {
                identity: id,
                end_receipt: receipt()
            })
        ),
        Err(ControllerReject::Capture(CaptureReject::CommitUnresolved))
    );
}
#[test]
fn continued_original_then_destroy_reclaims_original_resources_without_reopening_old_capture() {
    for frozen in [false, true] {
        let (live_world, binding) = live();
        drop(live_world);
        let world = continued(&failed_pause(frozen));
        let world = destroy(&world, binding);
        let world = cleanup(&world, binding);
        let world = release(&world, 0, binding);
        assert_eq!(
            world.sandbox_record(binding.sandbox).unwrap().phase,
            Phase::Destroyed
        );
        let world = retire(&world, binding.sandbox);
        assert!(world.sandbox_record(binding.sandbox).is_none());
    }
}

fn begin_destroy(world: &World) -> World {
    advance(
        world,
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SandboxId(10),
            operation: OperationId(9),
        })),
    )
}
fn stop_destroy(world: &World, binding: crate::maintenance::slots::HostBinding) -> World {
    let epoch = world.sandbox_record(binding.sandbox).unwrap().epoch;
    let fence = effect(
        world,
        binding.sandbox,
        |command| matches!(command,Command::InstallFence {epoch:found,terminal:true,..} if *found==epoch),
    );
    let world = finish(world, fence);
    let world = advance(
        &world,
        ControllerInput::Scheduler(SchedulerInput::StopVm(
            binding.sandbox,
            binding.generation,
            binding.vm,
        )),
    );
    let stop = effect(
        &world,
        binding.sandbox,
        |command| matches!(command,Command::StopVm {epoch:found,..} if *found==epoch),
    );
    finish(&world, stop)
}
fn destroy(world: &World, binding: crate::maintenance::slots::HostBinding) -> World {
    let world = stop_destroy(&begin_destroy(world), binding);
    advance(
        &world,
        ControllerInput::Publication(PublicationInput::Destroyed(binding.sandbox)),
    )
}
#[test]
fn unknown_continue_then_destroy_is_closed_and_original_stop_can_retire_only_that_activation() {
    let (_, binding) = live();
    let world = prepare(&failed_pause(true));
    let id = identity(&world);
    let activate = effect(&world, id.key.sandbox, |command| {
        *command == id.activation_command()
    });
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(activate.clone())),
    );
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(activate.clone())),
    );
    let world = begin_destroy(&world);
    assert_eq!(
        handle_controller(
            &world,
            ControllerInput::Capture(CaptureInput::PublishOriginalContinued { identity: id })
        ),
        Err(ControllerReject::Capture(CaptureReject::Terminal))
    );
    assert_eq!(
        handle_effect_dispatch(&world, activate.clone()),
        Err(EffectReject::NotDispatchable)
    );
    let world = stop_destroy(&world, binding);
    let stop = effect(&world, id.key.sandbox, |command| {
        matches!(command, Command::StopVm { .. })
    });
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::RetireActivation {
            activation: activate.clone(),
            stop,
        }),
    );
    assert!(matches!(
        world.effect_status(&activate),
        Some(EffectStatus::RetiredByStop(_))
    ));
    let world = advance(
        &world,
        ControllerInput::Publication(PublicationInput::Destroyed(id.key.sandbox)),
    );
    let world = cleanup(&world, binding);
    let world = release(&world, 0, binding);
    let world = retire(&world, id.key.sandbox);
    assert!(world.sandbox_record(id.key.sandbox).is_none());
}

fn dispatched_capture(unknown: bool) -> (World, EffectId, crate::capture::CaptureSealEnd) {
    let world = pause_source(true);
    let id = identity(&world);
    let target = world
        .operation_plan(id.key)
        .unwrap()
        .target_snapshot
        .unwrap();
    let world = advance(
        &world,
        ControllerInput::Scheduler(SchedulerInput::SealSnapshot(
            id.key.sandbox,
            target.snapshot,
            target.manifest,
        )),
    );
    let seal = effect(&world, id.key.sandbox, |command| {
        matches!(command, Command::SealSnapshot { .. })
    });
    let Command::SealSnapshot {
        workspace,
        snapshot,
        manifest,
        ..
    } = *seal.command()
    else {
        unreachable!()
    };
    let end = crate::capture::CaptureSealEnd {
        attempt: seal.attempt,
        workspace,
        snapshot,
        manifest,
    };
    let world = advance(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(seal.clone())),
    );
    let world = if unknown {
        advance(
            &world,
            ControllerInput::Effect(EffectInput::Timeout(seal.clone())),
        )
    } else {
        world
    };
    (compensate(&world), seal, end)
}

#[test]
fn exact_never_released_or_returned_end_retires_only_original_seal_and_keeps_no_snapshot_success() {
    for returned in [false, true] {
        for unknown in [false, true] {
            let (_, binding) = live();
            let (world, seal, coordinates) = dispatched_capture(unknown);
            let id = identity(&world);
            let mut end = receipt();
            end.end = if returned {
                crate::capture::CaptureWorkEnd::Returned { seal: coordinates }
            } else {
                crate::capture::CaptureWorkEnd::NeverReleased {
                    seal: Some(coordinates),
                }
            };
            let world = advance(
                &world,
                ControllerInput::Capture(CaptureInput::PrepareOriginalContinue {
                    identity: id,
                    end_receipt: end,
                }),
            );
            assert_eq!(
                world.effect_status(&seal),
                Some(EffectStatus::RetiredByCaptureEnd(end))
            );
            assert!(!crate::effect::effect_succeeded_check(
                &world,
                seal.command()
            ));
            assert!(!crate::effect::command_result_present(
                &world,
                seal.command()
            ));
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
            assert_eq!(
                handle_effect_timeout(&world, seal.clone()),
                Err(EffectReject::NotDispatched)
            );
            assert!(crate::worker::handle_worker(&world, &seal).is_err());
            let world = advance(&world, ControllerInput::Runtime(RuntimeInput::Crash));
            let world = advance(&world, ControllerInput::Runtime(RuntimeInput::Recover));
            let activate = effect(&world, id.key.sandbox, |command| {
                *command == id.activation_command()
            });
            let world = finish(&world, activate);
            let world = advance(
                &world,
                ControllerInput::Capture(CaptureInput::PublishOriginalContinued { identity: id }),
            );
            assert_eq!(
                world.effect_status(&seal),
                Some(EffectStatus::RetiredByCaptureEnd(end))
            );
            let world = destroy(&world, binding);
            let world = cleanup(&world, binding);
            let world = release(&world, 0, binding);
            let world = retire(&world, id.key.sandbox);
            assert!(world.sandbox_record(id.key.sandbox).is_none());
        }
    }
}

#[test]
fn end_digests_without_exact_seal_coordinates_or_after_destroy_never_retire_work() {
    let (world, seal, coordinates) = dispatched_capture(true);
    let id = identity(&world);
    for field in ["missing", "attempt", "workspace", "snapshot", "manifest"] {
        let mut altered = coordinates;
        match field {
            "missing" => {}
            "attempt" => altered.attempt += 1,
            "workspace" => altered.workspace.0 += 1,
            "snapshot" => altered.snapshot.0 += 1,
            "manifest" => altered.manifest.digest.0 += 1,
            _ => unreachable!(),
        }
        let mut end = receipt();
        if field != "missing" {
            end.end = crate::capture::CaptureWorkEnd::NeverReleased {
                seal: Some(altered),
            };
        }
        assert!(
            handle_controller(
                &world,
                ControllerInput::Capture(CaptureInput::PrepareOriginalContinue {
                    identity: id,
                    end_receipt: end
                })
            )
            .is_err(),
            "accepted {field}"
        );
        assert_eq!(
            world.effect_status(&seal),
            Some(EffectStatus::OutcomeUnknown)
        );
    }
    let world = begin_destroy(&world);
    let mut end = receipt();
    end.end = crate::capture::CaptureWorkEnd::Returned { seal: coordinates };
    assert_eq!(
        handle_controller(
            &world,
            ControllerInput::Capture(CaptureInput::PrepareOriginalContinue {
                identity: id,
                end_receipt: end
            })
        ),
        Err(ControllerReject::Capture(CaptureReject::Terminal))
    );
    assert_eq!(
        world.effect_status(&seal),
        Some(EffectStatus::OutcomeUnknown)
    );
}
