import SandboxModel.Lifecycle.Protocol.RenewalModel
import SandboxModel.Lifecycle.Protocol.EnvelopeValues

/-! Shared legal values and distinguishing witnesses; no durable or physical facts. -/
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.renewal
open SandboxModel.Stage1Protocol.Validation
namespace SandboxModel.Stage1Protocol.RenewalValues

def intent : AcceptedLifecycleIntent := {
  EnvelopeValues.intent 1#u64 with
  accepted_version := 2#u64
  request := {
    (EnvelopeValues.intent 1#u64).request with
    binding := {
      (EnvelopeValues.intent 1#u64).request.binding with
      key := { (EnvelopeValues.intent 1#u64).request.binding.key with window := 2#u64 }
      expected := .Version 1#u64 }
    operation := .Renew 1#u64 200#i64 false } }

def exactResult : RenewalConfirmation := ⟨intent, 200#i64⟩
def extended (effective : I64) : RenewalConfirmation :=
  ⟨{ intent with request := { intent.request with operation := .Renew 1#u64 200#i64 true } },
    effective⟩
def changedRetention : AcceptedLifecycleIntent :=
  { intent with retention := ⟨1#u16, 2#u64⟩ }
def firstVersion : RenewalConfirmation := {
  exactResult with request := {
    intent with
    accepted_version := 1#u64
    request := { intent.request with binding := { intent.request.binding with expected := .Version 0#u64 } } } }

theorem legal_exact : Renewal.Shape exactResult := by
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  simp [Renewal.Shape, Renewal.Deadline, exactResult, intent, EnvelopeValues.intent, EnvelopeValues.center, EnvelopeValues.node,
    retentionOK, versionOK, ValidRequest, headersOK, expectationOK, operationOK, checked]

theorem legal_extension : Renewal.Shape (extended 201#i64) := by
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  simp [Renewal.Shape, Renewal.Deadline, extended, intent, EnvelopeValues.intent, EnvelopeValues.center, EnvelopeValues.node,
    retentionOK, versionOK, ValidRequest, headersOK, expectationOK, operationOK, checked]

theorem changed_request_is_distinct : intent ≠ changedRetention := by
  simp [intent, changedRetention, EnvelopeValues.intent]

theorem changed_deadline_is_distinct : extended 200#i64 ≠ extended 201#i64 := by
  simp [extended]

theorem lower_extension_is_invalid : ¬ Renewal.Shape (extended 199#i64) := by
  simp [Renewal.Shape, Renewal.Deadline, extended, intent]

theorem first_version_is_invalid : ¬ Renewal.Shape firstVersion := by
  simp [Renewal.Shape, firstVersion]

end SandboxModel.Stage1Protocol.RenewalValues
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.legal_exact
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.legal_extension
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.changed_request_is_distinct
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.changed_deadline_is_distinct
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.lower_extension_is_invalid
#audit_controller_proof SandboxModel.Stage1Protocol.RenewalValues.first_version_is_invalid
