import SandboxModel.RustRefinement.ControllerWorkerFencePrepareNetwork
import SandboxModel.RustRefinement.WorkerActivate
import SandboxModel.RustRefinement.WorkerFreeze
import SandboxModel.RustRefinement.WorkerSealSnapshot
import SandboxModel.RustRefinement.WorkerCommitSnapshot
import SandboxModel.RustRefinement.WorkerStop

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

/-- Per-invocation finite-machine bounds. Preparing enqueues three commands
and performs two index writes; committing a snapshot enqueues one stop.
Other work commands do not enqueue. These are sufficient, not minimal, bounds. -/
def WorkerCapacity (rust : SandboxControllerRust.model.World)
    (id : SandboxControllerRust.model.EffectId) : Prop :=
  match id.logical.command with
  | .PrepareVm .. => WorkerPrepareCapacity rust
  | .CommitSnapshot .. => EffectCapacity rust 1
  | _ => True

/-- All eleven command families, including ordered rejection reasons and the
complete successor world. Queue membership here is removed at the public
controller boundary by the already-proved actual membership check. -/
theorem handleWorker_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : WorkerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust) (id : SandboxControllerRust.model.EffectId)
    (capacity : WorkerCapacity rust id) (queued : model.inflight (effectId id)) :
    ∃ result, SandboxControllerRust.worker.handle_worker rust id = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId id, queued⟩) := by
  rcases id with ⟨⟨cmd⟩, attempt⟩
  cases cmd with
  | InstallFence sid epoch terminal =>
      exact handleWorker_fence_refines rust model rep valid room sid epoch terminal attempt queued
  | PrepareVm sid epoch generation vid slot source =>
      exact handleWorker_prepare_refines rust model rep valid room capacity sid epoch generation vid slot source attempt queued
  | ConfigureNetwork sid epoch generation vid slot policy =>
      exact handleWorker_network_refines rust model rep valid room sid epoch generation vid slot policy attempt queued
  | ApplyFilePolicy sid epoch generation workspace policy =>
      exact handleWorker_filePolicy_refines rust model rep valid room.appliedFilePolicies sid epoch generation workspace policy attempt queued
  | ActivateVm sid epoch generation vid =>
      exact handleWorker_activate_refines rust model rep valid room sid epoch generation vid attempt queued
  | FreezeVm sid epoch generation vid =>
      exact handleWorker_freeze_refines rust model rep valid room sid epoch generation vid attempt queued
  | SealSnapshot sid epoch generation vid workspace snap manifest =>
      exact handleWorker_sealSnapshot_refines rust model rep valid room sid epoch generation vid workspace snap manifest attempt queued
  | CommitSnapshot sid epoch generation snap manifest =>
      exact handleWorker_commitSnapshot_refines rust model rep valid room capacity sid epoch generation snap manifest attempt queued
  | StopVm sid epoch generation vid =>
      exact handleWorker_stop_refines rust model rep valid room sid epoch generation vid attempt queued
  | CleanupVm sid generation vid =>
      exact handleWorker_cleanupVm_refines rust model rep valid room sid generation vid attempt queued
  | CleanupSlot sid generation slot =>
      exact handleWorker_cleanupSlot_refines rust model rep valid room sid generation slot attempt queued

/-- No restriction on command kind, attempt number, queue membership or
awaiting status. Rejected messages are matched, not excluded by premises. -/
theorem handleController_worker_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust) (id : SandboxControllerRust.model.EffectId)
    (capacity : WorkerCapacity rust id) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Worker id) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.worker (workerEnvelope model id))) := by
  apply handleController_worker_lift rust model rep valid
  intro queued
  exact handleWorker_refines rust model rep valid room id capacity queued

end SandboxModel.RustRefinement
