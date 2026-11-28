import SandboxModel.Lifecycle.Protocol.ResultModel
import SandboxModel.Lifecycle.Protocol.EnvelopeValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.terminal sandproof_protocol.lifecycle.confirmation
open sandproof_protocol.lifecycle.identity
open SandboxModel.Stage1Protocol.Validation
namespace SandboxModel.Stage1Protocol.ConfirmationValues

def intent : AcceptedLifecycleIntent :=
  { EnvelopeValues.intent 1#u64 with request :=
    { (EnvelopeValues.intent 1#u64).request with operation := .Destroy } }
def terminal (status : TerminalStatus) : TerminalLifecycleObservation := ⟨intent, status⟩
def close : LifecycleWindowClose := ⟨terminal .Cleaned⟩
def closed (window : U64) : ClosedLifecycleWindow := ⟨terminal .Cleaned, window⟩
def changed : TerminalLifecycleObservation :=
  ⟨{ intent with retention := ⟨1#u16, 2#u64⟩ }, .Cleaned⟩
def legacy : TerminalLifecycleObservation :=
  ⟨{ intent with request := { intent.request with binding :=
    { intent.request.binding with protocol_version := 3#u16 } } }, .Cleaned⟩

theorem intent_shape : Confirmation.TerminalShape intent := by
  have checked : U64.checked_add 0#u64 1#u64 = some 1#u64 := by decide
  simp [Confirmation.TerminalShape, intent, EnvelopeValues.intent, retentionOK, versionOK, checked]

theorem legal_intent : ValidEnvelope intent EnvelopeValues.context := by
  have checked : U64.checked_add 0#u64 1#u64 = some 1#u64 := by decide
  simp [ValidEnvelope, intent, EnvelopeValues.intent, EnvelopeValues.context,
    EnvelopeValues.node, EnvelopeValues.center, ValidRequest, headersOK, expectationOK,
    operationOK, retentionOK, versionOK, checked]

theorem changed_legal : ValidEnvelope changed.request EnvelopeValues.context := by
  have checked : U64.checked_add 0#u64 1#u64 = some 1#u64 := by decide
  simp [ValidEnvelope, changed, intent, EnvelopeValues.intent, EnvelopeValues.context,
    EnvelopeValues.node, EnvelopeValues.center, ValidRequest, headersOK, expectationOK,
    operationOK, retentionOK, versionOK, checked]

theorem changed_complete_result : changed ≠ terminal .Cleaned := by
  simp [changed, terminal, intent, EnvelopeValues.intent]


end SandboxModel.Stage1Protocol.ConfirmationValues

#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.intent_shape
#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.legal_intent
#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.changed_legal
#audit_controller_proof SandboxModel.Stage1Protocol.ConfirmationValues.changed_complete_result
