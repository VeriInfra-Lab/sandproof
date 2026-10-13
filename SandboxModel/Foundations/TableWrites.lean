import SandboxModel.Foundations.Containers

namespace SandboxModel.Foundations
open Aeneas Aeneas.Std Result ControlFlow SandproofFoundationsRust

theorem table_lookup_put [DecidableEq K]
    (query key : K) (value : V) (entries : List (K × V)) :
    tableLookup query (tablePut key value entries) =
      if query = key then some value else tableLookup query entries := by
  induction entries with
  | nil => simp [tablePut, tableLookup]
  | cons entry rest ih =>
    rcases entry with ⟨stored, old⟩
    by_cases same : stored = key
    · subst stored
      by_cases hit : query = key <;> simp [tablePut, tableLookup, hit]
    · by_cases hit : query = stored
      · simp [tablePut, tableLookup, same, hit]
      · simp [tablePut, tableLookup, same, hit, ih]

theorem table_put_keys [DecidableEq K]
    (key : K) (value : V) (entries : List (K × V)) :
    (tablePut key value entries).map Prod.fst =
      if key ∈ entries.map Prod.fst then entries.map Prod.fst
      else entries.map Prod.fst ++ [key] := by
  induction entries with
  | nil => simp [tablePut]
  | cons entry rest ih =>
    rcases entry with ⟨stored, old⟩
    by_cases same : stored = key
    · subst stored; simp [tablePut]
    · by_cases member : key ∈ rest.map Prod.fst
      · simp [tablePut, same, ih, member]
      · have reverse : key ≠ stored := fun equal => same equal.symm
        simp [tablePut, same, ih, member, reverse]

theorem table_put_preserves_unique [DecidableEq K]
    (key : K) (value : V) (entries : List (K × V))
    (unique : (entries.map Prod.fst).Nodup) :
    ((tablePut key value entries).map Prod.fst).Nodup := by
  rw [table_put_keys]
  by_cases member : key ∈ entries.map Prod.fst
  · simp [member, unique]
  · simp only [member, ↓reduceIte]
    rw [List.nodup_append]
    refine ⟨unique, by simp, ?_⟩
    intro existing existingMem appended appendedMem
    simp only [List.mem_singleton] at appendedMem
    subst appended
    intro equal
    apply member
    simpa [equal] using existingMem

theorem table_put_length_present [DecidableEq K]
    (key : K) (value : V) (entries : List (K × V)) (old : V)
    (present : tableLookup key entries = some old) :
    (tablePut key value entries).length = entries.length := by
  induction entries with
  | nil => simp [tableLookup] at present
  | cons entry rest ih =>
    rcases entry with ⟨stored, oldValue⟩
    by_cases same : stored = key
    · simp [tablePut, same]
    · have reverse : key ≠ stored := fun equal => same equal.symm
      have tailPresent : tableLookup key rest = some old := by
        simpa [tableLookup, reverse] using present
      simp [tablePut, same, ih tailPresent]

theorem table_put_length_absent [DecidableEq K]
    (key : K) (value : V) (entries : List (K × V))
    (absent : tableLookup key entries = none) :
    (tablePut key value entries).length = entries.length + 1 := by
  induction entries with
  | nil => simp [tablePut]
  | cons entry rest ih =>
    rcases entry with ⟨stored, old⟩
    by_cases same : stored = key
    · subst stored; simp [tableLookup] at absent
    · have reverse : key ≠ stored := fun equal => same equal.symm
      have tailAbsent : tableLookup key rest = none := by
        simpa [tableLookup, reverse] using absent
      simp [tablePut, same, ih tailAbsent]

-- A replacement has no append requirement. Allocator/OOM behavior is outside
-- the fixed pure Vec model; this only states logical representability.
def TableWriteCapacity [DecidableEq K] (entries : List (K × V)) (key : K) : Prop :=
  tableLookup key entries ≠ none ∨ entries.length < Usize.max

