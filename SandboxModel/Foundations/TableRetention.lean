import SandboxModel.Foundations.TableWriteContracts

/-! Shared finite-table deletion contracts. Concrete retirement authorization,
keys, time and callback laws remain the calling module's obligations. -/
namespace SandboxModel.Foundations

theorem table_retain_membership (keep : K → V → Bool) (entries : List (K × V)) (entry : K × V) :
    entry ∈ tableRetain keep entries ↔ entry ∈ entries ∧ keep entry.1 entry.2 = true := by
  simp [tableRetain]

theorem table_retain_order (keep : K → V → Bool) (entries : List (K × V)) :
    (tableRetain keep entries).Sublist entries := by
  exact List.filter_sublist

theorem table_retain_length (keep : K → V → Bool) (entries : List (K × V)) :
    (tableRetain keep entries).length ≤ entries.length := by
  exact (table_retain_order keep entries).length_le

theorem table_retain_preserves_unique (keep : K → V → Bool) (entries : List (K × V))
    (unique : (entries.map Prod.fst).Nodup) :
    ((tableRetain keep entries).map Prod.fst).Nodup := by
  exact unique.sublist ((table_retain_order keep entries).map Prod.fst)

theorem table_retain_idempotent (keep : K → V → Bool) (entries : List (K × V)) :
    tableRetain keep (tableRetain keep entries) = tableRetain keep entries := by
  simp [tableRetain, List.filter_filter]

theorem table_retain_preserves_row_invariant (keep : K → V → Bool) (entries : List (K × V))
    (invariant : K → V → Prop) (valid : ∀ k v, (k, v) ∈ entries → invariant k v) :
    ∀ k v, (k, v) ∈ tableRetain keep entries → invariant k v := by
  intro key value member
  exact valid key value ((table_retain_membership keep entries (key, value)).mp member).1

theorem table_retain_view (keyView : K → MK) (valueView : V → MV)
    (keep : K → V → Bool) (modelKeep : MK → MV → Bool)
    (compatible : ∀ key value, modelKeep (keyView key) (valueView value) = keep key value)
    (entries : List (K × V)) :
    tableValueView keyView valueView (tableRetain keep entries) =
      tableRetain modelKeep (tableValueView keyView valueView entries) := by
  induction entries with
  | nil => rfl
  | cons entry rest ih =>
    rcases entry with ⟨key, value⟩
    cases decision : keep key value
    · simpa [tableRetain, tableValueView, compatible, decision] using ih
    · simpa [tableRetain, tableValueView, compatible, decision] using
        congrArg (fun tail => (keyView key, valueView value) :: tail) ih

theorem table_retain_query_frame [DecidableEq K] (keep : K → V → Bool)
    (entries : List (K × V)) (query : K)
    (kept : ∀ value, (query, value) ∈ entries → keep query value = true) :
    tableLookup query (tableRetain keep entries) = tableLookup query entries := by
  induction entries with
  | nil => rfl
  | cons entry rest ih =>
    rcases entry with ⟨key, value⟩
    have restKept : ∀ v, (query, v) ∈ rest → keep query v = true :=
      fun v member => kept v (List.mem_cons_of_mem _ member)
    by_cases same : query = key
    · subst key
      have thisKept := kept value (by simp)
      simp [tableRetain, tableLookup, thisKept]
    · cases decision : keep key value <;>
        simpa [tableRetain, tableLookup, same, decision] using ih restKept

theorem table_retain_query_removed [DecidableEq K] (keep : K → V → Bool)
    (entries : List (K × V)) (query : K)
    (removed : ∀ value, (query, value) ∈ entries → keep query value = false) :
    tableLookup query (tableRetain keep entries) = none := by
  apply (table_lookup_none_iff _ _).mpr
  intro value member
  have info := (table_retain_membership keep entries (query, value)).mp member
  have no := removed value info.1
  simp [no] at info

theorem table_delete_target_absent [DecidableEq K] (key : K) (entries : List (K × V)) :
    tableLookup key (tableDelete key entries) = none := by
  apply table_retain_query_removed
  intro value _
  simp

theorem table_delete_other_query_unchanged [DecidableEq K] (key query : K)
    (different : query ≠ key) (entries : List (K × V)) :
    tableLookup query (tableDelete key entries) = tableLookup query entries := by
  apply table_retain_query_frame
  intro value _
  simp [different]

theorem table_delete_idempotent [DecidableEq K] (key : K) (entries : List (K × V)) :
    tableDelete key (tableDelete key entries) = tableDelete key entries := by
  exact table_retain_idempotent _ _

