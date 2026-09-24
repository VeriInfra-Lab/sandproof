import SandboxModel.RustRefinement.WorkerAdmissionChecks
import SandboxModel.RustRefinement.InsertUnique

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

theorem generated_put_preserves_defaultStoreRep
    [DecidableEq RustKey] [DecidableEq ModelKey]
    (partialEq : core.cmp.PartialEq RustKey RustKey) (lawful : LawfulPartialEq partialEq)
    (keyMap : RustKey → ModelKey) (keyMapInjective : Function.Injective keyMap)
    (valueRel : RustValue → ModelValue → Prop)
    (rust : SandboxControllerRust.store.Store RustKey RustValue)
    (model : ModelKey → ModelValue) (default : ModelValue)
    (rep : DefaultStoreRep keyMap valueRel rust model default)
    (key : RustKey) (value : RustValue) (mapped : ModelValue) (related : valueRel value mapped)
    (room : StoreHeadroom rust) :
    SandboxControllerRust.store.Store.put partialEq rust key value
      ⦃ next => DefaultStoreRep keyMap valueRel next
        (fun candidate => if candidate = keyMap key then mapped else model candidate) default ⦄ := by
  apply WP.spec_mono (generated_put_refines partialEq lawful rust key value room)
  intro next entries
  constructor
  · rw [entries]
    exact keys_associationPut_nodup key value rust.entries.val rep.keyUnique
  · intro candidate
    unfold storeLookup
    rw [entries, lookup_associationPut]
    by_cases same : candidate = key
    · subst candidate
      simp [related]
    · have different : keyMap candidate ≠ keyMap key := fun equal => same (keyMapInjective equal)
      simpa [same, different, storeLookup] using rep.lookupAgrees candidate
  · intro candidate nondefault
    by_cases same : candidate = keyMap key
    · exact ⟨key, same.symm⟩
    · exact rep.modelSupport candidate (by simpa [same] using nondefault)
  · rw [entries]
    have bound := length_associationPut_le key value rust.entries.val
    unfold StoreHeadroom at room
    omega

@[step] theorem generated_workerInsertUnique_copyLoop
    (cloner : core.clone.Clone T) (clones : ∀ value, cloner.clone value = ok value)
    (values next : alloc.vec.Vec T)
    (capacity : next.val.length + values.val.length ≤ Std.Usize.max) :
    SandboxControllerRust.worker.insert_unique_loop0_loop0
        cloner
        values 0#usize next
      ⦃ result => result.val = next.val ++ values.val ⦄ := by
  unfold SandboxControllerRust.worker.insert_unique_loop0_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun state : Std.Usize × alloc.vec.Vec T =>
      values.val.length - state.1.val)
    (fun state : Std.Usize × alloc.vec.Vec T =>
      state.1.val ≤ values.val.length ∧
      state.2.val = next.val ++ values.val.take state.1.val)
    (fun (result : alloc.vec.Vec T) =>
      result.val = next.val ++ values.val)
  · intro state invariant
    rcases state with ⟨index, current⟩
    rcases invariant with ⟨indexBound, currentEq⟩
    simp only [Prod.fst, Prod.snd] at indexBound currentEq
    unfold SandboxControllerRust.worker.insert_unique_loop0_loop0.body
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
@[step] theorem generated_workerInsertUnique_refines [DecidableEq T]
    (cloner : core.clone.Clone T) (clones : ∀ value, cloner.clone value = ok value)
    (partialEq : core.cmp.PartialEq T T) (lawful : LawfulPartialEq partialEq)
    (values : alloc.vec.Vec T)
    (value : T)
    (headroom : VecHeadroom values) :
    SandboxControllerRust.worker.insert_unique
        cloner
        partialEq
        values value
      ⦃ result => result.val = SandboxModel.insertUnique value values.val ⦄ := by
  unfold SandboxControllerRust.worker.insert_unique
  unfold SandboxControllerRust.worker.insert_unique_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index : Std.Usize => values.val.length - index.val)
    (fun index : Std.Usize =>
      index.val ≤ values.val.length ∧ value ∉ values.val.take index.val)
    (fun (result : alloc.vec.Vec T) =>
      result.val = SandboxModel.insertUnique value values.val)
  · intro index invariant
    rcases invariant with ⟨indexBound, absentPrefix⟩
    unfold SandboxControllerRust.worker.insert_unique_loop0.body
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
