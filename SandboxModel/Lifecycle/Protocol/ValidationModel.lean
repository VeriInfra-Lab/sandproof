import SandproofStage1Request.Types
import SandboxModel.Foundations.Model

/-! Independent guard predicates and ordered errors. No generated function is imported.
Create validity reuses the existing foundations model; this is not authorization. -/

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.internal.lifecycle
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
namespace SandboxModel.Stage1Protocol.Validation
open SandboxModel.Foundations

def resourceView (r : Resources) : ResourceVector :=
  ⟨r.instances.val, r.cpu.val, r.memory_bytes.val, r.disk_bytes.val⟩
def createView (r : CreateBinding) : CreatePlan :=
  ⟨r.template_revision, r.template_digest.val.map (·.val), resourceView r.resources,
   r.options.val.map (·.val), r.startup_deadline_ms.val, r.timeout_ms.val, r.max_lifetime_ms.val⟩

def expectationOK (operation : LifecycleOperation) (expected : ExpectedSession) : Prop :=
  match operation, expected with
  | .Create _ _, .Absent => True
  | .Unload _ _, .Version _ | .Resume _ _ _ _, .Version _
  | .Renew _ _ _, .Version _ | .Destroy, .Version _ => True
  | _, _ => False

def operationOK (operation : LifecycleOperation) : Prop :=
  match operation with
  | .Create create _ => (createView create).valid
  | .Resume source _ target _ => source.val < target.val
  | .Unload _ _ | .Renew _ _ _ | .Destroy => True

def headersOK (request : LifecycleRequest) (node : NodeIdentity) : Prop :=
  request.binding.protocol_version.val = 3 ∧ request.binding.node = node ∧
    request.binding.key.deployment = request.binding.node.deployment_id

def ValidRequest (request : LifecycleRequest) (node : NodeIdentity) : Prop :=
  headersOK request node ∧ expectationOK request.operation request.binding.expected ∧
    operationOK request.operation

def FirstError (request : LifecycleRequest) (node : NodeIdentity) (error : ValidationError) : Prop :=
  match error with
  | .UnsupportedVersion => request.binding.protocol_version.val ≠ 3
  | .ForeignNode => request.binding.protocol_version.val = 3 ∧ request.binding.node ≠ node
  | .ForeignDeployment => request.binding.protocol_version.val = 3 ∧ request.binding.node = node ∧
      request.binding.key.deployment ≠ request.binding.node.deployment_id
  | .InvalidExpectation => headersOK request node ∧
      ¬ expectationOK request.operation request.binding.expected
  | .InvalidCreate => headersOK request node ∧
      expectationOK request.operation request.binding.expected ∧
      match request.operation with
      | .Create create _ => ¬ (createView create).valid
      | _ => False
  | .InvalidIncarnation => headersOK request node ∧
      expectationOK request.operation request.binding.expected ∧
      match request.operation with
      | .Resume source _ target _ => target.val ≤ source.val
      | _ => False

noncomputable def expectedResult (request : LifecycleRequest) (node : NodeIdentity) :
    core.result.Result Unit ValidationError := by
  classical
  exact if request.binding.protocol_version.val ≠ 3 then .Err .UnsupportedVersion
    else if request.binding.node ≠ node then .Err .ForeignNode
    else if request.binding.key.deployment ≠ request.binding.node.deployment_id then .Err .ForeignDeployment
    else if ¬ expectationOK request.operation request.binding.expected then .Err .InvalidExpectation
    else match request.operation with
      | .Create create _ => if (createView create).valid then .Ok () else .Err .InvalidCreate
      | .Resume source _ target _ => if source.val < target.val then .Ok () else .Err .InvalidIncarnation
      | .Unload _ _ | .Renew _ _ _ | .Destroy => .Ok ()

def FullValidationContract
    (run : LifecycleRequest → NodeIdentity → Result (core.result.Result Unit ValidationError)) : Prop :=
  ∀ request node,
    (run request node = .ok (.Ok ()) ↔ ValidRequest request node) ∧
    ∀ error, (run request node = .ok (.Err error) ↔ FirstError request node error)

end SandboxModel.Stage1Protocol.Validation
