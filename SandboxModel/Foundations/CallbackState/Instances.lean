import SandboxModel.Foundations.CallbackState.Core
import SandboxModel.Foundations.CallbackState.Sequence
namespace SandboxModel.Foundations.CallbackState
open Aeneas Aeneas.Std
open SandboxModel.Foundations

def toggleModel (state : Bool) (_key _value : U64) : Bool × Bool := (!state, !state)
def traceModel (floor : U64) (visited : List Row) (key value : U64) : Bool × List Row :=
  (decide (floor.val < value.val), visited ++ [(key, value)])
def traceReady (visited : TraceValue) (_key _value : U64) : Prop :=
  visited.val.length < Usize.max

theorem successful_spec_has_return {run : Result α} {post : α → Prop}
    (completed : run ⦃ post ⦄) :
    ∃ value, run = .ok value ∧ post value := by
  cases completed with
  | ret property => exact ⟨_, rfl, property⟩

theorem toggle_step_contract :
    StepContract SandproofCallbackStateRust.toggle_step (fun _ _ _ => True) id toggleModel := by
  intro state key value _ keep next evaluated
  rw [toggle_core_contract state key value] at evaluated
  simpa [toggleModel] using evaluated

theorem trace_step_contract (floor : U64) :
    StepContract (fun visited key value => SandproofCallbackStateRust.trace_step visited key value floor)
      traceReady (fun visited => visited.val) (traceModel floor) := by
  intro visited key value capacity keep next evaluated
  have post := trace_core_contract visited key value floor capacity
  change SandproofCallbackStateRust.trace_step visited key value floor = .ok (keep, next) at evaluated
  rw [evaluated] at post
  simp only [WP.spec_ok] at post
  simp [traceModel, post.1, post.2]

theorem toggle_normal_refines
    (observed : NormalObservation SandproofCallbackStateRust.toggle_step
      (fun _ _ _ => True) before rows survivors after) :
    (survivors, after) = tableRetainState toggleModel before rows := by
  exact normal_refines toggle_step_contract observed

theorem trace_normal_refines (floor : U64)
    (observed : NormalObservation
      (fun visited key value => SandproofCallbackStateRust.trace_step visited key value floor)
      traceReady before rows survivors after) :
    (survivors, after.val) = tableRetainState (traceModel floor) before.val rows := by
  exact normal_refines (trace_step_contract floor) observed

theorem trace_model_final (floor : U64) (prior rows : List Row) :
    (tableRetainState (traceModel floor) prior rows).2 = prior ++ rows := by
  induction rows generalizing prior with
  | nil => simp [tableRetainState]
  | cons row rest ih =>
    rcases row with ⟨key, value⟩
    simp [tableRetainState, traceModel, ih, List.append_assoc]

theorem trace_normal_preserves_complete_visit_history (floor : U64)
    (observed : NormalObservation
      (fun visited key value => SandproofCallbackStateRust.trace_step visited key value floor)
      traceReady before rows survivors after) :
    after.val = before.val ++ rows := by
  have same := congrArg Prod.snd (trace_normal_refines floor observed)
  exact same.trans (trace_model_final floor before.val rows)

theorem total_normal_accepts (call : S → K → V → Result (Bool × S))
    (step : S → K → V → Bool × S)
    (lawful : ∀ state key value, call state key value = .ok (step state key value))
    (state : S) (rows : List (K × V)) :
    ∃ survivors after, NormalObservation call (fun _ _ _ => True) state rows survivors after := by
  induction rows generalizing state with
  | nil => exact ⟨[], state, [], .nil state, .nil⟩
  | cons row rest ih =>
    rcases row with ⟨key, value⟩
    rcases decision : step state key value with ⟨keep, next⟩
    obtain ⟨survivors, after, bits, history, selection⟩ := ih next
    have evaluated : call state key value = .ok (keep, next) := by rw [lawful, decision]
    cases keep
    · exact ⟨survivors, after, false :: bits, .cons trivial evaluated history, .skip selection⟩
    · exact ⟨(key, value) :: survivors, after, true :: bits,
        .cons trivial evaluated history, .keep selection⟩

theorem toggle_normal_accepts (state : Bool) (rows : List Row) :
    ∃ survivors after, NormalObservation SandproofCallbackStateRust.toggle_step
      (fun _ _ _ => True) state rows survivors after := by
  exact total_normal_accepts _ toggleModel toggle_core_contract state rows

theorem trace_normal_accepts (floor : U64) (visited : TraceValue) (rows : List Row)
    (fits : visited.val.length + rows.length ≤ Usize.max) :
    ∃ survivors after, NormalObservation
      (fun visited key value => SandproofCallbackStateRust.trace_step visited key value floor)
      traceReady visited rows survivors after := by
  induction rows generalizing visited with
  | nil => exact ⟨[], visited, [], .nil visited, .nil⟩
  | cons row rest ih =>
    rcases row with ⟨key, value⟩
    have capacity : visited.val.length < Usize.max := by
      simp only [List.length_cons] at fits
      omega
    obtain ⟨⟨keep, next⟩, evaluated, post⟩ :=
      successful_spec_has_return (trace_core_contract visited key value floor capacity)
    have nextFits : next.val.length + rest.length ≤ Usize.max := by
      rw [post.2]
      simp only [List.length_append, List.length_cons, List.length_nil] at *
      omega
    obtain ⟨survivors, after, bits, history, selection⟩ := ih next nextFits
    cases keep
    · exact ⟨survivors, after, false :: bits, .cons capacity evaluated history, .skip selection⟩
    · exact ⟨(key, value) :: survivors, after, true :: bits,
        .cons capacity evaluated history, .keep selection⟩

end SandboxModel.Foundations.CallbackState

