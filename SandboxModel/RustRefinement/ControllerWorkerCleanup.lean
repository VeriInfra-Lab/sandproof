import SandboxModel.RustRefinement.ControllerWorkerFilePolicy
import SandboxModel.RustRefinement.WorkerCleanupVm
import SandboxModel.RustRefinement.WorkerCleanupSlot

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

theorem handleController_workerCleanupVm_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : U64)
    (vid : SandboxControllerRust.ids.VmId) (attempt : U64) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust
        (.Worker ⟨⟨.CleanupVm sid generation vid⟩, attempt⟩) = ok result ∧
      ControllerResultRep model result (UnifiedController.handle model valid
        (.worker (workerEnvelope model ⟨⟨.CleanupVm sid generation vid⟩, attempt⟩))) := by
  apply handleController_worker_lift rust model rep valid
  intro queued
  exact handleWorker_cleanupVm_refines rust model rep valid room sid generation vid attempt queued

theorem handleController_workerCleanupSlot_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : U64)
    (slot : SandboxControllerRust.ids.NetSlotId) (attempt : U64) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust
        (.Worker ⟨⟨.CleanupSlot sid generation slot⟩, attempt⟩) = ok result ∧
      ControllerResultRep model result (UnifiedController.handle model valid
        (.worker (workerEnvelope model ⟨⟨.CleanupSlot sid generation slot⟩, attempt⟩))) := by
  apply handleController_worker_lift rust model rep valid
  intro queued
  exact handleWorker_cleanupSlot_refines rust model rep valid room sid generation slot attempt queued

end SandboxModel.RustRefinement
