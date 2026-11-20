//! Real reducer histories. A physical Worker success is an explicit host input,
//! not evidence that these component tests ran a VM.
use super::*;
use crate::maintenance::slots::{HostBinding, SlotInput};

const SID: SandboxId = SandboxId(10);

fn raw(world: &World, input: ControllerInput) -> World {
    let label = format!("{input:?}");
    let next = handle_controller(world, input)
        .unwrap_or_else(|error| panic!("{label}: {error:?}"))
        .next;
    let image = capture(&next, LIMITS)
        .unwrap_or_else(|error| panic!("checkpoint after {label}: {error:?}"));
    assert_eq!(restore(image, LIMITS).unwrap(), next);
    next
}

fn success(world: &World, id: EffectId) -> World {
    let world = raw(
        world,
        ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
    );
    let world = raw(&world, ControllerInput::Worker(id.clone()));
    raw(
        &world,
        ControllerInput::Effect(EffectInput::Receipt(
            id,
            EffectOutcome::Succeeded(ResultDigest(1)),
        )),
    )
}

fn finish_ready(world: &World) -> World {
    let mut world = world.clone();
    for _ in 0..32 {
        let ready = world
            .effect_ids()
            .iter()
            .find(|id| {
                world.effect_status(id) == Some(EffectStatus::Committed)
                    && handle_controller(
                        &world,
                        ControllerInput::Effect(EffectInput::Dispatch((*id).clone())),
                    )
                    .is_ok_and(|d| {
                        handle_controller(&d.next, ControllerInput::Worker((*id).clone())).is_ok()
                    })
            })
            .cloned();
        let Some(id) = ready else {
            return world;
        };
        world = success(&world, id);
    }
    panic!("bounded effect graph did not drain");
}

fn paused() -> World {
    let world = unload::drain(&unload::commit_planned());
    let source = unload::target(&world);
    let world = step(
        &world,
        ControllerInput::Publication(PublicationInput::Paused(SID, source)),
    );
    let binding = world
        .maintenance_state()
        .unwrap()
        .host_slot(HostSlotId(0))
        .unwrap()
        .binding;
    let world = cleanup(&world, binding);
    release(&world, 0, binding)
}

fn resume_admitted() -> World {
    raw(
        &paused(),
        ControllerInput::Request(Request::Resume(ResumeRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(3),
        })),
    )
}

fn reserve_current(world: &World) -> World {
    let operation = world
        .sandbox_record(SID)
        .unwrap()
        .current_operation
        .unwrap()
        .id;
    let world = raw(
        world,
        ControllerInput::Scheduler(SchedulerInput::ReserveStartup(OperationKey {
            sandbox: SID,
            operation,
        })),
    );
    let plan = world
        .operation_plan(OperationKey {
            sandbox: SID,
            operation,
        })
        .unwrap();
    let binding = HostBinding {
        owner: OWNER,
        sandbox: SID,
        generation: plan.generation,
        vm: plan.vm.unwrap(),
        network: plan.slot.unwrap(),
    };
    let previous = world
        .maintenance_state()
        .unwrap()
        .host_slot(HostSlotId(0))
        .unwrap()
        .binding
        .vm;
    raw(
        &world,
        ControllerInput::Maintenance(MaintenanceInput::Slot(SlotInput::Bind {
            position: HostSlotId(0),
            expected_previous: Some(previous),
            binding,
        })),
    )
}

fn resume_reserved() -> World {
    reserve_current(&resume_admitted())
}

fn unobserved_restore_then_kill() -> (World, EffectId, EffectId) {
    let world = finish_ready(&resume_reserved());
    let plan = world
        .operation_plan(OperationKey {
            sandbox: SID,
            operation: OperationId(3),
        })
        .unwrap();
    let world = raw(
        &world,
        ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
            SID,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        )),
    );
    let prepare = effect(&world, SID, |c| {
        matches!(c, Command::PrepareVm { epoch: 3, .. })
    });
    let world = raw(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(prepare.clone())),
    );
    let world = raw(
        &world,
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(4),
        })),
    );
    let fence = effect(&world, SID, |c| {
        matches!(
            c,
            Command::InstallFence {
                epoch: 4,
                terminal: true,
                ..
            }
        )
    });
    (world, prepare, fence)
}

