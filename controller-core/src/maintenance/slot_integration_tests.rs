use super::retirement::{QuiescenceCertificate, QuiescenceStatus, RetireRequest, RetirementReject};
use super::slots::{HostBinding, HostSlotId, HostSlotPhase, SlotInput, SlotReject};
use super::windows::WindowLimits;
use super::{MaintenanceInput, MaintenanceLimits, MaintenanceReject};
use crate::*;

const OWNER: TenantId = TenantId(7);
const POSITION: HostSlotId = HostSlotId(0);

fn step(world: &World, input: ControllerInput) -> World {
    handle_controller(world, input)
        .expect("valid controller history")
        .next
}

fn maintenance(world: &World, input: MaintenanceInput) -> World {
    step(world, ControllerInput::Maintenance(input))
}

fn slot(world: &World, input: SlotInput) -> World {
    maintenance(world, MaintenanceInput::Slot(input))
}

fn enabled() -> World {
    maintenance(
        &initial_world(),
        MaintenanceInput::Enable {
            limits: MaintenanceLimits {
                max_retained_results: 4,
                max_host_slots: 2,
                windows: WindowLimits::new(2, 100).unwrap(),
            },
            now_ms: 0,
        },
    )
}

fn reserve(world: &World, sandbox: SandboxId) -> (World, HostBinding) {
    let world = step(
        world,
        ControllerInput::Request(Request::Create(CreateRequest {
            actor: Actor::Tenant(OWNER),
            owner: OWNER,
            sandbox,
            workspace: FsId(sandbox.0),
            file_policy: FilePolicyRef {
                digest: PolicyDigest(1),
            },
            policy: NetworkPolicy::DenyAll,
            operation: OperationId(1),
        })),
    );
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::ReserveStartup(OperationKey {
            sandbox,
            operation: OperationId(1),
        })),
    );
    let plan = world
        .operation_plan(OperationKey {
            sandbox,
            operation: OperationId(1),
        })
        .unwrap();
    let binding = HostBinding {
        owner: OWNER,
        sandbox,
        generation: plan.generation,
        vm: plan.vm.unwrap(),
        network: plan.slot.unwrap(),
    };
    (world, binding)
}

fn execute(world: &World, sandbox: SandboxId, predicate: impl Fn(&Command) -> bool) -> World {
    let id = world
        .effect_ids()
        .iter()
        .find(|id| id.command().sandbox() == sandbox && predicate(id.command()))
        .expect("expected effect")
        .clone();
    let world = step(
        world,
        ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
    );
    let world = step(&world, ControllerInput::Worker(id.clone()));
    step(
        &world,
        ControllerInput::Effect(EffectInput::Receipt(
            id,
            EffectOutcome::Succeeded(ResultDigest(1)),
        )),
    )
}

fn run(world: &World, binding: HostBinding) -> World {
    let sandbox = binding.sandbox;
    let world = execute(world, sandbox, |c| {
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
        ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
            sandbox,
            binding.vm,
            binding.network,
        )),
    );
    let world = execute(&world, sandbox, |c| matches!(c, Command::PrepareVm { .. }));
    let world = execute(&world, sandbox, |c| {
        matches!(c, Command::ConfigureNetwork { .. })
    });
    let world = execute(&world, sandbox, |c| {
        matches!(c, Command::ApplyFilePolicy { .. })
    });
    let world = execute(&world, sandbox, |c| matches!(c, Command::ActivateVm { .. }));
    step(
        &world,
        ControllerInput::Publication(PublicationInput::Running(sandbox)),
    )
}

fn destroy_and_reclaim(world: &World, binding: HostBinding) -> World {
    let sandbox = binding.sandbox;
    let world = step(
        world,
        ControllerInput::Request(Request::Kill(KillRequest {
            actor: Actor::Tenant(OWNER),
            sandbox,
            operation: OperationId(2),
        })),
    );
    let world = execute(&world, sandbox, |c| {
        matches!(c, Command::InstallFence { terminal: true, .. })
    });
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::StopVm(
            sandbox,
            binding.generation,
            binding.vm,
        )),
    );
    let world = execute(&world, sandbox, |c| matches!(c, Command::StopVm { .. }));
    let world = step(
        &world,
        ControllerInput::Publication(PublicationInput::Destroyed(sandbox)),
    );
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::CleanupVm(
            sandbox,
            binding.generation,
            binding.vm,
        )),
    );
    let world = execute(&world, sandbox, |c| matches!(c, Command::CleanupVm { .. }));
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::CleanupSlot(
            sandbox,
            binding.generation,
            binding.network,
        )),
    );
    execute(&world, sandbox, |c| {
        matches!(c, Command::CleanupSlot { .. })
    })
}

fn retirement(world: &World, sandbox: SandboxId) -> MaintenanceInput {
    let record = world.sandbox_record(sandbox).unwrap();
    let operation = record.current_operation.unwrap().id;
    MaintenanceInput::Retire(RetireRequest {
        sandbox,
        owner: OWNER,
        epoch: record.epoch,
        operation,
        retain_until: 100,
        quiescence: QuiescenceCertificate {
            sandbox,
            owner: OWNER,
            epoch: record.epoch,
            operation,
            workspace: record.workspace,
            worker_drained: QuiescenceStatus::Confirmed,
            data_connections_drained: QuiescenceStatus::Confirmed,
            host_resources_reclaimed: QuiescenceStatus::Confirmed,
            external_pins_released: QuiescenceStatus::Confirmed,
        },
    })
}

fn reject_slot(world: &World, input: SlotInput, reason: MaintenanceReject) {
    let before = world.clone();
    assert_eq!(
        handle_controller(
            world,
            ControllerInput::Maintenance(MaintenanceInput::Slot(input))
        ),
        Err(ControllerReject::Maintenance(reason))
    );
    assert_eq!(*world, before);
}

