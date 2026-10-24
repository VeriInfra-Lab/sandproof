//! Actual original reducer histories; no physical receipt is inferred by time.
use super::*;
use crate::maintenance::{
    MaintenanceInput, MaintenanceLimits,
    slots::{HostBinding, HostSlotId, HostSlotPhase, SlotInput},
    windows::WindowLimits,
};
use crate::{Actor, CreateRequest, EffectId, EffectOutcome, FailureKind, FilePolicyRef, FsId,
    NetworkPolicy, OperationId, PolicyDigest, Request, TenantId};

const SID: SandboxId = SandboxId(1);
const POSITION: HostSlotId = HostSlotId(0);
const FAILURE: FailureKind = FailureKind::DeadlineExceeded;
fn step(world: &World, input: ControllerInput) -> World {
    crate::handle_controller(world, input).unwrap().next
}
fn effect(world: &World, matches: impl Fn(&Command) -> bool) -> EffectId {
    world.effect_ids().iter().find(|id| matches(id.command())).unwrap().clone()
}
fn observe_success(world: &World, id: &EffectId) -> World {
    let world = step(world, ControllerInput::Worker(id.clone()));
    step(&world, ControllerInput::Effect(EffectInput::Receipt(
        id.clone(), EffectOutcome::Succeeded(crate::ResultDigest(1)),
    )))
}
fn finish(world: &World, id: &EffectId) -> World {
    let world = step(world, ControllerInput::Effect(EffectInput::Dispatch(id.clone())));
    observe_success(&world, id)
}
fn bound_startup() -> (World, HostBinding) {
    let world = step(&crate::initial_world(), ControllerInput::Maintenance(MaintenanceInput::Enable {
        limits: MaintenanceLimits { max_retained_results: 4, max_host_slots: 1,
            windows: WindowLimits::new(1, 100).unwrap() }, now_ms: 0,
    }));
    let world = step(&world, ControllerInput::Request(Request::Create(CreateRequest {
        actor: Actor::Tenant(TenantId(1)), owner: TenantId(1), sandbox: SID, workspace: FsId(1),
        file_policy: FilePolicyRef { digest: PolicyDigest(1) }, policy: NetworkPolicy::DenyAll,
        operation: OperationId(1),
    })));
    let world = step(&world, ControllerInput::Scheduler(SchedulerInput::ReserveStartup(OperationKey {
        sandbox: SID, operation: OperationId(1),
    })));
    let plan = world.operation_plan(OperationKey { sandbox: SID, operation: OperationId(1) }).unwrap();
    let binding = HostBinding { owner: TenantId(1), sandbox: SID, generation: plan.generation,
        vm: plan.vm.unwrap(), network: plan.slot.unwrap() };
    let world = step(&world, ControllerInput::Maintenance(MaintenanceInput::Slot(SlotInput::Bind {
        position: POSITION, expected_previous: None, binding,
    })));
    (world, binding)
}
fn unknown_prepare() -> (World, HostBinding, EffectId) {
    let (world, binding) = bound_startup();
    let opening = effect(&world, |c| matches!(c, Command::InstallFence { terminal: false, .. }));
    let world = finish(&world, &opening);
    let world = step(&world, ControllerInput::Scheduler(SchedulerInput::DispatchPrepare(
        SID, binding.vm, binding.network,
    )));
    let prepare = effect(&world, |c| matches!(c, Command::PrepareVm { .. }));
    let world = step(&world, ControllerInput::Effect(EffectInput::Dispatch(prepare.clone())));
    (step(&world, ControllerInput::Effect(EffectInput::Timeout(prepare.clone()))), binding, prepare)
}
struct AbortStages {
    no_terminal_fence: World,
    waiting_for_abort: World,
    ready: World,
    prepare: EffectId,
    abort: EffectId,
}
fn abort_stages() -> AbortStages {
    let (world, binding, prepare) = unknown_prepare();
    let no_terminal_fence = step(&world, ControllerInput::Request(Request::Compensate(
        crate::CompensationRequest { sandbox: SID, failure: FAILURE, evidence: None },
    )));
    let terminal = effect(&no_terminal_fence, |c| matches!(c, Command::InstallFence { terminal: true, .. }));
    let world = finish(&no_terminal_fence, &terminal);
    let world = step(&world, ControllerInput::Scheduler(SchedulerInput::AbortPrepare(
        SID, binding.generation, binding.vm, binding.network,
    )));
    let abort = effect(&world, |c| matches!(c, Command::AbortPrepare { .. }));
    let waiting_for_abort = step(&world, ControllerInput::Effect(EffectInput::Dispatch(abort.clone())));
    let ready = observe_success(&waiting_for_abort, &abort);
    AbortStages { no_terminal_fence, waiting_for_abort, ready, prepare, abort }
}

#[test]
fn exact_abort_allows_failed_publication_but_keeps_unknown_prepare_and_occupied_slot() {
    let stages = abort_stages();
    assert!(crate::publication::handle_publish_failure(&stages.ready, SID, FAILURE).is_ok());
    let next = next_step(&stages.ready, SID);
    assert_eq!(next, apply(ControllerInput::Publication(PublicationInput::Failed(SID, FAILURE))));
    let WorkflowStep::Apply(input) = next else { panic!("expected original publication") };
    let published = step(&stages.ready, *input);
    let row = published.sandbox_record(SID).unwrap();
    assert_eq!(row.phase, Phase::Destroyed);
    assert_eq!(row.current_operation.unwrap().status, OperationStatus::Failed(FAILURE));
    assert_eq!(published.effect_status(&stages.prepare), Some(EffectStatus::OutcomeUnknown));
    assert_eq!(published.maintenance_state().unwrap().host_slot(POSITION).unwrap().phase, HostSlotPhase::Occupied);
    assert_eq!(crate::effect::handle_effect_dispatch(&published, stages.prepare).unwrap_err(), crate::EffectReject::NotDispatchable);
}

#[test]
fn missing_terminal_fence_cannot_publish_failure() {
    let stages = abort_stages();
    assert_eq!(crate::publication::handle_publish_failure(&stages.no_terminal_fence, SID, FAILURE).unwrap_err(), crate::PublicationReject::FenceNotInstalled);
    assert_eq!(next_step(&stages.no_terminal_fence, SID), WorkflowStep::Claim);
}

#[test]
fn another_dispatched_or_unknown_abort_still_blocks_failed_publication() {
    for unknown in [false, true] {
        let stages = abort_stages();
        let world = if unknown {
            step(&stages.waiting_for_abort, ControllerInput::Effect(EffectInput::Timeout(stages.abort.clone())))
        } else { stages.waiting_for_abort };
        assert!(crate::publication::handle_publish_failure(&world, SID, FAILURE).is_err());
        assert_eq!(next_step(&world, SID), WorkflowStep::Blocked);
        assert_eq!(world.effect_status(&stages.prepare), Some(EffectStatus::OutcomeUnknown));
        assert_eq!(world.maintenance_state().unwrap().host_slot(POSITION).unwrap().phase, HostSlotPhase::Occupied);
    }
}