theorem table_delete_length [DecidableEq K] (key : K) (entries : List (K × V)) :
    (tableDelete key entries).length ≤ entries.length := by
  exact table_retain_length _ _

theorem table_retain_state_length (step : S → K → V → Bool × S) (state : S)
    (entries : List (K × V)) :
    (tableRetainState step state entries).1.length ≤ entries.length := by
  induction entries generalizing state with
  | nil => simp [tableRetainState]
  | cons entry rest ih =>
    rcases entry with ⟨key, value⟩
    rcases decision : step state key value with ⟨keep, next⟩
    cases keep
    · simpa [tableRetainState, decision] using Nat.le_succ_of_le (ih next)
    · simpa [tableRetainState, decision] using ih next

theorem table_retain_state_order (step : S → K → V → Bool × S) (state : S)
    (entries : List (K × V)) :
    (tableRetainState step state entries).1.Sublist entries := by
  induction entries generalizing state with
  | nil => simp [tableRetainState]
  | cons entry rest ih =>
    rcases entry with ⟨key, value⟩
    rcases decision : step state key value with ⟨keep, next⟩
    cases keep
    · simpa [tableRetainState, decision] using (ih next).cons (key, value)
    · simpa [tableRetainState, decision] using (ih next).cons_cons (key, value)

theorem table_retain_state_pure (keep : K → V → Bool) (state : S) (entries : List (K × V)) :
    tableRetainState (fun s k v => (keep k v, s)) state entries =
      (tableRetain keep entries, state) := by
  induction entries with
  | nil => rfl
  | cons entry rest ih =>
    rcases entry with ⟨key, value⟩
    cases decision : keep key value <;> simp [tableRetainState, tableRetain, ih, decision]

theorem table_retain_state_read_only (keep : S → K → V → Bool) (state : S)
    (entries : List (K × V)) :
    tableRetainState (fun s k v => (keep s k v, s)) state entries =
      (tableRetain (keep state) entries, state) := by
  induction entries with
  | nil => rfl
  | cons entry rest ih =>
    rcases entry with ⟨key, value⟩
    cases decision : keep state key value <;>
      simp [tableRetainState, tableRetain, ih, decision]

open Aeneas Aeneas.Std SandproofFoundationsRust

theorem table_retain_list_refines_state (call : S → (K × V) → Result (Bool × S))
    (step : S → K → V → Bool × S)
    (lawful : ∀ s k v, call s (k, v) = .ok (step s k v))
    (state : S) (entries : List (K × V)) :
    retainList call state entries = .ok (tableRetainState step state entries).1 := by
  induction entries generalizing state with
  | nil => rfl
  | cons entry rest ih =>
    rcases entry with ⟨key, value⟩
    rcases decision : step state key value with ⟨keep, next⟩
    simp [retainList, tableRetainState, lawful, decision, ih]

theorem table_retain_refines_state (eq : core.cmp.PartialEq K K)
    (callback : core.ops.function.FnMut S (K × V) Bool)
    (step : S → K → V → Bool × S)
    (lawful : ∀ s k v, callback.call_mut s (k, v) = .ok (step s k v))
    (store : sandbox_controller_core.store.Store K V) (state : S) :
    ∃ next, sandbox_controller_core.store.Store.retain eq callback store state = .ok next ∧
      next.entries.val = (tableRetainState step state store.entries.val).1 := by
  have bound : (tableRetainState step state store.entries.val).1.length ≤ Usize.max :=
    (table_retain_state_length step state store.entries.val).trans store.entries.property
  have wrappedLaw : ∀ s k v,
      sandbox_controller_core.store.Store.retain.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairBool.call_mut
        eq callback s (k, v) = .ok (step s k v) := by
    intro s k v
    rcases decision : step s k v with ⟨keep, next⟩
    simp [sandbox_controller_core.store.Store.retain.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairBool.call_mut,
      lawful, decision]
  have listResult := table_retain_list_refines_state
    (sandbox_controller_core.store.Store.retain.closure.Insts.CoreOpsFunctionFnMutTupleSharedPairBool.call_mut
      eq callback) step wrappedLaw state store.entries.val
  refine ⟨⟨Aeneas.Std.alloc.vec.Vec.from (tableRetainState step state store.entries.val).1 bound⟩,
    ?_, by simp⟩
  unfold sandbox_controller_core.store.Store.retain SandproofFoundationsRust.alloc.vec.Vec.retain
  rw [listResult]
  simp [bound]