#[test]
fn restore_terminal_fence_precedes_unknown_prepare_without_physical_release() {
    for unknown in [false, true] {
        let (world, prepare, fence) = unobserved_restore_then_kill();
        let world = if unknown {
            raw(
                &world,
                ControllerInput::Effect(EffectInput::Timeout(prepare.clone())),
            )
        } else {
            world
        };
        assert!(!crate::scheduler::only_uncertain_prepare_for_abort(
            &world, SID
        ));
        assert!(crate::scheduler::dispatch_allowed_during_abort(
            &world,
            fence.command()
        ));
        assert_eq!(
            crate::workflow::next_step(&world, SID),
            crate::workflow::WorkflowStep::Claim
        );
        let original_status = world.effect_status(&prepare);
        let world = success(&world, fence);
        assert_eq!(world.effect_status(&prepare), original_status);
        let row = world.sandbox_record(SID).unwrap();
        assert!(row.terminal && row.current_vm.is_none() && row.net_slot.is_none());
        assert_eq!(row.current_operation.unwrap().kind, OperationKind::Kill);
        assert_eq!(row.phase, Phase::Revoking);
        assert!(!crate::scheduler::dispatch_allowed_during_abort(
            &world,
            prepare.command()
        ));
        for command in [
            Command::ActivateVm {
                sandbox: SID,
                epoch: 4,
                generation: 2,
                vm: VmId(1),
            },
            Command::StopVm {
                sandbox: SID,
                epoch: 4,
                generation: 2,
                vm: VmId(1),
            },
            Command::AbortPrepare {
                sandbox: SID,
                epoch: 4,
                generation: 2,
                vm: VmId(1),
                slot: NetSlotId(1),
            },
            Command::CleanupVm {
                sandbox: SID,
                generation: 2,
                vm: VmId(1),
            },
        ] {
            assert!(!crate::scheduler::dispatch_allowed_during_abort(
                &world, &command
            ));
        }
        roundtrip(&world);
    }
}

#[test]
fn restore_terminal_fence_rejects_old_epoch_nonterminal_and_foreign_commands() {
    let (world, _, fence) = unobserved_restore_then_kill();
    for command in [
        Command::InstallFence {
            sandbox: SID,
            epoch: 3,
            terminal: true,
        },
        Command::InstallFence {
            sandbox: SID,
            epoch: 5,
            terminal: true,
        },
        Command::InstallFence {
            sandbox: SID,
            epoch: 4,
            terminal: false,
        },
        Command::InstallFence {
            sandbox: SandboxId(11),
            epoch: 4,
            terminal: true,
        },
    ] {
        assert!(!crate::scheduler::dispatch_allowed_during_abort(
            &world, &command
        ));
    }
    assert!(crate::scheduler::dispatch_allowed_during_abort(
        &world,
        fence.command()
    ));
}

