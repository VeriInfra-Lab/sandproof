import SandboxModel.Lifecycle.Protocol.ReplayEquality
import SandboxModel.Lifecycle.Protocol.ValidationModel
import SandboxModel.Lifecycle.Protocol.EnvelopeValues
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority
namespace SandboxModel.Stage1Protocol.Recorded
open SandboxModel.Stage1Protocol

theorem matches_refines (recorded : RecordedLifecycleIntent) (original : AcceptedLifecycleIntent) :
    recorded_matches recorded original = .ok (Validation.expectedRecorded recorded original) := by
  classical
  have hb : ∀ a b : Bool, (a == b) = decide (a = b) := by
    intro a b; cases a <;> cases b <;> rfl
  simp [recorded_matches, RecordedLifecycleIntent.matches, Replay.accepted_eq,
    Validation.expectedRecorded, Validation.RecordsOriginal, Replay.ok_and, hb]

theorem complete : Validation.FullRecordContract recorded_matches := by
  intro recorded original
  exact matches_refines recorded original

theorem accepts_iff (recorded : RecordedLifecycleIntent) (original : AcceptedLifecycleIntent) :
    recorded_matches recorded original = .ok true ↔ Validation.RecordsOriginal recorded original := by
  classical
  rw [matches_refines]
  simp [Validation.expectedRecorded]

theorem rejects_changed_intent (recorded : RecordedLifecycleIntent) (original : AcceptedLifecycleIntent)
    (changed : recorded.intent ≠ original) : recorded_matches recorded original = .ok false := by
  classical
  rw [matches_refines]
  simp [Validation.expectedRecorded, Validation.RecordsOriginal, changed]

theorem matches_keeps_original (recorded : RecordedLifecycleIntent) (original : AcceptedLifecycleIntent)
    (matched : recorded_matches recorded original = .ok true) : recorded.intent = original := by
  exact ((accepts_iff recorded original).mp matched).1

theorem terminal_flag_not_certified (recorded : RecordedLifecycleIntent)
    (original : AcceptedLifecycleIntent) (terminal : Bool) :
    recorded_matches { recorded with terminal := terminal } original =
      recorded_matches recorded original := by
  rw [matches_refines, matches_refines]
  rfl

/-- These prefix lemmas describe the Boolean guards, not new error variants. -/
theorem older_version_rejected_after_full_intent (recorded : RecordedLifecycleIntent)
    (original : AcceptedLifecycleIntent) (same : recorded.intent = original)
    (older : recorded.current_session_version < original.accepted_version) :
    recorded_matches recorded original = .ok false := by
  rw [matches_refines]
  simp [Validation.expectedRecorded, Validation.RecordsOriginal, same, not_le_of_gt older]

theorem wrong_current_rejected_after_version (recorded : RecordedLifecycleIntent)
    (original : AcceptedLifecycleIntent) (same : recorded.intent = original)
    (version : original.accepted_version ≤ recorded.current_session_version)
    (wrong : recorded.current ≠ decide (recorded.current_session_version = original.accepted_version)) :
    recorded_matches recorded original = .ok false := by
  rw [matches_refines]
  simp [Validation.expectedRecorded, Validation.RecordsOriginal, same, version, wrong]

theorem normal_recorded_matches :
    recorded_matches (EnvelopeValues.receipt 1#u64) (EnvelopeValues.intent 1#u64) = .ok true := by
  apply (accepts_iff _ _).mpr
  simp [Validation.RecordsOriginal, EnvelopeValues.receipt, EnvelopeValues.intent]

theorem same_session_changed_capture_rejected :
    recorded_matches (EnvelopeValues.receipt 2#u64) (EnvelopeValues.intent 1#u64) = .ok false :=
  rejects_changed_intent _ _ EnvelopeValues.changed_complete_intent

theorem changed_retention_window_rejected :
    recorded_matches
      { EnvelopeValues.receipt 1#u64 with
        intent := { EnvelopeValues.intent 1#u64 with retention := ⟨1#u16, 9#u64⟩ } }
      (EnvelopeValues.intent 1#u64) = .ok false := by
  apply rejects_changed_intent
  simp [EnvelopeValues.intent]

/-- Even an invalid accepted version can match itself; matches is not validate. -/
theorem matching_does_not_establish_validation :
    let invalid := { EnvelopeValues.intent 1#u64 with accepted_version := 0#u64 }
    recorded_matches ⟨invalid, 0#u64, true, false⟩ invalid = .ok true ∧
      ¬ Validation.ValidEnvelope invalid EnvelopeValues.context := by
  dsimp only
  constructor
  · apply (accepts_iff _ _).mpr
    simp [Validation.RecordsOriginal]
  · have checked : U64.checked_add 0#u64 1#u64 = some 1#u64 := by decide
    simp [Validation.ValidEnvelope, Validation.versionOK, EnvelopeValues.intent, checked]

end SandboxModel.Stage1Protocol.Recorded

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.matches_refines

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.complete

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.accepts_iff

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.rejects_changed_intent

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.matches_keeps_original

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.terminal_flag_not_certified

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.older_version_rejected_after_full_intent
#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.wrong_current_rejected_after_version
#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.normal_recorded_matches
#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.same_session_changed_capture_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.changed_retention_window_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.matching_does_not_establish_validation
