import SandboxModel.Lifecycle.Protocol.ReplayEquality
import SandboxModel.Lifecycle.Protocol.ValidationModel
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

end SandboxModel.Stage1Protocol.Recorded

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.matches_refines

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.complete

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.accepts_iff

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.rejects_changed_intent

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.matches_keeps_original

#audit_controller_proof SandboxModel.Stage1Protocol.Recorded.terminal_flag_not_certified
