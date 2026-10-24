import SandboxModel.Lifecycle.Protocol.RequestValues
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1RequestRust
namespace SandboxModel.Stage1Protocol.Request
theorem unsupported_version_first (request : WireRequest) (expected : Node)
    (wrong : request.binding.protocol_version ≠ 3#u16) :
    request_validate request expected = .ok (.Err sandproof_protocol.lifecycle.request.ValidationError.UnsupportedVersion) := by
  simp [request_validate, sandproof_protocol.lifecycle.request.LifecycleRequest.validate,
    sandproof_protocol.lifecycle.PROTOCOL_VERSION, wrong]

theorem resume_order : ResumeOrderContract sandproof_protocol.lifecycle.request.ValidationError.InvalidIncarnation runResume := by
  intro source target
  simp [runResume, resume, node, binding, request_validate, sandproof_protocol.lifecycle.request.LifecycleRequest.validate, sandproof_protocol.lifecycle.PROTOCOL_VERSION, sandproof_protocol.lifecycle.identity.IncarnationId.get, sandproof_protocol.internal.PositiveCounter.get, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq, sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq, sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq, sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq, SandproofStage1RequestRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq, SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
  split <;> simp_all

theorem resume_equal_refused (value : U64) :
    runResume value value = .ok (.Err sandproof_protocol.lifecycle.request.ValidationError.InvalidIncarnation) := by
  simpa using resume_order value value

theorem resume_nonempty_success : runResume 1#u64 2#u64 = .ok (.Ok ()) := by
  simpa using resume_order 1#u64 2#u64

theorem replay_complete_pair : ReplayPairContract replay_matches (unload 1#u64) (unload 2#u64) := by
  constructor
  all_goals simp [Aeneas.Std.lift, unload, replay_matches, node, binding, sandproof_protocol.lifecycle.request.LifecycleRequest.replay_matches, sandproof_protocol.lifecycle.request.LifecycleRequest.Insts.CoreCmpPartialEqLifecycleRequest.eq, sandproof_protocol.lifecycle.identity.OperationBinding.Insts.CoreCmpPartialEqOperationBinding.eq, sandproof_protocol.lifecycle.request.LifecycleOperation.Insts.CoreCmpPartialEqLifecycleOperation.eq, sandproof_protocol.lifecycle.identity.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq, sandproof_protocol.lifecycle.identity.ExpectedSession.Insts.CoreCmpPartialEqExpectedSession.eq, sandproof_protocol.lifecycle.identity.SessionId.Insts.CoreCmpPartialEqSessionId.eq, sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq, sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId.eq, sandproof_protocol.lifecycle.identity.IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq, sandproof_protocol.lifecycle.identity.CaptureId.Insts.CoreCmpPartialEqCaptureId.eq, sandproof_protocol.lifecycle.identity.RequestWindowId.Insts.CoreCmpPartialEqRequestWindowId.eq, sandproof_protocol.lifecycle.identity.RequestSequence.Insts.CoreCmpPartialEqRequestSequence.eq, sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq, sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq, sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq, sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq, SandproofStage1RequestRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq, SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
end SandboxModel.Stage1Protocol.Request
#audit_controller_proof SandboxModel.Stage1Protocol.Request.unsupported_version_first
#audit_controller_proof SandboxModel.Stage1Protocol.Request.resume_order
#audit_controller_proof SandboxModel.Stage1Protocol.Request.resume_equal_refused
#audit_controller_proof SandboxModel.Stage1Protocol.Request.resume_nonempty_success
#audit_controller_proof SandboxModel.Stage1Protocol.Request.replay_complete_pair
