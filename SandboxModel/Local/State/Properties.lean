import SandboxModel.Local.State.Representation
import SandboxModel.Access.Initial

namespace SandboxModel.Local.State
open Aeneas Aeneas.Std SandproofNetworkRust

inductive InputGroup where
  | maintenance | request | effect | scheduler | worker
  | publication | rpcTimeout | access | runtime
  deriving DecidableEq

def inputGroup : controller.ControllerInput → InputGroup
  | .Maintenance _ => .maintenance | .Request _ => .request
  | .Effect _ => .effect | .Scheduler _ => .scheduler | .Worker _ => .worker
  | .Publication _ => .publication | .RpcTimeout _ => .rpcTimeout
  | .Access _ => .access | .Runtime _ => .runtime

def rustInputGroup : sandbox_controller_core.controller.ControllerInput → InputGroup
  | .Maintenance _ => .maintenance | .Request _ => .request
  | .Effect _ => .effect | .Scheduler _ => .scheduler | .Worker _ => .worker
  | .Publication _ => .publication | .RpcTimeout _ => .rpcTimeout
  | .Access _ => .access | .Runtime _ => .runtime

theorem input_group_preserved (input : sandbox_controller_core.controller.ControllerInput) :
    inputGroup (controller.ControllerInput.view input) = rustInputGroup input := by
  cases input <;> rfl

theorem input_root_exact (input : sandbox_controller_core.controller.ControllerInput) :
    SandproofNetworkRust.controller_input input = .ok input := by rfl

theorem input_rep_preserves_full_payload (a b : sandbox_controller_core.controller.ControllerInput) :
    controller.ControllerInput.view a = controller.ControllerInput.view b ↔ a = b :=
  ⟨fun h => controller.ControllerInput.view_injective h, fun h => congrArg _ h⟩

-- Constructor witnesses only, not a claim that all requests are admissible.
theorem every_group_has_actual_input (group : InputGroup) :
    ∃ input : sandbox_controller_core.controller.ControllerInput,
      inputGroup (controller.ControllerInput.view input) = group := by
  let zeroEffect : sandbox_controller_core.model.EffectId :=
    ⟨⟨.InstallFence 0#u64 0#u64 false⟩, 0#u64⟩
  cases group with
  | maintenance => exact ⟨.Maintenance (.ExpireResults 0#u64), rfl⟩
  | request => exact ⟨.Request (.Query ⟨.SystemController, 0#u64, 0#u64⟩), rfl⟩
  | effect => exact ⟨.Effect (.Timeout zeroEffect), rfl⟩
  | scheduler => exact ⟨.Scheduler (.ReserveStartup ⟨0#u64, 0#u64⟩), rfl⟩
  | worker => exact ⟨.Worker zeroEffect, rfl⟩
  | publication => exact ⟨.Publication (.Destroyed 0#u64), rfl⟩
  | rpcTimeout => exact ⟨.RpcTimeout 0#u64, rfl⟩
  | access => exact ⟨.Access (.Exec .SystemController 0#u64 0#u64), rfl⟩
  | runtime => exact ⟨.Runtime .Crash, rfl⟩

def initialModel : model.World := {
  maintenance := none, sandboxes := ⟨[]⟩, vms := ⟨[]⟩,
  filesystems := ⟨[]⟩, snapshots := ⟨[]⟩, net_slots := ⟨[]⟩,
  applied_file_policies := ⟨[]⟩, worker_fences := ⟨[]⟩,
  operations := ⟨[]⟩, operation_plans := ⟨[]⟩, resource_indexes := ⟨[]⟩,
  logical_effects := ⟨[]⟩, effect_keys := [], effects := ⟨[]⟩,
  next_vm_id := 0, next_slot_id := 0, next_snapshot_id := 0,
  next_manifest_digest := 0, inflight := [], controller_up := true
}

theorem initial_rep :
    ∃ actual, SandproofNetworkRust.initial_world = .ok actual ∧ WorldRep actual initialModel := by
  exact ⟨SandboxModel.Access.emptyWorld, SandboxModel.Access.initial_world_root_exact, rfl⟩

theorem clone_rep (actual : sandbox_controller_core.model.World) (abstract : model.World)
    (rep : WorldRep actual abstract) :
    ∃ next, SandproofNetworkRust.clone_world actual = .ok next ∧ WorldRep next abstract :=
  ⟨actual, SandboxModel.Access.clone_world_root_exact actual, rep⟩

end SandboxModel.Local.State
