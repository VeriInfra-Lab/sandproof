import SandboxModel.Local.Containers
import SandboxModel.Local.Creation.Model
import SandboxModel.Local.Runtime.CurrentRefinement

/-! Concrete complete-attempt dictionaries for creation's two work indexes.
Only logical vector headroom remains a precondition. Commit/enqueue and the
whole creation update still require their own full-state connection. -/
namespace SandboxModel.Local.Creation
open Aeneas Aeneas.Std SandproofNetworkRust SandboxModel.Foundations

local instance : DecidableEq sandbox_controller_core.model.EffectId := fun a b =>
  decidable_of_iff (SandboxModel.Local.State.model.EffectId.view a =
    SandboxModel.Local.State.model.EffectId.view b)
    SandboxModel.Local.State.model.EffectId.view_injective.eq_iff

theorem effect_equality_exact : ValueEqExact
    sandbox_controller_core.model.EffectId.Insts.CoreCmpPartialEqEffectId := by
  exact SandboxModel.Local.CurrentRuntime.effect_equality_lawful

theorem effect_insert_model (values : alloc.vec.Vec sandbox_controller_core.model.EffectId)
    (value : sandbox_controller_core.model.EffectId)
    (capacity : value ∈ values.val ∨ values.val.length < Usize.max) :
    sandbox_controller_core.transition.insert_unique
      sandbox_controller_core.model.EffectId.Insts.CoreCloneClone
      sandbox_controller_core.model.EffectId.Insts.CoreCmpPartialEqEffectId values value
      ⦃ result => result.val.map SandboxModel.Local.State.model.EffectId.view =
        insertUnique (SandboxModel.Local.State.model.EffectId.view value)
          (values.val.map SandboxModel.Local.State.model.EffectId.view) ⦄ := by
  exact SandboxModel.Local.Containers.insert_view SandboxModel.Local.State.model.EffectId.view
    SandboxModel.Local.State.model.EffectId.view_injective
    sandbox_controller_core.model.EffectId.Insts.CoreCloneClone values value
    (fun _ => SandboxModel.Access.model_EffectId_clone_exact)
    sandbox_controller_core.model.EffectId.Insts.CoreCmpPartialEqEffectId
    effect_equality_exact capacity

theorem initial_effect_insert_realizable (input : sandbox_controller_core.request.CreateRequest) :
    ∃ result, sandbox_controller_core.transition.insert_unique
      sandbox_controller_core.model.EffectId.Insts.CoreCloneClone
      sandbox_controller_core.model.EffectId.Insts.CoreCmpPartialEqEffectId
      (alloc.vec.Vec.new sandbox_controller_core.model.EffectId)
      ⟨⟨.InstallFence input.sandbox 1#u64 false⟩, 0#u64⟩ = .ok result ∧
      result.val = [⟨⟨.InstallFence input.sandbox 1#u64 false⟩, 0#u64⟩] := by
  exact SandboxModel.Local.Containers.insert_nonempty
    sandbox_controller_core.model.EffectId.Insts.CoreCloneClone
    SandboxModel.Access.model_EffectId_clone_exact
    sandbox_controller_core.model.EffectId.Insts.CoreCmpPartialEqEffectId effect_equality_exact _

end SandboxModel.Local.Creation
