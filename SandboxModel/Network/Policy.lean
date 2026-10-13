import SandboxModel.Network.Ranges
import SandboxModel.Network.Static
import SandboxModel.Network.Library
open Aeneas Aeneas.Std SandproofNetworkRust
namespace SandboxModel.Network

theorem vec_deref_val (values : alloc.vec.Vec T) : values.deref.val = values.val := by
  simp [alloc.vec.Vec.deref]

theorem ipv4_policy_refines (internet : Bool)
    (allow deny : alloc.vec.Vec sandbox_controller_core.model.Ipv4Cidr)
    (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address) :
    sandbox_controller_core.network.NetworkPolicy.destination_allowed
        (.Ipv4 internet allow deny) destination =
      .ok (allows (policyView (.Ipv4 internet allow deny)) (endpointView destination)) := by
  unfold sandbox_controller_core.network.NetworkPolicy.destination_allowed
  rw [deref_refines destination.address bound]
  simp only [bind_tc_ok, core.str.Str.parse,
    core.net.ip_addr.Ipv4Addr.Insts.CoreStrTraitsFromStrAddrParseError.from_str,
    Slice.from_val]
  cases parsed : Ipv4Library.parseBits (Ipv4Library.stringBytes destination.address) with
  | none => simp [parsed, allows, policyView, endpointView, parseAddress]
  | some bits =>
    simp only [bind_tc_ok, U32.Insts.CoreConvertFromIpv4Addr.from]
    simp only [platform_protected_refines, ranges_refines, bind_tc_ok]
    have prefixValue : destination.prefix_length.val = 32 ↔
        destination.prefix_length = 32#u64 := by
      simp
    by_cases prefix32 : destination.prefix_length = 32#u64 <;>
      cases floorHit : platformProtected bits.val <;>
        cases allowHit : rangesMatch (allow.val.map rangeView) bits.val <;>
          cases denyHit : rangesMatch (deny.val.map rangeView) bits.val <;>
            simp [prefix32, allows, policyView, endpointView, parseAddress, parsed,
              prefixValue, vec_deref_val, floorHit, allowHit, denyHit]

theorem policy_refines (policy : sandbox_controller_core.model.NetworkPolicy)
    (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address) :
    sandbox_controller_core.network.NetworkPolicy.destination_allowed policy destination =
      .ok (allows (policyView policy) (endpointView destination)) := by
  cases policy with
  | DenyAll => rfl
  | StaticAllowlist endpoints => exact static_policy_refines endpoints destination
  | Ipv4 internet allow deny => exact ipv4_policy_refines internet allow deny destination bound

theorem policy_root_refines (policy : sandbox_controller_core.model.NetworkPolicy)
    (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address) :
    SandproofNetworkRust.destination_allowed policy destination =
      .ok (allows (policyView policy) (endpointView destination)) := by
  exact policy_refines policy destination bound

theorem ipv4_platform_never_overridden (internet : Bool)
    (allow deny : alloc.vec.Vec sandbox_controller_core.model.Ipv4Cidr)
    (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address) (address : Nat)
    (parsed : parseAddress destination.address = some address)
    (floorHit : platformProtected address = true) :
    SandproofNetworkRust.destination_allowed (.Ipv4 internet allow deny) destination = .ok false := by
  rw [policy_root_refines _ _ bound]
  simp [allows, policyView, endpointView, parsed, floorHit]

theorem ipv4_allow_precedes_deny (internet : Bool)
    (allow deny : alloc.vec.Vec sandbox_controller_core.model.Ipv4Cidr)
    (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address) (address : Nat)
    (parsed : parseAddress destination.address = some address)
    (prefix32 : destination.prefix_length.val = 32)
    (floorMiss : platformProtected address = false)
    (allowHit : rangesMatch (allow.val.map rangeView) address = true) :
    SandproofNetworkRust.destination_allowed (.Ipv4 internet allow deny) destination = .ok true := by
  rw [policy_root_refines _ _ bound]
  simp [allows, policyView, endpointView, parsed, prefix32, floorMiss, allowHit]

theorem ipv4_deny_after_allow_miss (internet : Bool)
    (allow deny : alloc.vec.Vec sandbox_controller_core.model.Ipv4Cidr)
    (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address) (address : Nat)
    (parsed : parseAddress destination.address = some address)
    (allowMiss : rangesMatch (allow.val.map rangeView) address = false)
    (denyHit : rangesMatch (deny.val.map rangeView) address = true) :
    SandproofNetworkRust.destination_allowed (.Ipv4 internet allow deny) destination = .ok false := by
  rw [policy_root_refines _ _ bound]
  simp [allows, policyView, endpointView, parsed, allowMiss, denyHit]

theorem ipv4_default_after_both_miss (internet : Bool)
    (allow deny : alloc.vec.Vec sandbox_controller_core.model.Ipv4Cidr)
    (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address) (address : Nat)
    (parsed : parseAddress destination.address = some address)
    (prefix32 : destination.prefix_length.val = 32)
    (floorMiss : platformProtected address = false)
    (allowMiss : rangesMatch (allow.val.map rangeView) address = false)
    (denyMiss : rangesMatch (deny.val.map rangeView) address = false) :
    SandproofNetworkRust.destination_allowed (.Ipv4 internet allow deny) destination = .ok internet := by
  rw [policy_root_refines _ _ bound]
  simp [allows, policyView, endpointView, parsed, prefix32, floorMiss, allowMiss, denyMiss]

theorem ipv4_parse_failure_rejected (internet : Bool)
    (allow deny : alloc.vec.Vec sandbox_controller_core.model.Ipv4Cidr)
    (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address)
    (invalid : parseAddress destination.address = none) :
    SandproofNetworkRust.destination_allowed (.Ipv4 internet allow deny) destination = .ok false := by
  rw [policy_root_refines _ _ bound]
  simp [allows, policyView, endpointView, invalid]

theorem ipv4_nonhost_prefix_rejected (internet : Bool)
    (allow deny : alloc.vec.Vec sandbox_controller_core.model.Ipv4Cidr)
    (destination : sandbox_controller_core.model.Endpoint)
    (bound : Ipv4Library.StringRepresentable destination.address)
    (invalid : destination.prefix_length.val ≠ 32) :
    SandproofNetworkRust.destination_allowed (.Ipv4 internet allow deny) destination = .ok false := by
  rw [policy_root_refines _ _ bound]
  cases parsed : parseAddress destination.address <;>
    simp [allows, policyView, endpointView, parsed, invalid]

theorem ipv4_ports_protocol_unchanged (internet : Bool)
    (allow deny : alloc.vec.Vec sandbox_controller_core.model.Ipv4Cidr)
    (left right : sandbox_controller_core.model.Endpoint)
    (leftBound : Ipv4Library.StringRepresentable left.address)
    (rightBound : Ipv4Library.StringRepresentable right.address)
    (sameAddress : left.address = right.address)
    (samePrefix : left.prefix_length.val = right.prefix_length.val) :
    SandproofNetworkRust.destination_allowed (.Ipv4 internet allow deny) left =
      SandproofNetworkRust.destination_allowed (.Ipv4 internet allow deny) right := by
  rw [policy_root_refines _ _ leftBound, policy_root_refines _ _ rightBound]
  simp [allows, policyView, endpointView, sameAddress, samePrefix]

end SandboxModel.Network
