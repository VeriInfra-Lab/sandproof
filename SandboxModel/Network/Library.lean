import SandproofNetwork.Funs
import SandboxModel.Network.Model
open Aeneas Aeneas.Std SandproofNetworkRust
namespace SandboxModel.Network

theorem bool_ne_model_exact (left right : Bool) :
    SandproofNetworkRust.Bool.Insts.CoreCmpPartialEqBool.ne left right = .ok (decide (left ≠ right)) := by
  cases left <;> cases right <;> rfl

theorem deref_refines (value : String) (bound : Ipv4Library.StringRepresentable value) :
    alloc.string.String.Insts.CoreOpsDerefDerefStr.deref value =
      .ok (Slice.from (Ipv4Library.stringBytes value) bound) := by
  simp [alloc.string.String.Insts.CoreOpsDerefDerefStr.deref, bound]

-- These connect the actual wrapper to the reviewed library model. They do not
-- prove that model agrees with the implementation of Rust's standard library.
theorem parse_ipv4_bits_refines (value : Str) :
    parse_ipv4_bits value = .ok (Ipv4Library.parseBits value.val) := by
  simp only [parse_ipv4_bits, core.str.Str.parse,
    core.net.ip_addr.Ipv4Addr.Insts.CoreStrTraitsFromStrAddrParseError.from_str]
  cases Ipv4Library.parseBits value.val <;> simp

theorem parse_failure_is_normal (value : Str)
    (invalid : Ipv4Library.parseBits value.val = none) :
    core.net.ip_addr.Ipv4Addr.Insts.CoreStrTraitsFromStrAddrParseError.from_str value =
      .ok (.Err .invalidIpv4) := by simp [invalid]

theorem parseOctet_success_spec (bytes : List U8) (value : Nat)
    (parsed : Ipv4Library.parseOctet bytes = some value) :
    0 < bytes.length ∧ bytes.length ≤ 3 ∧ value < 256 ∧
      bytes.all (fun byte => decide (48 ≤ byte.val ∧ byte.val ≤ 57)) = true ∧
      ¬ (bytes.length > 1 ∧ bytes.head?.map UScalar.val = some 48) ∧
      value = Ipv4Library.decimalValue bytes := by
  unfold Ipv4Library.parseOctet at parsed
  split at parsed
  · simp at parsed
  · rename_i lengthBound
    split at parsed
    · simp at parsed
    · rename_i noLeadingZero
      split at parsed
      · rename_i digits
        dsimp only at parsed
        split at parsed
        · rename_i valueBound
          cases Option.some.inj parsed
          exact ⟨by omega, by omega, valueBound, digits, noLeadingZero, rfl⟩
        · simp at parsed
      · simp at parsed

end SandboxModel.Network
