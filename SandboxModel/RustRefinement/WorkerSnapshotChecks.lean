import SandboxModel.RustRefinement.WorkerPrepareChecks
import SandboxModel.RustRefinement.WorkerCleanupVm

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

theorem worker_snapshot_fresh_some {α : Sort u} {value : Option Snapshot}
    {record : Snapshot} (found : value = some record)
    (present : (r : Snapshot) → value = some r → α) (missing : value = none → α) :
    Controller.handleWorkerCommand.match_9 (fun _ => α) value present missing = present record found := by cases found; rfl

theorem worker_snapshot_fresh_none {α : Sort u} {value : Option Snapshot}
    (found : value = none)
    (present : (r : Snapshot) → value = some r → α) (missing : value = none → α) :
    Controller.handleWorkerCommand.match_9 (fun _ => α) value present missing = missing found := by cases found; rfl

theorem worker_snapshot_some {α : Sort u} {value : Option Snapshot}
    {record : Snapshot} (found : value = some record)
    (missing : value = none → α) (present : (r : Snapshot) → value = some r → α) :
    Controller.handleWorkerCommand.match_11 (fun _ => α) value missing present = present record found := by cases found; rfl

theorem worker_snapshot_none {α : Sort u} {value : Option Snapshot}
    (found : value = none)
    (missing : value = none → α) (present : (r : Snapshot) → value = some r → α) :
    Controller.handleWorkerCommand.match_11 (fun _ => α) value missing present = missing found := by cases found; rfl

theorem worker_vmId_some {α : Sort u} {value : Option VmId}
    {record : VmId} (found : value = some record)
    (missing : value = none → α) (present : (r : VmId) → value = some r → α) :
    Controller.handleWorkerCommand.match_13 (fun _ => α) value missing present = present record found := by cases found; rfl

theorem worker_vmId_none {α : Sort u} {value : Option VmId}
    (found : value = none)
    (missing : value = none → α) (present : (r : VmId) → value = some r → α) :
    Controller.handleWorkerCommand.match_13 (fun _ => α) value missing present = missing found := by cases found; rfl

@[simp] theorem worker_snapshot_immutable (value : SandboxControllerRust.model.Snapshot) :
    (snapshot value).immutable = value.immutable := rfl

theorem generated_workerStoppable_refines (status : SandboxControllerRust.model.VmStatus) :
    SandboxControllerRust.worker.stoppable_vm_status status = ok (Controller.stoppableVmStatusCheck (vmStatus status)) := by
  cases status <;> rfl

theorem worker_stoppable_iff (status : VmStatus) :
    stoppableVmStatus status ↔ Controller.stoppableVmStatusCheck status = true := by
  cases status <;> simp [stoppableVmStatus, Controller.stoppableVmStatusCheck]

end SandboxModel.RustRefinement
