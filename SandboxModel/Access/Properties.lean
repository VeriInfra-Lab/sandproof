import SandboxModel.Access.Handlers

namespace SandboxModel.Access
open Aeneas Aeneas.Std SandproofNetworkRust

theorem network_guard_iff (record : Record) (sandbox vm slot : Nat) :
    networkAllowed record sandbox vm slot = true ↔
      ∃ binding, record.authority = some binding ∧
        binding.sandbox = sandbox ∧ binding.generation = record.generation ∧
        binding.vm = vm ∧ binding.slot = slot ∧ binding.policy = record.policy := by
  cases present : record.authority <;> simp [networkAllowed, present]

theorem closed_authority_denied (record : Record) (closed : record.authority = none)
    (sandbox vm slot : Nat) : networkAllowed record sandbox vm slot = false := by
  simp [networkAllowed, closed]

theorem running_guard_realizable (owner sandbox generation vm slot : Nat)
    (policy : SandboxModel.Network.Policy) :
    networkAllowed ⟨owner, generation, policy,
      some ⟨sandbox, generation, vm, slot, policy⟩⟩ sandbox vm slot = true := by
  simp [networkAllowed]

theorem ingress_model_accepts_iff (state : S) (records : List (Nat × Record))
    (actor : Actor) (sandbox vm slot : Nat) :
    (∃ event next, ingress state records actor sandbox vm slot = .ok (event, next)) ↔
      ∃ record, SandboxModel.Foundations.tableLookup sandbox records = some record ∧
        authorized actor record.owner = true ∧ networkAllowed record sandbox vm slot = true := by
  cases found : SandboxModel.Foundations.tableLookup sandbox records with
  | none => simp [ingress, found]
  | some record =>
    cases owner : authorized actor record.owner <;>
      cases allowed : networkAllowed record sandbox vm slot <;> simp [ingress, found, owner, allowed]

theorem egress_model_accepts_iff (state : S) (records : List (Nat × Record))
    (sandbox vm slot : Nat) (destination : SandboxModel.Network.Endpoint) :
    (∃ event next, egress state records sandbox vm slot destination = .ok (event, next)) ↔
      ∃ record, SandboxModel.Foundations.tableLookup sandbox records = some record ∧
        networkAllowed record sandbox vm slot = true ∧
        SandboxModel.Network.allows record.policy destination = true := by
  cases found : SandboxModel.Foundations.tableLookup sandbox records with
  | none => simp [egress, found]
  | some record =>
    cases allowed : networkAllowed record sandbox vm slot <;>
      cases destinationAllowed : SandboxModel.Network.allows record.policy destination <;>
        simp [egress, found, allowed, destinationAllowed]

theorem ingress_model_frame (state : S) (records : List (Nat × Record))
    (actor : Actor) (sandbox vm slot : Nat) (event : Event) (next : S)
    (accepted : ingress state records actor sandbox vm slot = .ok (event, next)) : next = state := by
  cases found : SandboxModel.Foundations.tableLookup sandbox records with
  | none => simp [ingress, found] at accepted
  | some record =>
    cases owner : authorized actor record.owner <;>
      cases allowed : networkAllowed record sandbox vm slot <;>
        simp [ingress, found, owner, allowed] at accepted
    exact accepted.2.symm

theorem egress_model_frame (state : S) (records : List (Nat × Record))
    (sandbox vm slot : Nat) (destination : SandboxModel.Network.Endpoint) (event : Event) (next : S)
    (accepted : egress state records sandbox vm slot destination = .ok (event, next)) : next = state := by
  cases found : SandboxModel.Foundations.tableLookup sandbox records with
  | none => simp [egress, found] at accepted
  | some record =>
    cases allowed : networkAllowed record sandbox vm slot <;>
      cases destinationAllowed : SandboxModel.Network.allows record.policy destination <;>
        simp [egress, found, allowed, destinationAllowed] at accepted
    exact accepted.2.symm

theorem ingress_preserves_world (world : sandbox_controller_core.model.World)
    (actor : sandbox_controller_core.model.Actor) (sandbox : sandbox_controller_core.ids.SandboxId)
    (vm : sandbox_controller_core.ids.VmId) (slot : sandbox_controller_core.ids.NetSlotId)
    (decision : sandbox_controller_core.auxiliary.AuxiliaryDecision)
    (returned : sandbox_controller_core.auxiliary.handle_ingress world actor sandbox vm slot =
      .ok (.Ok decision)) : decision.next = world := by
  have relation := ingress_refines world actor sandbox vm slot
  rw [returned] at relation
  have modeled : ingress world (recordsView world) (actorView actor) sandbox.val vm.val slot.val =
      .ok (eventView decision.event, decision.next) := by
    simpa [resultView] using relation.symm
  exact ingress_model_frame world _ _ _ _ _ _ _ modeled

theorem egress_preserves_world (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) (vm : sandbox_controller_core.ids.VmId)
    (slot : sandbox_controller_core.ids.NetSlotId) (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address)
    (decision : sandbox_controller_core.auxiliary.AuxiliaryDecision)
    (returned : sandbox_controller_core.auxiliary.handle_egress world sandbox vm slot destination =
      .ok (.Ok decision)) : decision.next = world := by
  have relation := egress_refines world sandbox vm slot destination bound
  rw [returned] at relation
  have modeled : egress world (recordsView world) sandbox.val vm.val slot.val
      (SandboxModel.Network.endpointView destination) =
      .ok (eventView decision.event, decision.next) := by
    simpa [resultView] using relation.symm
  exact egress_model_frame world _ _ _ _ _ _ _ modeled

theorem egress_preserves_any_invariant (invariant : sandbox_controller_core.model.World → Prop)
    (world : sandbox_controller_core.model.World) (holds : invariant world)
    (sandbox : sandbox_controller_core.ids.SandboxId) (vm : sandbox_controller_core.ids.VmId)
    (slot : sandbox_controller_core.ids.NetSlotId) (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address)
    (decision : sandbox_controller_core.auxiliary.AuxiliaryDecision)
    (returned : sandbox_controller_core.auxiliary.handle_egress world sandbox vm slot destination =
      .ok (.Ok decision)) : invariant decision.next := by
  rw [egress_preserves_world world sandbox vm slot destination bound decision returned]
  exact holds

theorem reject_view_injective : Function.Injective rejectView := by
  intro left right
  cases left <;> cases right <;> simp [rejectView]

end SandboxModel.Access
