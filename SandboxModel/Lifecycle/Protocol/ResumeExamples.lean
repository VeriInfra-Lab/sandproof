import SandboxModel.Lifecycle.Protocol.ResumeBindings
import SandboxModel.Lifecycle.Protocol.ResumeValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.resume
open SandboxModel.Stage1Protocol.Validation SandboxModel.Stage1Protocol.ReadyFactValues
open SandboxModel.Stage1Protocol.ResumeBinding SandboxModel.Stage1Protocol.ReadyFacts
namespace SandboxModel.Stage1Protocol.ResumeValues

theorem normal_original_and_successors :
    resume_matches_request (observation resumeIntent (.Observed facts)) resumeIntent = .ok true ∧
    resume_matches_request (observation (currentIntent 50#i64 false) (.Observed facts)) resumeIntent = .ok true ∧
    resume_matches_request (observation unloadIntent (.Observed facts)) resumeIntent = .ok true ∧
    resume_matches_request (observation destroyIntent .NotCurrent) resumeIntent = .ok true := by
  have checked1 : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  have checked2 : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
  simp [matches_refines, Matches, OriginalShape, CurrentShape, ReadinessMatches,
    SameReadiness, UnloadReadiness, RenewReadiness, Newer, Continued, Timing,
    ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    observation, unloadIntent, destroyIntent, currentIntent, resumeIntent, facts,
    EnvelopeValues.intent, EnvelopeValues.center, EnvelopeValues.node, checked1, checked2]

theorem omitted_fields_rejected :
    resume_matches_request changedRequestObservation resumeIntent = .ok false ∧
    resume_matches_request (observation changedRetention .Pending) resumeIntent = .ok false ∧
    resume_matches_request (observation wrongIncarnation .Pending) resumeIntent = .ok false := by
  have checked1 : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  have checked2 : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
  simp [matches_refines, Matches, OriginalShape, CurrentShape, ReadinessMatches,
    SameReadiness, RenewReadiness, Newer,
    ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    observation, changedRequestObservation, changedOriginal, changedRetention, wrongIncarnation,
    currentIntent, resumeIntent, EnvelopeValues.intent, EnvelopeValues.center,
    EnvelopeValues.node, checked1, checked2]

theorem readiness_boundaries :
    resume_matches_request (observation resumeIntent .StateLost) resumeIntent = .ok true ∧
    resume_matches_request (observation (currentIntent ((-1)#i64) false) .StateLost) resumeIntent = .ok true ∧
    resume_matches_request (observation (currentIntent ((-1)#i64) false) .Pending) resumeIntent = .ok true ∧
    resume_matches_request (observation unloadIntent .StateLost) resumeIntent = .ok true ∧
    resume_matches_request (observation (currentIntent ((-1)#i64) false) (.Observed negativeFacts)) resumeIntent = .ok false ∧
    resume_matches_request (observation resumeIntent .NotCurrent) resumeIntent = .ok false ∧
    resume_matches_request (observation resumeIntent (.Observed { facts with observed_at_ms := 50#u64 })) resumeIntent = .ok false := by
  have checked1 : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  have checked2 : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
  simp [matches_refines, Matches, OriginalShape, CurrentShape, ReadinessMatches,
    SameReadiness, UnloadReadiness, RenewReadiness, Newer, Continued, Timing,
    ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    observation, unloadIntent, currentIntent, resumeIntent, facts, negativeFacts,
    EnvelopeValues.intent, EnvelopeValues.center, EnvelopeValues.node, checked1, checked2]

theorem state_lost_preserves_source :
    resume_matches_request (observation unloadIntent .StateLost) resumeIntent = .ok true ∧
    resume_matches_request (observation wrongUnloadSource .StateLost) resumeIntent = .ok false := by
  have allowed := readiness_boundaries.2.2.2.1
  have rejected := counterexamples_outside_contract.2.2.1
  exact ⟨allowed, (matches_rejects_iff _ _).mpr rejected⟩

end SandboxModel.Stage1Protocol.ResumeValues

#audit_controller_proof SandboxModel.Stage1Protocol.ResumeValues.normal_original_and_successors
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeValues.omitted_fields_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeValues.readiness_boundaries

#audit_controller_proof SandboxModel.Stage1Protocol.ResumeValues.state_lost_preserves_source
