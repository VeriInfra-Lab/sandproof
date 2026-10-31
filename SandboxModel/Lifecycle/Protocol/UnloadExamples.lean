import SandboxModel.Lifecycle.Protocol.UnloadBindings
import SandboxModel.Lifecycle.Protocol.UnloadValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open SandboxModel.Stage1Protocol.Validation SandboxModel.Stage1Protocol.ReadyFactValues
open SandboxModel.Stage1Protocol.UnloadBinding SandboxModel.Stage1Protocol.ReadyFacts
namespace SandboxModel.Stage1Protocol.UnloadValues

theorem normal_outcomes :
    unload_matches (observation .Pending) original = .ok true ∧
    unload_matches (observation (.Unloaded payload)) original = .ok true ∧
    unload_matches (observation (.OriginalContinued facts)) original = .ok true := by
  have checked : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
  simp [matches_refines, Matches, UnloadedShape, Timing, observation, original, payload, source, facts,
    resumeIntent, EnvelopeValues.intent, versionOK, retentionOK, checked]

theorem omitted_fields_rejected :
    unload_matches changedRequest original = .ok false ∧
    unload_matches (observation (.Unloaded changedCapture)) original = .ok false ∧
    unload_matches (observation (.Unloaded emptyPayload)) original = .ok false ∧
    unload_matches (observation (.OriginalContinued expiredFacts)) original = .ok false := by
  rcases counterexamples_outside_contract with ⟨request, capture, bytes, expired⟩
  exact ⟨(matches_rejects_iff _ _).mpr request, (matches_rejects_iff _ _).mpr capture,
    (matches_rejects_iff _ _).mpr bytes, (matches_rejects_iff _ _).mpr expired⟩

theorem matching_does_not_establish_validation :
    unload_matches ⟨invalidOriginal, .Pending⟩ invalidOriginal = .ok true ∧
    ¬ ValidRequest invalidOriginal.request invalidOriginal.request.binding.node := by
  have checked : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
  simp [matches_refines, Matches, invalidOriginal, original, resumeIntent, EnvelopeValues.intent,
    versionOK, retentionOK, ValidRequest, headersOK, checked]

end SandboxModel.Stage1Protocol.UnloadValues

#audit_controller_proof SandboxModel.Stage1Protocol.UnloadValues.normal_outcomes
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadValues.omitted_fields_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadValues.matching_does_not_establish_validation
