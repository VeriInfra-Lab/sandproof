import SandboxModel.Foundations.TableRetention
import SandboxModel.Foundations.CallbackState.Contracts

namespace SandboxModel.Foundations.CallbackState
open Aeneas Aeneas.Std
open SandboxModel.Foundations

/- These are observations required at the A6 traversal/B4 forwarding boundary.
They do not execute Rust, decode MIR, or assume the final model equality. -/
inductive Chain (call : S → K → V → Result (Bool × S))
    (ready : S → K → V → Prop) : S → List (K × V) → List Bool → S → Prop where
  | nil (state) : Chain call ready state [] [] state
  | cons {state key value rows keep next bits final} :
      ready state key value →
      call state key value = .ok (keep, next) →
      Chain call ready next rows bits final →
      Chain call ready state ((key, value) :: rows) (keep :: bits) final

inductive Selected : List (K × V) → List Bool → List (K × V) → Prop where
  | nil : Selected [] [] []
  | keep {row rows bits survivors} :
      Selected rows bits survivors → Selected (row :: rows) (true :: bits) (row :: survivors)
  | skip {row rows bits survivors} :
      Selected rows bits survivors → Selected (row :: rows) (false :: bits) survivors

def StepContract (call : S → K → V → Result (Bool × S))
    (ready : S → K → V → Prop) (view : S → M)
    (step : M → K → V → Bool × M) : Prop :=
  ∀ state key value, ready state key value → ∀ keep next,
    call state key value = .ok (keep, next) →
      step (view state) key value = (keep, view next)

def NormalObservation (call : S → K → V → Result (Bool × S))
    (ready : S → K → V → Prop) (before : S) (rows survivors : List (K × V))
    (after : S) : Prop :=
  ∃ decisions, Chain call ready before rows decisions after ∧ Selected rows decisions survivors

theorem chain_exact_call_count
    (history : Chain call ready before rows decisions after) :
    decisions.length = rows.length := by
  induction history with
  | nil => rfl
  | cons _ _ _ ih => simpa using congrArg Nat.succ ih

theorem selected_preserves_order (selection : Selected rows decisions survivors) :
    survivors.Sublist rows := by
  induction selection with
  | nil => exact .slnil
  | keep _ ih => exact ih.cons_cons _
  | skip _ ih => exact ih.cons _

theorem chain_selected_refines (law : StepContract call ready view step)
    (history : Chain call ready before rows decisions after)
    (selection : Selected rows decisions survivors) :
    tableRetainState step (view before) rows = (survivors, view after) := by
  induction history generalizing survivors with
  | nil => cases selection; rfl
  | @cons state key value rows keep next bits final admissible evaluated tail ih =>
    have one := law state key value admissible keep next evaluated
    cases selection with
    | keep rest =>
      simpa [tableRetainState, one] using
        (congrArg (fun result => ((key, value) :: result.1, result.2)) (ih rest))
    | skip rest =>
      simpa [tableRetainState, one] using ih rest

theorem normal_refines (law : StepContract call ready view step)
    (observed : NormalObservation call ready before rows survivors after) :
    (survivors, view after) = tableRetainState step (view before) rows := by
  obtain ⟨bits, history, selection⟩ := observed
  exact (chain_selected_refines law history selection).symm

theorem normal_survivors_are_original_rows
    (observed : NormalObservation call ready before rows survivors after) :
    survivors.Sublist rows := by
  obtain ⟨bits, _, selection⟩ := observed
  exact selected_preserves_order selection

theorem normal_survivor_count
    (observed : NormalObservation call ready before rows survivors after) :
    survivors.length ≤ rows.length := by
  exact (normal_survivors_are_original_rows observed).length_le

/- A first interruption records completed callbacks as history. The current
external heap and survivor Store are deliberately not equated to that history. -/
structure Interruption (call : S → K → V → Result (Bool × S))
    (ready : S → K → V → Prop) (before : S) (rows : List (K × V)) where
  completed : List (K × V)
  interruptedRow : K × V
  unvisited : List (K × V)
  decomposition : rows = completed ++ interruptedRow :: unvisited
  decisions : List Bool
  lastCompleted : S
  history : Chain call ready before completed decisions lastCompleted
  current : S

theorem interruption_is_a_strict_prefix
    (event : Interruption call ready before rows) :
    event.completed.length < rows.length := by
  have lengths := congrArg List.length event.decomposition
  simp only [List.length_append, List.length_cons] at lengths
  omega

theorem interruption_completed_call_count
    (event : Interruption call ready before rows) :
    event.decisions.length = event.completed.length := by
  exact chain_exact_call_count event.history

theorem interruption_does_not_impose_rollback
    (event : Interruption (S := S) call ready before rows) (current : S) :
    ∃ next : Interruption call ready before rows,
      next.completed = event.completed ∧ HEq next.history event.history ∧ next.current = current := by
  exact ⟨{event with current := current}, rfl, HEq.rfl, rfl⟩

end SandboxModel.Foundations.CallbackState
