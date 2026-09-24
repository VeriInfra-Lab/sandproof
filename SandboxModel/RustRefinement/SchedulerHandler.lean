import SandboxModel.RustRefinement.HandlerValues
import SandboxModel.RustRefinement.Clone
import SandboxModel.SchedulerCompleteness

/-! Refinement of the generated scheduler reducer.  This file establishes the
shared result relation and the guard-priority prefix used by every scheduler
operation.  Accepted state transformations are added below this common layer
without weakening the explicit finite-counter boundary. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance schedulerHandlerOperationKeyDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance schedulerHandlerLogicalEffectIdDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance schedulerHandlerEffectIdDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

abbrev SchedulerWorldRep :=
  @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
    effectIdDecidableEq valueRelations

def SchedulerResultRep (model : World)
    (rustResult : core.result.Result SandboxControllerRust.request.Decision
      SandboxControllerRust.scheduler.SchedulerReject)
    (modelResult : Except SandboxModel.Controller.SchedulerReject
      (SandboxModel.Controller.Decision model)) : Prop :=
  match rustResult, modelResult with
  | .Err rustReject, .error modelReject =>
      schedulerReject? rustReject = some modelReject
  | .Ok rustDecision, .ok modelDecision =>
      modelEvent rustDecision.event = modelDecision.event ∧
        SchedulerWorldRep rustDecision.next modelDecision.next
  | _, _ => False

theorem generated_handleScheduler_controllerDown
    (world : SandboxControllerRust.model.World)
    (input : SandboxControllerRust.scheduler.SchedulerInput)
    (down : world.controller_up = false) :
    SandboxControllerRust.scheduler.handle_scheduler world input =
      ok (core.result.Result.Err
        SandboxControllerRust.scheduler.SchedulerReject.ControllerDown) := by
  cases input <;>
    simp [SandboxControllerRust.scheduler.handle_scheduler,
      SandboxControllerRust.scheduler.handle_reserve_startup,
      SandboxControllerRust.scheduler.handle_reserve_pause_snapshot,
      SandboxControllerRust.scheduler.handle_dispatch_prepare,
      SandboxControllerRust.scheduler.handle_schedule_seal_snapshot,
      SandboxControllerRust.scheduler.handle_schedule_commit_snapshot,
      SandboxControllerRust.scheduler.handle_schedule_stop_vm,
      SandboxControllerRust.scheduler.handle_schedule_cleanup_vm,
      SandboxControllerRust.scheduler.handle_schedule_cleanup_slot, down]

theorem model_handleScheduler_controllerDown
    (world : World) (input : SandboxModel.Controller.SchedulerInput)
    (down : world.controllerUp = false) :
    SandboxModel.Controller.handleScheduler world input =
      .error .controllerDown := by
  cases input <;>
    simp [SandboxModel.Controller.handleScheduler,
      SandboxModel.Controller.handleReserveStartup,
      SandboxModel.Controller.handleReservePauseSnapshot,
      SandboxModel.Controller.handleDispatchPrepare,
      SandboxModel.Controller.handleScheduleSealSnapshot,
      SandboxModel.Controller.handleScheduleCommitSnapshot,
      SandboxModel.Controller.handleScheduleStopVm,
      SandboxModel.Controller.handleScheduleCleanupVm,
      SandboxModel.Controller.handleScheduleCleanupSlot, down]

/-- All eight generated scheduler operations agree on the first ordered
    rejection and leave no successor when the controller is unavailable. -/
theorem handleScheduler_controllerDown_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : SchedulerWorldRep rust model)
    (input : SandboxControllerRust.scheduler.SchedulerInput)
    (down : rust.controller_up = false) :
    SandboxControllerRust.scheduler.handle_scheduler rust input =
        ok (core.result.Result.Err
          SandboxControllerRust.scheduler.SchedulerReject.ControllerDown) ∧
      SandboxModel.Controller.handleScheduler model (schedulerInput input) =
        .error .controllerDown := by
  have modelDown : model.controllerUp = false := by
    rw [← represented.controllerUp]
    exact down
  exact ⟨generated_handleScheduler_controllerDown rust input down,
    model_handleScheduler_controllerDown model (schedulerInput input)
      modelDown⟩

end SandboxModel.RustRefinement
