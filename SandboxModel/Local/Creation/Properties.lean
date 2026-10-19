import SandboxModel.Local.Creation.Model
import SandboxModel.Foundations.TableWrites
namespace SandboxModel.Local.Creation
open SandboxModel.Local.State SandboxModel.Foundations

theorem check_accepts_iff (world : model.World) (input : request.CreateRequest) :
    check world input = .ok () ↔ Admitted world input := by
  simp [check, Admitted]
  aesop

theorem rejects_iff (world : model.World) (input : request.CreateRequest) (error : Reject) :
    transition world input = .error error ↔ check world input = .error error := by
  cases outcome : check world input <;> simp [transition, outcome]

theorem accepts_iff (world : model.World) (input : request.CreateRequest) :
    (∃ decision, transition world input = .ok decision) ↔ Admitted world input := by
  rw [← check_accepts_iff]
  cases outcome : check world input <;> simp [transition, outcome]

theorem accepted_exact (world : model.World) (input : request.CreateRequest) (decision : Decision)
    (accepted : transition world input = .ok decision) :
    decision = ⟨input, beginCreate world input⟩ := by
  cases outcome : check world input <;> simp [transition, outcome] at accepted
  exact accepted.symm

theorem begin_sandbox (world : model.World) (input : request.CreateRequest) :
    (beginCreate world input).sandboxes.entries =
      tablePut input.sandbox (sandboxRecord input) world.sandboxes.entries := by
  dsimp only [beginCreate, commitFence, recordIdentity]
  split <;> rfl

theorem create_closed (world : model.World) (input : request.CreateRequest) :
    tableLookup input.sandbox (beginCreate world input).sandboxes.entries =
      some (sandboxRecord input) ∧
    (sandboxRecord input).desired = .Closed ∧
    (sandboxRecord input).effective = .Closed ∧
    (sandboxRecord input).current_vm = none := by
  rw [begin_sandbox, table_lookup_put]
  simp [sandboxRecord]

def frame (world : model.World) :=
  (world.vms, world.snapshots, world.net_slots, world.applied_file_policies,
    world.worker_fences, world.resource_indexes, world.next_vm_id,
    world.next_slot_id, world.next_snapshot_id, world.next_manifest_digest, world.controller_up)

theorem begin_frame (world : model.World) (input : request.CreateRequest) :
    frame (beginCreate world input) = frame world := by
  dsimp only [frame, beginCreate, commitFence, recordIdentity]
  split <;> rfl

def maintenanceFrame (world : model.World) :=
  world.maintenance.map (fun state =>
    {state with sandbox_frontier := none, workspace_frontier := none})

theorem begin_maintenance_frame (world : model.World) (input : request.CreateRequest) :
    maintenanceFrame (beginCreate world input) = maintenanceFrame world := by
  dsimp only [maintenanceFrame, beginCreate, commitFence, recordIdentity]
  split <;> cases world.maintenance <;> rfl

theorem begin_filesystem (world : model.World) (input : request.CreateRequest) :
    (beginCreate world input).filesystems.entries =
      tablePut input.workspace ⟨input.owner, input.sandbox, 1⟩ world.filesystems.entries := by
  dsimp only [beginCreate, commitFence, recordIdentity]
  split <;> rfl

theorem begin_ledger (world : model.World) (input : request.CreateRequest) :
    (beginCreate world input).operations.entries =
      tablePut (operationKey input) (ledger input) world.operations.entries := by
  dsimp only [beginCreate, commitFence, recordIdentity]
  split <;> rfl

theorem begin_plan (world : model.World) (input : request.CreateRequest) :
    (beginCreate world input).operation_plans.entries =
      tablePut (operationKey input) (plan world input) world.operation_plans.entries := by
  dsimp only [beginCreate, commitFence, recordIdentity]
  split <;> rfl

end SandboxModel.Local.Creation