theorem table_put_loop_refines [DecidableEq K]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (store : sandbox_controller_core.store.Store K V) (key : K) (value : V)
    (start : Usize) (bound : start.val ≤ store.entries.val.length)
    (capacity : tableLookup key (store.entries.val.drop start.val) ≠ none ∨
      store.entries.val.length < Usize.max) :
    sandbox_controller_core.store.Store.put_loop eq store key value start
      ⦃ entries => entries.val = store.entries.val.take start.val ++
        tablePut key value (store.entries.val.drop start.val) ⦄ := by
  unfold sandbox_controller_core.store.Store.put_loop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun (index : Usize) => store.entries.val.length - index.val)
    (fun (index : Usize) => index.val ≤ store.entries.val.length ∧
      (store.entries.val.take index.val ++ tablePut key value (store.entries.val.drop index.val) =
        store.entries.val.take start.val ++ tablePut key value (store.entries.val.drop start.val)) ∧
      (tableLookup key (store.entries.val.drop index.val) ≠ none ∨
        store.entries.val.length < Usize.max))
    (fun (entries : alloc.vec.Vec (K × V)) => entries.val = store.entries.val.take start.val ++
      tablePut key value (store.entries.val.drop start.val))
  · intro index invariant
    unfold sandbox_controller_core.store.Store.put_loop.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨stored, old, entryPost⟩
      have indexBound : index.val < store.entries.val.length := by simpa using inBounds
      have dropEq : store.entries.val.drop index.val =
          (stored, old) :: store.entries.val.drop (index.val + 1) := by
        rw [List.drop_eq_getElem_cons indexBound, ← entryPost]
      have takeEq : store.entries.val.take (index.val + 1) =
          store.entries.val.take index.val ++ [(stored, old)] := by
        rw [List.take_succ_eq_append_getElem indexBound, ← entryPost]
      rw [lawful stored key]
      by_cases same : stored = key
      · simp [same]
        step
        have sameKey : t1 = stored := congrArg Prod.fst (t1_post.trans entryPost.symm)
        subst t1
        rw [index_mut_back_post]
        rcases invariant with ⟨_, updateEq, _⟩
        rw [dropEq] at updateEq
        simpa [alloc.vec.Vec.set, List.set_eq_take_append_cons_drop,
          indexBound, tablePut, same] using updateEq
      · simp [same]
        step as ⟨next, nextPost⟩
        rcases invariant with ⟨_, updateEq, remainingCapacity⟩
        have reverse : key ≠ stored := fun equal => same equal.symm
        constructor
        · scalar_tac
        · constructor
          · rw [nextPost, takeEq]
            rw [dropEq] at updateEq
            simpa [tablePut, same, List.append_assoc] using updateEq
          · constructor
            · rw [nextPost]
              rw [dropEq] at remainingCapacity
              simpa [tableLookup, reverse] using remainingCapacity
            · omega
    · rename_i outside
      rcases invariant with ⟨upper, updateEq, remainingCapacity⟩
      have lower : store.entries.val.length ≤ index.val := by simpa using outside
      have atEnd : index.val = store.entries.val.length := by omega
      have appendCapacity : store.entries.val.length < Usize.max := by
        simpa [atEnd, tableLookup] using remainingCapacity
      step as ⟨appended, appendedPost⟩
      rw [appendedPost]
      calc
        store.entries.val ++ [(key, value)] = store.entries.val.take index.val ++
            tablePut key value (store.entries.val.drop index.val) := by
          rw [atEnd]; simp [tablePut]
        _ = store.entries.val.take start.val ++
            tablePut key value (store.entries.val.drop start.val) := updateEq
  · exact ⟨bound, rfl, capacity⟩

theorem table_put_refines [DecidableEq K]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (store : sandbox_controller_core.store.Store K V) (key : K) (value : V)
    (capacity : TableWriteCapacity store.entries.val key) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => next.entries.val = tablePut key value store.entries.val ⦄ := by
  unfold sandbox_controller_core.store.Store.put
  apply Aeneas.Std.WP.spec_bind
    (table_put_loop_refines eq lawful store key value 0#usize (by simp) (by
      simpa [TableWriteCapacity] using capacity))
  intro entries entriesPost
  simpa using entriesPost

end SandboxModel.Foundations
