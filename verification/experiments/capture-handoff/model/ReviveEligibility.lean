import SandboxModel.Lifecycle.CaptureHandoff.Conditions
import SandboxModel.ProofAuditCommand
import CaptureHandoffRevive.Funs

open Aeneas Aeneas.Std
open CaptureHandoffReviveRust.sandbox_controller_core.capture

namespace SandboxModel.Lifecycle.CaptureHandoffProof.ReviveRejected

abbrev State := CaptureHandoffReviveRust.sandbox_controller_core.capture.CaptureHandoff
def useView (state : State) : Bool × Bool := (state.capture_work, state.unload_source)
def revoked (state : State) : Prop := state.eligibility = RestoreEligibility.Revoked

theorem same_property_rejects :
    ¬ LateSuccessSafe CaptureHandoffReviveRust.observe revoked useView
      CaptureHandoffReviveRust.stop_permit := by
  intro law
  let before : State := ⟨.Unknown, .Revoked, true, false⟩
  let after : State := ⟨.Registered, .Recoverable, true, true⟩
  have observed : CaptureHandoffReviveRust.observe before = .ok (some after) := by
    simp [before, after, CaptureHandoffReviveRust.observe, CaptureHandoff.observe_bundle,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
      CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]
  have impossible := (law before after rfl observed).1
  cases impossible

theorem actual_late_success_grants_stop :
    ∃ before after : State, revoked before ∧
      CaptureHandoffReviveRust.observe before = .ok (some after) ∧
      CaptureHandoffReviveRust.stop_permit after = .ok true := by
  let before : State := ⟨.Unknown, .Revoked, true, false⟩
  let after : State := ⟨.Registered, .Recoverable, true, true⟩
  refine ⟨before, after, rfl, ?_, ?_⟩ <;>
    simp [before, after, CaptureHandoffReviveRust.observe, CaptureHandoff.observe_bundle,
      CaptureHandoffReviveRust.stop_permit, CaptureHandoff.may_stop_original,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
      CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]

end SandboxModel.Lifecycle.CaptureHandoffProof.ReviveRejected

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffProof.ReviveRejected.same_property_rejects
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffProof.ReviveRejected.actual_late_success_grants_stop
