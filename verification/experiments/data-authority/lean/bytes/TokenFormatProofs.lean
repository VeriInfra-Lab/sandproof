import ByteProofs
open Aeneas Aeneas.Std Aeneas.Std.WP
open Authority.credential_core.byte_core

namespace TokenFormatProofs

def HexByte (byte : U8) : Prop :=
  (48 ≤ byte.val ∧ byte.val ≤ 57) ∨
  (97 ≤ byte.val ∧ byte.val ≤ 102) ∨
  (65 ≤ byte.val ∧ byte.val ≤ 70)

instance (byte : U8) : Decidable (HexByte byte) := by
  unfold HexByte
  infer_instance

def Format (token : Slice U8) : Prop :=
  token.length = 64 ∧ ∀ byte ∈ token.val, HexByte byte

instance (token : Slice U8) : Decidable (Format token) := by
  unfold Format
  infer_instance

theorem is_hex_exact (byte : U8) :
    is_hex byte = .ok (decide (HexByte byte)) := by
  by_cases h1 : 48 ≤ byte.val <;> by_cases h2 : byte.val ≤ 57 <;>
    by_cases h3 : 97 ≤ byte.val <;> by_cases h4 : byte.val ≤ 102 <;>
    by_cases h5 : 65 ≤ byte.val <;> by_cases h6 : byte.val ≤ 70 <;>
    simp [is_hex, HexByte, h1, h2, h3, h4, h5, h6]

@[step]
theorem is_hex_spec (byte : U8) :
    is_hex byte ⦃ output => output = true ↔ HexByte byte ⦄ := by
  rw [is_hex_exact]
  simp

def TailHex (token : Slice U8) (index : Nat) : Prop :=
  ∀ j, index ≤ j → ∀ h : j < token.val.length, HexByte token.val[j]

theorem hex_tail_step (token : Slice U8) (index : Nat) (hindex : index < token.val.length) :
    TailHex token index ↔ HexByte token.val[index] ∧ TailHex token (index + 1) := by
  constructor
  · intro tail
    exact ⟨tail index (by omega) hindex, fun j hij hj => tail j (by omega) hj⟩
  · rintro ⟨head, tail⟩ j hij hj
    by_cases eq : j = index
    · subst j
      exact head
    · exact tail j (by omega) hj

@[step]
theorem token_format_body_spec (token : Slice U8) (index : Usize)
    (hindex : index.val ≤ token.length) :
    token_format_loop.body token index ⦃ result =>
      match result with
      | .cont next_index => next_index.val = index.val + 1 ∧
          next_index.val ≤ token.length ∧
          ∀ h : index.val < token.val.length, HexByte token.val[index.val]
      | .done output => output = true ↔ TailHex token index.val ⦄ := by
  unfold token_format_loop.body
  dsimp only
  split
  · step as ⟨byte, hbyte⟩
    step as ⟨hex, hhex⟩
    split
    · step as ⟨next_index, hnext⟩
      refine ⟨?_, ?_, ?_⟩
      · scalar_tac
      · scalar_tac
      · intro h
        simp_all
    · simp only [spec_ok]
      simp only [Bool.false_eq_true, false_iff]
      intro tail
      have bound : index.val < token.val.length := by scalar_tac
      have head := tail index.val (by omega) bound
      simp_all
  · simp only [spec_ok]
    constructor
    · intro _ j hij hj
      scalar_tac
    · intro _
      trivial

@[step]
theorem token_format_loop_spec (token : Slice U8) (index : Usize)
    (hindex : index.val ≤ token.length) :
    token_format_loop token index ⦃ output => output = true ↔ TailHex token index.val ⦄ := by
  unfold token_format_loop
  refine loop.spec_decr_nat
    (fun (current : Usize) => token.length - current.val)
    (fun current => current.val ≤ token.length ∧
      (TailHex token current.val ↔ TailHex token index.val))
    _ _ _ ?_ ⟨hindex, Iff.rfl⟩
  intro current invariant
  obtain ⟨result, run, properties⟩ := spec_imp_exists
    (token_format_body_spec token current invariant.1)
  rw [run]
  simp only [spec_ok]
  cases result with
  | done output => exact properties.trans invariant.2
  | cont next =>
    rcases properties with ⟨advance, bound, hex⟩
    have before_end : current.val < token.val.length := by scalar_tac
    refine ⟨⟨bound, ?_⟩, ?_⟩
    · rw [advance]
      have head := hex before_end
      have tail := hex_tail_step token current.val before_end
      simpa [head] using tail.symm.trans invariant.2
    · omega

theorem tail_hex_zero (token : Slice U8) :
    TailHex token 0 ↔ ∀ byte ∈ token.val, HexByte byte := by
  constructor
  · intro tail byte member
    obtain ⟨index, bound, equal⟩ := List.mem_iff_getElem.mp member
    simpa [equal] using tail index (by omega) bound
  · intro all j _ hj
    exact all token.val[j] (List.getElem_mem hj)

@[step]
theorem token_format_spec (token : Slice U8) :
    token_format token ⦃ output => output = true ↔ Format token ⦄ := by
  unfold token_format
  dsimp only
  split
  · simp [Format]
    scalar_tac
  · step with token_format_loop_spec token 0#usize (by scalar_tac)
      as ⟨output, property⟩
    have length : token.length = 64 := by scalar_tac
    simpa [Format, length, tail_hex_zero] using property

theorem token_format_exact (token : Slice U8) :
    token_format token = .ok (decide (Format token)) := by
  obtain ⟨result, run, property⟩ := spec_imp_exists (token_format_spec token)
  rw [run]
  cases result with
  | false =>
    have invalid : ¬ Format token := by simpa using property.symm
    simp [invalid]
  | true =>
    have valid : Format token := property.mp rfl
    simp [valid]

end TokenFormatProofs
