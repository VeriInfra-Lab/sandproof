import SandboxModel.Access.Initial
import SandboxModel.Access.Guards
import SandboxModel.Access.Lookup

namespace AccessWitness
open Aeneas Aeneas.Std SandproofNetworkRust
open SandboxModel.Access

/-- A deliberately Closed effective authority, with all outer execution
    identity/phase fields plausible. This is a guard witness, not a claim that
    every field combination is reachable from the controller invariant. -/
def record : sandbox_controller_core.model.SandboxRecord := {
  owner := 1#u64, phase := .Running, terminal := false, epoch := 0#u64,
  generation := 1#u64, current_vm := some 3#u64, workspace := 4#u64,
  workspace_access := .ManagedReadWrite, file_policy := ⟨0#u64⟩,
  durable_snapshot := none, restore_from := none, net_slot := none,
  policy := .DenyAll, desired := .Closed, effective := .Closed,
  current_operation := none
}

def world : sandbox_controller_core.model.World := {
  emptyWorld with sandboxes := ⟨alloc.vec.Vec.from [(2#u64, record)] (by scalar_tac)⟩
}

theorem lookup_record :
    SandboxModel.Foundations.tableLookup 2#u64 world.sandboxes.entries.val = some record := by
  rfl

end AccessWitness
