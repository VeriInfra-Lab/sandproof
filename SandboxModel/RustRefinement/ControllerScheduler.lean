import SandboxModel.RustRefinement.SchedulerDispatchPrepare
import SandboxModel.RustRefinement.Controller

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

theorem handleController_scheduler_lift
    (rust : SandboxControllerRust.model.World) (model : World)
    (valid : ResourceIndexValid model)
    (input : SandboxControllerRust.scheduler.SchedulerInput)
    (component : ∃ result, SandboxControllerRust.scheduler.handle_scheduler rust input = ok result ∧
      SchedulerResultRep model result (Controller.handleScheduler model (schedulerInput input))) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Scheduler input) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.scheduler (schedulerInput input))) := by
  obtain ⟨result, evaluated, related⟩ := component
  cases result with
  | Err reason =>
    cases abstractEval : Controller.handleScheduler model (schedulerInput input) with
    | error abstractReason =>
      have same : schedulerReject? reason = some abstractReason := by
        simpa [SchedulerResultRep, abstractEval] using related
      refine ⟨.Err (.Scheduler reason), ?_, ?_⟩
      · simp [SandboxControllerRust.controller.handle_controller, evaluated]
      · simp [ControllerResultRep, controllerReject?, UnifiedController.handle,
          UnifiedController.liftController, abstractEval, same]
    | ok decision => simp [SchedulerResultRep, abstractEval] at related
  | Ok decision =>
    cases abstractEval : Controller.handleScheduler model (schedulerInput input) with
    | error reason => simp [SchedulerResultRep, abstractEval] at related
    | ok abstractDecision =>
      have same : modelEvent decision.event = abstractDecision.event ∧
          ControllerWorldRep decision.next abstractDecision.next := by
        simpa [SchedulerResultRep, abstractEval] using related
      refine ⟨.Ok { event := .Protocol decision.event, next := decision.next }, ?_, ?_⟩
      · simp [SandboxControllerRust.controller.handle_controller, evaluated,
          SandboxControllerRust.controller.protocol_decision]
      · simpa [ControllerResultRep, controllerEvent, UnifiedController.handle,
          UnifiedController.liftController, abstractEval, UnifiedController.fromControllerDecision] using same

theorem handleController_dispatchPrepare_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (capacity : EffectCapacity rust 1) (sid : SandboxControllerRust.ids.SandboxId) (vid : SandboxControllerRust.ids.VmId) (slot : SandboxControllerRust.ids.NetSlotId) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Scheduler (.DispatchPrepare sid vid slot)) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.scheduler (schedulerInput (.DispatchPrepare sid vid slot)))) := by
  apply handleController_scheduler_lift
  exact handleDispatchPrepare_refines rust model rep capacity sid vid slot

theorem handleController_sealSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (capacity : EffectCapacity rust 1) (sid : SandboxControllerRust.ids.SandboxId) (snap : SandboxControllerRust.ids.SnapshotId) (manifest : SandboxControllerRust.model.SnapshotManifestRef) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Scheduler (.SealSnapshot sid snap manifest)) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.scheduler (schedulerInput (.SealSnapshot sid snap manifest)))) := by
  apply handleController_scheduler_lift
  exact handleScheduleSealSnapshot_refines rust model rep capacity sid snap manifest

theorem handleController_commitSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (capacity : EffectCapacity rust 1) (sid : SandboxControllerRust.ids.SandboxId) (snap : SandboxControllerRust.ids.SnapshotId) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Scheduler (.CommitSnapshot sid snap)) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.scheduler (schedulerInput (.CommitSnapshot sid snap)))) := by
  apply handleController_scheduler_lift
  exact handleScheduleCommitSnapshot_refines rust model rep capacity sid snap

theorem handleController_stopVm_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (capacity : EffectCapacity rust 1) (sid : SandboxControllerRust.ids.SandboxId) (generation : U64) (vid : SandboxControllerRust.ids.VmId) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Scheduler (.StopVm sid generation vid)) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.scheduler (schedulerInput (.StopVm sid generation vid)))) := by
  apply handleController_scheduler_lift
  exact handleScheduleStopVm_refines rust model rep capacity sid generation vid

theorem handleController_cleanupVm_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (capacity : EffectCapacity rust 1) (sid : SandboxControllerRust.ids.SandboxId) (generation : U64) (vid : SandboxControllerRust.ids.VmId) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Scheduler (.CleanupVm sid generation vid)) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.scheduler (schedulerInput (.CleanupVm sid generation vid)))) := by
  apply handleController_scheduler_lift
  exact handleScheduleCleanupVm_refines rust model rep capacity sid generation vid

theorem handleController_cleanupSlot_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (capacity : EffectCapacity rust 1) (sid : SandboxControllerRust.ids.SandboxId) (generation : U64) (slot : SandboxControllerRust.ids.NetSlotId) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Scheduler (.CleanupSlot sid generation slot)) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.scheduler (schedulerInput (.CleanupSlot sid generation slot)))) := by
  apply handleController_scheduler_lift
  exact handleScheduleCleanupSlot_refines rust model rep capacity sid generation slot

end SandboxModel.RustRefinement
