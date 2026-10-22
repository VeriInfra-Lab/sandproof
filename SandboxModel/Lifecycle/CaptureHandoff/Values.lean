import CaptureHandoff.Funs
import SandboxModel.Lifecycle.CaptureHandoff.Conditions
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std CaptureHandoffRust
open CaptureHandoffRust.sandbox_controller_core.capture

namespace SandboxModel.Lifecycle.CaptureHandoffProof

abbrev State := CaptureHandoffRust.sandbox_controller_core.capture.CaptureHandoff

def useView (state : State) : Bool × Bool := (state.capture_work, state.unload_source)
def revoked (state : State) : Prop := state.eligibility = RestoreEligibility.Revoked

theorem material_projection (state : State) : material_uses state = .ok (useView state) := by
  simp [material_uses, useView, CaptureHandoff.has_capture_work, CaptureHandoff.has_unload_source]

theorem failure_preserves_uses : FailurePreservesUses pause_failure useView := by
  intro state
  rcases state with ⟨commit, eligibility, work, source⟩
  cases commit <;> cases eligibility <;> cases work <;> cases source <;>
    simp [pause_failure, useView, CaptureHandoff.ordinary_failure,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq, CaptureCommit.read_discriminant]

theorem late_success_safe : LateSuccessSafe observe revoked useView stop_permit := by
  intro state after wasRevoked observed
  rcases state with ⟨commit, eligibility, work, source⟩
  cases commit <;> cases eligibility <;> cases work <;> cases source <;>
    simp_all [revoked, useView, observe, stop_permit, CaptureHandoff.observe_bundle,
      CaptureHandoff.may_stop_original,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq, CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]
    <;> subst after <;> simp

theorem unknown_failure_preserves_unknown (state : State)
    (unknown : state.commit = CaptureCommit.Unknown) :
    ∃ after, pause_failure state = .ok after ∧
      after.commit = CaptureCommit.Unknown ∧ revoked after ∧ useView after = useView state := by
  rcases state with ⟨commit, eligibility, work, source⟩
  cases commit <;> cases eligibility <;> cases work <;> cases source <;>
    simp_all [pause_failure, revoked, useView, CaptureHandoff.ordinary_failure,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq, CaptureCommit.read_discriminant]

theorem normal_handoff_accepts :
    ∃ initial pending complete : State,
      accepted = .ok initial ∧ dispatch initial = .ok pending ∧
      register pending = .ok (some complete) ∧ stop_permit complete = .ok true ∧
      useView complete = (true, true) := by
  let initial : State := ⟨.NotRegistered, .Capturing, true, false⟩
  let pending : State := ⟨.Unknown, .Capturing, true, false⟩
  let complete : State := ⟨.Registered, .Recoverable, true, true⟩
  refine ⟨initial, pending, complete, ?_, ?_, ?_, ?_, rfl⟩ <;>
    simp [initial, pending, complete, accepted, dispatch, register, stop_permit,
      CaptureHandoff.accepted, CaptureHandoff.registration_dispatched,
      CaptureHandoff.register_bundle, CaptureHandoff.accepts_registration,
      CaptureHandoff.observe_bundle, CaptureHandoff.may_stop_original,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]

theorem revoked_late_success_accepts_material :
    ∃ pending cancelled material : State,
      pending.commit = CaptureCommit.Unknown ∧
      pause_failure pending = .ok cancelled ∧ observe cancelled = .ok (some material) ∧
      revoked material ∧ stop_permit material = .ok false ∧
      useView material = (true, true) := by
  let pending : State := ⟨.Unknown, .Capturing, true, false⟩
  let cancelled : State := ⟨.Unknown, .Revoked, true, false⟩
  let material : State := ⟨.Registered, .Revoked, true, true⟩
  refine ⟨pending, cancelled, material, rfl, ?_, ?_, rfl, ?_, rfl⟩ <;>
    simp [pending, cancelled, material, pause_failure, observe, stop_permit,
      CaptureHandoff.ordinary_failure, CaptureHandoff.observe_bundle,
      CaptureHandoff.may_stop_original,
      CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,
      RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, CaptureCommit.read_discriminant, RestoreEligibility.read_discriminant]

end SandboxModel.Lifecycle.CaptureHandoffProof

#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffProof.material_projection
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffProof.failure_preserves_uses
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffProof.late_success_safe
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffProof.unknown_failure_preserves_unknown
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffProof.normal_handoff_accepts
#audit_controller_proof SandboxModel.Lifecycle.CaptureHandoffProof.revoked_late_success_accepts_material
