import SandboxModel.Foundations.Representation

namespace SandboxModel.Foundations
open Aeneas.Std SandproofFoundationsRust

inductive Outcome (E Failure Reason : Type) where
  | observedSuccess : E → Outcome E Failure Reason
  | definitiveFailure : Failure → Bool → Option E → Outcome E Failure Reason
  | unknown : Reason → Option E → Outcome E Failure Reason

def outcomeView {E : Type} (out : sandproof_protocol.WorkOutcome E) :
    Outcome E sandproof_protocol.FailureCode sandproof_protocol.UnknownReason :=
  match out with
  | .ObservedSuccess evidence => .observedSuccess evidence
  | .DefinitiveFailure code retry residue => .definitiveFailure code retry residue
  | .Unknown reason last => .unknown reason last

def Outcome.retryHint {E F U : Type} (out : Outcome E F U) : Bool :=
  match out with
  | .definitiveFailure _ true none => true
  | _ => false
def Outcome.observation {E F U : Type} (out : Outcome E F U) : Option E :=
  match out with
  | .observedSuccess evidence => some evidence
  | .definitiveFailure _ _ residue => residue
  | .unknown _ last => last

theorem retry_hint_refines {E : Type} (out : sandproof_protocol.WorkOutcome E) :
    sandproof_protocol.WorkOutcome.permits_new_attempt out = .ok (outcomeView out).retryHint := by
  cases out with
  | ObservedSuccess e => rfl
  | Unknown reason last => rfl
  | DefinitiveFailure code retry residue =>
    cases retry <;> cases residue <;> rfl

theorem observation_refines {E : Type} (out : sandproof_protocol.WorkOutcome E) :
    sandproof_protocol.WorkOutcome.observation out = .ok (outcomeView out).observation := by
  cases out <;> simp [sandproof_protocol.WorkOutcome.observation,
    outcomeView, Outcome.observation]

theorem unknown_never_allows_new_attempt {E : Type}
    (reason : sandproof_protocol.UnknownReason) (last : Option E) :
    sandproof_protocol.WorkOutcome.permits_new_attempt (.Unknown reason last) = .ok false := by
  rfl

theorem partial_failure_never_allows_new_attempt {E : Type}
    (code : sandproof_protocol.FailureCode) (retry : Bool) (e : E) :
    sandproof_protocol.WorkOutcome.permits_new_attempt
      (.DefinitiveFailure code retry (some e)) = .ok false := by
  cases retry <;> rfl

theorem retry_hint_complete {E : Type} (out : sandproof_protocol.WorkOutcome E) :
    sandproof_protocol.WorkOutcome.permits_new_attempt out = .ok true ↔
      ∃ code, out = .DefinitiveFailure code true none := by
  cases out with
  | ObservedSuccess e => simp [sandproof_protocol.WorkOutcome.permits_new_attempt]
  | Unknown reason last => simp [sandproof_protocol.WorkOutcome.permits_new_attempt]
  | DefinitiveFailure code retry residue =>
    cases retry <;> cases residue <;>
      simp [sandproof_protocol.WorkOutcome.permits_new_attempt]

theorem evidence_kind_separated {B : Type} (e : Evidence B)
    (h : e.kind = .unknown) :
    e.kind ≠ .accepted ∧ e.kind ≠ .ready ∧ e.kind ≠ .stopped ∧
      e.kind ≠ .cleaned ∧ e.kind ≠ .released := by
  simp [h]

theorem unknown_observation_is_not_success {E : Type}
    (reason : sandproof_protocol.UnknownReason) (last : E) :
    sandproof_protocol.WorkOutcome.observation (.Unknown reason (some last)) = .ok (some last) ∧
      sandproof_protocol.WorkOutcome.permits_new_attempt (.Unknown reason (some last)) = .ok false ∧
      ∀ e, sandproof_protocol.WorkOutcome.Unknown reason (some last) ≠ .ObservedSuccess e := by
  simp [sandproof_protocol.WorkOutcome.observation, sandproof_protocol.WorkOutcome.permits_new_attempt]

theorem retry_wrapper_refines (out : sandproof_protocol.WorkOutcome U64) :
    retry_hint out = .ok (outcomeView out).retryHint := by
  exact retry_hint_refines out

theorem observation_wrapper_refines (out : sandproof_protocol.WorkOutcome U64) :
    outcome_observation out = .ok (outcomeView out).observation := by
  exact observation_refines out

end SandboxModel.Foundations