#[test]
fn restore_terminal_fence_rejects_substituted_origin_and_other_unknown_work() {
    let (world, prepare, fence) = unobserved_restore_then_kill();
    for variant in 0..12 {
        let mut altered = world.clone();
        let mut row = altered.sandbox_record(SID).unwrap().clone();
        let key = OperationKey {
            sandbox: SID,
            operation: OperationId(3),
        };
        let mut plan = altered.operation_plan(key).unwrap().clone();
        match variant {
            0 => {
                row.owner = TenantId(99);
                altered.sandboxes.put(SID, row);
            }
            1 => {
                row.generation = 3;
                altered.sandboxes.put(SID, row);
            }
            2 => {
                row.terminal = false;
                altered.sandboxes.put(SID, row);
            }
            3 => {
                plan.vm = Some(VmId(999));
                altered.operation_plans.put(key, plan);
            }
            4 => {
                plan.slot = Some(NetSlotId(999));
                altered.operation_plans.put(key, plan);
            }
            5 => {
                plan.restore_from.as_mut().unwrap().manifest.digest = ManifestDigest(999);
                altered.operation_plans.put(key, plan);
            }
            6 => {
                plan.sealed = false;
                altered.operation_plans.put(key, plan);
            }
            7 => {
                let mut logical = altered.logical_effect(&prepare.logical).unwrap();
                logical.current_attempt += 1;
                altered
                    .logical_effects
                    .put(prepare.logical.clone(), logical);
            }
            8 => {
                let mut logical = altered.logical_effect(&fence.logical).unwrap();
                logical.origin = EffectOrigin::Operation(key);
                altered.logical_effects.put(fence.logical.clone(), logical);
            }
            9 => {
                let other = EffectId::initial(Command::ApplyFilePolicy {
                    sandbox: SID,
                    epoch: 3,
                    generation: 2,
                    workspace: row.workspace,
                    policy: row.file_policy,
                });
                altered.effect_keys.push(other.clone());
                altered.effects.put(other, EffectStatus::Dispatched);
            }
            10 => {
                row.desired = Authority::Paused {
                    sandbox: SID,
                    source: row.durable_snapshot.unwrap(),
                };
                altered.sandboxes.put(SID, row);
            }
            _ => {
                row.effective = Authority::Paused {
                    sandbox: SID,
                    source: row.durable_snapshot.unwrap(),
                };
                altered.sandboxes.put(SID, row);
            }
        }
        assert!(
            !crate::scheduler::dispatch_allowed_during_abort(&altered, fence.command()),
            "accepted substituted terminal-fence basis {variant}"
        );
    }
}

fn resumed() -> World {
    let world = finish_ready(&resume_reserved());
    let plan = world
        .operation_plan(OperationKey {
            sandbox: SID,
            operation: OperationId(3),
        })
        .unwrap();
    let world = raw(
        &world,
        ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
            SID,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        )),
    );
    let world = finish_ready(&world);
    raw(
        &world,
        ControllerInput::Publication(PublicationInput::Running(SID)),
    )
}

fn drive(world: &World) -> World {
    let mut world = world.clone();
    for _ in 0..128 {
        world = match crate::workflow::next_step(&world, SID) {
            crate::workflow::WorkflowStep::Apply(input) => raw(&world, *input),
            crate::workflow::WorkflowStep::Claim => finish_ready(&world),
            crate::workflow::WorkflowStep::PublishRunning => raw(
                &world,
                ControllerInput::Publication(PublicationInput::Running(SID)),
            ),
            crate::workflow::WorkflowStep::Complete => return world,
            crate::workflow::WorkflowStep::Blocked => {
                panic!("actual driver parked: {:?}", world.sandbox_record(SID))
            }
        };
    }
    panic!("bounded lifecycle did not complete");
}

#[test]
fn resume_admission_and_reservation_are_recoverable_checkpoints() {
    for world in [resume_admitted(), resume_reserved()] {
        assert_eq!(world.sandbox_record(SID).unwrap().generation, 2);
        roundtrip(&world);
    }
}

#[test]
fn original_resume_driver_advances_after_its_fence() {
    let world = finish_ready(&resume_reserved());
    let plan = world
        .operation_plan(OperationKey {
            sandbox: SID,
            operation: OperationId(3),
        })
        .unwrap();
    assert_eq!(
        crate::workflow::next_step(&world, SID),
        crate::workflow::WorkflowStep::Apply(Box::new(ControllerInput::Scheduler(
            SchedulerInput::DispatchPrepare(SID, plan.vm.unwrap(), plan.slot.unwrap())
        )))
    );
}

