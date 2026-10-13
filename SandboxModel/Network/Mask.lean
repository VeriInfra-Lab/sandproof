import Aeneas

/-! Mathematics of the IPv4 prefix mask, independent of the generated
controller algorithm. Addresses remain natural numbers in these statements;
the implementation connection is provided separately by `Contains`. -/
namespace SandboxModel.Network.Mask

def mask (hostBits : Nat) : Nat :=
  ((2^32 - 1) * 2^hostBits) % 2^32

theorem mask_testBit (hostBits bit : Nat) :
    (mask hostBits).testBit bit = decide (hostBits ≤ bit ∧ bit < 32) := by
  unfold mask
  simp only [Nat.testBit_mod_two_pow, Nat.testBit_mul_two_pow,
    Nat.testBit_two_pow_sub_one]
  by_cases lower : hostBits ≤ bit
  · by_cases upper : bit < 32
    · have subBound : bit - hostBits < 32 := by omega
      simp [lower, upper, subBound]
    · simp [lower, upper]
  · simp [lower]

theorem masked_quotient (address hostBits : Nat) (bound : address < 2^32) :
    address &&& mask hostBits = (address / 2^hostBits) * 2^hostBits := by
  apply Nat.eq_of_testBit_eq
  intro bit
  rw [Nat.testBit_and, mask_testBit, Nat.testBit_mul_two_pow,
    Nat.testBit_div_two_pow]
  by_cases lower : hostBits ≤ bit
  · have subAdd : bit - hostBits + hostBits = bit := Nat.sub_add_cancel lower
    rw [subAdd]
    by_cases upper : bit < 32
    · simp [lower, upper]
    · have addressBit : address.testBit bit = false :=
        Nat.testBit_lt_two_pow (Nat.lt_of_lt_of_le bound
          (Nat.pow_le_pow_right Nat.zero_lt_two (by omega)))
      simp [lower, upper, addressBit]
  · simp [lower]

theorem canonical_iff_low_bits_zero (network hostBits : Nat)
    (bound : network < 2^32) :
    network &&& mask hostBits = network ↔ network % 2^hostBits = 0 := by
  rw [masked_quotient network hostBits bound]
  have decomposition :
      network % 2^hostBits + (network / 2^hostBits) * 2^hostBits = network := by
    simpa [Nat.mul_comm] using Nat.mod_add_div network (2^hostBits)
  constructor <;> intro h <;> omega

theorem canonical_match_iff_same_prefix (network address hostBits : Nat)
    (networkBound : network < 2^32) (addressBound : address < 2^32) :
    (network &&& mask hostBits = network ∧ address &&& mask hostBits = network) ↔
      (network % 2^hostBits = 0 ∧
        address / 2^hostBits = network / 2^hostBits) := by
  rw [canonical_iff_low_bits_zero network hostBits networkBound,
    masked_quotient address hostBits addressBound]
  have decomposition :
      network % 2^hostBits + (network / 2^hostBits) * 2^hostBits = network := by
    simpa [Nat.mul_comm] using Nat.mod_add_div network (2^hostBits)
  constructor
  · rintro ⟨canonical, same⟩
    have networkEq : (network / 2^hostBits) * 2^hostBits = network := by omega
    have products :
        (address / 2^hostBits) * 2^hostBits = (network / 2^hostBits) * 2^hostBits :=
      same.trans networkEq.symm
    exact ⟨canonical,
      Nat.eq_of_mul_eq_mul_right (Nat.two_pow_pos hostBits) products⟩
  · rintro ⟨canonical, same⟩
    constructor
    · exact canonical
    · rw [same]
      omega

end SandboxModel.Network.Mask
