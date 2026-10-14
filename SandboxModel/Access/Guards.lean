import SandboxModel.Access.Representation
import SandboxModel.Network.Static

namespace SandboxModel.Access
open Aeneas Aeneas.Std SandproofNetworkRust

theorem list_allM_view_eq [DecidableEq M] (eq : core.cmp.PartialEq T T)
    (view : T → M) (lawful : ∀ left right,
      eq.eq left right = .ok (decide (view left = view right))) :
    ∀ left right : List T, left.length = right.length →
      List.allM (fun pair => eq.eq pair.1 pair.2) (left.zip right) =
        .ok (decide (left.map view = right.map view)) := by
  intro left
  induction left with
  | nil =>
      intro right equalLength
      cases right with
      | nil => rfl
      | cons _ _ => simp at equalLength
  | cons head tail ih =>
      intro right equalLength
      cases right with
      | nil => simp at equalLength
      | cons other rest =>
          have tailLength : tail.length = rest.length := by simpa using equalLength
          simp only [List.zip_cons_cons, List.allM_cons]
          rw [lawful head other]
          by_cases same : view head = view other
          · simpa [same, Aeneas.Std.instMonadResult] using ih rest tailLength
          · simp [same]
            rfl

theorem vector_eq_view [DecidableEq M] (eq : core.cmp.PartialEq T T)
    (view : T → M) (lawful : ∀ left right,
      eq.eq left right = .ok (decide (view left = view right)))
    (left right : alloc.vec.Vec T) :
    alloc.vec.partial_eq.PartialEqVec.eq eq left right =
      .ok (decide (left.val.map view = right.val.map view)) := by
  unfold alloc.vec.partial_eq.PartialEqVec.eq
  by_cases equalLength : left.length = right.length
  · simp only [equalLength, if_pos]
    exact list_allM_view_eq eq view lawful left.val right.val (by simpa using equalLength)
  · have different : left.val.map view ≠ right.val.map view := by
      intro same
      apply equalLength
      simpa using congrArg List.length same
    simp [equalLength, different]

theorem cidr_eq_view (left right : sandbox_controller_core.model.Ipv4Cidr) :
    sandbox_controller_core.model.Ipv4Cidr.Insts.CoreCmpPartialEqIpv4Cidr.eq left right =
      .ok (decide (SandboxModel.Network.rangeView left = SandboxModel.Network.rangeView right)) := by
  unfold sandbox_controller_core.model.Ipv4Cidr.Insts.CoreCmpPartialEqIpv4Cidr.eq
  by_cases same : left.network = right.network <;>
    simp [same, SandboxModel.Network.rangeView, SandboxModel.Network.Cidr.mk.injEq,
      ← UScalar.eq_equiv]

theorem policy_eq_view (left right : sandbox_controller_core.model.NetworkPolicy) :
    sandbox_controller_core.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq left right =
      .ok (decide (SandboxModel.Network.policyView left = SandboxModel.Network.policyView right)) := by
  cases left <;> cases right <;>
    simp [sandbox_controller_core.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
      sandbox_controller_core.model.NetworkPolicy.read_discriminant,
      vector_eq_view sandbox_controller_core.model.Endpoint.Insts.CoreCmpPartialEqEndpoint
        SandboxModel.Network.endpointView SandboxModel.Network.endpoint_eq_refines,
      vector_eq_view sandbox_controller_core.model.Ipv4Cidr.Insts.CoreCmpPartialEqIpv4Cidr
        SandboxModel.Network.rangeView cidr_eq_view,
      core.cmp.impls.PartialEqBool.eq, SandboxModel.Network.policyView]
  all_goals split_ifs <;> simp_all

theorem authorized_refines (actor : sandbox_controller_core.model.Actor)
    (owner : sandbox_controller_core.ids.TenantId) :
    sandbox_controller_core.transition.authorized actor owner =
      .ok (authorized (actorView actor) owner.val) := by
  cases actor with
  | Tenant tenant =>
    by_cases same : tenant = owner
    · subst owner
      simp [sandbox_controller_core.transition.authorized,
        sandbox_controller_core.model.Actor.Insts.CoreCmpPartialEqActor.eq,
        sandbox_controller_core.model.Actor.read_discriminant,
        sandbox_controller_core.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
        authorized, actorView]
    · have different : tenant.val ≠ owner.val := fun h => same (UScalar.eq_of_val_eq h)
      simp [same, different, sandbox_controller_core.transition.authorized,
        sandbox_controller_core.model.Actor.Insts.CoreCmpPartialEqActor.eq,
        sandbox_controller_core.model.Actor.read_discriminant,
        sandbox_controller_core.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
        authorized, actorView]
  | SystemController =>
    simp [sandbox_controller_core.transition.authorized,
      sandbox_controller_core.model.Actor.Insts.CoreCmpPartialEqActor.eq,
      sandbox_controller_core.model.Actor.read_discriminant,
      authorized, actorView]

theorem network_guard_refines (record : sandbox_controller_core.model.SandboxRecord)
    (sandbox : sandbox_controller_core.ids.SandboxId) (vm : sandbox_controller_core.ids.VmId)
    (slot : sandbox_controller_core.ids.NetSlotId) :
    sandbox_controller_core.auxiliary.allows_network record sandbox vm slot =
      .ok (networkAllowed (recordView record) sandbox.val vm.val slot.val) := by
  cases authority : record.effective with
  | Closed => simp [sandbox_controller_core.auxiliary.allows_network, authority,
      networkAllowed, recordView, authorityView]
  | Paused _ _ => simp [sandbox_controller_core.auxiliary.allows_network, authority,
      networkAllowed, recordView, authorityView]
  | Running sid generation machine workspace access position policy =>
      simp only [sandbox_controller_core.auxiliary.allows_network, authority,
        sandbox_controller_core.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
        sandbox_controller_core.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
        sandbox_controller_core.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId.eq]
      by_cases a : sid = sandbox <;> by_cases b : generation = record.generation <;>
        by_cases c : machine = vm <;> by_cases d : position = slot <;>
          simp [a, b, c, d, policy_eq_view, networkAllowed, recordView, authorityView,
            authority, ← UScalar.eq_equiv]

end SandboxModel.Access