#[test]
fn second_pause_keeps_the_prior_generation_history_and_completes() {
    let world = resumed();
    assert!(
        world
            .vms
            .entries()
            .iter()
            .any(|(_, vm)| vm.generation == 1 && vm.status == VmStatus::Reclaimed)
    );
    let world = raw(
        &world,
        ControllerInput::Request(Request::Pause(PauseRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(4),
        })),
    );
    let world = drive(&world);
    assert_eq!(world.sandbox_record(SID).unwrap().phase, Phase::Paused);
    assert_eq!(world.sandbox_record(SID).unwrap().generation, 2);
    roundtrip(&world);
}

#[test]
fn destroy_after_resume_does_not_forget_or_restop_reclaimed_old_generation() {
    let world = raw(
        &resumed(),
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(4),
        })),
    );
    let world = drive(&world);
    assert_eq!(world.sandbox_record(SID).unwrap().phase, Phase::Destroyed);
    assert!(
        world
            .vms
            .entries()
            .iter()
            .all(|(_, vm)| vm.status == VmStatus::Reclaimed)
    );
    roundtrip(&world);
}

fn second_paused_released() -> World {
    let world = raw(
        &resumed(),
        ControllerInput::Request(Request::Pause(PauseRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(4),
        })),
    );
    let world = drive(&world);
    let binding = world
        .maintenance_state()
        .unwrap()
        .host_slot(HostSlotId(0))
        .unwrap()
        .binding;
    release(&world, 0, binding)
}

#[test]
fn two_restore_cycles_roundtrip_every_microstep_without_reusing_old_vm() {
    let world = raw(
        &second_paused_released(),
        ControllerInput::Request(Request::Resume(ResumeRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(5),
        })),
    );
    let world = drive(&reserve_current(&world));
    let row = world.sandbox_record(SID).unwrap();
    assert_eq!(row.phase, Phase::Running);
    assert_eq!(row.generation, 3);
    assert_eq!(row.epoch, 5);
    assert!(row.restore_from.is_none());
    assert_eq!(world.vms.entries().len(), 3);
    assert_eq!(
        world
            .vms
            .entries()
            .iter()
            .filter(|(_, v)| v.status == VmStatus::Reclaimed)
            .count(),
        2
    );
    let world = raw(
        &world,
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(6),
        })),
    );
    let world = drive(&world);
    assert_eq!(world.sandbox_record(SID).unwrap().phase, Phase::Destroyed);
    assert!(
        world
            .vms
            .entries()
            .iter()
            .all(|(_, v)| v.status == VmStatus::Reclaimed)
    );
    let binding = world
        .maintenance_state()
        .unwrap()
        .host_slot(HostSlotId(0))
        .unwrap()
        .binding;
    let world = release(&world, 0, binding);
    let world = retire(&world, SID);
    assert!(world.sandbox_record(SID).is_none());
    assert_eq!(world.maintenance_state().unwrap().retained_count(), 6);
    roundtrip(&world);
}

#[test]
fn resume_checkpoint_rejects_source_epoch_generation_and_resource_substitution() {
    let base = capture(&resume_reserved(), LIMITS).unwrap();
    for variant in 0..7 {
        let mut image = base.clone();
        let plan = image
            .0
            .operation_plans
            .iter_mut()
            .find(|(key, _)| key.operation == OperationId(3))
            .unwrap();
        match variant {
            0 => plan.1.restore_from.as_mut().unwrap().manifest.digest = ManifestDigest(999),
            1 => plan.1.generation = 3,
            2 => plan.1.epoch = 4,
            3 => plan.1.vm = Some(VmId(0)),
            4 => image.0.snapshots[0].1.source_generation = 2,
            5 => image.0.sandboxes[0].1.generation = 1,
            _ => {
                image
                    .0
                    .operations
                    .iter_mut()
                    .find(|(key, _)| key.operation == OperationId(2))
                    .unwrap()
                    .1
                    .operation
                    .epoch = 3;
            }
        }
        assert!(
            restore(image, LIMITS).is_err(),
            "accepted corruption {variant}"
        );
    }
}

