import SandboxModel.Foundations.LoopCallbacks.Contracts
import SandproofLoopCallbacks.Funs

/-! Connection to the existing finite loop proof. No Copy instance for V is required.
The same loop lemmas can check an independently extracted wrong entry point. -/
namespace SandboxModel.Foundations.LoopCallbacks
open Aeneas Aeneas.Std SandproofLoopCallbacksRust
open SandboxModel.Foundations

def foundationStore (store : sandbox_controller_core.store.Store K V) :
    SandproofFoundationsRust.sandbox_controller_core.store.Store K V := ⟨store.entries⟩

theorem original_loop_body_agrees (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (index : Usize) :
    sandbox_controller_core.store.Store.get_loop.body eq store key index =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.get_copy_loop.body
        eq (foundationStore store) key index := by
  rfl

theorem original_loop_agrees (eq : core.cmp.PartialEq K K)
    (store : sandbox_controller_core.store.Store K V) (key : K) (index : Usize) :
    sandbox_controller_core.store.Store.get_loop eq store key index =
      SandproofFoundationsRust.sandbox_controller_core.store.Store.get_copy_loop
        eq (foundationStore store) key index := by
  unfold sandbox_controller_core.store.Store.get_loop
    SandproofFoundationsRust.sandbox_controller_core.store.Store.get_copy_loop
  simp only [original_loop_body_agrees]

theorem original_loop_refines [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (lawful : TableLawfulEq eq)
    (store : sandbox_controller_core.store.Store K V) (key : K)
    (start : Usize) (bound : start.val ≤ store.entries.val.length) :
    sandbox_controller_core.store.Store.get_loop eq store key start =
      .ok (tableLookup key (store.entries.val.drop start.val)) := by
  obtain ⟨out, he, hr⟩ := WP.spec_imp_exists
    (table_get_copy_loop_refines eq lawful (foundationStore store) key start bound)
  subst out
  simpa [original_loop_agrees, foundationStore] using he

end SandboxModel.Foundations.LoopCallbacks
