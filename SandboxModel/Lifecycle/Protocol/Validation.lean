import SandboxModel.Lifecycle.Protocol.ReplayEquality
import SandboxModel.Lifecycle.Protocol.ValidationModel
import SandboxModel.ProofAuditCommand

open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.internal.lifecycle
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
namespace SandboxModel.Stage1Protocol.Validation
open SandboxModel.Foundations

theorem resources_positive (r : Resources) :
    Resources.positive r = .ok (decide (resourceView r).positive) := by
  simp [Resources.positive, resourceView, ResourceVector.positive, Replay.ok_and]

theorem checked_add_present (a b : U64) :
    (do let value ← lift (U64.checked_add a b)
        Result.ok value.isSome) =
      (.ok (decide (a.val + b.val ≤ U64.max)) : Result Bool) := by
  have hs := U64.checked_add_bv_spec a b
  cases he : U64.checked_add a b <;> simp_all [lift]

theorem ok_unless (p : Prop) [Decidable p] (q : Bool) :
    (if p then (Result.ok false : Result Bool) else .ok q) = .ok (decide (¬p) && q) := by
  by_cases hp : p <;> simp [hp]

theorem create_valid (p : CreateBinding) :
    CreateBinding.is_valid p = .ok (decide (createView p).valid) := by
  have hi : (p.resources.instances = 1#u64) ↔ p.resources.instances.val = 1 := by scalar_tac
  simp [CreateBinding.is_valid, resources_positive,
    alloc.vec.Vec.is_empty, SandproofFoundationsRust.alloc.vec.Vec.is_empty,
    MAX_OPTIONS_BYTES, checked_add_present, Replay.ok_and, ok_unless, hi, Bool.and_assoc, List.length_pos_iff,
    createView, CreatePlan.valid, resourceView, ResourceVector.positive, maxCounter, U64.max_eq]

theorem ok_ite {T : Type} (p : Prop) [Decidable p] (a b : T) :
    Result.ok (if p then a else b) = (if p then .ok a else .ok b) := by
  split <;> rfl

theorem validate_refines (request : LifecycleRequest) (node : NodeIdentity) :
    request_validate request node = .ok (expectedResult request node) := by
  classical
  cases ho : request.operation <;> cases he : request.binding.expected <;>
    simp [request_validate, LifecycleRequest.validate, expectedResult,
      expectationOK, ho, he, create_valid,
      sandproof_protocol.lifecycle.PROTOCOL_VERSION,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      Replay.node_eq, sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
      alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      IncarnationId.get, PositiveCounter.get, ok_ite]
  all_goals simp only [← Nat.not_lt, ite_not]

theorem expected_success_iff (request : LifecycleRequest) (node : NodeIdentity) :
    expectedResult request node = .Ok () ↔ ValidRequest request node := by
  classical
  cases ho : request.operation <;> cases he : request.binding.expected <;>
    simp [expectedResult, ValidRequest, headersOK, expectationOK, operationOK, ho, he, ite_eq_iff]

theorem expected_error_iff (request : LifecycleRequest) (node : NodeIdentity) (error : ValidationError) :
    expectedResult request node = .Err error ↔ FirstError request node error := by
  classical
  cases error <;> cases ho : request.operation <;> cases he : request.binding.expected <;>
    simp [expectedResult, FirstError, headersOK, expectationOK, ho, he, ite_eq_iff]

theorem validate_accepts_iff (request : LifecycleRequest) (node : NodeIdentity) :
    request_validate request node = .ok (.Ok ()) ↔ ValidRequest request node := by
  rw [validate_refines]
  simpa using expected_success_iff request node

theorem validate_error_iff (request : LifecycleRequest) (node : NodeIdentity) (error : ValidationError) :
    request_validate request node = .ok (.Err error) ↔ FirstError request node error := by
  rw [validate_refines]
  simpa using expected_error_iff request node error

theorem validate_complete : FullValidationContract request_validate := by
  intro request node
  exact ⟨validate_accepts_iff request node, validate_error_iff request node⟩

theorem validate_total (request : LifecycleRequest) (node : NodeIdentity) :
    ∃ result, request_validate request node = .ok result :=
  ⟨expectedResult request node, validate_refines request node⟩

end SandboxModel.Stage1Protocol.Validation

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.resources_positive
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.checked_add_present
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.ok_unless
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.create_valid
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.ok_ite
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.validate_refines
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.expected_success_iff
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.expected_error_iff
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.validate_accepts_iff
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.validate_error_iff
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.validate_complete
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.validate_total
