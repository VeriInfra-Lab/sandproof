import Aeneas

/-! A terminating finite scan, used only after proving an actual extracted
loop body is equal to this body.  No external loop or termination assumption. -/

namespace SandboxModel.Foundations

open Aeneas Aeneas.Std Result

def vectorAllBody (values : alloc.vec.Vec T) (predicate : T → Bool)
    (cursor : Std.Usize) : Result (ControlFlow Std.Usize Bool) :=
  if bound : cursor.val < values.val.length then
    if predicate values.val[cursor.val] then do
      let next ← cursor + 1#usize
      ok (.cont next)
    else ok (.done false)
  else ok (.done true)

theorem vectorAllLoop_refines (values : alloc.vec.Vec T) (predicate : T → Bool)
    (start : Std.Usize) (startBound : start.val ≤ values.val.length) :
    loop (vectorAllBody values predicate) start
      ⦃ found => found = (values.val.drop start.val).all predicate ⦄ := by
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index => values.val.length - index.val)
    (fun index => index.val ≤ values.val.length ∧
      (values.val.drop index.val).all predicate = (values.val.drop start.val).all predicate)
    (fun found => found = (values.val.drop start.val).all predicate)
  · intro index invariant
    unfold vectorAllBody
    split
    · rename_i bound
      split
      · rename_i holds
        step as ⟨next, nextPost⟩
        rcases invariant with ⟨upper, same⟩
        constructor
        · scalar_tac
        · constructor
          · rw [List.drop_eq_getElem_cons bound] at same
            simpa only [List.all_cons, holds, Bool.true_and, nextPost] using same
          · omega
      · rename_i fails
        simp only [WP.spec_ok]
        rcases invariant with ⟨_, same⟩
        rw [List.drop_eq_getElem_cons bound] at same
        have failsFalse : predicate values.val[index.val] = false := Bool.eq_false_iff.mpr fails
        simpa only [List.all_cons, failsFalse, Bool.false_and] using same
    · rename_i outside
      simp only [WP.spec_ok]
      rcases invariant with ⟨_, same⟩
      rw [List.drop_eq_nil_of_le (Nat.le_of_not_gt outside)] at same
      simpa using same
  · exact ⟨startBound, rfl⟩

def vectorAnyBody (values : alloc.vec.Vec T) (predicate : T → Bool)
    (cursor : Std.Usize) : Result (ControlFlow Std.Usize Bool) :=
  if bound : cursor.val < values.val.length then
    if predicate values.val[cursor.val] then ok (.done true)
    else do
      let next ← cursor + 1#usize
      ok (.cont next)
  else ok (.done false)

theorem vectorAnyLoop_refines (values : alloc.vec.Vec T) (predicate : T → Bool)
    (start : Std.Usize) (startBound : start.val ≤ values.val.length) :
    loop (vectorAnyBody values predicate) start
      ⦃ found => found = (values.val.drop start.val).any predicate ⦄ := by
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index => values.val.length - index.val)
    (fun index => index.val ≤ values.val.length ∧
      (values.val.drop index.val).any predicate = (values.val.drop start.val).any predicate)
    (fun found => found = (values.val.drop start.val).any predicate)
  · intro index invariant
    unfold vectorAnyBody
    split
    · rename_i bound
      split
      · rename_i holds
        simp only [WP.spec_ok]
        rcases invariant with ⟨_, same⟩
        rw [List.drop_eq_getElem_cons bound] at same
        simpa only [List.any_cons, holds, Bool.true_or] using same
      · rename_i fails
        step as ⟨next, nextPost⟩
        rcases invariant with ⟨upper, same⟩
        constructor
        · scalar_tac
        · constructor
          · rw [List.drop_eq_getElem_cons bound] at same
            have failsFalse : predicate values.val[index.val] = false := Bool.eq_false_iff.mpr fails
            simpa only [List.any_cons, failsFalse, Bool.false_or, nextPost] using same
          · omega
    · rename_i outside
      simp only [WP.spec_ok]
      rcases invariant with ⟨_, same⟩
      rw [List.drop_eq_nil_of_le (Nat.le_of_not_gt outside)] at same
      simpa using same
  · exact ⟨startBound, rfl⟩

end SandboxModel.Foundations
