import SandboxModel.RustRefinement.ControllerWorkerCleanup
import SandboxModel.RustRefinement.WorkerFence
import SandboxModel.RustRefinement.WorkerPrepare
import SandboxModel.RustRefinement.WorkerNetwork

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

theorem handleController_workerFence_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch : U64) (terminal : Bool) (attempt : U64) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust
        (.Worker ⟨⟨.InstallFence sid epoch terminal⟩, attempt⟩) = ok result ∧
      ControllerResultRep model result (UnifiedController.handle model valid
        (.worker (workerEnvelope model ⟨⟨.InstallFence sid epoch terminal⟩, attempt⟩))) := by
  apply handleController_worker_lift rust model rep valid
  intro queued
  exact handleWorker_fence_refines rust model rep valid room sid epoch terminal attempt queued

theorem handleController_workerPrepare_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust) (capacity : WorkerPrepareCapacity rust)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch generation : U64) (vid : SandboxControllerRust.ids.VmId) (slotId : SandboxControllerRust.ids.NetSlotId)
    (source : Option SandboxControllerRust.model.RestoreSource) (attempt : U64) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust
        (.Worker ⟨⟨.PrepareVm sid epoch generation vid slotId source⟩, attempt⟩) = ok result ∧
      ControllerResultRep model result (UnifiedController.handle model valid
        (.worker (workerEnvelope model ⟨⟨.PrepareVm sid epoch generation vid slotId source⟩, attempt⟩))) := by
  apply handleController_worker_lift rust model rep valid
  intro queued
  exact handleWorker_prepare_refines rust model rep valid room capacity sid epoch generation vid slotId source attempt queued

theorem handleController_workerNetwork_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch generation : U64) (vid : SandboxControllerRust.ids.VmId) (slotId : SandboxControllerRust.ids.NetSlotId)
    (policy : SandboxControllerRust.model.NetworkPolicy) (attempt : U64) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust
        (.Worker ⟨⟨.ConfigureNetwork sid epoch generation vid slotId policy⟩, attempt⟩) = ok result ∧
      ControllerResultRep model result (UnifiedController.handle model valid
        (.worker (workerEnvelope model ⟨⟨.ConfigureNetwork sid epoch generation vid slotId policy⟩, attempt⟩))) := by
  apply handleController_worker_lift rust model rep valid
  intro queued
  exact handleWorker_network_refines rust model rep valid room sid epoch generation vid slotId policy attempt queued

end SandboxModel.RustRefinement
