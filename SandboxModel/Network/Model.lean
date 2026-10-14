import SandproofNetwork.FunsExternal

/-! Independent network decision model. The byte parser is the reviewed library
contract, not a second version of the project's CIDR/policy algorithm. -/
namespace SandboxModel.Network
open Aeneas.Std SandproofNetworkRust

structure Cidr where
  network : Nat
  prefixLength : Nat
  deriving DecidableEq

def Cidr.matches (range : Cidr) (address : Nat) : Prop :=
  range.prefixLength ≤ 32 ∧ range.network < 2^32 ∧ address < 2^32 ∧
  range.network % 2^(32 - range.prefixLength) = 0 ∧
  address / 2^(32 - range.prefixLength) = range.network / 2^(32 - range.prefixLength)

instance (range : Cidr) (address : Nat) : Decidable (range.matches address) :=
  inferInstanceAs (Decidable (_ ∧ _ ∧ _ ∧ _ ∧ _))

def rangeView (range : sandbox_controller_core.model.Ipv4Cidr) : Cidr :=
  ⟨range.network.val, range.prefix_length.val⟩

def rangesMatch (ranges : List Cidr) (address : Nat) : Bool :=
  ranges.any (fun range => decide (range.matches address))

def platformRanges : List Cidr :=
  [⟨0, 8⟩, ⟨167772160, 8⟩, ⟨1681915904, 10⟩, ⟨2130706432, 8⟩,
   ⟨2851995648, 16⟩, ⟨2886729728, 12⟩, ⟨3221225472, 24⟩,
   ⟨3221225984, 24⟩, ⟨3227017984, 24⟩, ⟨3232235520, 16⟩,
   ⟨3323068416, 15⟩, ⟨3325256704, 24⟩, ⟨3405803776, 24⟩,
   ⟨3758096384, 4⟩, ⟨4026531840, 4⟩]

def platformProtected (address : Nat) : Bool := rangesMatch platformRanges address

structure Endpoint where
  address : String
  prefixLength : Nat
  udp : Bool
  port : Nat
  deriving DecidableEq

def endpointView (endpoint : sandbox_controller_core.model.Endpoint) : Endpoint :=
  ⟨endpoint.address, endpoint.prefix_length.val,
   match endpoint.protocol with | .Tcp => false | .Udp => true,
   endpoint.port.val⟩

inductive Policy where
  | denyAll
  | staticAllowlist (endpoints : List Endpoint)
  | ipv4 (internet : Bool) (allow deny : List Cidr)

def policyView (policy : sandbox_controller_core.model.NetworkPolicy) : Policy :=
  match policy with
  | .DenyAll => .denyAll
  | .StaticAllowlist endpoints => .staticAllowlist (endpoints.val.map endpointView)
  | .Ipv4 internet allow deny => .ipv4 internet
      (allow.val.map rangeView) (deny.val.map rangeView)

def parseAddress (value : String) : Option Nat :=
  (Ipv4Library.parseBits (Ipv4Library.stringBytes value)).map UScalar.val

-- IPv4 policies constrain destination IP only. Protocol/port are intentionally
-- part of legacy static endpoint equality, not silently added to this mode.
def allows (policy : Policy) (destination : Endpoint) : Bool :=
  match policy with
  | .denyAll => false
  | .staticAllowlist endpoints => endpoints.any (fun endpoint => decide (endpoint = destination))
  | .ipv4 internet allow deny =>
      match parseAddress destination.address with
      | none => false
      | some address =>
          destination.prefixLength == 32 && !platformProtected address &&
            (rangesMatch allow address || (!rangesMatch deny address && internet))

end SandboxModel.Network
