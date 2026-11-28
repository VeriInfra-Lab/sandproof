import SandboxModel.Lifecycle.Protocol.CreateObservationModel
import SandboxModel.Lifecycle.Protocol.ReadyFactValues

/-! Contract witnesses only: these are not a refinement of matches_request. -/
set_option autoImplicit false
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.authority sandproof_protocol.lifecycle.request
open sandproof_protocol.lifecycle.observation
open SandboxModel.Stage1Protocol.Validation SandboxModel.Stage1Protocol.ReadyFactValues
open SandboxModel.Stage1Protocol.CreateBinding SandboxModel.Stage1Protocol.ReadyFacts
namespace SandboxModel.Stage1Protocol.CreateValues

def newer (operation : LifecycleOperation) : AcceptedLifecycleIntent := {
  createIntent with
  accepted_version := 2#u64
  request := {
    binding := { createIntent.request.binding with
      expected := .Version 1#u64
      key := { createIntent.request.binding.key with window := 2#u64, sequence := 2#u64 } }
    operation := operation } }
def renewed (deadline : I64) (extend : Bool) : AcceptedLifecycleIntent :=
  newer (.Renew 1#u64 deadline extend)
def unloaded : AcceptedLifecycleIntent := newer (.Unload 1#u64 1#u64)
def destroyed : AcceptedLifecycleIntent := newer .Destroy
def observation (current : AcceptedLifecycleIntent) (readiness : CurrentCreateReadiness)
    (history : Bool) : CreateObservation := ⟨createIntent, current, history, readiness⟩
def changedRequest : CreateObservation :=
  { observation createIntent .Pending false with request := changedCreate }
def changedRetention : AcceptedLifecycleIntent := {
  renewed 50#i64 false with retention := { createIntent.retention with window := 2#u64 } }
def negativeOriginal : AcceptedLifecycleIntent := {
  createIntent with request := { createIntent.request with
    operation := .Create
      { createPayload with
        startup_deadline_ms := 1#u64
        max_lifetime_ms := 18446744073709551614#u64 } 1#u64 } }
def wrapFacts : CurrentReadyFacts :=
  ⟨0#u64, 18446744073709551614#u64, 18446744073709551614#u64, 1#u64, 0#u64⟩
def wrapObservation : CreateObservation :=
  ⟨negativeOriginal, renewed ((-2)#i64) false, true, .Observed wrapFacts⟩

theorem renewed_is_valid (deadline : I64) (extend : Bool) :
    ValidEnvelope (renewed deadline extend) EnvelopeValues.context := by
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  simp [ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    renewed, newer, createIntent, EnvelopeValues.intent, EnvelopeValues.context,
    EnvelopeValues.center, EnvelopeValues.node, checked]

theorem unload_destroy_are_valid :
    ValidEnvelope unloaded EnvelopeValues.context ∧
    ValidEnvelope destroyed EnvelopeValues.context := by
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  simp [ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    unloaded, destroyed, newer, createIntent, EnvelopeValues.intent, EnvelopeValues.context,
    EnvelopeValues.center, EnvelopeValues.node, checked]

theorem negative_original_is_valid : ValidEnvelope negativeOriginal EnvelopeValues.context := by
  simp [ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    negativeOriginal, createIntent, createPayload, EnvelopeValues.intent, EnvelopeValues.context,
    EnvelopeValues.center, EnvelopeValues.node, createView, resourceView,
    SandboxModel.Foundations.CreatePlan.valid, SandboxModel.Foundations.ResourceVector.positive,
    SandboxModel.Foundations.maxCounter]

theorem changed_retention_is_valid : ValidEnvelope changedRetention EnvelopeValues.context := by
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  simp [ValidEnvelope, ValidRequest, headersOK, expectationOK, operationOK, retentionOK, versionOK,
    changedRetention, renewed, newer, createIntent, EnvelopeValues.intent, EnvelopeValues.context,
    EnvelopeValues.center, EnvelopeValues.node, checked]

theorem normal_contract_witnesses :
    Matches (observation createIntent .Pending false) createIntent ∧
    Matches (observation createIntent (.Observed facts) true) createIntent ∧
    Matches (observation (renewed 50#i64 false) (.Observed facts) true) createIntent ∧
    Matches (observation unloaded (.Observed facts) true) createIntent ∧
    Matches (observation destroyed .NotCurrent false) createIntent ∧
    Matches (observation createIntent (.Refused .Capacity) false) createIntent ∧
    Matches (observation destroyed (.Refused .Capacity) false) createIntent := by
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  have endChecked : U64.checked_add 10#u64 40#u64 = some 50#u64 := by decide
  have maxChecked : U64.checked_add 10#u64 90#u64 = some 100#u64 := by decide
  simp [Matches, Header, Body, Refused, SameReadiness, ContinuedReadiness, RenewReadiness,
    RenewedFacts, ResumeBinding.Continued, ResumeBinding.Newer, Timing, CreateMatches,
    ValidRequest, headersOK, expectationOK, operationOK, versionOK,
    observation, renewed, unloaded, destroyed, newer, createIntent, createPayload, facts,
    EnvelopeValues.intent, EnvelopeValues.center, EnvelopeValues.node, checked, endChecked, maxChecked,
    createView, resourceView, SandboxModel.Foundations.CreatePlan.valid,
    SandboxModel.Foundations.ResourceVector.positive, SandboxModel.Foundations.maxCounter]

theorem omitted_fields_violate_contract :
    ¬ Matches changedRequest createIntent ∧
    ¬ Matches (observation changedRetention .Pending false) createIntent ∧
    ¬ Matches (observation createIntent (.Refused .Capacity) true) createIntent ∧
    ¬ Matches wrapObservation negativeOriginal := by
  simp [Matches, Header, Body, Refused, RenewReadiness, RenewedFacts,
    changedRequest, observation, changedRetention, wrapObservation, negativeOriginal, wrapFacts,
    renewed, newer, createIntent, changedCreate, createPayload, EnvelopeValues.intent,
    EnvelopeValues.center, EnvelopeValues.node]

theorem negative_pending_satisfies_contract :
    Matches (observation (renewed ((-2)#i64) false) .Pending false) createIntent := by
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  simp [Matches, Header, Body, RenewReadiness, ValidRequest, headersOK, expectationOK, operationOK,
    versionOK, observation, renewed, newer, createIntent, createPayload,
    EnvelopeValues.intent, EnvelopeValues.center, EnvelopeValues.node, checked]

theorem reused_key_satisfies_contract :
    let sameKeyRenew := { renewed 50#i64 false with request := {
      (renewed 50#i64 false).request with binding := {
        (renewed 50#i64 false).request.binding with key := createIntent.request.binding.key } } }
    let sameKeyDestroy := { destroyed with request := { destroyed.request with binding := {
      destroyed.request.binding with key := createIntent.request.binding.key } } }
    Matches (observation sameKeyRenew .Pending false) createIntent ∧
    Matches (observation sameKeyDestroy .NotCurrent false) createIntent := by
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  simp [Matches, Header, Body, RenewReadiness, ValidRequest, headersOK, expectationOK, operationOK,
    versionOK, observation, renewed, destroyed, newer, createIntent, createPayload,
    EnvelopeValues.intent, EnvelopeValues.center, EnvelopeValues.node, checked]

theorem changed_non_refused_does_not_require_valid_original :
    let old := { createIntent with request := { createIntent.request with binding := {
      createIntent.request.binding with protocol_version := 3#u16 } } }
    ¬ ValidRequest old.request old.request.binding.node ∧
      Matches ⟨old, destroyed, false, .NotCurrent⟩ old := by
  simp [Matches, Header, Body, ValidRequest, headersOK, expectationOK, operationOK,
    destroyed, newer, createIntent, createPayload, EnvelopeValues.intent,
    EnvelopeValues.center, EnvelopeValues.node]

end SandboxModel.Stage1Protocol.CreateValues

#audit_controller_proof SandboxModel.Stage1Protocol.CreateValues.renewed_is_valid
#audit_controller_proof SandboxModel.Stage1Protocol.CreateValues.unload_destroy_are_valid
#audit_controller_proof SandboxModel.Stage1Protocol.CreateValues.negative_original_is_valid
#audit_controller_proof SandboxModel.Stage1Protocol.CreateValues.changed_retention_is_valid
#audit_controller_proof SandboxModel.Stage1Protocol.CreateValues.normal_contract_witnesses
#audit_controller_proof SandboxModel.Stage1Protocol.CreateValues.omitted_fields_violate_contract
#audit_controller_proof SandboxModel.Stage1Protocol.CreateValues.negative_pending_satisfies_contract
#audit_controller_proof SandboxModel.Stage1Protocol.CreateValues.reused_key_satisfies_contract
#audit_controller_proof SandboxModel.Stage1Protocol.CreateValues.changed_non_refused_does_not_require_valid_original
