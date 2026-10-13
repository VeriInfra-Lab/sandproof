import SandboxModel.Foundations.Model
import SandproofFoundations.Funs
namespace SandboxModel.Foundations
open Aeneas Aeneas.Std Result ControlFlow SandproofFoundationsRust

def TableLawfulEq [DecidableEq K] (eq : core.cmp.PartialEq K K) : Prop :=
  ∀ left right, eq.eq left right = .ok (decide (left = right))

theorem table_get_copy_loop_refines [DecidableEq K]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (store : sandbox_controller_core.store.Store K V) (key : K)
    (start : Usize) (bound : start.val ≤ store.entries.val.length) :
    sandbox_controller_core.store.Store.get_copy_loop eq store key start
      ⦃ out => out = tableLookup key (store.entries.val.drop start.val) ⦄ := by
  unfold sandbox_controller_core.store.Store.get_copy_loop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index => store.entries.val.length - index.val)
    (fun index => index.val ≤ store.entries.val.length ∧
      tableLookup key (store.entries.val.drop index.val) =
        tableLookup key (store.entries.val.drop start.val))
    (fun out => out = tableLookup key (store.entries.val.drop start.val))
  · intro index invariant
    unfold sandbox_controller_core.store.Store.get_copy_loop.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step
      have indexBound : index.val < store.entries.val.length := by simpa using inBounds
      have dropEq : store.entries.val.drop index.val =
          (t, t1) :: store.entries.val.drop (index.val + 1) := by
        rw [List.drop_eq_getElem_cons indexBound, ← t_post]
      simp only [core.cmp.impls.PartialEqShared.eq]
      rw [lawful t key]
      by_cases same : t = key
      · simp [same]
        rcases invariant with ⟨_, lookupEq⟩
        rw [dropEq] at lookupEq
        simpa [tableLookup, same] using lookupEq
      · simp [same]
        step as ⟨next, nextPost⟩
        rcases invariant with ⟨_, lookupEq⟩
        have different : key ≠ t := fun h => same h.symm
        constructor
        · scalar_tac
        · constructor
          · rw [nextPost, dropEq] at *
            simpa [tableLookup, different] using lookupEq
          · omega
    · rename_i outside
      simp only [WP.spec_ok]
      rcases invariant with ⟨_, lookupEq⟩
      have lower : store.entries.val.length ≤ index.val := by simpa using outside
      rw [List.drop_eq_nil_of_le lower] at lookupEq
      simpa [tableLookup] using lookupEq
  · exact ⟨bound, rfl⟩

theorem table_get_copy_refines [DecidableEq K]
    (eq : core.cmp.PartialEq K K) (lawful : TableLawfulEq eq)
    (copy : core.marker.Copy V) (store : sandbox_controller_core.store.Store K V) (key : K) :
    sandbox_controller_core.store.Store.get_copy eq copy store key
      ⦃ out => out = tableLookup key store.entries.val ⦄ := by
  unfold sandbox_controller_core.store.Store.get_copy
  simpa using table_get_copy_loop_refines eq lawful store key 0#usize (by simp)

theorem table_u64_equality_lawful : TableLawfulEq core.cmp.PartialEqU64 := by
  intro left right
  simp

theorem table_u64_query_refines (store : sandbox_controller_core.store.Store U64 U64) (key : U64) :
    store_get_copy store key = .ok (tableLookup key store.entries.val) := by
  obtain ⟨out, he, hr⟩ := Aeneas.Std.WP.spec_imp_exists
    (table_get_copy_refines core.cmp.PartialEqU64 table_u64_equality_lawful
      core.marker.CopyU64 store key)
  subst out
  simpa [store_get_copy] using he

theorem table_lookup_hit_first [DecidableEq K] (key : K) (value : V) (rest : List (K × V)) :
    tableLookup key ((key, value) :: rest) = some value := by simp [tableLookup]

theorem table_lookup_none_iff [DecidableEq K] (entries : List (K × V)) (key : K) :
    tableLookup key entries = none ↔ ∀ value, (key, value) ∉ entries := by
  induction entries with
  | nil => simp [tableLookup]
  | cons head tail ih =>
    rcases head with ⟨stored, value⟩
    by_cases h : key = stored
    · subst stored
      constructor
      · intro hn; simp [tableLookup] at hn
      · intro hs; exact False.elim (hs value (by simp))
    · simpa [tableLookup, h, Prod.mk.injEq] using ih

end SandboxModel.Foundations
