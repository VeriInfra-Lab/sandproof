import SandboxModel.Lifecycle.Protocol.ManifestWitnesses
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.Manifest
theorem unsupported_version_first (value : WireManifest) (expected : Capture) (p : Profile)
    (wrong : value.format_version ≠ 1#u16) :
    manifest_validate value expected p =
      .ok (.Err sandproof_protocol.lifecycle.manifest.ManifestError.UnsupportedVersion) := by
  simp [manifest_validate, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate,
    sandproof_protocol.lifecycle.manifest.MANIFEST_FORMAT_VERSION, wrong]

theorem changed_capture_key_first (p : Profile) (values : alloc.vec.Vec File) :
    manifest_validate (manifestWith p values) changed p =
      .ok (.Err sandproof_protocol.lifecycle.manifest.ManifestError.BindingMismatch) := by
  simp [manifestWith, original, changed, manifest_validate, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate, sandproof_protocol.lifecycle.manifest.MANIFEST_FORMAT_VERSION, sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq, sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq, sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq, sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq, sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq, sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId.eq, sandproof_protocol.lifecycle.digest.ContentDigest.Insts.CoreCmpPartialEqContentDigest.eq, sandproof_protocol.lifecycle.identity.SessionId.Insts.CoreCmpPartialEqSessionId.eq, sandproof_protocol.lifecycle.identity.IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq, sandproof_protocol.lifecycle.identity.CaptureId.Insts.CoreCmpPartialEqCaptureId.eq, sandproof_protocol.lifecycle.identity.RequestWindowId.Insts.CoreCmpPartialEqRequestWindowId.eq, sandproof_protocol.lifecycle.identity.RequestSequence.Insts.CoreCmpPartialEqRequestSequence.eq, sandproof_protocol.lifecycle.identity.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq, sandproof_protocol.lifecycle.identity.ExpectedSession.Insts.CoreCmpPartialEqExpectedSession.eq, sandproof_protocol.lifecycle.identity.OperationBinding.Insts.CoreCmpPartialEqOperationBinding.eq, sandproof_protocol.lifecycle.manifest.CaptureBinding.Insts.CoreCmpPartialEqCaptureBinding.eq, sandproof_protocol.lifecycle.manifest.VmCompatibility.Insts.CoreCmpPartialEqVmCompatibility.eq, Aeneas.Std.lift, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, SandproofStage1ManifestRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq, SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]

theorem normal_manifest_accepted :
    manifest_validate manifest original profile = .ok (.Ok 3#u64) := by
  have fileCount : alloc.vec.Vec.len files = 3#usize := by simp [files]; scalar_tac
  have driveCount : alloc.vec.Vec.len profile.writable_drives = 1#usize := by simp [profile]; scalar_tac
  have countSum : ((2#usize + 1#usize) : Result Usize) = .ok 3#usize :=
    ManifestLoop.add_exact _ _ _ (by scalar_tac)
  simp [manifest_validate, manifest, manifestWith,
    sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate,
    sandproof_protocol.lifecycle.manifest.MANIFEST_FORMAT_VERSION,
    sandproof_protocol.lifecycle.PROTOCOL_VERSION, original,
    profile_eq_self, profile_valid, files_loop, fileCount, driveCount, countSum,
    core.result.Result.Insts.CoreOpsTry.branch, Aeneas.Std.lift,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq, sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq, sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq, sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq, sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq, sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId.eq, sandproof_protocol.lifecycle.identity.SessionId.Insts.CoreCmpPartialEqSessionId.eq, sandproof_protocol.lifecycle.identity.IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq, sandproof_protocol.lifecycle.identity.CaptureId.Insts.CoreCmpPartialEqCaptureId.eq, sandproof_protocol.lifecycle.identity.RequestWindowId.Insts.CoreCmpPartialEqRequestWindowId.eq, sandproof_protocol.lifecycle.identity.RequestSequence.Insts.CoreCmpPartialEqRequestSequence.eq, sandproof_protocol.lifecycle.identity.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq, sandproof_protocol.lifecycle.identity.ExpectedSession.Insts.CoreCmpPartialEqExpectedSession.eq, sandproof_protocol.lifecycle.identity.OperationBinding.Insts.CoreCmpPartialEqOperationBinding.eq, sandproof_protocol.lifecycle.manifest.CaptureBinding.Insts.CoreCmpPartialEqCaptureBinding.eq]
theorem original_capture_contract :
    CaptureBindingPairContract sandproof_protocol.lifecycle.manifest.ManifestError.BindingMismatch
      manifest_validate manifest original changed profile 3#u64 := by
  exact ⟨normal_manifest_accepted, by simpa [manifest] using changed_capture_key_first profile files⟩
end SandboxModel.Stage1Protocol.Manifest
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.unsupported_version_first
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.changed_capture_key_first
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.normal_manifest_accepted
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.original_capture_contract
