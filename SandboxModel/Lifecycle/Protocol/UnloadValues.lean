import SandboxModel.Lifecycle.Protocol.UnloadModel
import SandboxModel.Lifecycle.Protocol.ReadyFactValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.unload sandproof_protocol.lifecycle.observation
open SandboxModel.Stage1Protocol.Validation SandboxModel.Stage1Protocol.ReadyFactValues
namespace SandboxModel.Stage1Protocol.UnloadValues

def original : AcceptedLifecycleIntent := {
  resumeIntent with
  accepted_version := 3#u64
  request := {
    binding := { resumeIntent.request.binding with expected := .Version 2#u64 }
    operation := .Unload 3#u64 2#u64 } }
def payload : UnloadedFacts := { source with source_incarnation := 3#u64, capture := 2#u64 }
def observation (outcome : UnloadOutcome) : UnloadObservation := ⟨original, outcome⟩
def changedOriginal : AcceptedLifecycleIntent := { original with retention := ⟨1#u16, 2#u64⟩ }
def changedRequest : UnloadObservation := ⟨changedOriginal, .Pending⟩
def changedCapture : UnloadedFacts := { payload with capture := 1#u64 }
def emptyPayload : UnloadedFacts := { payload with material_bytes := 0#u64 }
def expiredFacts : CurrentReadyFacts := { facts with observed_at_ms := 50#u64 }
def invalidOriginal : AcceptedLifecycleIntent := {
  original with request := {
    original.request with binding := { original.request.binding with protocol_version := 3#u16 } } }

theorem original_is_valid : ValidEnvelope original EnvelopeValues.context := by
  have checked : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
  simp [ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    original, resumeIntent, EnvelopeValues.intent, EnvelopeValues.context, EnvelopeValues.center,
    EnvelopeValues.node, checked]

theorem changed_original_is_valid : ValidEnvelope changedOriginal EnvelopeValues.context := by
  have valid := original_is_valid
  simpa [changedOriginal, ValidEnvelope, retentionOK, versionOK, original, resumeIntent, EnvelopeValues.intent] using valid

theorem counterexamples_outside_contract :
    ¬ UnloadBinding.Matches changedRequest original ∧
    ¬ UnloadBinding.Matches (observation (.Unloaded changedCapture)) original ∧
    ¬ UnloadBinding.Matches (observation (.Unloaded emptyPayload)) original ∧
    ¬ UnloadBinding.Matches (observation (.OriginalContinued expiredFacts)) original := by
  simp [UnloadBinding.Matches, UnloadBinding.UnloadedShape, ReadyFacts.Timing,
    changedRequest, changedOriginal, observation, original, changedCapture, emptyPayload, expiredFacts,
    payload, source, facts, resumeIntent, EnvelopeValues.intent]

end SandboxModel.Stage1Protocol.UnloadValues

#audit_controller_proof SandboxModel.Stage1Protocol.UnloadValues.original_is_valid
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadValues.changed_original_is_valid
#audit_controller_proof SandboxModel.Stage1Protocol.UnloadValues.counterexamples_outside_contract
