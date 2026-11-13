import SandboxModel.Lifecycle.Protocol.ValidationModel
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
namespace SandboxModel.Stage1Protocol.EnvelopeValues

def node : NodeIdentity := ⟨"deployment", "host", 1#u64⟩
def center : CenterIdentity := ⟨"deployment", "center"⟩
def context : AuthenticatedCenterContext := ⟨center, node⟩
def intent (capture : U64) : AcceptedLifecycleIntent := {
  request := {
    binding := {
      protocol_version := 4#u16, node := node, session := "session",
      key := ⟨"deployment", "owner", 1#u64, 1#u64⟩,
      expected := .Version 0#u64 },
    operation := .Unload 1#u64 capture },
  accepted_version := 1#u64, center := center,
  retention := ⟨1#u16, 1#u64⟩ }
def receipt (capture : U64) : RecordedLifecycleIntent :=
  ⟨intent capture, 1#u64, true, false⟩

theorem same_session :
    (receipt 2#u64).intent.request.binding.session = (intent 1#u64).request.binding.session := by rfl

theorem changed_complete_intent : (receipt 2#u64).intent ≠ intent 1#u64 := by
  simp [receipt, intent]

theorem both_inputs_have_valid_shape :
    Validation.ValidEnvelope (intent 1#u64) context ∧ Validation.ValidEnvelope (intent 2#u64) context := by
  have checked : U64.checked_add 0#u64 1#u64 = some 1#u64 := by decide
  simp [Validation.ValidEnvelope, intent, context, node, center, Validation.ValidRequest,
    Validation.headersOK, Validation.expectationOK, Validation.operationOK, Validation.retentionOK,
    Validation.versionOK, checked]

end SandboxModel.Stage1Protocol.EnvelopeValues

#audit_controller_proof SandboxModel.Stage1Protocol.EnvelopeValues.same_session
#audit_controller_proof SandboxModel.Stage1Protocol.EnvelopeValues.changed_complete_intent
#audit_controller_proof SandboxModel.Stage1Protocol.EnvelopeValues.both_inputs_have_valid_shape
