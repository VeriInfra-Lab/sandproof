import Authority.Funs
import BytePolicy
open Aeneas Aeneas.Std Aeneas.Std.WP
open Authority.credential_core.byte_core

namespace ByteProofs

def TailEq (left right : Slice U8) (index : Nat) : Prop :=
  ∀ j, index ≤ j → j < left.length → left.val[j]? = right.val[j]?

@[step]
theorem same_secret_body_spec (left right : Slice U8) (difference : U8) (index : Usize)
    (hlen : left.length = right.length) (hindex : index.val ≤ left.length) :
    same_secret_loop.body left right difference index ⦃ result =>
      match result with
      | .cont (next_difference, next_index) =>
        next_index.val = index.val + 1 ∧ next_index.val ≤ left.length ∧
          (next_difference = 0#u8 ↔ difference = 0#u8 ∧
            left.val[index.val]? = right.val[index.val]?)
      | .done output => index.val = left.length ∧ output = difference ⦄ := by
  unfold same_secret_loop.body
  dsimp only
  split
  · step as ⟨ a, ha ⟩
    step as ⟨ b, hb ⟩
    simp only [lift]
    step as ⟨ next_index, hnext ⟩
    constructor
    · scalar_tac
    constructor
    · scalar_tac
    rw [U8.or_eq_zero_iff, U8.xor_eq_zero_iff]
    have hil : index.val < left.val.length := by scalar_tac
    have hir : index.val < right.val.length := by scalar_tac
    simp_all
  · simp
    scalar_tac

theorem tail_step (left right : Slice U8) (index : Nat) (hindex : index < left.length) :
    TailEq left right index ↔
      left.val[index]? = right.val[index]? ∧ TailEq left right (index + 1) := by
  constructor
  · intro tail
    exact ⟨tail index (by omega) hindex, fun j hij hj => tail j (by omega) hj⟩
  · rintro ⟨head, tail⟩ j hij hj
    by_cases eq : j = index
    · simpa [eq] using head
    · exact tail j (by omega) hj

@[step]
theorem same_secret_loop_spec (left right : Slice U8) (difference : U8) (index : Usize)
    (hlen : left.length = right.length) (hindex : index.val ≤ left.length) :
    same_secret_loop left right difference index ⦃ output =>
      output = 0#u8 ↔ difference = 0#u8 ∧ TailEq left right index.val ⦄ := by
  unfold same_secret_loop
  refine loop.spec_decr_nat
    (fun (state : U8 × Usize) => left.length - state.2.val)
    (fun state => state.2.val ≤ left.length ∧
      ((state.1 = 0#u8 ∧ TailEq left right state.2.val) ↔
        (difference = 0#u8 ∧ TailEq left right index.val)))
    _ _ _ ?_ ⟨hindex, Iff.rfl⟩
  rintro ⟨current_difference, current_index⟩ invariant
  dsimp only at invariant ⊢
  obtain ⟨result, run, properties⟩ := spec_imp_exists
    (same_secret_body_spec left right current_difference current_index hlen invariant.1)
  rw [run]
  simp only [spec_ok]
  cases result with
  | done output =>
    have tail : TailEq left right current_index.val := by
      intro j hij hj
      omega
    simpa [properties.2, tail] using invariant.2
  | cont next =>
    rcases next with ⟨next_difference, next_index⟩
    rcases properties with ⟨advance, bound, zeros⟩
    have before_end : current_index.val < left.length := by omega
    refine ⟨⟨bound, ?_⟩, ?_⟩
    · rw [zeros, advance, and_assoc, ← tail_step left right current_index.val before_end]
      exact invariant.2
    · dsimp only
      omega

theorem tail_zero_iff (left right : Slice U8) (hlen : left.length = right.length) :
    TailEq left right 0 ↔ left.val = right.val := by
  constructor
  · intro tail
    apply List.ext_getElem?
    intro j
    by_cases hj : j < left.length
    · exact tail j (by omega) hj
    · have hjr : ¬ j < right.length := by omega
      simp_all
  · intro equal j _ _
    simp [equal]

@[step]
theorem same_secret_spec (left right : Slice U8) :
    same_secret left right ⦃ result => result = true ↔ left.val = right.val ⦄ := by
  unfold same_secret
  dsimp only
  split
  · have unequal : left.val ≠ right.val := by
      intro equal
      have lengths := congrArg List.length equal
      scalar_tac
    simp [unequal]
  · have lengths : left.length = right.length := by scalar_tac
    step with same_secret_loop_spec left right 0#u8 0#usize lengths (by scalar_tac)
      as ⟨ output, property ⟩
    simpa [tail_zero_iff left right lengths] using property

theorem same_secret_exact (left right : Slice U8) :
    same_secret left right = .ok (decide (left.val = right.val)) := by
  obtain ⟨result, run, property⟩ := spec_imp_exists (same_secret_spec left right)
  rw [run]
  cases result with
  | false =>
    have unequal : left.val ≠ right.val := by simpa using property.symm
    simp [unequal]
  | true =>
    have equal : left.val = right.val := property.mp rfl
    simp [equal]

theorem lifetime_same_secret_exact (left right : Slice U8) :
    Authority.lifetime_core.byte_core.same_secret left right =
      .ok (decide (left.val = right.val)) := by
  change same_secret left right = .ok (decide (left.val = right.val))
  exact same_secret_exact left right

theorem digest_matches_exact (actual registered : BytePolicy.Digest) :
    Authority.credential_core.digest_matches actual registered =
      .ok (decide (actual.val = registered.val)) := by
  simp [Authority.credential_core.digest_matches, lift, same_secret_exact, Array.to_slice]

theorem digest_safe : BytePolicy.DigestSafe Authority.credential_core.digest_matches := by
  intro actual registered permitted
  rw [digest_matches_exact] at permitted
  have equality := Result.ok_injective permitted
  simpa using equality

end ByteProofs
