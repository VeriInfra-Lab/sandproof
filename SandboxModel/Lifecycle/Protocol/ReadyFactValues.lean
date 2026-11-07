import SandboxModel.Lifecycle.Protocol.ReadyFactModel
import SandboxModel.Lifecycle.Protocol.Accepted
import SandboxModel.Lifecycle.Protocol.RequestValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal.lifecycle sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.observation sandproof_protocol.lifecycle.unload
open SandboxModel.Stage1Protocol.ReadyFacts SandboxModel.Stage1Protocol.Validation
namespace SandboxModel.Stage1Protocol.ReadyFactValues

def createPayload : CreateBinding := {
  template_revision := "base", template_digest := Request.digest,
  resources := ⟨1#u64, 1#u64, 1#u64, 1#u64⟩,
  options := alloc.vec.Vec.from [1#u8] (by scalar_tac),
  startup_deadline_ms := 20#u64, timeout_ms := 40#u64, max_lifetime_ms := 90#u64 }
def createIntent : AcceptedLifecycleIntent := {
  EnvelopeValues.intent 1#u64 with request := {
    binding := { (EnvelopeValues.intent 1#u64).request.binding with expected := .Absent },
    operation := .Create createPayload 1#u64 } }
def resumeIntent : AcceptedLifecycleIntent := {
  EnvelopeValues.intent 1#u64 with accepted_version := 2#u64, request := {
    binding := { (EnvelopeValues.intent 1#u64).request.binding with expected := .Version 1#u64 },
    operation := .Resume 1#u64 1#u64 3#u64 Request.digest } }
def currentIntent (deadline : I64) (extend : Bool) : AcceptedLifecycleIntent := {
  resumeIntent with
  accepted_version := 3#u64
  request := {
    binding := {
      resumeIntent.request.binding with
      expected := .Version 2#u64
      key := { resumeIntent.request.binding.key with window := 2#u64, sequence := 2#u64 } }
    operation := .Renew 3#u64 deadline extend } }
def facts : CurrentReadyFacts := ⟨10#u64, 50#u64, 100#u64, 20#u64, 30#u64⟩
def source : UnloadedFacts := ⟨1#u64, 1#u64, Request.digest, 64#u64, 10#u64, 50#u64, 100#u64, 20#u64⟩
def changedDigest : sandproof_protocol.lifecycle.digest.ContentDigest :=
  Array.from (1#u8 :: List.replicate 31 (0#u8)) (by simp)
def changedSource : UnloadedFacts := { source with manifest_digest := changedDigest }
def changedCreate : AcceptedLifecycleIntent := {
  createIntent with
  request := {
    createIntent.request with
    operation := .Create { createPayload with timeout_ms := 30#u64 } 1#u64 } }
def negativeFacts : CurrentReadyFacts := {
  facts with
  end_at_ms := 18446744073709551615#u64
  max_end_at_ms := 18446744073709551615#u64 }
def negativeSource : UnloadedFacts := { source with max_end_at_ms := 18446744073709551615#u64 }

theorem create_is_valid : ValidEnvelope createIntent EnvelopeValues.context := by
  simp [ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    createIntent, createPayload, EnvelopeValues.intent, EnvelopeValues.context, EnvelopeValues.center,
    EnvelopeValues.node, createView, resourceView, SandboxModel.Foundations.CreatePlan.valid,
    SandboxModel.Foundations.ResourceVector.positive, SandboxModel.Foundations.maxCounter]

theorem changed_create_is_valid : ValidEnvelope changedCreate EnvelopeValues.context := by
  simp [ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    changedCreate, createIntent, createPayload, EnvelopeValues.intent, EnvelopeValues.context, EnvelopeValues.center,
    EnvelopeValues.node, createView, resourceView, SandboxModel.Foundations.CreatePlan.valid,
    SandboxModel.Foundations.ResourceVector.positive, SandboxModel.Foundations.maxCounter]

theorem resume_is_valid : ValidEnvelope resumeIntent EnvelopeValues.context := by
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  simp [ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    resumeIntent, EnvelopeValues.intent, EnvelopeValues.context, EnvelopeValues.center, EnvelopeValues.node, checked]

theorem current_is_valid (deadline : I64) (extend : Bool) :
    ValidEnvelope (currentIntent deadline extend) EnvelopeValues.context := by
  have checked : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
  simp [ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    currentIntent, resumeIntent, EnvelopeValues.intent, EnvelopeValues.context, EnvelopeValues.center,
    EnvelopeValues.node, checked]

theorem changed_digest_differs : changedDigest ≠ Request.digest := by
  intro same
  have values := congrArg (fun x => x.val) same
  simp [changedDigest, Request.digest] at values


end SandboxModel.Stage1Protocol.ReadyFactValues

#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.create_is_valid
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.changed_create_is_valid
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.resume_is_valid
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.current_is_valid
#audit_controller_proof SandboxModel.Stage1Protocol.ReadyFactValues.changed_digest_differs
