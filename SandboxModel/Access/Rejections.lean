import SandboxModel.Access.Guards
import SandboxModel.Access.Lookup
import SandboxModel.Network.Policy

namespace SandboxModel.Access
open Aeneas Aeneas.Std SandproofNetworkRust

theorem ingress_missing_rejected (world : sandbox_controller_core.model.World)
    (actor : sandbox_controller_core.model.Actor) (sandbox : sandbox_controller_core.ids.SandboxId)
    (vm : sandbox_controller_core.ids.VmId) (slot : sandbox_controller_core.ids.NetSlotId)
    (missing : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val = none) :
    sandbox_controller_core.auxiliary.handle_ingress world actor sandbox vm slot =
      .ok (.Err .SandboxNotFound) := by
  unfold sandbox_controller_core.auxiliary.handle_ingress
  rw [store_get_refines _ sandbox_equality_lawful]
  simp [missing]

theorem egress_missing_rejected (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) (vm : sandbox_controller_core.ids.VmId)
    (slot : sandbox_controller_core.ids.NetSlotId) (destination : sandbox_controller_core.model.Endpoint)
    (missing : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val = none) :
    sandbox_controller_core.auxiliary.handle_egress world sandbox vm slot destination =
      .ok (.Err .SandboxNotFound) := by
  unfold sandbox_controller_core.auxiliary.handle_egress
  rw [store_get_refines _ sandbox_equality_lawful]
  simp [missing]

theorem ingress_wrong_owner_rejected_first (world : sandbox_controller_core.model.World)
    (tenant : sandbox_controller_core.ids.TenantId) (sandbox : sandbox_controller_core.ids.SandboxId)
    (vm : sandbox_controller_core.ids.VmId) (slot : sandbox_controller_core.ids.NetSlotId)
    (record : sandbox_controller_core.model.SandboxRecord)
    (found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val = some record)
    (wrong : tenant.val ≠ record.owner.val) :
    sandbox_controller_core.auxiliary.handle_ingress world (.Tenant tenant) sandbox vm slot =
      .ok (.Err .Unauthorized) := by
  unfold sandbox_controller_core.auxiliary.handle_ingress
  rw [store_get_refines _ sandbox_equality_lawful]
  simp only [found, bind_tc_ok]
  rw [authorized_refines]
  simp [actorView, authorized, wrong]

theorem egress_capability_rejected_first (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) (vm : sandbox_controller_core.ids.VmId)
    (slot : sandbox_controller_core.ids.NetSlotId) (destination : sandbox_controller_core.model.Endpoint)
    (record : sandbox_controller_core.model.SandboxRecord)
    (found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val = some record)
    (denied : networkAllowed (recordView record) sandbox.val vm.val slot.val = false) :
    sandbox_controller_core.auxiliary.handle_egress world sandbox vm slot destination =
      .ok (.Err .CapabilityDenied) := by
  unfold sandbox_controller_core.auxiliary.handle_egress
  rw [store_get_refines _ sandbox_equality_lawful]
  simp only [found, bind_tc_ok]
  rw [network_guard_refines]
  simp [denied]

theorem egress_destination_rejected_after_capability (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) (vm : sandbox_controller_core.ids.VmId)
    (slot : sandbox_controller_core.ids.NetSlotId) (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address)
    (record : sandbox_controller_core.model.SandboxRecord)
    (found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val = some record)
    (allowed : networkAllowed (recordView record) sandbox.val vm.val slot.val = true)
    (denied : SandboxModel.Network.allows (SandboxModel.Network.policyView record.policy)
      (SandboxModel.Network.endpointView destination) = false) :
    sandbox_controller_core.auxiliary.handle_egress world sandbox vm slot destination =
      .ok (.Err .DestinationDenied) := by
  unfold sandbox_controller_core.auxiliary.handle_egress
  rw [store_get_refines _ sandbox_equality_lawful]
  simp only [found, bind_tc_ok]
  rw [network_guard_refines]
  simp only [allowed, bind_tc_ok, ↓reduceIte]
  unfold sandbox_controller_core.auxiliary.destination_allowed
  rw [SandboxModel.Network.policy_refines _ _ bound]
  simp [denied]

theorem ipv4_platform_rejected_after_capability (world : sandbox_controller_core.model.World)
    (sandbox : sandbox_controller_core.ids.SandboxId) (vm : sandbox_controller_core.ids.VmId)
    (slot : sandbox_controller_core.ids.NetSlotId) (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address)
    (record : sandbox_controller_core.model.SandboxRecord)
    (found : SandboxModel.Foundations.tableLookup sandbox world.sandboxes.entries.val = some record)
    (allowed : networkAllowed (recordView record) sandbox.val vm.val slot.val = true)
    (internet : Bool) (allow deny : alloc.vec.Vec sandbox_controller_core.model.Ipv4Cidr)
    (policy : record.policy = .Ipv4 internet allow deny) (address : Nat)
    (parsed : SandboxModel.Network.parseAddress destination.address = some address)
    (floorHit : SandboxModel.Network.platformProtected address = true) :
    sandbox_controller_core.auxiliary.handle_egress world sandbox vm slot destination =
      .ok (.Err .DestinationDenied) := by
  apply egress_destination_rejected_after_capability world sandbox vm slot destination bound
    record found allowed
  simp [policy, SandboxModel.Network.policyView, SandboxModel.Network.endpointView,
    SandboxModel.Network.allows, parsed, floorHit]

end SandboxModel.Access
