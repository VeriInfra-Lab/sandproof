import CaptureCaller.Funs
import SandboxModel.Foundations.TableWriteContracts

namespace SandboxModel.Lifecycle.CaptureHandoffCaller
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core

/-- Only the Store representation changes; keys and values are identical. -/
def foundationStore (store : sandbox_controller_core.store.Store K V) :
    SandproofFoundationsRust.sandbox_controller_core.store.Store K V := ⟨store.entries⟩

theorem get_body_agrees (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (index : Usize) :
    sandbox_controller_core.store.Store.get_loop.body eq store key index =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.get_copy_loop.body
        eq (foundationStore store) key index := by
  rfl

theorem get_loop_agrees (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (index : Usize) :
    sandbox_controller_core.store.Store.get_loop eq store key index =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.get_copy_loop
        eq (foundationStore store) key index := by
  unfold sandbox_controller_core.store.Store.get_loop
    SandproofFoundationsRust.sandbox_controller_core.store.Store.get_copy_loop
  simp only [get_body_agrees]

@[simp] theorem get_exact [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V) (key : K) :
    sandbox_controller_core.store.Store.get eq store key =
      .ok (tableLookup key store.entries.val) := by
  obtain ⟨out, evaluated, post⟩ := WP.spec_imp_exists
    (table_get_copy_loop_refines eq lawful (foundationStore store) key 0#usize (by simp))
  subst out
  simpa [sandbox_controller_core.store.Store.get, get_loop_agrees, foundationStore] using evaluated

theorem put_body_agrees (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (value : V) (index : Usize) :
    sandbox_controller_core.store.Store.put_loop.body eq store key value index =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.put_loop.body
        eq (foundationStore store) key value index := by
  rfl

theorem put_loop_agrees (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (value : V) (index : Usize) :
    sandbox_controller_core.store.Store.put_loop eq store key value index =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.put_loop
        eq (foundationStore store) key value index := by
  unfold sandbox_controller_core.store.Store.put_loop
    SandproofFoundationsRust.sandbox_controller_core.store.Store.put_loop
  simp only [put_body_agrees]

theorem put_refines [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V)
    (key : K) (value : V) (capacity : TableWriteCapacity store.entries.val key) :
    sandbox_controller_core.store.Store.put eq store key value
      ⦃ next => next.entries.val = tablePut key value store.entries.val ⦄ := by
  unfold sandbox_controller_core.store.Store.put
  rw [put_loop_agrees]
  apply WP.spec_bind
    (table_put_loop_refines eq lawful (foundationStore store) key value 0#usize
      (by simp) (by simpa [TableWriteCapacity, foundationStore] using capacity))
  intro entries post
  simpa [foundationStore] using post

theorem put_exact [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq) (store : sandbox_controller_core.store.Store K V)
    (key : K) (value : V) (capacity : TableWriteCapacity store.entries.val key) :
    ∃ next, sandbox_controller_core.store.Store.put eq store key value = .ok next ∧
      next.entries.val = tablePut key value store.entries.val := by
  exact WP.spec_imp_exists (put_refines eq lawful store key value capacity)

end SandboxModel.Lifecycle.CaptureHandoffCaller
