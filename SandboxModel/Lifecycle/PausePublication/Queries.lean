import SandboxModel.Lifecycle.CaptureHandoff.CallerLate

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

private def exactCompare [DecidableEq K] : core.cmp.PartialEq K K :=
  {eq := fun left right => .ok (decide (left = right))
   ne := fun left right => .ok (decide (left ≠ right))}

/-- Only comparisons against this query are needed. Other stored keys remain unrestricted. -/
theorem get_exact_at [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (query : K) (lawfulAt : ∀ stored, eq.eq stored query = .ok (decide (stored = query)))
    (rows : store.Store K V) :
    store.Store.get eq rows query = .ok (tableLookup query rows.entries.val) := by
  have bodies (cursor : Usize) : store.Store.get_loop.body eq rows query cursor =
      store.Store.get_loop.body exactCompare rows query cursor := by
    simp only [store.Store.get_loop.body, core.cmp.impls.PartialEqShared.eq,
      lawfulAt, exactCompare]
  have loops : store.Store.get_loop eq rows query = store.Store.get_loop exactCompare rows query := by
    unfold store.Store.get_loop
    simp only [bodies]
  have lawful : TableLawfulEq (exactCompare (K := K)) := by
    intro left right
    rfl
  simpa only [store.Store.get, loops] using get_exact exactCompare lawful rows query

theorem get_copy_agrees (eq : core.cmp.PartialEq K K) (copy : core.marker.Copy V)
    (rows : store.Store K V) (query : K) :
    store.Store.get_copy eq copy rows query = store.Store.get eq rows query := by
  unfold store.Store.get_copy store.Store.get
    store.Store.get_copy_loop store.Store.get_loop
  have bodies (cursor : Usize) : store.Store.get_copy_loop.body eq rows query cursor =
      store.Store.get_loop.body eq rows query cursor := by rfl
  simp only [bodies]

end SandboxModel.Lifecycle.PausePublication
