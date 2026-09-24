import SandboxModel.RustRefinement.ControllerScheduler
import SandboxModel.RustRefinement.SchedulerReserveStartup

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

/-- Input-specific capacity beyond the shared store and cursor bounds.
Reservation does not enqueue; the six other inputs can enqueue one command. -/
def SchedulerCapacity (rust : SandboxControllerRust.model.World) :
    SandboxControllerRust.scheduler.SchedulerInput → Prop
  | .ReserveStartup key => StartupPlanHeadroom rust key
  | .ReservePauseSnapshot _ => True
  | _ => EffectCapacity rust 1

/-- All eight real scheduler entry points, including ordered rejections and
complete successor worlds, within the explicitly stated finite capacity. -/
theorem handleScheduler_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : SchedulerWorldRep rust model) (room : HandlerMachineHeadroom rust)
    (input : SandboxControllerRust.scheduler.SchedulerInput)
    (capacity : SchedulerCapacity rust input) :
    ∃ result, SandboxControllerRust.scheduler.handle_scheduler rust input = ok result ∧
      SchedulerResultRep model result (Controller.handleScheduler model (schedulerInput input)) := by
  cases input with
  | ReserveStartup key =>
    exact handleReserveStartup_refines rust model rep room.cursors.vm room.cursors.slot room.operationPlans key capacity
  | ReservePauseSnapshot key =>
    exact handleReservePauseSnapshot_refines rust model rep room.cursors.snapshot room.cursors.manifest room.operationPlans key
  | DispatchPrepare sid vid slot =>
    exact handleDispatchPrepare_refines rust model rep capacity sid vid slot
  | SealSnapshot sid snap manifest =>
    exact handleScheduleSealSnapshot_refines rust model rep capacity sid snap manifest
  | CommitSnapshot sid snap =>
    exact handleScheduleCommitSnapshot_refines rust model rep capacity sid snap
  | StopVm sid generation vid =>
    exact handleScheduleStopVm_refines rust model rep capacity sid generation vid
  | CleanupVm sid generation vid =>
    exact handleScheduleCleanupVm_refines rust model rep capacity sid generation vid
  | CleanupSlot sid generation slot =>
    exact handleScheduleCleanupSlot_refines rust model rep capacity sid generation slot

theorem handleController_scheduler_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust) (input : SandboxControllerRust.scheduler.SchedulerInput)
    (capacity : SchedulerCapacity rust input) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Scheduler input) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.scheduler (schedulerInput input))) := by
  apply handleController_scheduler_lift
  exact handleScheduler_refines rust model rep room input capacity

end SandboxModel.RustRefinement
