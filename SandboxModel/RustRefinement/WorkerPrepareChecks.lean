import SandboxModel.RustRefinement.WorkerPrepareUpdate
import SandboxModel.RustRefinement.WorkerFilePolicy

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
theorem generated_workerUsableSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (record : SandboxControllerRust.model.SandboxRecord)
    (source : SandboxControllerRust.model.RestoreSource) :
    SandboxControllerRust.worker.usable_snapshot rust sid record source =
      ok (Controller.usableSnapshotCheck model (sandboxId sid) (sandboxRecord record) (restoreSource source)) := by
  exact generated_usableSnapshot_refines rust model rep sid record source

theorem worker_prepare_operation_some {α : Sort u} {value : Option Operation}
    {record : Operation} (found : value = some record)
    (missing : value = none → α) (present : (r : Operation) → value = some r → α) :
    Controller.handlePrepareVmCommand.match_3 (fun _ => α) value missing present = present record found := by cases found; rfl

theorem worker_prepare_operation_none {α : Sort u} {value : Option Operation}
    (found : value = none)
    (missing : value = none → α) (present : (r : Operation) → value = some r → α) :
    Controller.handlePrepareVmCommand.match_3 (fun _ => α) value missing present = missing found := by cases found; rfl

theorem worker_prepare_plan_some {α : Sort u} {value : Option OperationPlan}
    {record : OperationPlan} (found : value = some record)
    (missing : value = none → α) (present : (r : OperationPlan) → value = some r → α) :
    Controller.handlePrepareVmCommand.match_5 (fun _ => α) value missing present = present record found := by cases found; rfl

theorem worker_prepare_plan_none {α : Sort u} {value : Option OperationPlan}
    (found : value = none)
    (missing : value = none → α) (present : (r : OperationPlan) → value = some r → α) :
    Controller.handlePrepareVmCommand.match_5 (fun _ => α) value missing present = missing found := by cases found; rfl

theorem worker_prepare_fence_some {α : Sort u} {value : Option WorkerFence}
    {record : WorkerFence} (found : value = some record)
    (missing : value = none → α) (present : (r : WorkerFence) → value = some r → α) :
    Controller.handlePrepareVmCommand.match_7 (fun _ => α) value missing present = present record found := by cases found; rfl

theorem worker_prepare_fence_none {α : Sort u} {value : Option WorkerFence}
    (found : value = none)
    (missing : value = none → α) (present : (r : WorkerFence) → value = some r → α) :
    Controller.handlePrepareVmCommand.match_7 (fun _ => α) value missing present = missing found := by cases found; rfl

theorem worker_prepare_freshVm_some {α : Sort u} {value : Option Vm}
    {record : Vm} (found : value = some record)
    (present : (r : Vm) → value = some r → α) (missing : value = none → α) :
    Controller.handlePrepareVmCommand.match_9 (fun _ => α) value present missing = present record found := by cases found; rfl

theorem worker_prepare_freshVm_none {α : Sort u} {value : Option Vm}
    (found : value = none)
    (present : (r : Vm) → value = some r → α) (missing : value = none → α) :
    Controller.handlePrepareVmCommand.match_9 (fun _ => α) value present missing = missing found := by cases found; rfl

theorem worker_prepare_freshSlot_some {α : Sort u} {value : Option NetSlot}
    {record : NetSlot} (found : value = some record)
    (present : (r : NetSlot) → value = some r → α) (missing : value = none → α) :
    Controller.handlePrepareVmCommand.match_11 (fun _ => α) value present missing = present record found := by cases found; rfl

theorem worker_prepare_freshSlot_none {α : Sort u} {value : Option NetSlot}
    (found : value = none)
    (present : (r : NetSlot) → value = some r → α) (missing : value = none → α) :
    Controller.handlePrepareVmCommand.match_11 (fun _ => α) value present missing = missing found := by cases found; rfl

theorem worker_prepare_durable_some {α : Sort u} {value : Option RestoreSource}
    {record : RestoreSource} (found : value = some record)
    (missing : value = none → α) (present : (r : RestoreSource) → value = some r → α) :
    Controller.handlePrepareVmCommand.match_15 (fun _ => α) value missing present = present record found := by cases found; rfl

theorem worker_prepare_durable_none {α : Sort u} {value : Option RestoreSource}
    (found : value = none)
    (missing : value = none → α) (present : (r : RestoreSource) → value = some r → α) :
    Controller.handlePrepareVmCommand.match_15 (fun _ => α) value missing present = missing found := by cases found; rfl

end SandboxModel.RustRefinement
