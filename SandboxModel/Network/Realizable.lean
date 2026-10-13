import SandboxModel.Network.Ranges
import SandboxModel.Network.Library

/-! A concrete, representable public-address witness for the actual policy
implementation. Character-list encoding makes the byte correspondence
kernel-checkable; this is not a native-evaluation or deployment-network test. -/
namespace SandboxModel.Network.Realizable
open Aeneas.Std SandproofNetworkRust SandboxModel.Network

def publicAddress : String := String.ofList ['8', '.', '8', '.', '8', '.', '8']

theorem publicAddress_bytes : Ipv4Library.stringBytes publicAddress =
    [56#u8, 46#u8, 56#u8, 46#u8, 56#u8, 46#u8, 56#u8] := by
  unfold Ipv4Library.stringBytes publicAddress
  rw [String.toByteArray_ofList]
  simp [List.utf8Encode, String.utf8EncodeChar, ByteArray.toList,
    ByteArray.toList.loop, ByteArray.get!]

theorem publicAddress_representable : Ipv4Library.StringRepresentable publicAddress := by
  unfold Ipv4Library.StringRepresentable
  rw [publicAddress_bytes]
  scalar_tac

theorem publicAddress_parsed : parseAddress publicAddress = some 134744072 := by
  unfold parseAddress
  rw [publicAddress_bytes]
  decide

theorem publicAddress_not_platform : platformProtected 134744072 = false := by
  decide

theorem publicAddress_allows : allows (.ipv4 true [] [])
    ⟨publicAddress, 32, false, 443⟩ = true := by
  simp [allows, publicAddress_parsed, publicAddress_not_platform, rangesMatch]

theorem publicAddress_bits : Ipv4Library.parseBits
    (Ipv4Library.stringBytes publicAddress) = some 134744072#u32 := by
  rw [publicAddress_bytes]
  decide

theorem publicAddress_root_realizable :
    let destination : sandbox_controller_core.model.Endpoint :=
      ⟨publicAddress, 32#u64, .Tcp, 443#u64⟩
    SandproofNetworkRust.destination_allowed
      (.Ipv4 true (alloc.vec.Vec.new _) (alloc.vec.Vec.new _)) destination = .ok true := by
  dsimp only
  unfold SandproofNetworkRust.destination_allowed
    sandbox_controller_core.network.NetworkPolicy.destination_allowed
  rw [deref_refines publicAddress publicAddress_representable]
  simp [core.str.Str.parse,
    core.net.ip_addr.Ipv4Addr.Insts.CoreStrTraitsFromStrAddrParseError.from_str,
    publicAddress_bits, U32.Insts.CoreConvertFromIpv4Addr.from,
    platform_protected_refines, publicAddress_not_platform, ranges_refines,
    rangesMatch, alloc.vec.Vec.deref]

end SandboxModel.Network.Realizable