#[test]
fn third_generation_cannot_rewind_to_an_older_valid_same_session_snapshot() {
    let world = raw(
        &second_paused_released(),
        ControllerInput::Request(Request::Resume(ResumeRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(5),
        })),
    );
    let mut image = capture(&world, LIMITS).unwrap();
    let old = image
        .0
        .operation_plans
        .iter()
        .find(|(key, _)| key.operation == OperationId(2))
        .unwrap()
        .1
        .target_snapshot
        .unwrap();
    image
        .0
        .operations
        .iter_mut()
        .find(|(key, _)| key.operation == OperationId(5))
        .unwrap()
        .1
        .input = OperationInput::Resume { source: old };
    image
        .0
        .operation_plans
        .iter_mut()
        .find(|(key, _)| key.operation == OperationId(5))
        .unwrap()
        .1
        .restore_from = Some(old);
    image.0.sandboxes[0].1.restore_from = Some(old);
    image.0.sandboxes[0].1.durable_snapshot = Some(old);
    assert!(restore(image, LIMITS).is_err());
}

#[test]
fn unknown_activation_survives_restart_and_never_requests_another_load() {
    let world = finish_ready(&resume_reserved());
    let plan = world
        .operation_plan(OperationKey {
            sandbox: SID,
            operation: OperationId(3),
        })
        .unwrap();
    let world = raw(
        &world,
        ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
            SID,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        )),
    );
    let prepare = effect(&world, SID, |c| {
        matches!(c, Command::PrepareVm { epoch: 3, .. })
    });
    let world = success(&world, prepare);
    let network = effect(&world, SID, |c| {
        matches!(c, Command::ConfigureNetwork { epoch: 3, .. })
    });
    let world = success(&world, network);
    let activation = effect(&world, SID, |c| {
        matches!(c, Command::ActivateVm { epoch: 3, .. })
    });
    let world = raw(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(activation.clone())),
    );
    let world = raw(&world, ControllerInput::Worker(activation.clone()));
    let world = raw(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(activation.clone())),
    );
    let vm = world.sandbox_record(SID).unwrap().current_vm;
    let next_vm = world.next_vm_id;
    let world = raw(&world, ControllerInput::Runtime(RuntimeInput::Crash));
    let world = raw(&world, ControllerInput::Runtime(RuntimeInput::Recover));
    assert_eq!(
        world.effect_status(&activation),
        Some(EffectStatus::OutcomeUnknown)
    );
    assert_eq!(world.sandbox_record(SID).unwrap().current_vm, vm);
    assert_eq!(world.next_vm_id, next_vm);
    assert_ne!(
        crate::workflow::next_step(&world, SID),
        crate::workflow::WorkflowStep::PublishRunning
    );
    assert!(
        handle_controller(
            &world,
            ControllerInput::Scheduler(SchedulerInput::ReserveStartup(OperationKey {
                sandbox: SID,
                operation: OperationId(3)
            }))
        )
        .is_err()
    );
}

#[test]
fn restore_allocation_cannot_adopt_an_old_vm_even_if_slot_binding_is_forged_with_it() {
    let mut image = capture(&resume_reserved(), LIMITS).unwrap();
    let plan = &mut image
        .0
        .operation_plans
        .iter_mut()
        .find(|(key, _)| key.operation == OperationId(3))
        .unwrap()
        .1;
    plan.vm = Some(VmId(0));
    plan.slot = Some(NetSlotId(0));
    plan.stop_vms = vec![VmId(0)];
    plan.close_slots = vec![NetSlotId(0)];
    let slot = &mut image.0.maintenance.as_mut().unwrap().slots[0].1;
    slot.binding.vm = VmId(0);
    slot.binding.network = NetSlotId(0);
    assert!(restore(image, LIMITS).is_err());
    let mut image = capture(&resumed(), LIMITS).unwrap();
    image
        .0
        .operations
        .iter_mut()
        .find(|(key, _)| key.operation == OperationId(3))
        .unwrap()
        .1
        .result = Some(OperationResult::Running(3));
    assert!(restore(image, LIMITS).is_err());
}

