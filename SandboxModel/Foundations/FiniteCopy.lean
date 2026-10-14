import SandboxModel.Foundations.Cloning

/-! Reusable finite copy loops, independent of generated business namespaces.
An implementation may use these contracts only after proving equality of its
actual extracted body (or a separately proved semantic connection).
CloneExact is caller-owned; the fixed pure Vec model excludes physical OOM,
custom Drop effects and failure rollback of externally visible state. -/

namespace SandboxModel.Foundations
open Aeneas Aeneas.Std Result ControlFlow

def vectorCopyBody (cloner : core.clone.Clone T) (values : alloc.vec.Vec T)
    (state : alloc.vec.Vec T × Usize) :
    Result (ControlFlow (alloc.vec.Vec T × Usize) (alloc.vec.Vec T)) := do
  if state.2 < alloc.vec.Vec.len values then
    let value ← alloc.vec.Vec.index (core.slice.index.SliceIndexUsizeSlice T) values state.2
    let copied ← cloner.clone value
    let pushed ← alloc.vec.Vec.push state.1 copied
    let next ← state.2 + 1#usize
    ok (.cont (pushed, next))
  else ok (.done state.1)

def vectorCopyLoop (cloner : core.clone.Clone T) (values prepared : alloc.vec.Vec T) :
    Result (alloc.vec.Vec T) :=
  loop (vectorCopyBody cloner values) (prepared, 0#usize)

@[step] theorem vectorCopyLoop_refines (cloner : core.clone.Clone T)
    (clones : CloneExact cloner) (values prepared : alloc.vec.Vec T)
    (capacity : prepared.val.length + values.val.length ≤ Usize.max) :
    vectorCopyLoop cloner values prepared
      ⦃ result => result.val = prepared.val ++ values.val ⦄ := by
  unfold vectorCopyLoop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun state : alloc.vec.Vec T × Usize => values.val.length - state.2.val)
    (fun state : alloc.vec.Vec T × Usize =>
      state.2.val ≤ values.val.length ∧
      state.1.val = prepared.val ++ values.val.take state.2.val)
    (fun (result : alloc.vec.Vec T) => result.val = prepared.val ++ values.val)
  · intro state invariant
    rcases state with ⟨current, index⟩
    rcases invariant with ⟨indexBound, currentEq⟩
    dsimp only [Prod.fst, Prod.snd] at indexBound currentEq
    unfold vectorCopyBody
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨value, valuePost⟩
      rw [clones]
      have indexStrict : index.val < values.val.length := by simpa using inBounds
      have strict : current.val.length < Usize.max := by
        rw [currentEq, List.length_append, List.length_take]
        simp only [Nat.min_eq_left indexBound]
        omega
      step as ⟨pushed, pushedPost⟩
      step as ⟨nextIndex, nextIndexPost⟩
      have takeStep : values.val.take (index.val + 1) =
          values.val.take index.val ++ [value] := by
        rw [List.take_succ_eq_append_getElem indexStrict, ← valuePost]
      constructor
      · scalar_tac
      · constructor
        · rw [nextIndexPost, pushedPost, currentEq, takeStep, List.append_assoc]
        · rw [nextIndexPost]
          omega
    · rename_i outside
      simp only [WP.spec_ok]
      have lower : values.val.length ≤ index.val := by simpa using outside
      have same : index.val = values.val.length := by omega
      simpa [same] using currentEq
  · exact ⟨by simp, by simp⟩

def vectorPrepend (cloner : core.clone.Clone T) (values : alloc.vec.Vec T) (value : T) :
    Result (alloc.vec.Vec T) := do
  let initial ← alloc.vec.Vec.push (alloc.vec.Vec.new T) value
  vectorCopyLoop cloner values initial

@[step] theorem vectorPrepend_refines (cloner : core.clone.Clone T)
    (clones : CloneExact cloner) (values : alloc.vec.Vec T) (value : T)
    (capacity : values.val.length < Usize.max) :
    vectorPrepend cloner values value ⦃ result => result.val = value :: values.val ⦄ := by
  unfold vectorPrepend
  have emptyRoom : (alloc.vec.Vec.new T).val.length < Usize.max := by
    simp [alloc.vec.Vec.new]
    scalar_tac
  step as ⟨initial, initialPost⟩
  have initialLength : initial.val.length = 1 := by
    rw [initialPost]
    simp [alloc.vec.Vec.new]
  have copyRoom : initial.val.length + values.val.length ≤ Usize.max := by
    rw [initialLength]
    omega
  step as ⟨result, resultPost⟩
  simpa [initialPost, alloc.vec.Vec.new] using resultPost

theorem vectorCopyLoop_transport (cloner : core.clone.Clone T)
    (clones : CloneExact cloner) (values prepared : alloc.vec.Vec T)
    (capacity : prepared.val.length + values.val.length ≤ Usize.max)
    (body : alloc.vec.Vec T × Usize →
      Result (ControlFlow (alloc.vec.Vec T × Usize) (alloc.vec.Vec T)))
    (connected : ∀ state, body state = vectorCopyBody cloner values state) :
    loop body (prepared, 0#usize) ⦃ result => result.val = prepared.val ++ values.val ⦄ := by
  rw [funext connected]
  exact vectorCopyLoop_refines cloner clones values prepared capacity

theorem vectorCopyBody_clone_failure (cloner : core.clone.Clone T)
    (values current : alloc.vec.Vec T) (index : Usize) (error : Error)
    (bound : index.val < values.val.length)
    (failed : cloner.clone values.val[index.val] = fail error) :
    vectorCopyBody cloner values (current, index) = fail error := by
  obtain ⟨stored, evaluated, post⟩ := WP.spec_imp_exists
    (alloc.vec.Vec.index_usize_spec values index bound)
  have inBounds : index < alloc.vec.Vec.len values := by simpa using bound
  simp [vectorCopyBody, inBounds, alloc.vec.Vec.index_slice_index,
    evaluated, post, failed]

theorem vectorPrepend_capacity_iff (values : alloc.vec.Vec T) :
    1 + values.val.length ≤ Usize.max ↔ values.val.length < Usize.max := by omega

end SandboxModel.Foundations
