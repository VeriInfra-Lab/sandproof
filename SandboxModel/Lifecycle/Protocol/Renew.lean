import SandboxModel.Lifecycle.Protocol.Validation
import SandboxModel.Lifecycle.Protocol.RequestValues
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std SandproofStage1RequestRust
namespace SandboxModel.Stage1Protocol.Renew
open Request

/-! These are properties of the pure request guard and full-value comparison.
The actual lifetime, current incarnation and renewal authorization are checked
by later business functions, not established by this guard. -/

theorem accepts_all_represented (incarnation : U64) (endAt : I64) (extendOnly : Bool) :
    request_validate (renew incarnation endAt extendOnly) node = .ok (.Ok ()) := by
  apply (Validation.validate_accepts_iff _ _).mpr
  simp [Validation.ValidRequest, Validation.headersOK, Validation.expectationOK,
    Validation.operationOK, renew, binding, node]

theorem absent_rejected (incarnation : U64) (endAt : I64) (extendOnly : Bool) :
    request_validate
      { renew incarnation endAt extendOnly with binding := { binding with expected := .Absent } }
      node = .ok (.Err .InvalidExpectation) := by
  apply (Validation.validate_error_iff _ _ _).mpr
  simp [Validation.FirstError, Validation.headersOK, Validation.expectationOK,
    renew, binding, node]

theorem replay_iff (leftInc rightInc : U64) (leftEnd rightEnd : I64)
    (leftMode rightMode : Bool) :
    replay_matches (renew leftInc leftEnd leftMode) (renew rightInc rightEnd rightMode) = .ok true ↔
      leftInc = rightInc ∧ leftEnd = rightEnd ∧ leftMode = rightMode := by
  simpa [renew] using Replay.replay_accepts_iff
    (renew leftInc leftEnd leftMode) (renew rightInc rightEnd rightMode)

theorem changed_incarnation_rejected (left right : U64) (endAt : I64) (extendOnly : Bool)
    (different : left ≠ right) :
    replay_matches (renew left endAt extendOnly) (renew right endAt extendOnly) = .ok false := by
  apply (Replay.replay_rejects_iff _ _).mpr
  simpa [renew] using different

theorem changed_deadline_rejected (incarnation : U64) (left right : I64) (extendOnly : Bool)
    (different : left ≠ right) :
    replay_matches (renew incarnation left extendOnly) (renew incarnation right extendOnly) = .ok false := by
  apply (Replay.replay_rejects_iff _ _).mpr
  simpa [renew] using different

theorem changed_mode_rejected (incarnation : U64) (endAt : I64) (extendOnly : Bool) :
    replay_matches (renew incarnation endAt extendOnly) (renew incarnation endAt (!extendOnly)) =
      .ok false := by
  apply (Replay.replay_rejects_iff _ _).mpr
  cases extendOnly <;> simp [renew]

theorem accepted : request_validate (renew 1#u64 100#i64 false) node = .ok (.Ok ()) :=
  accepts_all_represented _ _ _

theorem negative_deadline_passes_guard :
    request_validate (renew 1#u64 (-1)#i64 false) node = .ok (.Ok ()) :=
  accepts_all_represented _ _ _

end SandboxModel.Stage1Protocol.Renew

#audit_controller_proof SandboxModel.Stage1Protocol.Renew.accepts_all_represented
#audit_controller_proof SandboxModel.Stage1Protocol.Renew.absent_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.Renew.replay_iff
#audit_controller_proof SandboxModel.Stage1Protocol.Renew.changed_incarnation_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.Renew.changed_deadline_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.Renew.changed_mode_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.Renew.accepted
#audit_controller_proof SandboxModel.Stage1Protocol.Renew.negative_deadline_passes_guard
