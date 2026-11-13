use super::*;
use crate::maintenance::{
    MaintenanceInput,
    slots::{HostSlotId, HostSlotPhase},
    windows::WindowInput,
};
use crate::*;
mod history;
use history::*;
mod capture_drain;
mod continuation;
mod malformed;
mod released_unload;
mod resume;
mod unload;

const LIMITS: CheckpointLimits = CheckpointLimits {
    max_records: 1000,
    max_references: 1000,
};

#[test]
fn empty_roundtrip_and_all_four_cursors_survive_without_retained_maxima() {
    let world = World {
        next_vm_id: u64::MAX,
        next_slot_id: 123,
        next_snapshot_id: 789,
        next_manifest_digest: u64::MAX,
        ..initial_world()
    };
    roundtrip(&world);
    let crashed = step(&world, ControllerInput::Runtime(RuntimeInput::Crash));
    assert!(!crashed.controller_up);
}

#[test]
fn every_create_kill_cleanup_step_roundtrips_and_historical_prepare_stays_successful() {
    let (world, binding) = live();
    let world = cleanup(&kill(&world, binding), binding);
    let prepare = effect(&world, binding.sandbox, |c| {
        matches!(c, Command::PrepareVm { .. })
    });
    assert!(!effect::command_result_present(&world, prepare.command()));
    assert!(matches!(
        world.effect_status(&prepare),
        Some(EffectStatus::Acknowledged(EffectOutcome::Succeeded(_)))
    ));
    roundtrip(&world);
}

#[test]
fn order_is_preserved_for_tables_effects_inflight_and_plans() {
    let (world, binding) = live();
    let world = kill(&world, binding);
    let mut wire = capture(&world, LIMITS).unwrap();
    wire.0.effect_keys.reverse();
    wire.0.inflight.rotate_left(1);
    wire.0.operations.reverse();
    let decoded = restore(wire.clone(), LIMITS).unwrap();
    assert_eq!(capture(&decoded, LIMITS).unwrap(), wire);
}

#[test]
fn retired_results_free_tombstones_and_expired_frontiers_survive() {
    let world = retired();
    let restored = restore(capture(&world, LIMITS).unwrap(), LIMITS).unwrap();
    assert_eq!(
        restored
            .maintenance_state()
            .unwrap()
            .host_slot(HostSlotId(0))
            .unwrap()
            .phase,
        HostSlotPhase::Free
    );
    assert_eq!(restored.maintenance_state().unwrap().retained_count(), 2);
    let query = ControllerInput::Request(Request::Query(QueryRequest {
        actor: Actor::Tenant(OWNER),
        sandbox: SandboxId(10),
        operation: OperationId(1),
    }));
    assert_eq!(
        handle_controller(&world, query.clone()),
        handle_controller(&restored, query.clone())
    );
    let expired = maintenance(&restored, MaintenanceInput::ExpireResults { now_ms: 100 });
    assert_eq!(expired.maintenance_state().unwrap().retained_count(), 0);
    assert!(handle_controller(&expired, query).is_err());
    let duplicate = ControllerInput::Request(Request::Create(CreateRequest {
        actor: Actor::Tenant(OWNER),
        owner: OWNER,
        sandbox: SandboxId(10),
        workspace: FsId(10),
        file_policy: FilePolicyRef {
            digest: PolicyDigest(3),
        },
        policy: NetworkPolicy::DenyAll,
        operation: OperationId(1),
    }));
    assert!(handle_controller(&expired, duplicate).is_err());
    let (world, binding) = reserve(&expired, SandboxId(11));
    let world = bind(&world, 0, binding);
    assert_eq!(
        world
            .maintenance_state()
            .unwrap()
            .host_slot(HostSlotId(0))
            .unwrap()
            .binding
            .vm,
        binding.vm
    );
}

#[test]
fn windows_clock_frontier_and_closed_ids_survive_with_unknown_older_work() {
    let (world, binding) = reserve(&enabled(), SandboxId(1));
    let world = execute(&world, binding.sandbox, |c| {
        matches!(c, Command::InstallFence { .. })
    });
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
            binding.sandbox,
            binding.vm,
            binding.network,
        )),
    );
    let prepare = effect(&world, binding.sandbox, |c| {
        matches!(c, Command::PrepareVm { .. })
    });
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::Dispatch(prepare.clone())),
    );
    let mut world = step(
        &world,
        ControllerInput::Effect(EffectInput::Timeout(prepare.clone())),
    );
    for sandbox in [10, 11, 12] {
        let (next, binding) = reserve(&world, SandboxId(sandbox));
        let next = bind(&next, 0, binding);
        let next = running(&next, binding);
        let next = cleanup(&kill(&next, binding), binding);
        let next = release(&next, 0, binding);
        world = retire(&next, binding.sandbox);
    }
    world = maintenance(
        &world,
        MaintenanceInput::Window(WindowInput::Issue {
            tenant: OWNER,
            now_ms: 6,
            duration_ms: 10,
        }),
    );
    world = maintenance(
        &world,
        MaintenanceInput::Window(WindowInput::AdvanceTime { now_ms: 16 }),
    );
    assert!(
        world
            .maintenance_state()
            .unwrap()
            .windows()
            .open_windows()
            .is_empty()
    );
    assert_eq!(
        world
            .maintenance_state()
            .unwrap()
            .windows()
            .issued_through(),
        Some(0)
    );
    assert_eq!(
        world.effect_status(&prepare),
        Some(EffectStatus::OutcomeUnknown)
    );
    roundtrip(&world);
}

