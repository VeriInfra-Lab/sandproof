import SandboxModel.Local.State.Representation
import SandboxModel.Access.Guards
open Aeneas Aeneas.Std SandproofNetworkRust
namespace SandboxModel.Local.State

theorem option_eq_view [DecidableEq M] (eq : core.cmp.PartialEq T T)
    (view : T → M) (lawful : ∀ a b, eq.eq a b = .ok (decide (view a = view b)))
    (a b : Option T) :
    core.option.Option.Insts.CoreCmpPartialEqOption.eq eq a b =
      .ok (decide (a.map view = b.map view)) := by
  cases a <;> cases b <;> simp [core.option.Option.Insts.CoreCmpPartialEqOption.eq, lawful]

theorem file_policy_eq (a b : sandbox_controller_core.model.FilePolicyRef) :
    sandbox_controller_core.model.FilePolicyRef.Insts.CoreCmpPartialEqFilePolicyRef.eq a b =
      .ok (decide (model.FilePolicyRef.view a = model.FilePolicyRef.view b)) := by
  simp [sandbox_controller_core.model.FilePolicyRef.Insts.CoreCmpPartialEqFilePolicyRef.eq,
    sandbox_controller_core.ids.PolicyDigest.Insts.CoreCmpPartialEqPolicyDigest.eq,
    model.FilePolicyRef.view]

theorem restore_source_eq (a b : sandbox_controller_core.model.RestoreSource) :
    sandbox_controller_core.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource.eq a b =
      .ok (decide (model.RestoreSource.view a = model.RestoreSource.view b)) := by
  cases a with
  | mk sa ma =>
    cases b with
    | mk sb mb =>
      cases ma; cases mb
      simp [sandbox_controller_core.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource.eq,
        sandbox_controller_core.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq,
        sandbox_controller_core.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
        sandbox_controller_core.ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq,
        sandbox_controller_core.model.SnapshotManifestRef.mk.injEq, model.RestoreSource.view]
      split_ifs <;> simp_all

theorem network_endpoint_injective : Function.Injective SandboxModel.Network.endpointView := by
  intro a b h
  cases a with
  | mk aa ap aProtocol ax =>
    cases b with
    | mk ba bp bProtocol bx =>
      cases aProtocol <;> cases bProtocol <;>
        simp_all [SandboxModel.Network.endpointView, ← UScalar.eq_equiv]

theorem network_cidr_injective : Function.Injective SandboxModel.Network.rangeView := by
  intro a b h
  cases a; cases b
  simp_all [SandboxModel.Network.rangeView, ← UScalar.eq_equiv]

theorem vec_map_eq_iff (f : R → M) (inj : Function.Injective f)
    (a b : alloc.vec.Vec R) :
    a.val.map f = b.val.map f ↔ a = b :=
  ⟨fun h => alloc.vec.Vec.ext a b (list_map_injective f inj h),
    fun h => congrArg (fun xs : alloc.vec.Vec R => xs.val.map f) h⟩

theorem policy_views_same (a b : sandbox_controller_core.model.NetworkPolicy) :
    SandboxModel.Network.policyView a = SandboxModel.Network.policyView b ↔
      model.NetworkPolicy.view a = model.NetworkPolicy.view b := by
  cases a <;> cases b <;>
    simp [SandboxModel.Network.policyView, model.NetworkPolicy.view,
      vec_map_eq_iff SandboxModel.Network.endpointView network_endpoint_injective,
      vec_map_eq_iff SandboxModel.Network.rangeView network_cidr_injective]

theorem policy_eq (a b : sandbox_controller_core.model.NetworkPolicy) :
    sandbox_controller_core.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq a b =
      .ok (decide (model.NetworkPolicy.view a = model.NetworkPolicy.view b)) := by
  rw [SandboxModel.Access.policy_eq_view]
  simp only [policy_views_same]

end SandboxModel.Local.State
