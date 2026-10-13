import SandboxModel.Access.Cloning
import SandboxModel.Access.Representation

namespace SandboxModel.Access
open Aeneas.Std SandproofNetworkRust

def emptyStore : sandbox_controller_core.store.Store K V := ⟨alloc.vec.Vec.new _⟩

def emptyWorld : sandbox_controller_core.model.World := {
  maintenance := none, sandboxes := emptyStore, vms := emptyStore,
  filesystems := emptyStore, snapshots := emptyStore, net_slots := emptyStore,
  applied_file_policies := emptyStore, worker_fences := emptyStore,
  operations := emptyStore, operation_plans := emptyStore, resource_indexes := emptyStore,
  logical_effects := emptyStore, effect_keys := alloc.vec.Vec.new _, effects := emptyStore,
  next_vm_id := 0#u64, next_slot_id := 0#u64, next_snapshot_id := 0#u64,
  next_manifest_digest := 0#u64, inflight := alloc.vec.Vec.new _, controller_up := true
}

theorem initial_world_root_exact : SandproofNetworkRust.initial_world = .ok emptyWorld := by
  unfold SandproofNetworkRust.initial_world sandbox_controller_core.model.initial_world
  simp [sandbox_controller_core.model.World.Insts.CoreDefaultDefault.default,
    sandbox_controller_core.store.Store.Insts.CoreDefaultDefault.default, emptyStore, emptyWorld]

theorem initial_records_empty : recordsView emptyWorld = [] := by
  rfl

theorem clone_world_root_exact (world : sandbox_controller_core.model.World) :
    SandproofNetworkRust.clone_world world = .ok world :=
  model_World_clone_exact world

end SandboxModel.Access