#[test]
fn snapshot_retirement_still_requires_every_original_quiescence_condition() {
    use crate::maintenance::retirement::{
        QuiescenceCertificate, QuiescenceStatus, RetireRequest, RetirementReject,
    };
    let world = drive(&raw(
        &resumed(),
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox: SID,
            operation: OperationId(4),
        })),
    ));
    let binding = world
        .maintenance_state()
        .unwrap()
        .host_slot(HostSlotId(0))
        .unwrap()
        .binding;
    let world = release(&world, 0, binding);
    let request = RetireRequest {
        sandbox: SID,
        owner: OWNER,
        epoch: 4,
        operation: OperationId(4),
        retain_until: 100,
        quiescence: QuiescenceCertificate {
            sandbox: SID,
            owner: OWNER,
            epoch: 4,
            operation: OperationId(4),
            workspace: FsId(10),
            worker_drained: QuiescenceStatus::Confirmed,
            data_connections_drained: QuiescenceStatus::Confirmed,
            host_resources_reclaimed: QuiescenceStatus::Confirmed,
            external_pins_released: QuiescenceStatus::Confirmed,
        },
    };
    for missing in 0..4 {
        let mut invalid = request;
        match missing {
            0 => invalid.quiescence.worker_drained = QuiescenceStatus::Unconfirmed,
            1 => invalid.quiescence.data_connections_drained = QuiescenceStatus::Unconfirmed,
            2 => invalid.quiescence.host_resources_reclaimed = QuiescenceStatus::Unconfirmed,
            _ => invalid.quiescence.external_pins_released = QuiescenceStatus::Unconfirmed,
        }
        assert_eq!(
            crate::maintenance::retirement::retire(&world, invalid),
            Err(RetirementReject::CertificateMismatch)
        );
    }
    let retired = crate::maintenance::retirement::retire(&world, request).unwrap();
    roundtrip(&retired);
    let mut invalid = capture(&retired, LIMITS).unwrap();
    for (_, row) in &mut invalid.0.maintenance.as_mut().unwrap().retained {
        if let Some(OperationResult::Paused(source)) = &mut row.ledger.result {
            source.snapshot = SnapshotId(retired.next_snapshot_id);
        }
        if let OperationInput::Resume { source } = &mut row.ledger.input {
            source.snapshot = SnapshotId(retired.next_snapshot_id);
        }
    }
    assert!(
        restore(invalid, LIMITS).is_err(),
        "retained results cannot invent future snapshot identities"
    );
}

