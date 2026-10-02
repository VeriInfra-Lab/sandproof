use super::*;
use crate::maintenance::retirement::{QuiescenceCertificate, QuiescenceStatus, RetireRequest};
use crate::maintenance::slots::{HostBinding, HostSlotId};
use crate::maintenance::{
    MaintenanceInput, MaintenanceLimits, slots::SlotInput, windows::WindowLimits,
};

pub(super) const OWNER: TenantId = TenantId(7);

pub(super) fn roundtrip(world: &World) {
    assert_eq!(
        restore(
            capture(world, LIMITS).expect("reachable V1 state captures"),
            LIMITS
        )
        .unwrap(),
        *world
    );
}
pub(super) fn step(world: &World, input: ControllerInput) -> World {
    let description = format!("{input:?}");
    let next = handle_controller(world, input)
        .unwrap_or_else(|error| panic!("{description}: {error:?}"))
        .next;
    let checkpoint = capture(&next, LIMITS)
        .unwrap_or_else(|error| panic!("checkpoint after {description}: {error:?}"));
    assert_eq!(restore(checkpoint, LIMITS).unwrap(), next);
    next
}
pub(super) fn maintenance(world: &World, input: MaintenanceInput) -> World {
    step(world, ControllerInput::Maintenance(input))
}
pub(super) fn enabled() -> World {
    maintenance(
        &initial_world(),
        MaintenanceInput::Enable {
            limits: MaintenanceLimits {
                max_retained_results: 100,
                max_host_slots: 4,
                windows: WindowLimits::new(4, 100).unwrap(),
            },
            now_ms: 5,
        },
    )
}
pub(super) fn create(world: &World, sandbox: SandboxId) -> World {
    step(
        world,
        ControllerInput::Request(Request::Create(CreateRequest {
            actor: Actor::Tenant(OWNER),
            owner: OWNER,
            sandbox,
            workspace: FsId(sandbox.0),
            file_policy: FilePolicyRef {
                digest: PolicyDigest(3),
            },
            policy: NetworkPolicy::DenyAll,
            operation: OperationId(1),
        })),
    )
}
pub(super) fn effect(
    world: &World,
    sandbox: SandboxId,
    predicate: impl Fn(&Command) -> bool,
) -> EffectId {
    world
        .effect_ids()
        .iter()
        .find(|id| id.command().sandbox() == sandbox && predicate(id.command()))
        .unwrap()
        .clone()
}
pub(super) fn execute_id(world: &World, id: EffectId) -> World {
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
pub(super) fn execute(
    world: &World,
    sandbox: SandboxId,
    predicate: impl Fn(&Command) -> bool,
) -> World {
    execute_id(world, effect(world, sandbox, predicate))
}
pub(super) fn reserve(world: &World, sandbox: SandboxId) -> (World, HostBinding) {
    let world = create(world, sandbox);
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
        generation: 1,
        vm: plan.vm.unwrap(),
        network: plan.slot.unwrap(),
    };
    (world, binding)
}
pub(super) fn bind(world: &World, position: u64, binding: HostBinding) -> World {
    let expected_previous = world
        .maintenance_state()
        .unwrap()
        .host_slot(HostSlotId(position))
        .map(|row| row.binding.vm);
    maintenance(
        world,
        MaintenanceInput::Slot(SlotInput::Bind {
            position: HostSlotId(position),
            expected_previous,
            binding,
        }),
    )
}
pub(super) fn prepared(world: &World, binding: HostBinding) -> World {
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
    execute(&world, sandbox, |c| matches!(c, Command::PrepareVm { .. }))
}
pub(super) fn running(world: &World, binding: HostBinding) -> World {
    let world = prepared(world, binding);
    let sandbox = binding.sandbox;
    let world = execute(&world, sandbox, |c| {
        matches!(c, Command::ConfigureNetwork { .. })
    });
    let world = execute(&world, sandbox, |c| matches!(c, Command::ActivateVm { .. }));
    let world = execute(&world, sandbox, |c| {
        matches!(c, Command::ApplyFilePolicy { .. })
    });
    step(
        &world,
        ControllerInput::Publication(PublicationInput::Running(sandbox)),
    )
}
pub(super) fn kill(world: &World, binding: HostBinding) -> World {
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
        ControllerInput::Scheduler(SchedulerInput::StopVm(sandbox, 1, binding.vm)),
    );
    let world = execute(&world, sandbox, |c| matches!(c, Command::StopVm { .. }));
    step(
        &world,
        ControllerInput::Publication(PublicationInput::Destroyed(sandbox)),
    )
}
pub(super) fn cleanup(world: &World, binding: HostBinding) -> World {
    let sandbox = binding.sandbox;
    let world = step(
        world,
        ControllerInput::Scheduler(SchedulerInput::CleanupVm(sandbox, 1, binding.vm)),
    );
    let world = execute(&world, sandbox, |c| matches!(c, Command::CleanupVm { .. }));
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::CleanupSlot(sandbox, 1, binding.network)),
    );
    execute(&world, sandbox, |c| {
        matches!(c, Command::CleanupSlot { .. })
    })
}
pub(super) fn release(world: &World, position: u64, binding: HostBinding) -> World {
    let world = maintenance(
        world,
        MaintenanceInput::Slot(SlotInput::BeginRelease {
            position: HostSlotId(position),
            binding,
        }),
    );
    maintenance(
        &world,
        MaintenanceInput::Slot(SlotInput::ConfirmRelease {
            position: HostSlotId(position),
            binding,
        }),
    )
}
pub(super) fn retire(world: &World, sandbox: SandboxId) -> World {
    let record = world.sandbox_record(sandbox).unwrap();
    let operation = record.current_operation.unwrap().id;
    maintenance(
        world,
        MaintenanceInput::Retire(RetireRequest {
            sandbox,
            owner: record.owner,
            epoch: record.epoch,
            operation,
            retain_until: 100,
            quiescence: QuiescenceCertificate {
                sandbox,
                owner: record.owner,
                epoch: record.epoch,
                operation,
                workspace: record.workspace,
                worker_drained: QuiescenceStatus::Confirmed,
                data_connections_drained: QuiescenceStatus::Confirmed,
                host_resources_reclaimed: QuiescenceStatus::Confirmed,
                external_pins_released: QuiescenceStatus::Confirmed,
            },
        }),
    )
}
pub(super) fn live() -> (World, HostBinding) {
    let (world, binding) = reserve(&enabled(), SandboxId(10));
    let world = bind(&world, 0, binding);
    (running(&world, binding), binding)
}
pub(super) fn retired() -> World {
    let (world, binding) = live();
    let world = cleanup(&kill(&world, binding), binding);
    let world = release(&world, 0, binding);
    retire(&world, binding.sandbox)
}
