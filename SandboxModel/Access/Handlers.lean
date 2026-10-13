import SandboxModel.Access.Guards
import SandboxModel.Access.Cloning
import SandboxModel.Access.Lookup
import SandboxModel.Network.Policy

namespace SandboxModel.Access
open Aeneas Aeneas.Std SandproofNetworkRust

theorem ingress_refines (world : sandbox_controller_core.model.World)
    (actor : sandbox_controller_core.model.Actor) (sandbox : sandbox_controller_core.ids.SandboxId)
    (vm : sandbox_controller_core.ids.VmId) (slot : sandbox_controller_core.ids.NetSlotId) :
    (do let result ← sandbox_controller_core.auxiliary.handle_ingress world actor sandbox vm slot; .ok (resultView result)) =
      .ok (ingress world (recordsView world) (actorView actor) sandbox.val vm.val slot.val) := by
  unfold sandbox_controller_core.auxiliary.handle_ingress ingress
  rw [store_get_refines _ sandbox_equality_lawful, sandbox_lookup_refines]
  cases found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val with
  | none => simp [resultView, rejectView]
  | some record =>
      simp only [bind_tc_ok, Option.map_some]
      rw [authorized_refines]
      by_cases owner : authorized (actorView actor) record.owner.val = true
      · rw [network_guard_refines]
        cases allowed : networkAllowed (recordView record) sandbox.val vm.val slot.val <;>
          simp [owner, allowed, recordView, resultView, eventView, rejectView]
      · simp [owner, recordView, resultView, rejectView]

theorem egress_refines (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) (vm : sandbox_controller_core.ids.VmId)
    (slot : sandbox_controller_core.ids.NetSlotId) (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address) :
    (do let result ← sandbox_controller_core.auxiliary.handle_egress world sandbox vm slot destination; .ok (resultView result)) =
      .ok (egress world (recordsView world) sandbox.val vm.val slot.val
        (SandboxModel.Network.endpointView destination)) := by
  unfold sandbox_controller_core.auxiliary.handle_egress egress
  rw [store_get_refines _ sandbox_equality_lawful, sandbox_lookup_refines]
  cases found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val with
  | none => simp [resultView, rejectView]
  | some record =>
      simp only [bind_tc_ok, Option.map_some]
      rw [network_guard_refines]
      cases allowed : networkAllowed (recordView record) sandbox.val vm.val slot.val with
      | false => simp [allowed, resultView, rejectView]
      | true =>
          simp only [bind_tc_ok, allowed, Bool.true_eq, ↓reduceIte]
          unfold sandbox_controller_core.auxiliary.destination_allowed
          rw [SandboxModel.Network.policy_refines _ _ bound]
          cases destinationAllowed : SandboxModel.Network.allows
              (SandboxModel.Network.policyView record.policy)
              (SandboxModel.Network.endpointView destination) <;>
            simp [allowed, destinationAllowed, recordView, resultView, eventView, rejectView]

theorem ingress_root_refines (world : sandbox_controller_core.model.World)
    (actor : sandbox_controller_core.model.Actor) (sandbox : sandbox_controller_core.ids.SandboxId)
    (vm : sandbox_controller_core.ids.VmId) (slot : sandbox_controller_core.ids.NetSlotId) :
    (do let result ← SandproofNetworkRust.ingress world actor sandbox vm slot; .ok (resultView result)) =
      .ok (ingress world (recordsView world) (actorView actor) sandbox.val vm.val slot.val) :=
  ingress_refines world actor sandbox vm slot

theorem egress_root_refines (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) (vm : sandbox_controller_core.ids.VmId)
    (slot : sandbox_controller_core.ids.NetSlotId) (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address) :
    (do let result ← SandproofNetworkRust.egress world sandbox vm slot destination; .ok (resultView result)) =
      .ok (egress world (recordsView world) sandbox.val vm.val slot.val
        (SandboxModel.Network.endpointView destination)) :=
  egress_refines world sandbox vm slot destination bound

end SandboxModel.Access