#[test]
fn resumed_compensation_keeps_live_unknown_and_current_generation_resources() {
    let world = resume_reserved();
    let index = world.resource_index(SID);
    let old_vm = index
        .vms
        .iter()
        .copied()
        .find(|id| world.vm_record(*id).unwrap().generation == 1)
        .unwrap();
    let old_slot = index
        .slots
        .iter()
        .copied()
        .find(|id| world.slot_record(*id).unwrap().generation == 1)
        .unwrap();
    let operation = world
        .sandbox_record(SID)
        .unwrap()
        .current_operation
        .unwrap();
    let compensated_plan = |world: &World| {
        let record = world.sandbox_record(SID).unwrap();
        let next = crate::transition::begin_compensation(
            world,
            SID,
            record,
            operation,
            FailureKind::DeadlineExceeded,
        );
        next.operation_plan(crate::transition::operation_key(SID, operation.id))
            .unwrap()
            .clone()
    };
    let record = world.sandbox_record(SID).unwrap();
    let compensated = crate::transition::begin_compensation(
        &world,
        SID,
        record,
        operation,
        FailureKind::DeadlineExceeded,
    );
    assert_eq!(
        restore(capture(&compensated, LIMITS).unwrap(), LIMITS).unwrap(),
        compensated
    );
    let clean = compensated_plan(&world);
    assert!(!clean.stop_vms.contains(&old_vm));
    assert!(!clean.close_slots.contains(&old_slot));
    for status in [
        VmStatus::Prepared,
        VmStatus::Executing,
        VmStatus::Frozen,
        VmStatus::Stopped,
    ] {
        let mut pending = world.clone();
        let mut vm = *pending.vm_record(old_vm).unwrap();
        vm.status = status;
        pending.vms.put(old_vm, vm);
        let plan = compensated_plan(&pending);
        assert!(plan.stop_vms.contains(&old_vm));
        assert!(plan.close_slots.contains(&old_slot));
    }
    let mut active = world.clone();
    let mut slot = active.slot_record(old_slot).unwrap().clone();
    slot.active = true;
    active.net_slots.put(old_slot, slot);
    assert!(compensated_plan(&active).close_slots.contains(&old_slot));
    let mut unknown = world.clone();
    let mut indexed = index;
    indexed.vms.push(VmId(999));
    indexed.slots.push(NetSlotId(999));
    unknown.resource_indexes.put(SID, indexed);
    let key = crate::transition::operation_key(SID, operation.id);
    let mut pending_plan = unknown.operation_plan(key).unwrap().clone();
    pending_plan.stop_vms.push(VmId(999));
    pending_plan.close_slots.push(NetSlotId(999));
    unknown.operation_plans.put(key, pending_plan);
    let plan = compensated_plan(&unknown);
    assert!(plan.stop_vms.contains(&VmId(999)));
    assert!(plan.close_slots.contains(&NetSlotId(999)));
    let mut current = world.clone();
    let mut vm = *current.vm_record(old_vm).unwrap();
    vm.generation = 2;
    current.vms.put(old_vm, vm);
    assert!(compensated_plan(&current).stop_vms.contains(&old_vm));
}

#[test]
fn late_resumed_prepare_rejects_wrong_source_manifest_and_generation() {
    let world = finish_ready(&resume_reserved());
    let key = OperationKey {
        sandbox: SID,
        operation: OperationId(3),
    };
    let plan = world.operation_plan(key).unwrap();
    let world = raw(
        &world,
        ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
            SID,
            plan.vm.unwrap(),
            plan.slot.unwrap(),
        )),
    );
    let prepare = world
        .effect_ids()
        .iter()
        .find(|id| {
            matches!(
                id.command(),
                Command::PrepareVm {
                    source: Some(_),
                    ..
                }
            )
        })
        .unwrap()
        .clone();
    let world = raw(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(prepare.clone())),
    );
    let world = raw(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(prepare.clone())),
    );
    let record = world.sandbox_record(SID).unwrap();
    let world = crate::transition::begin_compensation(
        &world,
        SID,
        record,
        record.current_operation.unwrap(),
        FailureKind::DeadlineExceeded,
    );
    assert!(crate::terminal::startup_effect_in_current_abort(
        &world, &prepare
    ));
    let source = world.operation_plan(key).unwrap().restore_from.unwrap();
    let mut wrong = world.clone();
    let mut snapshot = *wrong.snapshots.get(&source.snapshot).unwrap();
    snapshot.manifest.digest = ManifestDigest(9999);
    wrong.snapshots.put(source.snapshot, snapshot);
    assert!(!crate::terminal::startup_effect_in_current_abort(
        &wrong, &prepare
    ));
    let mut wrong = world.clone();
    let mut snapshot = *wrong.snapshots.get(&source.snapshot).unwrap();
    snapshot.source_generation = 2;
    wrong.snapshots.put(source.snapshot, snapshot);
    assert!(!crate::terminal::startup_effect_in_current_abort(
        &wrong, &prepare
    ));
}