theorem table_retain_refines (eq : core.cmp.PartialEq K K)
    (callback : core.ops.function.FnMut S (K × V) Bool)
    (keep : K → V → Bool)
    (lawful : ∀ s k v, callback.call_mut s (k, v) = .ok (keep k v, s))
    (store : sandbox_controller_core.store.Store K V) (state : S) :
    ∃ next, sandbox_controller_core.store.Store.retain eq callback store state = .ok next ∧
      next.entries.val = tableRetain keep store.entries.val := by
  obtain ⟨next, evaluated, retained⟩ := table_retain_refines_state eq callback
    (fun s k v => (keep k v, s)) lawful store state
  exact ⟨next, evaluated, by simpa [table_retain_state_pure] using retained⟩

theorem table_u64_retain_above_refines (store : sandbox_controller_core.store.Store U64 U64) (floor : U64) :
    ∃ next, store_retain_above store floor = .ok next ∧
      next.entries.val = tableRetain (fun (_ : U64) value => decide (floor.val < value.val))
        store.entries.val := by
  obtain ⟨next, evaluated, retained⟩ := table_retain_refines_state core.cmp.PartialEqU64
    store_retain_above.closure.Insts.CoreOpsFunctionFnMutPairSharedU64SharedU64Bool
    (fun (s : U64) (_ : U64) value => (decide (s.val < value.val), s)) (by
      intro s k v
      simp [store_retain_above.closure.Insts.CoreOpsFunctionFnMutPairSharedU64SharedU64Bool.call_mut])
    store floor
  exact ⟨next, by simpa [store_retain_above] using evaluated,
    by simpa [table_retain_state_read_only] using retained⟩

theorem table_retain_preserves_rows_refines (eq : core.cmp.PartialEq K K)
    (callback : core.ops.function.FnMut S (K × V) Bool)
    (step : S → K → V → Bool × S)
    (lawful : ∀ s k v, callback.call_mut s (k, v) = .ok (step s k v))
    (store : sandbox_controller_core.store.Store K V) (state : S) :
    ∃ next, sandbox_controller_core.store.Store.retain eq callback store state = .ok next ∧
      next.entries.val.Sublist store.entries.val := by
  obtain ⟨next, evaluated, retained⟩ := table_retain_refines_state eq callback step lawful store state
  exact ⟨next, evaluated, by rw [retained]; exact table_retain_state_order step state _⟩

theorem table_delete_refines [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (callback : core.ops.function.FnMut S (K × V) Bool) (target : K)
    (lawful : ∀ s k v, callback.call_mut s (k, v) = .ok (decide (k ≠ target), s))
    (store : sandbox_controller_core.store.Store K V) (state : S) :
    ∃ next, sandbox_controller_core.store.Store.retain eq callback store state = .ok next ∧
      next.entries.val = tableDelete target store.entries.val := by
  exact table_retain_refines eq callback (fun k _ => decide (k ≠ target)) lawful store state

theorem table_delete_query_frame_refines [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (callback : core.ops.function.FnMut S (K × V) Bool) (target : K)
    (lawful : ∀ s k v, callback.call_mut s (k, v) = .ok (decide (k ≠ target), s))
    (store : sandbox_controller_core.store.Store K V) (state : S) :
    ∃ next, sandbox_controller_core.store.Store.retain eq callback store state = .ok next ∧
      ∀ query, tableLookup query next.entries.val =
        if query = target then none else tableLookup query store.entries.val := by
  obtain ⟨next, evaluated, retained⟩ := table_delete_refines eq callback target lawful store state
  refine ⟨next, evaluated, ?_⟩
  intro query
  rw [retained]
  by_cases same : query = target
  · subst query
    simp [table_delete_target_absent]
  · simp [same, table_delete_other_query_unchanged target query same]

theorem table_retain_represented_refines (eq : core.cmp.PartialEq K K)
    (callback : core.ops.function.FnMut S (K × V) Bool) (keep : K → V → Bool)
    (lawful : ∀ s k v, callback.call_mut s (k, v) = .ok (keep k v, s))
    (keyView : K → MK) (valueView : V → MV) (modelKeep : MK → MV → Bool)
    (compatible : ∀ k v, modelKeep (keyView k) (valueView v) = keep k v)
    (store : sandbox_controller_core.store.Store K V) (state : S)
    (model : List (MK × MV)) (represented : tableValueView keyView valueView store.entries.val = model) :
    ∃ next, sandbox_controller_core.store.Store.retain eq callback store state = .ok next ∧
      tableValueView keyView valueView next.entries.val = tableRetain modelKeep model := by
  obtain ⟨next, evaluated, retained⟩ := table_retain_refines eq callback keep lawful store state
  refine ⟨next, evaluated, ?_⟩
  rw [retained, table_retain_view keyView valueView keep modelKeep compatible, represented]

end SandboxModel.Foundations
