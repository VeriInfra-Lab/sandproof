import SandboxModel.Lifecycle.Protocol.Validation
import SandboxModel.Lifecycle.Protocol.RequestValues
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.internal.lifecycle
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
namespace SandboxModel.Stage1Protocol.Validation
open SandboxModel.Foundations

namespace Examples
open SandboxModel.Stage1Protocol.Request

def createPayload : CreateBinding := {
  template_revision := "base", template_digest := digest,
  resources := ⟨1#u64, 1#u64, 1#u64, 1#u64⟩,
  options := alloc.vec.Vec.from [1#u8] (by scalar_tac),
  startup_deadline_ms := 2#u64, timeout_ms := 1#u64, max_lifetime_ms := 1#u64 }
def createRequest : LifecycleRequest := ⟨{ binding with expected := .Absent }, .Create createPayload 1#u64⟩
def destroyRequest : LifecycleRequest := ⟨binding, .Destroy⟩

theorem create_accepted : request_validate createRequest node = .ok (.Ok ()) := by
  apply (validate_accepts_iff createRequest node).mpr
  simp [ValidRequest, headersOK, expectationOK, operationOK, createRequest, createPayload,
    binding, node, createView, CreatePlan.valid, resourceView, ResourceVector.positive, maxCounter]

theorem unload_accepted : request_validate (unload 1#u64) node = .ok (.Ok ()) := by
  apply (validate_accepts_iff (unload 1#u64) node).mpr
  simp [ValidRequest, headersOK, expectationOK, operationOK, unload, binding, node]

theorem resume_accepted : request_validate (resume 1#u64 2#u64) node = .ok (.Ok ()) := by
  apply (validate_accepts_iff (resume 1#u64 2#u64) node).mpr
  simp [ValidRequest, headersOK, expectationOK, operationOK, resume, binding, node]

theorem destroy_accepted : request_validate destroyRequest node = .ok (.Ok ()) := by
  apply (validate_accepts_iff destroyRequest node).mpr
  simp [ValidRequest, headersOK, expectationOK, operationOK, destroyRequest, binding, node]
theorem renew_accepted :
    request_validate ⟨binding, .Renew 1#u64 (-1#i64) false⟩ node = .ok (.Ok ()) := by
  apply validate_accepts_iff.mpr
  simp [ValidRequest, headersOK, expectationOK, operationOK, binding, node]

end Examples
end SandboxModel.Stage1Protocol.Validation

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.Examples.create_accepted
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.Examples.unload_accepted
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.Examples.resume_accepted
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.Examples.destroy_accepted

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.Examples.renew_accepted
