import SandboxModel.Foundations.FiniteCopy

/-! First-match short-circuit and prepend-if-absent contracts. The existing
branch needs neither append capacity nor any clone contract. The absent branch
requires caller-proved equality, value-preserving cloning and finite headroom.
The generic body is not a proof of a Rust helper until its actual extracted
body and wrapper are connected, e.g. with vectorInsert_transport. -/

namespace SandboxModel.Foundations
open Aeneas Aeneas.Std Result ControlFlow

def prependIfAbsent [DecidableEq T] (value : T) (values : List T) : List T :=
  if value ∈ values then values else value :: values

def ValueEqExact [DecidableEq T] (dictionary : core.cmp.PartialEq T T) : Prop :=
  ∀ left right, dictionary.eq left right = ok (decide (left = right))

def vectorInsertBody (cloner : core.clone.Clone T) (eq : core.cmp.PartialEq T T)
    (values : alloc.vec.Vec T) (value : T) (index : Usize) :
    Result (ControlFlow Usize (alloc.vec.Vec T)) := do
  if index < alloc.vec.Vec.len values then
    let stored ← alloc.vec.Vec.index (core.slice.index.SliceIndexUsizeSlice T) values index
    let same ← eq.eq stored value
    if same then ok (.done values)
    else do
      let next ← index + 1#usize
      ok (.cont next)
  else do
    let updated ← vectorPrepend cloner values value
    ok (.done updated)

def vectorInsert (cloner : core.clone.Clone T) (eq : core.cmp.PartialEq T T)
    (values : alloc.vec.Vec T) (value : T) : Result (alloc.vec.Vec T) :=
  loop (vectorInsertBody cloner eq values value) 0#usize

@[step] theorem vectorInsert_refines [DecidableEq T] (cloner : core.clone.Clone T)
    (values : alloc.vec.Vec T) (value : T)
    (clones : value ∉ values.val → CloneExact cloner)
    (eq : core.cmp.PartialEq T T) (lawful : ValueEqExact eq)
    (capacity : value ∈ values.val ∨ values.val.length < Usize.max) :
    vectorInsert cloner eq values value
      ⦃ result => result.val = prependIfAbsent value values.val ⦄ := by
  unfold vectorInsert
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index : Usize => values.val.length - index.val)
    (fun index : Usize => index.val ≤ values.val.length ∧ value ∉ values.val.take index.val)
    (fun (result : alloc.vec.Vec T) => result.val = prependIfAbsent value values.val)
  · intro index invariant
    rcases invariant with ⟨indexBound, absentPrefix⟩
    unfold vectorInsertBody
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨stored, storedPost⟩
      have indexStrict : index.val < values.val.length := by simpa using inBounds
      simp only [lawful stored value]
      by_cases same : stored = value
      · simp [same]
        simp only [prependIfAbsent]
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
          · rw [nextIndexPost, List.take_succ_eq_append_getElem indexStrict, ← storedPost]
            simp [absentPrefix]
            exact fun equal => same equal.symm
          · rw [nextIndexPost]
            omega
    · rename_i outside
      have lower : values.val.length ≤ index.val := by simpa using outside
      have sameIndex : index.val = values.val.length := by omega
      have absent : value ∉ values.val := by simpa [sameIndex] using absentPrefix
      have room : values.val.length < Usize.max := capacity.resolve_left absent
      have exactClone : CloneExact cloner := clones absent
      step as ⟨result, resultPost⟩
      simp only [prependIfAbsent, absent, if_false]
      exact resultPost
  · exact ⟨by simp, by simp⟩

theorem vectorInsert_transport [DecidableEq T] (cloner : core.clone.Clone T)
    (values : alloc.vec.Vec T) (value : T)
    (clones : value ∉ values.val → CloneExact cloner)
    (eq : core.cmp.PartialEq T T) (lawful : ValueEqExact eq)
    (capacity : value ∈ values.val ∨ values.val.length < Usize.max)
    (body : Usize → Result (ControlFlow Usize (alloc.vec.Vec T)))
    (connected : ∀ index, body index = vectorInsertBody cloner eq values value index) :
    loop body 0#usize ⦃ result => result.val = prependIfAbsent value values.val ⦄ := by
  rw [funext connected]
  exact vectorInsert_refines cloner values value clones eq lawful capacity

theorem vectorInsert_present_refines [DecidableEq T] (cloner : core.clone.Clone T)
    (eq : core.cmp.PartialEq T T) (lawful : ValueEqExact eq)
    (values : alloc.vec.Vec T) (value : T) (present : value ∈ values.val) :
    vectorInsert cloner eq values value ⦃ result => result.val = values.val ⦄ := by
  simpa [prependIfAbsent, present] using vectorInsert_refines cloner values value
    (fun absent => False.elim (absent present)) eq lawful (Or.inl present)

