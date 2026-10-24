import SandboxModel.Lifecycle.Protocol.Validation
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1RequestRust
open sandproof_protocol.internal sandproof_protocol.lifecycle.authority
open sandproof_protocol.lifecycle.identity sandproof_protocol.lifecycle.request
namespace SandboxModel.Stage1Protocol.Validation
open SandboxModel.Stage1Protocol

theorem retention_valid (value : RetentionBinding) :
    RetentionBinding.valid_for value 4096#u16 = .ok (by classical exact decide (retentionOK value)) := by
  classical
  simp [RetentionBinding.valid_for, retentionOK, Replay.ok_and]

theorem version_comparison (intent : AcceptedLifecycleIntent) :
    (do let version ← AcceptedLifecycleIntent.accepted_version_for_request intent
        core.option.Option.Insts.CoreCmpPartialEqOption.eq core.cmp.PartialEqU64
          version (some intent.accepted_version)) =
      .ok (by classical exact decide (versionOK intent)) := by
  classical
  cases he : intent.request.binding.expected <;>
    simp [AcceptedLifecycleIntent.accepted_version_for_request, he, versionOK,
      Library.option_u64_eq]
  all_goals exact eq_comm

theorem envelope_refines (intent : AcceptedLifecycleIntent) (context : AuthenticatedCenterContext) :
    accepted_validate intent context = .ok (expectedAccepted intent context) := by
  classical
  have h_one : (1#u64 = intent.accepted_version) ↔ intent.accepted_version.val = 1 := by scalar_tac
  cases he : intent.request.binding.expected <;>
    cases hr : expectedResult intent.request context.expected_node <;>
    simp [accepted_validate, AcceptedLifecycleIntent.validate, expectedAccepted,
      AuthenticatedCenterContext.impl.center, AuthenticatedCenterContext.impl.expected_node,
      core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      Replay.center_eq, sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq,
      alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
      validate_body_refines, hr, retention_valid, ok_ite, h_one,
      AcceptedLifecycleIntent.accepted_version_for_request, he, versionOK,
      Library.option_u64_eq, core.result.Result.map_err,
      P.Insts.CoreOpsFunctionFnOnceTupleValidationErrorAcceptedIntentError.call_once,
      AcceptedIntentError.Request.constructor, core.result.Result.Insts.CoreOpsTry.branch,
      core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual,
      ]

theorem envelope_complete : FullEnvelopeContract accepted_validate := by
  intro intent context
  exact envelope_refines intent context


theorem expectedAccepted_success_iff (intent : AcceptedLifecycleIntent)
    (context : AuthenticatedCenterContext) :
    expectedAccepted intent context = .Ok () ↔ ValidEnvelope intent context := by
  classical
  cases hr : expectedResult intent.request context.expected_node with
  | Ok value =>
    cases value
    have request_ok := (expected_success_iff intent.request context.expected_node).mp hr
    simp [expectedAccepted, ValidEnvelope, hr, request_ok, ite_eq_iff]
  | Err error =>
    have request_bad : ¬ ValidRequest intent.request context.expected_node := by
      intro valid
      have h := (expected_success_iff intent.request context.expected_node).mpr valid
      rw [hr] at h
      cases h
    simp [expectedAccepted, ValidEnvelope, hr, request_bad, ite_eq_iff]

theorem envelope_accepts_iff (intent : AcceptedLifecycleIntent)
    (context : AuthenticatedCenterContext) :
    accepted_validate intent context = .ok (.Ok ()) ↔ ValidEnvelope intent context := by
  rw [envelope_refines]
  simpa using expectedAccepted_success_iff intent context

theorem envelope_keeps_context_center (intent : AcceptedLifecycleIntent)
    (context : AuthenticatedCenterContext)
    (accepted : accepted_validate intent context = .ok (.Ok ())) : intent.center = context.center :=
  ((envelope_accepts_iff intent context).mp accepted).1

theorem envelope_keeps_context_node (intent : AcceptedLifecycleIntent)
    (context : AuthenticatedCenterContext)
    (accepted : accepted_validate intent context = .ok (.Ok ())) :
    intent.request.binding.node = context.expected_node :=
  (((envelope_accepts_iff intent context).mp accepted).2.2.1).1.2.1

theorem envelope_retention_shape (intent : AcceptedLifecycleIntent)
    (context : AuthenticatedCenterContext)
    (accepted : accepted_validate intent context = .ok (.Ok ())) : retentionOK intent.retention :=
  ((envelope_accepts_iff intent context).mp accepted).2.2.2.1

theorem envelope_version_condition (intent : AcceptedLifecycleIntent)
    (context : AuthenticatedCenterContext)
    (accepted : accepted_validate intent context = .ok (.Ok ())) : versionOK intent :=
  ((envelope_accepts_iff intent context).mp accepted).2.2.2.2

theorem foreign_center_rejected_first (intent : AcceptedLifecycleIntent)
    (context : AuthenticatedCenterContext) (foreign : intent.center ≠ context.center) :
    accepted_validate intent context = .ok (.Err .ForeignCenter) := by
  rw [envelope_refines]
  simp [expectedAccepted, foreign]

end SandboxModel.Stage1Protocol.Validation

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.retention_valid
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.version_comparison
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.envelope_refines
#audit_controller_proof SandboxModel.Stage1Protocol.Validation.envelope_complete

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.expectedAccepted_success_iff

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.envelope_accepts_iff

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.envelope_keeps_context_center

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.envelope_keeps_context_node

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.envelope_retention_shape

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.envelope_version_condition

#audit_controller_proof SandboxModel.Stage1Protocol.Validation.foreign_center_rejected_first