#[test]
fn retries_final_failure_and_terminal_compensation_roundtrip() {
    let mut world = create(&enabled(), SandboxId(10));
    let first = effect(&world, SandboxId(10), |_| true);
    for attempt in 0..=3 {
        let id = EffectId {
            logical: first.logical.clone(),
            attempt,
        };
        world = step(
            &world,
            ControllerInput::Effect(EffectInput::Dispatch(id.clone())),
        );
        world = step(
            &world,
            ControllerInput::Effect(EffectInput::Receipt(
                id,
                EffectOutcome::Failed(FailureKind::ResourceFailure),
            )),
        );
        if attempt < 3 {
            world = step(
                &world,
                ControllerInput::Effect(EffectInput::Retry(
                    first.logical.clone(),
                    FailureKind::ResourceFailure,
                )),
            );
        }
    }
    world = step(
        &world,
        ControllerInput::Effect(EffectInput::FinalizeFailure(
            first.logical.clone(),
            FailureKind::ResourceFailure,
        )),
    );
    world = step(
        &world,
        ControllerInput::Request(Request::Compensate(CompensationRequest {
            sandbox: SandboxId(10),
            failure: FailureKind::ResourceFailure,
            evidence: Some(EffectId {
                logical: first.logical,
                attempt: 3,
            }),
        })),
    );
    world = execute(&world, SandboxId(10), |c| {
        matches!(c, Command::InstallFence { terminal: true, .. })
    });
    world = step(
        &world,
        ControllerInput::Publication(PublicationInput::Failed(
            SandboxId(10),
            FailureKind::ResourceFailure,
        )),
    );
    retire(&world, SandboxId(10));
}

#[test]
fn retired_activation_stop_coverage_roundtrips_through_resource_reclamation() {
    let (world, binding) = reserve(&enabled(), SandboxId(10));
    let world = prepared(&world, binding);
    let world = execute(&world, binding.sandbox, |c| {
        matches!(c, Command::ConfigureNetwork { .. })
    });
    let activation = effect(&world, binding.sandbox, |c| {
        matches!(c, Command::ActivateVm { .. })
    });
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
            sandbox: binding.sandbox,
            failure: FailureKind::DeadlineExceeded,
            evidence: None,
        })),
    );
    let world = execute(&world, binding.sandbox, |c| {
        matches!(c, Command::InstallFence { terminal: true, .. })
    });
    let world = step(
        &world,
        ControllerInput::Scheduler(SchedulerInput::StopVm(binding.sandbox, 1, binding.vm)),
    );
    let world = execute(&world, binding.sandbox, |c| {
        matches!(c, Command::StopVm { .. })
    });
    let stop = effect(&world, binding.sandbox, |c| {
        matches!(c, Command::StopVm { .. })
    });
    let world = step(
        &world,
        ControllerInput::Effect(EffectInput::RetireActivation {
            activation: activation.clone(),
            stop,
        }),
    );
    assert!(matches!(
        world.effect_status(&activation),
        Some(EffectStatus::RetiredByStop(_))
    ));
    let world = step(
        &world,
        ControllerInput::Publication(PublicationInput::Failed(
            binding.sandbox,
            FailureKind::DeadlineExceeded,
        )),
    );
    let world = cleanup(&world, binding);
    roundtrip(&world);
    retire(&world, binding.sandbox);
}

#[test]
fn window_clock_can_expire_results_before_explicit_result_pruning() {
    let world = retired();
    let advanced = handle_controller(
        &world,
        ControllerInput::Maintenance(MaintenanceInput::Window(WindowInput::AdvanceTime {
            now_ms: 100,
        })),
    )
    .unwrap()
    .next;
    assert_eq!(advanced.maintenance_state().unwrap().retained_count(), 2);
    assert_eq!(
        handle_controller(
            &advanced,
            ControllerInput::Request(Request::Query(QueryRequest {
                actor: Actor::Tenant(OWNER),
                sandbox: SandboxId(10),
                operation: OperationId(1),
            })),
        ),
        Err(ControllerReject::Request(request::Reject::ResultExpired)),
    );
    roundtrip(&advanced);
    let pruned = maintenance(&advanced, MaintenanceInput::ExpireResults { now_ms: 100 });
    assert_eq!(pruned.maintenance_state().unwrap().retained_count(), 0);
}