theorem vectorInsert_absent_refines [DecidableEq T] (cloner : core.clone.Clone T)
    (clones : CloneExact cloner) (eq : core.cmp.PartialEq T T) (lawful : ValueEqExact eq)
    (values : alloc.vec.Vec T) (value : T) (absent : value ∉ values.val)
    (capacity : values.val.length < Usize.max) :
    vectorInsert cloner eq values value ⦃ result => result.val = value :: values.val ⦄ := by
  simpa [prependIfAbsent, absent] using vectorInsert_refines cloner values value
    (fun _ => clones) eq lawful (Or.inr capacity)

theorem prependIfAbsent_present [DecidableEq T] (value : T) (values : List T)
    (present : value ∈ values) : prependIfAbsent value values = values := by
  simp [prependIfAbsent, present]

theorem prependIfAbsent_absent [DecidableEq T] (value : T) (values : List T)
    (absent : value ∉ values) : prependIfAbsent value values = value :: values := by
  simp [prependIfAbsent, absent]

theorem prependIfAbsent_membership [DecidableEq T] (value query : T) (values : List T) :
    query ∈ prependIfAbsent value values ↔ query = value ∨ query ∈ values := by
  by_cases present : value ∈ values
  · simp only [prependIfAbsent, present, if_true]
    constructor
    · exact Or.inr
    · rintro (same | old)
      · simpa [same] using present
      · exact old
  · simp [prependIfAbsent, present]

theorem prependIfAbsent_length [DecidableEq T] (value : T) (values : List T) :
    (prependIfAbsent value values).length =
      if value ∈ values then values.length else values.length + 1 := by
  by_cases present : value ∈ values <;> simp [prependIfAbsent, present]

theorem prependIfAbsent_unique [DecidableEq T] (value : T) (values : List T)
    (unique : values.Nodup) : (prependIfAbsent value values).Nodup := by
  by_cases present : value ∈ values <;> simp [prependIfAbsent, present, unique]

theorem prependIfAbsent_idempotent [DecidableEq T] (value : T) (values : List T) :
    prependIfAbsent value (prependIfAbsent value values) = prependIfAbsent value values := by
  apply prependIfAbsent_present
  exact (prependIfAbsent_membership value value values).mpr (Or.inl rfl)

theorem prependIfAbsent_view [DecidableEq T] [DecidableEq M]
    (view : T → M) (injective : Function.Injective view) (value : T) (values : List T) :
    (prependIfAbsent value values).map view = prependIfAbsent (view value) (values.map view) := by
  have membership : view value ∈ values.map view ↔ value ∈ values := by
    simp only [List.mem_map]
    constructor
    · rintro ⟨stored, member, same⟩
      simpa [injective same] using member
    · intro member
      exact ⟨value, member, rfl⟩
  by_cases present : value ∈ values
  · simp [prependIfAbsent, present, membership]
  · simp [prependIfAbsent, present, membership]

theorem vectorInsert_init_realizable [DecidableEq T] (cloner : core.clone.Clone T)
    (clones : CloneExact cloner) (eq : core.cmp.PartialEq T T) (lawful : ValueEqExact eq)
    (value : T) :
    ∃ result, vectorInsert cloner eq (alloc.vec.Vec.new T) value = ok result ∧
      result.val = [value] := by
  obtain ⟨result, evaluated, post⟩ := WP.spec_imp_exists
    (vectorInsert_absent_refines cloner clones eq lawful (alloc.vec.Vec.new T) value
      (by simp [alloc.vec.Vec.new]) (by simp [alloc.vec.Vec.new]; scalar_tac))
  exact ⟨result, evaluated, by simpa [alloc.vec.Vec.new] using post⟩

theorem vectorInsertBody_match_shortcircuits (cloner : core.clone.Clone T)
    (eq : core.cmp.PartialEq T T) (values : alloc.vec.Vec T) (value : T) (index : Usize)
    (bound : index.val < values.val.length)
    (matched : eq.eq values.val[index.val] value = ok true) :
    vectorInsertBody cloner eq values value index ⦃ result => result = .done values ⦄ := by
  unfold vectorInsertBody
  have inBounds : index < alloc.vec.Vec.len values := by simpa using bound
  simp only [inBounds, if_true]
  rw [alloc.vec.Vec.index_slice_index]
  step as ⟨stored, storedPost⟩
  rw [storedPost, matched]
  simp

theorem vectorInsertBody_compare_failure (cloner : core.clone.Clone T)
    (eq : core.cmp.PartialEq T T) (values : alloc.vec.Vec T) (value : T) (index : Usize)
    (error : Error) (bound : index.val < values.val.length)
    (failed : eq.eq values.val[index.val] value = fail error) :
    vectorInsertBody cloner eq values value index = fail error := by
  obtain ⟨stored, evaluated, post⟩ := WP.spec_imp_exists
    (alloc.vec.Vec.index_usize_spec values index bound)
  have inBounds : index < alloc.vec.Vec.len values := by simpa using bound
  simp [vectorInsertBody, inBounds, alloc.vec.Vec.index_slice_index,
    evaluated, post, failed]

end SandboxModel.Foundations
