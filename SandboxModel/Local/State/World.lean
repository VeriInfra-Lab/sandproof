import SandboxModel.Local.State.Values

namespace SandboxModel.Local.State
open Aeneas Aeneas.Std

def model.World.view (value : SandproofNetworkRust.sandbox_controller_core.model.World) : model.World := {
  maintenance := ValueView.view value.maintenance,
  sandboxes := ValueView.view value.sandboxes,
  vms := ValueView.view value.vms,
  filesystems := ValueView.view value.filesystems,
  snapshots := ValueView.view value.snapshots,
  net_slots := ValueView.view value.net_slots,
  applied_file_policies := ValueView.view value.applied_file_policies,
  worker_fences := ValueView.view value.worker_fences,
  operations := ValueView.view value.operations,
  operation_plans := ValueView.view value.operation_plans,
  resource_indexes := ValueView.view value.resource_indexes,
  logical_effects := ValueView.view value.logical_effects,
  effect_keys := ValueView.view value.effect_keys,
  effects := ValueView.view value.effects,
  next_vm_id := ValueView.view value.next_vm_id,
  next_slot_id := ValueView.view value.next_slot_id,
  next_snapshot_id := ValueView.view value.next_snapshot_id,
  next_manifest_digest := ValueView.view value.next_manifest_digest,
  inflight := ValueView.view value.inflight,
  controller_up := ValueView.view value.controller_up
}

theorem model.World.view_injective : Function.Injective model.World.view := by
  intro a b h
  cases a; cases b; simp_all [model.World.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.World model.World :=
  ⟨model.World.view, model.World.view_injective⟩

def auxiliary.AuxiliaryDecision.view (value : SandproofNetworkRust.sandbox_controller_core.auxiliary.AuxiliaryDecision) : auxiliary.AuxiliaryDecision := {
  event := ValueView.view value.event,
  next := ValueView.view value.next
}

theorem auxiliary.AuxiliaryDecision.view_injective : Function.Injective auxiliary.AuxiliaryDecision.view := by
  intro a b h
  cases a; cases b; simp_all [auxiliary.AuxiliaryDecision.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.auxiliary.AuxiliaryDecision auxiliary.AuxiliaryDecision :=
  ⟨auxiliary.AuxiliaryDecision.view, auxiliary.AuxiliaryDecision.view_injective⟩

/-- Exact whole-state relation, including maintenance and ordered tables. -/
def WorldRep (actual : SandproofNetworkRust.sandbox_controller_core.model.World)
    (abstract : model.World) : Prop := model.World.view actual = abstract

/-- Two actual states cannot collapse to one abstract state. -/
theorem world_rep_separates {a b : SandproofNetworkRust.sandbox_controller_core.model.World}
    {abstract : model.World} (ha : WorldRep a abstract) (hb : WorldRep b abstract) : a = b :=
  model.World.view_injective (ha.trans hb.symm)

theorem world_rep_total (actual : SandproofNetworkRust.sandbox_controller_core.model.World) :
    ∃ abstract, WorldRep actual abstract := ⟨model.World.view actual, rfl⟩

theorem world_view_set_up (actual : SandproofNetworkRust.sandbox_controller_core.model.World)
    (up : Bool) : model.World.view {actual with controller_up := up} =
      {(model.World.view actual) with controller_up := up} := by rfl

end SandboxModel.Local.State
