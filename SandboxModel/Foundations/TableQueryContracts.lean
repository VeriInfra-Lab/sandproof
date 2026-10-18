import SandboxModel.Foundations.TableWriteContracts

/-! Generic query projection of actual get_copy, independent of World.
Keys must remain injective; values may be abstracted. No uniqueness is assumed.
Self-owned equality and representation must be proved by the calling module. -/
namespace SandboxModel.Foundations
open Aeneas Aeneas.Std SandproofFoundationsRust

theorem table_lookup_view [DecidableEq K] [DecidableEq MK]
    (keyView : K → MK) (valueView : V → MV)
    (injective : Function.Injective keyView)
    (entries : List (K × V)) (key : K) :
    tableLookup (keyView key) (tableValueView keyView valueView entries) =
      (tableLookup key entries).map valueView := by
  induction entries with
  | nil => rfl
  | cons pair rest ih =>
    rcases pair with ⟨stored, value⟩
    by_cases same : key = stored
    · simp [tableValueView, tableLookup, same]
    · have mappedDifferent : keyView key ≠ keyView stored :=
        fun equal => same (injective equal)
      simpa [tableValueView, tableLookup, same, mappedDifferent] using ih

theorem table_get_copy_represented [DecidableEq K] [DecidableEq MK]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (copy : core.marker.Copy V)
    (keyView : K → MK) (valueView : V → MV)
    (injective : Function.Injective keyView)
    (store : sandbox_controller_core.store.Store K V)
    (model : List (MK × MV))
    (represented : tableValueView keyView valueView store.entries.val = model)
    (key : K) :
    sandbox_controller_core.store.Store.get_copy eq copy store key
      ⦃ out => out.map valueView = tableLookup (keyView key) model ⦄ := by
  apply WP.spec_mono (table_get_copy_refines eq lawful copy store key)
  intro out queried
  rw [queried, ← table_lookup_view keyView valueView injective, represented]

theorem table_u64_query_represented
    (store : sandbox_controller_core.store.Store U64 U64)
    (model : List (Nat × Nat))
    (represented : tableValueView UScalar.val UScalar.val store.entries.val = model)
    (key : U64) :
    ∃ out, store_get_copy store key = .ok out ∧
      out.map UScalar.val = tableLookup key.val model := by
  refine ⟨tableLookup key store.entries.val, table_u64_query_refines store key, ?_⟩
  rw [← table_lookup_view UScalar.val UScalar.val
    (fun _ _ equal => UScalar.eq_of_val_eq equal), represented]

theorem table_query_presence_preserved [DecidableEq K] [DecidableEq MK]
    (keyView : K → MK) (valueView : V → MV)
    (injective : Function.Injective keyView)
    (entries : List (K × V)) (key : K) :
    (tableLookup (keyView key) (tableValueView keyView valueView entries)).isSome =
      (tableLookup key entries).isSome := by
  rw [table_lookup_view keyView valueView injective]
  cases tableLookup key entries <;> rfl

theorem table_key_identity_cannot_be_erased :
    tableLookup (0 : Nat) (tableValueView (fun (_ : Nat) => 0) id [(1, 10), (2, 20)]) ≠
      (tableLookup 2 [(1, 10), (2, 20)]).map id := by
  decide

end SandboxModel.Foundations
