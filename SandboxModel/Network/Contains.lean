import SandproofNetwork
import SandboxModel.Network.Model
import SandboxModel.Network.Mask

/-! Refinement of the actual extracted CIDR membership implementation.
No canonical-input assumption is required: malformed lengths and noncanonical
network values are covered by the same total implementation theorem. -/
namespace SandboxModel.Network
open Aeneas Aeneas.Std SandproofNetworkRust

@[simp] private theorem scalar_eq_iff_val_eq {ty} {left right : UScalar ty} :
    left = right ↔ left.val = right.val :=
  ⟨fun same => congrArg UScalar.val same, UScalar.eq_of_val_eq⟩

private theorem checked_mask (length : U8) (positive : 0 < length.val)
    (bound : length.val ≤ 32) :
    ∃ (difference : U8) (scalarMask : U32),
      (32#u8 - length : Result U8) = Result.ok difference ∧
      (core.num.U32.MAX <<< difference : Result U32) = Result.ok scalarMask ∧
      scalarMask.val = Mask.mask (32 - length.val) := by
  obtain ⟨difference, subEq, differencePost⟩ :=
    WP.spec_imp_exists (U8.sub_spec (x := 32#u8) (y := length) (by simpa using bound))
  have differenceValue : difference.val = 32 - length.val := by
    simpa using differencePost.1
  have shiftBound : difference.val < 32 := by
    omega
  obtain ⟨scalarMask, shiftEq, maskPost⟩ :=
    WP.spec_imp_exists (U32.ShiftLeft_spec core.num.U32.MAX difference shiftBound)
  refine ⟨difference, scalarMask, subEq, shiftEq, ?_⟩
  simpa [Mask.mask, Nat.shiftLeft_eq, U32.size, U32.rMax, U32.numBits,
    core.num.U32.MAX, differenceValue] using maskPost.1

theorem cidr_contains_refines
    (cidr : sandbox_controller_core.model.Ipv4Cidr) (address : U32) :
    sandbox_controller_core.model.Ipv4Cidr.contains cidr address =
      .ok (decide ((rangeView cidr).matches address.val)) := by
  have networkBound : cidr.network.val < 2^32 := cidr.network.hBounds
  have addressBound : address.val < 2^32 := address.hBounds
  by_cases zeroPrefix : cidr.prefix_length = 0#u8
  · have prefixValue : cidr.prefix_length.val = 0 := by simpa using zeroPrefix
    have modelZero : (rangeView cidr).matches address.val ↔ cidr.network.val = 0 := by
      simp only [rangeView, Cidr.matches, prefixValue, Nat.sub_zero]
      rw [Nat.mod_eq_of_lt networkBound, Nat.div_eq_of_lt addressBound,
        Nat.div_eq_of_lt networkBound]
      simp only [networkBound, addressBound, Nat.zero_le, true_and, and_true]
    simp [sandbox_controller_core.model.Ipv4Cidr.contains, zeroPrefix,
      lift, scalar_eq_iff_val_eq, modelZero]
  · by_cases prefixBound : cidr.prefix_length.val ≤ 32
    · have prefixPositive : 0 < cidr.prefix_length.val := by
        have prefixNeZero : cidr.prefix_length.val ≠ 0 :=
          fun same => zeroPrefix (UScalar.eq_of_val_eq same)
        omega
      obtain ⟨difference, scalarMask, subEq, shiftEq, maskValue⟩ :=
        checked_mask cidr.prefix_length prefixPositive prefixBound
      have modelMask : (rangeView cidr).matches address.val ↔
          (cidr.network &&& scalarMask = cidr.network ∧
            address &&& scalarMask = cidr.network) := by
        rw [scalar_eq_iff_val_eq, scalar_eq_iff_val_eq]
        simp only [UScalar.val_and, maskValue]
        rw [Mask.canonical_match_iff_same_prefix _ _ _ networkBound addressBound]
        simp only [rangeView, Cidr.matches, prefixBound, networkBound, addressBound,
          true_and]
      simp only [sandbox_controller_core.model.Ipv4Cidr.contains, zeroPrefix,
        ↓reduceIte]
      simp only [show cidr.prefix_length ≤ 32#u8 from prefixBound, ↓reduceIte,
        subEq, shiftEq, lift, bind_tc_ok]
      by_cases canonical : cidr.network &&& scalarMask = cidr.network
      · simp [canonical, modelMask]
      · simp [canonical, modelMask]
    · simp [sandbox_controller_core.model.Ipv4Cidr.contains, zeroPrefix,
        prefixBound, rangeView, Cidr.matches]

theorem cidr_root_refines
    (cidr : sandbox_controller_core.model.Ipv4Cidr) (address : U32) :
    SandproofNetworkRust.cidr_contains cidr address =
      .ok (decide ((rangeView cidr).matches address.val)) := by
  exact cidr_contains_refines cidr address

theorem cidr_prefix32_refines
    (cidr : sandbox_controller_core.model.Ipv4Cidr) (address : U32)
    (length : cidr.prefix_length.val = 32) :
    sandbox_controller_core.model.Ipv4Cidr.contains cidr address =
      .ok (decide (address.val = cidr.network.val)) := by
  rw [cidr_contains_refines]
  have networkBound : cidr.network.val < 2^32 := cidr.network.hBounds
  have addressBound : address.val < 2^32 := address.hBounds
  simp only [rangeView, Cidr.matches, length, le_refl, networkBound, addressBound,
    Nat.sub_self, Nat.pow_zero, Nat.mod_one, Nat.div_one, true_and]

theorem cidr_prefix0_refines
    (cidr : sandbox_controller_core.model.Ipv4Cidr) (address : U32)
    (length : cidr.prefix_length.val = 0) :
    sandbox_controller_core.model.Ipv4Cidr.contains cidr address =
      .ok (decide (cidr.network.val = 0)) := by
  rw [cidr_contains_refines]
  have networkBound : cidr.network.val < 2^32 := cidr.network.hBounds
  have addressBound : address.val < 2^32 := address.hBounds
  simp only [rangeView, Cidr.matches, length, Nat.sub_zero]
  rw [Nat.mod_eq_of_lt networkBound, Nat.div_eq_of_lt addressBound,
    Nat.div_eq_of_lt networkBound]
  simp only [networkBound, addressBound, Nat.zero_le, true_and, and_true]

theorem cidr_malformed_prefix_rejected
    (cidr : sandbox_controller_core.model.Ipv4Cidr) (address : U32)
    (length : 32 < cidr.prefix_length.val) :
    sandbox_controller_core.model.Ipv4Cidr.contains cidr address = .ok false := by
  rw [cidr_contains_refines]
  have invalid : ¬ cidr.prefix_length.val ≤ 32 := by omega
  simp only [rangeView, Cidr.matches, invalid, false_and, decide_false]

theorem cidr_noncanonical_rejected
    (cidr : sandbox_controller_core.model.Ipv4Cidr) (address : U32)
    (noncanonical : cidr.network.val % 2^(32 - cidr.prefix_length.val) ≠ 0) :
    sandbox_controller_core.model.Ipv4Cidr.contains cidr address = .ok false := by
  rw [cidr_contains_refines]
  simp [rangeView, Cidr.matches, noncanonical]

end SandboxModel.Network
