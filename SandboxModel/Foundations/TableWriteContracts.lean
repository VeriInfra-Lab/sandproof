import SandboxModel.Foundations.TableWrites

namespace SandboxModel.Foundations
open Aeneas Aeneas.Std Result SandproofFoundationsRust

theorem table_put_existing_refines [DecidableEq K]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (store : sandbox_controller_core.store.Store K V) (key : K) (old value : V)
    (present : tableLookup key store.entries.val = some old) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => next.entries.val = tablePut key value store.entries.val ∧
        next.entries.val.length = store.entries.val.length ⦄ := by
  apply WP.spec_mono (table_put_refines eq lawful store key value
    (Or.inl (by rw [present]; simp)))
  intro next updated
  exact ⟨updated, by rw [updated]; exact table_put_length_present key value _ old present⟩

theorem table_put_fresh_refines [DecidableEq K]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (store : sandbox_controller_core.store.Store K V) (key : K) (value : V)
    (absent : tableLookup key store.entries.val = none)
    (capacity : store.entries.val.length < Usize.max) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => next.entries.val = tablePut key value store.entries.val ∧
        next.entries.val.length = store.entries.val.length + 1 ⦄ := by
  apply WP.spec_mono (table_put_refines eq lawful store key value (Or.inr capacity))
  intro next updated
  exact ⟨updated, by rw [updated]; exact table_put_length_absent key value _ absent⟩

theorem table_put_query_frame [DecidableEq K]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (store : sandbox_controller_core.store.Store K V) (key : K) (value : V)
    (capacity : TableWriteCapacity store.entries.val key) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => ∀ query, tableLookup query next.entries.val =
        if query = key then some value else tableLookup query store.entries.val ⦄ := by
  apply WP.spec_mono (table_put_refines eq lawful store key value capacity)
  intro next updated query
  rw [updated, table_lookup_put]

theorem table_put_unique_refines [DecidableEq K]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (store : sandbox_controller_core.store.Store K V) (key : K) (value : V)
    (capacity : TableWriteCapacity store.entries.val key)
    (unique : (store.entries.val.map Prod.fst).Nodup) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => (next.entries.val.map Prod.fst).Nodup ⦄ := by
  apply WP.spec_mono (table_put_refines eq lawful store key value capacity)
  intro next updated
  rw [updated]
  exact table_put_preserves_unique key value _ unique

def tableValueView (keyView : K → MK) (valueView : V → MV)
    (entries : List (K × V)) : List (MK × MV) :=
  entries.map (fun entry => (keyView entry.1, valueView entry.2))

theorem table_put_view [DecidableEq K] [DecidableEq MK]
    (keyView : K → MK) (injective : Function.Injective keyView) (valueView : V → MV)
    (key : K) (value : V) (entries : List (K × V)) :
    tableValueView keyView valueView (tablePut key value entries) =
      tablePut (keyView key) (valueView value) (tableValueView keyView valueView entries) := by
  induction entries with
  | nil => rfl
  | cons entry rest ih =>
    rcases entry with ⟨stored, old⟩
    by_cases same : stored = key
    · simp [tablePut, tableValueView, same]
    · have different : keyView stored ≠ keyView key := fun equal => same (injective equal)
      simpa [tablePut, tableValueView, same, different] using
        congrArg (fun tail => (keyView stored, valueView old) :: tail) ih

theorem table_put_represented_refines [DecidableEq K] [DecidableEq MK]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (keyView : K → MK) (injective : Function.Injective keyView) (valueView : V → MV)
    (store : sandbox_controller_core.store.Store K V) (model : List (MK × MV))
    (represented : tableValueView keyView valueView store.entries.val = model)
    (key : K) (value : V) (capacity : TableWriteCapacity store.entries.val key) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => tableValueView keyView valueView next.entries.val =
        tablePut (keyView key) (valueView value) model ⦄ := by
  apply WP.spec_mono (table_put_refines eq lawful store key value capacity)
  intro next updated
  rw [updated, table_put_view keyView injective valueView, represented]

theorem table_u64_write_refines
    (store : sandbox_controller_core.store.Store U64 U64) (key value : U64)
    (capacity : TableWriteCapacity store.entries.val key) :
    ∃ next, store_put store key value = .ok next ∧
      next.entries.val = tablePut key value store.entries.val := by
  obtain ⟨next, evaluated, updated⟩ := WP.spec_imp_exists
    (table_put_refines core.cmp.PartialEqU64 table_u64_equality_lawful store key value capacity)
  exact ⟨next, by simpa [store_put] using evaluated, updated⟩

theorem table_u64_write_then_query
    (store : sandbox_controller_core.store.Store U64 U64) (key value query : U64)
    (capacity : TableWriteCapacity store.entries.val key) :
    ∃ next, store_put store key value = .ok next ∧
      store_get_copy next query = .ok
        (if query = key then some value else tableLookup query store.entries.val) := by
  obtain ⟨next, evaluated, updated⟩ := table_u64_write_refines store key value capacity
  refine ⟨next, evaluated, ?_⟩
  rw [table_u64_query_refines, updated, table_lookup_put]

theorem table_u64_write_represented
    (store : sandbox_controller_core.store.Store U64 U64) (model : List (Nat × Nat))
    (represented : tableValueView UScalar.val UScalar.val store.entries.val = model)
    (key value : U64) (capacity : TableWriteCapacity store.entries.val key) :
    ∃ next, store_put store key value = .ok next ∧
      tableValueView UScalar.val UScalar.val next.entries.val = tablePut key.val value.val model := by
  obtain ⟨next, evaluated, updated⟩ := table_u64_write_refines store key value capacity
  refine ⟨next, evaluated, ?_⟩
  rw [updated, table_put_view (fun (v : U64) => v.val) (fun _ _ equal => UScalar.eq_of_val_eq equal),
    represented]

def emptyTable (K V : Type) : sandbox_controller_core.store.Store K V :=
  ⟨alloc.vec.Vec.new (K × V)⟩

theorem table_put_init_realizable (key value : U64) :
    ∃ next, store_put (emptyTable U64 U64) key value = .ok next ∧
      next.entries.val = [(key, value)] := by
  obtain ⟨next, evaluated, updated⟩ := table_u64_write_refines (emptyTable U64 U64) key value
    (Or.inr (by simp [emptyTable, alloc.vec.Vec.new]; scalar_tac))
  exact ⟨next, evaluated, by simpa [emptyTable, alloc.vec.Vec.new, tablePut] using updated⟩

end SandboxModel.Foundations
