import SandboxModel.RustRefinement.LifecycleCommon

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

@[step] theorem generated_insertUnique_copyLoop_generic
    (cloner : core.clone.Clone T) (clones : ∀ value, cloner.clone value = ok value)
    (values next : alloc.vec.Vec T)
    (capacity : next.val.length + values.val.length ≤ Std.Usize.max) :
    SandboxControllerRust.transition.insert_unique_loop0_loop0
        cloner
        values next 0#usize
      ⦃ result => result.val = next.val ++ values.val ⦄ := by
  unfold SandboxControllerRust.transition.insert_unique_loop0_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun state : alloc.vec.Vec T × Std.Usize =>
      values.val.length - state.2.val)
    (fun state : alloc.vec.Vec T × Std.Usize =>
      state.2.val ≤ values.val.length ∧
      state.1.val = next.val ++ values.val.take state.2.val)
    (fun (result : alloc.vec.Vec T) =>
      result.val = next.val ++ values.val)
  · intro state invariant
    rcases state with ⟨current, index⟩
    rcases invariant with ⟨indexBound, currentEq⟩
    simp only [Prod.fst, Prod.snd] at indexBound currentEq
    unfold SandboxControllerRust.transition.insert_unique_loop0_loop0.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨value, valuePost⟩
      rw [clones]
      have strict : current.val.length < Std.Usize.max := by
        have indexStrict : index.val < values.val.length := by simpa using inBounds
        rw [currentEq, List.length_append, List.length_take]
        simp only [Nat.min_eq_left indexBound]
        omega
      step as ⟨pushed, pushedPost⟩
      step as ⟨nextIndex, nextIndexPost⟩
      have indexStrict : index.val < values.val.length := by simpa using inBounds
      have takeStep : values.val.take (index.val + 1) =
          values.val.take index.val ++ [value] := by
        rw [List.take_succ_eq_append_getElem indexStrict]
        rw [← valuePost]
      constructor
      · scalar_tac
      · constructor
        · rw [nextIndexPost, pushedPost, currentEq, takeStep,
            List.append_assoc]
        · rw [nextIndexPost]
          omega
    · rename_i outOfBounds
      simp only [WP.spec_ok]
      have lower : values.val.length ≤ index.val := by simpa using outOfBounds
      have same : index.val = values.val.length := by omega
      simpa [same] using currentEq
  · exact ⟨by simp, by simp⟩

/-- The complete generated helper implements the model's prepend-if-absent
    list operation.  The explicit headroom premise is exactly the finite
    vector boundary of the absent branch. -/
@[step] theorem generated_insertUnique_generic [DecidableEq T]
    (cloner : core.clone.Clone T) (clones : ∀ value, cloner.clone value = ok value)
    (partialEq : core.cmp.PartialEq T T) (lawful : LawfulPartialEq partialEq)
    (values : alloc.vec.Vec T)
    (value : T)
    (headroom : VecHeadroom values) :
    SandboxControllerRust.transition.insert_unique
        cloner
        partialEq
        values value
      ⦃ result => result.val = SandboxModel.insertUnique value values.val ⦄ := by
  unfold SandboxControllerRust.transition.insert_unique
  unfold SandboxControllerRust.transition.insert_unique_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index : Std.Usize => values.val.length - index.val)
    (fun index : Std.Usize =>
      index.val ≤ values.val.length ∧ value ∉ values.val.take index.val)
    (fun (result : alloc.vec.Vec T) =>
      result.val = SandboxModel.insertUnique value values.val)
  · intro index invariant
    rcases invariant with ⟨indexBound, absentPrefix⟩
    unfold SandboxControllerRust.transition.insert_unique_loop0.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨stored, storedPost⟩
      have indexStrict : index.val < values.val.length := by
        simpa using inBounds
      have compare :
          partialEq.eq
              stored value = ok (decide (stored = value)) :=
        lawful stored value
      simp only [compare, bind_ok]
      by_cases same : stored = value
      · simp [same]
        simp only [WP.spec_ok, SandboxModel.insertUnique]
        have present : value ∈ values.val := by
          rw [← same, storedPost]
          exact List.getElem_mem (l := values.val) indexStrict
        simp [present]
      · simp [same]
        step as ⟨nextIndex, nextIndexPost⟩
        constructor
        · rw [nextIndexPost]
          omega
        · constructor
          · rw [nextIndexPost]
            rw [List.take_succ_eq_append_getElem indexStrict]
            rw [← storedPost]
            simp [absentPrefix]
            exact fun equal => same equal.symm
          · rw [nextIndexPost]
            omega
    · rename_i outOfBounds
      have lower : values.val.length ≤ index.val := by simpa using outOfBounds
      have sameIndex : index.val = values.val.length := by omega
      have absent : value ∉ values.val := by
        simpa [sameIndex] using absentPrefix
      have emptyRoom : (alloc.vec.Vec.new
          T).val.length < Std.Usize.max := by
        simp [alloc.vec.Vec.new]
        scalar_tac
      step as ⟨initial, initialPost⟩
      have initialLength : initial.val.length = 1 := by
        rw [initialPost]
        simp [alloc.vec.Vec.new]
      have copyRoom : initial.val.length + values.val.length ≤
          Std.Usize.max := by
        rw [initialLength]
        unfold VecHeadroom at headroom
        omega
      step as ⟨result, resultPost⟩
      simp only [WP.spec_ok, SandboxModel.insertUnique, absent, if_false]
      rw [resultPost, initialPost]
      simp
  · exact ⟨by simp, by simp⟩

end SandboxModel.RustRefinement
