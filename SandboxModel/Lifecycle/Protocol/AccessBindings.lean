import SandboxModel.Lifecycle.Protocol.AccessModel
import SandboxModel.Lifecycle.Protocol.Accepted
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.lifecycle.access sandproof_protocol.lifecycle.authority
namespace SandboxModel.Stage1Protocol.AccessBinding

theorem validate_refines (identity : LifecycleAccessIdentity) (context : AuthenticatedCenterContext) :
    access_validate identity context = .ok (expected identity context) := by
  classical
  have zero : (identity.accepted_version = 0#u64) ↔ identity.accepted_version.val = 0 := by scalar_tac
  by_cases center : identity.center.deployment = identity.node.deployment_id <;>
    by_cases operation : identity.operation.deployment = identity.node.deployment_id <;>
    simp [access_validate, LifecycleAccessIdentity.validate, expected, Deployments, center, operation,
      sandproof_protocol.lifecycle.PROTOCOL_VERSION,
      AuthenticatedCenterContext.impl.center, AuthenticatedCenterContext.impl.expected_node,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      Replay.center_eq, Replay.node_eq,
      sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
      alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq, Validation.ok_ite, zero]

theorem complete : FullContract access_validate := validate_refines

theorem expected_success_iff (identity : LifecycleAccessIdentity) (context : AuthenticatedCenterContext) :
    expected identity context = .Ok () ↔ Valid identity context := by
  classical
  simp [expected, Valid, ite_eq_iff]

theorem expected_error_iff (identity : LifecycleAccessIdentity) (context : AuthenticatedCenterContext)
    (error : AccessIdentityError) : expected identity context = .Err error ↔ FirstError identity context error := by
  classical
  cases error <;> simp [expected, FirstError, ite_eq_iff]

theorem accepts_iff (identity : LifecycleAccessIdentity) (context : AuthenticatedCenterContext) :
    access_validate identity context = .ok (.Ok ()) ↔ Valid identity context := by
  rw [validate_refines]
  simpa using expected_success_iff identity context

theorem error_iff (identity : LifecycleAccessIdentity) (context : AuthenticatedCenterContext)
    (error : AccessIdentityError) :
    access_validate identity context = .ok (.Err error) ↔ FirstError identity context error := by
  rw [validate_refines]
  simpa using expected_error_iff identity context error

theorem preserves_independent_context (identity : LifecycleAccessIdentity) (context : AuthenticatedCenterContext)
    (accepted : access_validate identity context = .ok (.Ok ())) :
    identity.center = context.center ∧ identity.node = context.expected_node := by
  have valid := (accepts_iff identity context).mp accepted
  exact ⟨valid.2.1, valid.2.2.1⟩

theorem business_fields_not_checked (identity other : LifecycleAccessIdentity)
    (context : AuthenticatedCenterContext) :
    access_validate (replaceBusinessFields identity other) context = access_validate identity context := by
  rw [validate_refines, validate_refines]
  rfl

end SandboxModel.Stage1Protocol.AccessBinding

#audit_controller_proof SandboxModel.Stage1Protocol.AccessBinding.validate_refines
#audit_controller_proof SandboxModel.Stage1Protocol.AccessBinding.complete
#audit_controller_proof SandboxModel.Stage1Protocol.AccessBinding.expected_success_iff
#audit_controller_proof SandboxModel.Stage1Protocol.AccessBinding.expected_error_iff
#audit_controller_proof SandboxModel.Stage1Protocol.AccessBinding.accepts_iff
#audit_controller_proof SandboxModel.Stage1Protocol.AccessBinding.error_iff
#audit_controller_proof SandboxModel.Stage1Protocol.AccessBinding.preserves_independent_context
#audit_controller_proof SandboxModel.Stage1Protocol.AccessBinding.business_fields_not_checked