#[test]
fn bind_checks_core_plan_identity_position_and_uniqueness() {
    let (world, binding) = reserve(&enabled(), SandboxId(1));
    let bind = |position, binding| SlotInput::Bind {
        position,
        expected_previous: None,
        binding,
    };
    for altered in [
        HostBinding {
            owner: TenantId(8),
            ..binding
        },
        HostBinding {
            generation: binding.generation + 1,
            ..binding
        },
        HostBinding {
            vm: VmId(binding.vm.0 + 1),
            ..binding
        },
        HostBinding {
            network: NetSlotId(binding.network.0 + 1),
            ..binding
        },
    ] {
        reject_slot(
            &world,
            bind(POSITION, altered),
            MaintenanceReject::ResourceMismatch,
        );
    }
    reject_slot(
        &world,
        bind(HostSlotId(2), binding),
        MaintenanceReject::Slot(SlotReject::OutOfRange),
    );
    reject_slot(
        &world,
        SlotInput::Bind {
            position: POSITION,
            expected_previous: Some(VmId(0)),
            binding,
        },
        MaintenanceReject::Slot(SlotReject::BindingMismatch),
    );
    let world = slot(&world, bind(POSITION, binding));
    reject_slot(
        &world,
        bind(HostSlotId(1), binding),
        MaintenanceReject::Slot(SlotReject::IdentityAlreadyBound),
    );
    reject_slot(
        &world,
        SlotInput::BeginRelease {
            position: POSITION,
            binding,
        },
        MaintenanceReject::ResourcesNotReclaimed,
    );
    assert_eq!(
        slot(
            &world,
            SlotInput::CheckOccupied {
                position: POSITION,
                binding
            }
        ),
        world
    );
}

#[test]
fn retirement_and_reuse_require_release_and_reject_old_incarnation_messages() {
    let (world, old) = reserve(&enabled(), SandboxId(1));
    let world = slot(
        &world,
        SlotInput::Bind {
            position: POSITION,
            expected_previous: None,
            binding: old,
        },
    );
    let world = destroy_and_reclaim(&run(&world, old), old);
    let req = retirement(&world, old.sandbox);
    assert_eq!(
        handle_controller(&world, ControllerInput::Maintenance(req.clone())),
        Err(ControllerReject::Maintenance(
            MaintenanceReject::Retirement(RetirementReject::ResourcesNotReclaimed)
        ))
    );
    reject_slot(
        &world,
        SlotInput::ConfirmRelease {
            position: POSITION,
            binding: old,
        },
        MaintenanceReject::Slot(SlotReject::NotReleasing),
    );
    let releasing = slot(
        &world,
        SlotInput::BeginRelease {
            position: POSITION,
            binding: old,
        },
    );
    assert_eq!(
        handle_controller(&releasing, ControllerInput::Maintenance(req.clone())),
        Err(ControllerReject::Maintenance(
            MaintenanceReject::Retirement(RetirementReject::ResourcesNotReclaimed)
        ))
    );
    let (world, new) = reserve(&releasing, SandboxId(2));
    assert!(new.vm > old.vm);
    reject_slot(
        &world,
        SlotInput::Bind {
            position: POSITION,
            expected_previous: Some(old.vm),
            binding: new,
        },
        MaintenanceReject::Slot(SlotReject::NotFree),
    );
    let free = slot(
        &world,
        SlotInput::ConfirmRelease {
            position: POSITION,
            binding: old,
        },
    );
    reject_slot(
        &free,
        SlotInput::CheckOccupied {
            position: POSITION,
            binding: old,
        },
        MaintenanceReject::Slot(SlotReject::NotOccupied),
    );
    let retired = maintenance(&free, req);
    assert!(retired.sandbox_record(old.sandbox).is_none());
    assert_eq!(
        retired
            .maintenance_state()
            .unwrap()
            .host_slot(POSITION)
            .unwrap()
            .binding,
        old
    );
    reject_slot(
        &retired,
        SlotInput::Bind {
            position: POSITION,
            expected_previous: None,
            binding: new,
        },
        MaintenanceReject::Slot(SlotReject::BindingMismatch),
    );
    let reused = slot(
        &retired,
        SlotInput::Bind {
            position: POSITION,
            expected_previous: Some(old.vm),
            binding: new,
        },
    );
    check_delayed_messages(&reused, old, new);
}

fn check_delayed_messages(reused: &World, old: HostBinding, new: HostBinding) {
    let row = reused
        .maintenance_state()
        .unwrap()
        .host_slot(POSITION)
        .unwrap();
    assert_eq!(row.binding, new);
    assert_eq!(row.phase, HostSlotPhase::Occupied);
    for delayed in [
        SlotInput::Bind {
            position: POSITION,
            expected_previous: Some(old.vm),
            binding: old,
        },
        SlotInput::BeginRelease {
            position: POSITION,
            binding: old,
        },
        SlotInput::ConfirmRelease {
            position: POSITION,
            binding: old,
        },
    ] {
        reject_slot(reused, delayed, MaintenanceReject::ResourceMismatch);
    }
    reject_slot(
        reused,
        SlotInput::CheckOccupied {
            position: POSITION,
            binding: old,
        },
        MaintenanceReject::Slot(SlotReject::BindingMismatch),
    );
    assert_eq!(
        slot(
            reused,
            SlotInput::CheckOccupied {
                position: POSITION,
                binding: new
            }
        ),
        *reused
    );
    assert_eq!(reused.maintenance_state().unwrap().host_slots().len(), 1);
}
