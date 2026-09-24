import SandboxModel.RustRefinement.Create

/-!
Representation bridge for the durable effect protocol.

Every definition below consumes the types emitted by Aeneas from the actual
Rust crate.  The bridge keeps the only machine-specific difference explicit:
Rust counters are `u64`, whereas the protocol uses natural numbers.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

def effectInput : SandboxControllerRust.effect.EffectInput →
    SandboxModel.Controller.EffectInput
  | .Dispatch id => .dispatch (effectId id)
  | .Receipt id outcome => .receipt (effectId id) (effectOutcome outcome)
  | .Timeout id => .timeout (effectId id)
  | .Retry logical failure => .retry (logicalEffectId logical) (failureKind failure)
  | .FinalizeFailure logical failure =>
      .finalizeFailure (logicalEffectId logical) (failureKind failure)

/-- Counter exhaustion is a concrete-machine refusal outside the unbounded
    model.  Shared effect rejection constructors map one-for-one. -/
def effectReject? : SandboxControllerRust.effect.EffectReject →
    Option SandboxModel.Controller.EffectReject
  | .ControllerDown => some .controllerDown
  | .EffectNotFound => some .effectNotFound
  | .LogicalEffectNotFound => some .logicalEffectNotFound
  | .LogicalEffectAlreadyFinal => some .logicalEffectAlreadyFinal
  | .NotDispatchable => some .notDispatchable
  | .NotAwaitingResult => some .notAwaitingResult
  | .InvalidOutcomeEvidence => some .invalidOutcomeEvidence
  | .ConflictingReceipt => some .conflictingReceipt
  | .NotDispatched => some .notDispatched
  | .CurrentAttemptNotFailed => some .currentAttemptNotFailed
  | .FailureMismatch => some .failureMismatch
  | .FailureNotRetryable => some .failureNotRetryable
  | .RetryBudgetExhausted => some .retryBudgetExhausted
  | .NextAttemptExists => some .nextAttemptExists
  | .RetryStillAvailable => some .retryStillAvailable
  | .CounterExhausted => none

/-- Direct check of the generated Rust controller-down branch. -/
theorem generated_handleEffectDispatch_controllerDown
    (world : SandboxControllerRust.model.World)
    (id : SandboxControllerRust.model.EffectId)
    (down : world.controller_up = false) :
    SandboxControllerRust.effect.handle_effect_dispatch world id =
      ok (core.result.Result.Err
        SandboxControllerRust.effect.EffectReject.ControllerDown) := by
  simp [SandboxControllerRust.effect.handle_effect_dispatch, down]

theorem model_handleEffectDispatch_controllerDown
    (world : World) (id : EffectId) (down : world.controllerUp = false) :
    SandboxModel.Controller.handleEffectDispatch world id =
      .error .controllerDown := by
  simp [SandboxModel.Controller.handleEffectDispatch, down]

theorem handleEffectDispatch_controllerDown_refines
    (rustWorld : SandboxControllerRust.model.World) (modelWorld : World)
    (id : SandboxControllerRust.model.EffectId)
    (related : ControllerAvailabilityRep rustWorld modelWorld)
    (down : modelWorld.controllerUp = false) :
    SandboxControllerRust.effect.handle_effect_dispatch rustWorld id =
        ok (core.result.Result.Err
          SandboxControllerRust.effect.EffectReject.ControllerDown) ∧
      SandboxModel.Controller.handleEffectDispatch modelWorld (effectId id) =
        .error .controllerDown := by
  constructor
  · apply generated_handleEffectDispatch_controllerDown
    simpa [ControllerAvailabilityRep, down] using related
  · exact model_handleEffectDispatch_controllerDown _ _ down

end SandboxModel.RustRefinement
