import SandboxModel.Foundations.LoopCallbacks.Loop

/-! Actual original Store and delegate refinements. Concrete comparison and
faithful key/value views remain explicit obligations; no Copy bound on V. -/
namespace SandboxModel.Foundations.LoopCallbacks
open Aeneas Aeneas.Std SandproofLoopCallbacksRust
open SandboxModel.Foundations

theorem original_get_refines [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq)
    (store : sandbox_controller_core.store.Store K V) (key : K) :
    sandbox_controller_core.store.Store.get eq store key =
      .ok (tableLookup key store.entries.val) := by
  simpa [sandbox_controller_core.store.Store.get] using
    original_loop_refines eq lawful store key 0#usize (by simp)

theorem original_query_contract [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) :
    QueryContract (fun (store : sandbox_controller_core.store.Store K V) => store.entries.val)
      (sandbox_controller_core.store.Store.get eq) := by
  intro store key
  exact original_get_refines eq lawful store key

theorem original_contains_connected (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) :
    sandbox_controller_core.store.Store.contains_key eq store key = (do
      let returned ← sandbox_controller_core.store.Store.get eq store key
      .ok returned.isSome) := by
  rfl

theorem original_membership_contract [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) :
    MembershipContract (fun (store : sandbox_controller_core.store.Store K V) => store.entries.val)
      (sandbox_controller_core.store.Store.contains_key eq) := by
  exact membership_from_original_query _ _ _ (original_query_contract eq lawful)
    (original_contains_connected eq)

theorem delegate_query_contract [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) :
    QueryContract (fun (store : sandbox_controller_core.store.Store K V) => store.entries.val)
      (lookup eq) := by
  intro store key
  simpa [lookup] using original_get_refines eq lawful store key

theorem delegate_membership_contract [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) :
    MembershipContract (fun (store : sandbox_controller_core.store.Store K V) => store.entries.val)
      (contains eq) := by
  intro store key
  simpa [contains] using original_membership_contract eq lawful store key

theorem u64_query_contract :
    QueryContract (fun (store : sandbox_controller_core.store.Store U64 U64) => store.entries.val)
      lookup_u64 := by
  intro store key
  simpa [lookup_u64, lookup] using
    original_get_refines core.cmp.PartialEqU64 table_u64_equality_lawful store key

theorem u64_membership_contract :
    MembershipContract (fun (store : sandbox_controller_core.store.Store U64 U64) => store.entries.val)
      contains_u64 := by
  intro store key
  simpa [contains_u64, contains] using
    original_membership_contract core.cmp.PartialEqU64 table_u64_equality_lawful store key

theorem query_empty [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V)
    (key : K) (empty : store.entries.val = []) :
    sandbox_controller_core.store.Store.get eq store key = .ok none := by
  rw [original_get_refines eq lawful, empty]
  rfl

theorem query_first_of_rows [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V)
    (key : K) (value : V) (rest : List (K × V))
    (rows : store.entries.val = (key, value) :: rest) :
    sandbox_controller_core.store.Store.get eq store key = .ok (some value) := by
  rw [original_get_refines eq lawful, rows, table_lookup_hit_first]

theorem query_missing_of_no_row [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V)
    (key : K) (absent : ∀ value, (key, value) ∉ store.entries.val) :
    sandbox_controller_core.store.Store.get eq store key = .ok none := by
  rw [original_get_refines eq lawful, (table_lookup_none_iff _ _).mpr absent]

theorem represented_query [DecidableEq K] [DecidableEq MK]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (keyView : K → MK) (valueView : V → MV)
    (injective : Function.Injective keyView)
    (store : sandbox_controller_core.store.Store K V) (model : List (MK × MV))
    (represented : tableValueView keyView valueView store.entries.val = model) (key : K) :
    ∃ out, sandbox_controller_core.store.Store.get eq store key = .ok out ∧
      out.map valueView = tableLookup (keyView key) model := by
  refine ⟨tableLookup key store.entries.val, original_get_refines eq lawful store key, ?_⟩
  rw [← table_lookup_view keyView valueView injective, represented]

theorem represented_membership [DecidableEq K] [DecidableEq MK]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (keyView : K → MK) (valueView : V → MV)
    (injective : Function.Injective keyView)
    (store : sandbox_controller_core.store.Store K V) (model : List (MK × MV))
    (represented : tableValueView keyView valueView store.entries.val = model) (key : K) :
    sandbox_controller_core.store.Store.contains_key eq store key =
      .ok (tableLookup (keyView key) model).isSome := by
  rw [original_membership_contract eq lawful store key, ← represented,
    table_query_presence_preserved keyView valueView injective]

theorem contains_failure_preserved (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (error : Error)
    (failed : sandbox_controller_core.store.Store.get eq store key = .fail error) :
    sandbox_controller_core.store.Store.contains_key eq store key = .fail error := by
  rw [original_contains_connected, failed]
  simp

end SandboxModel.Foundations.LoopCallbacks
