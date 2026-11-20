import SandboxModel.Lifecycle.Protocol.ReadyFactBindings
import SandboxModel.Lifecycle.Protocol.ReadyFactValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal.lifecycle sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.observation sandproof_protocol.lifecycle.unload
open SandboxModel.Stage1Protocol.ReadyFacts SandboxModel.Stage1Protocol.Validation
namespace SandboxModel.Stage1Protocol.ReadyFactValues


theorem normal_create : ready_matches_create facts createIntent = .ok true := by
  have timeout : U64.checked_add 10#u64 40#u64 = some 50#u64 := by decide
  have maximum : U64.checked_add 10#u64 90#u64 = some 100#u64 := by decide
  simp [create_refines, CreateMatches, createIntent, createPayload, facts, timeout, maximum]

theorem normal_resume : ready_matches_resumed facts resumeIntent source = .ok true := by
  simp [resumed_refines, ResumedMatches, ResumeSource, Timing, facts, source, resumeIntent]

theorem normal_renewed :
    ready_matches_resumed_renewal { facts with end_at_ms := 60#u64 }
      resumeIntent (currentIntent 60#i64 false) source = .ok true := by
  simp [renewed_refines, RenewedMatches, ResumeSource, Timing, facts, source, resumeIntent, currentIntent]

theorem changed_timeout_rejected : ready_matches_create facts changedCreate = .ok false := by
  have wrong : U64.checked_add 10#u64 30#u64 = some 40#u64 := by decide
  simp [create_refines, CreateMatches, changedCreate, createIntent, createPayload, facts, wrong]

theorem changed_digest_rejected : ready_matches_resumed facts resumeIntent changedSource = .ok false := by
  simp [resumed_refines, ResumedMatches, ResumeSource, changedSource, source, resumeIntent, changed_digest_differs]

theorem expired_observation_rejected :
    ready_matches_resumed { facts with observed_at_ms := 50#u64 } resumeIntent source = .ok false := by
  simp [resumed_refines, ResumedMatches, Timing, facts]

theorem negative_deadline_rejected :
    ready_matches_resumed_renewal negativeFacts resumeIntent (currentIntent ((-1)#i64) false) negativeSource = .ok false := by
  simp [renewed_refines, RenewedMatches, resumeIntent, currentIntent]

theorem extend_only_leaf_does_not_check_previous_end :
    ready_matches_resumed_renewal { facts with end_at_ms := 40#u64 }
      resumeIntent (currentIntent 30#i64 true) source = .ok true ∧ 40#u64 < source.end_at_ms := by
  simp [renewed_refines, RenewedMatches, ResumeSource, Timing, facts, source, resumeIntent, currentIntent]


end SandboxModel.Stage1Protocol.ReadyFactValues

#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.normal_create
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.normal_resume
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.normal_renewed
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.changed_timeout_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.changed_digest_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.expired_observation_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.negative_deadline_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.extend_only_leaf_does_not_check_previous_end
