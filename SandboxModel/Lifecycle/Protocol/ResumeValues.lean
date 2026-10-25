import SandboxModel.Lifecycle.Protocol.ResumeModel
import SandboxModel.Lifecycle.Protocol.ReadyFactValues
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.resume sandproof_protocol.lifecycle.observation
open SandboxModel.Stage1Protocol.Validation SandboxModel.Stage1Protocol.ReadyFactValues
namespace SandboxModel.Stage1Protocol.ResumeValues

def observation (current : AcceptedLifecycleIntent) (readiness : CurrentResumeReadiness) : ResumeObservation :=
  ⟨resumeIntent, current, readiness⟩
def changedOriginal : AcceptedLifecycleIntent := { resumeIntent with retention := ⟨1#u16, 2#u64⟩ }
def changedRequestObservation : ResumeObservation := ⟨changedOriginal, resumeIntent, .Pending⟩
def changedRetention : AcceptedLifecycleIntent := { currentIntent 50#i64 false with retention := ⟨1#u16, 2#u64⟩ }
def unloadIntent : AcceptedLifecycleIntent := {
  currentIntent 50#i64 false with request := {
    (currentIntent 50#i64 false).request with operation := .Unload 3#u64 2#u64 } }
def wrongUnloadSource : AcceptedLifecycleIntent := {
  unloadIntent with request := {
    unloadIntent.request with operation := .Unload 1#u64 2#u64 } }
def destroyIntent : AcceptedLifecycleIntent := {
  currentIntent 50#i64 false with request := {
    (currentIntent 50#i64 false).request with operation := .Destroy } }
def wrongIncarnation : AcceptedLifecycleIntent := {
  currentIntent 50#i64 false with request := {
    (currentIntent 50#i64 false).request with operation := .Renew 1#u64 50#i64 false } }

theorem changed_original_is_valid : ValidEnvelope changedOriginal EnvelopeValues.context := by
  have valid := resume_is_valid
  simpa [changedOriginal, ValidEnvelope, retentionOK, versionOK, resumeIntent, EnvelopeValues.intent] using valid

theorem followups_are_valid :
    ValidEnvelope unloadIntent EnvelopeValues.context ∧
    ValidEnvelope destroyIntent EnvelopeValues.context ∧
    ValidEnvelope wrongIncarnation EnvelopeValues.context ∧
    ValidEnvelope changedRetention EnvelopeValues.context := by
  have checked : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
  simp [unloadIntent, destroyIntent, wrongIncarnation, changedRetention, currentIntent, resumeIntent,
    ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    EnvelopeValues.context, EnvelopeValues.intent, EnvelopeValues.center, EnvelopeValues.node, checked]

theorem changed_original_differs : changedOriginal ≠ resumeIntent := by
  intro same
  have window := congrArg (fun x => x.retention.window) same
  simp [changedOriginal, resumeIntent, EnvelopeValues.intent] at window

theorem counterexamples_outside_contract :
    ¬ ResumeBinding.Matches changedRequestObservation resumeIntent ∧
    ¬ ResumeBinding.Matches (observation changedRetention .Pending) resumeIntent ∧
    ¬ ResumeBinding.Matches (observation wrongUnloadSource .StateLost) resumeIntent ∧
    ¬ ResumeBinding.Matches (observation wrongIncarnation .Pending) resumeIntent := by
  simp [ResumeBinding.Matches, ResumeBinding.CurrentShape, ResumeBinding.ReadinessMatches,
    ResumeBinding.UnloadReadiness, ResumeBinding.RenewReadiness, ResumeBinding.Newer, ResumeBinding.Continued,
    observation, changedRequestObservation, changedOriginal, changedRetention, wrongUnloadSource, unloadIntent,
    wrongIncarnation, currentIntent, resumeIntent, EnvelopeValues.intent]

theorem wrong_unload_source_is_valid : ValidEnvelope wrongUnloadSource EnvelopeValues.context := by
  have checked : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
  simp [wrongUnloadSource, unloadIntent, currentIntent, resumeIntent,
    ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    EnvelopeValues.context, EnvelopeValues.intent, EnvelopeValues.center, EnvelopeValues.node, checked]

end SandboxModel.Stage1Protocol.ResumeValues

#audit_controller_proof SandboxModel.Stage1Protocol.ResumeValues.changed_original_is_valid
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeValues.followups_are_valid
#audit_controller_proof SandboxModel.Stage1Protocol.ResumeValues.changed_original_differs

#audit_controller_proof SandboxModel.Stage1Protocol.ResumeValues.counterexamples_outside_contract

#audit_controller_proof SandboxModel.Stage1Protocol.ResumeValues.wrong_unload_source_is_valid
