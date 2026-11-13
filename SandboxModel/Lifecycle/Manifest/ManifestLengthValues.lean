import SandboxModel.Lifecycle.Manifest.ManifestValues
import SandboxModel.Lifecycle.Manifest.ManifestLengthModel
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestLength.Values
open SandboxModel.Stage1Protocol.ManifestChecks
open SandboxModel.Stage1Protocol.ManifestLoop

def lengthFiles (state memory disk : U64) : alloc.vec.Vec ManifestLength.File := alloc.vec.Vec.from [
  ⟨.State, "state.bin", state, digest⟩,
  ⟨.Memory, "memory.bin", memory, digest⟩,
  ⟨.Disk "rootfs", "disk-0.img", disk, digest⟩]
  (by simp only [List.length_cons, List.length_nil]; scalar_tac)

def value (state memory disk : U64) := manifestWith profile (lengthFiles state memory disk)

def fileScan (state memory disk : U64) (current : Checked) (index : Usize) :=
  sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop (lengthFiles state memory disk)
    profile.snapshot_format profile.tool profile.cpu profile.devices profile.image_configuration
    profile.writable_drives current index

def fileBody (state memory disk : U64) (current : Checked × Usize) :=
  sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body (lengthFiles state memory disk)
    profile.snapshot_format profile.tool profile.cpu profile.devices profile.image_configuration
    profile.writable_drives current.1 current.2

theorem state_length (length : U64) :
    sandproof_protocol.lifecycle.manifest.file_length
      ⟨.State, "state.bin", length, digest⟩ 0#usize profile =
      .ok (if length = 0#u64 then .Err .InvalidLength else .Ok length) := by
  simp [sandproof_protocol.lifecycle.manifest.file_length, literal_owned,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
  split <;> rfl

theorem memory_length (length : U64) :
    sandproof_protocol.lifecycle.manifest.file_length
      ⟨.Memory, "memory.bin", length, digest⟩ 1#usize profile =
      .ok (if length = 0#u64 then .Err .InvalidLength else .Ok length) := by
  simp [sandproof_protocol.lifecycle.manifest.file_length, literal_owned,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
  split <;> rfl

theorem disk_length (length : U64) :
    sandproof_protocol.lifecycle.manifest.file_length
      ⟨.Disk "rootfs", "disk-0.img", length, digest⟩ 2#usize profile =
      .ok (if length = 0#u64 then .Err .InvalidLength else .Ok length) := by
  have difference : ((2#usize - 2#usize) : Result Usize) = .ok 0#usize :=
    sub_exact _ _ _ (by scalar_tac) (by scalar_tac)
  simp [sandproof_protocol.lifecycle.manifest.file_length, profile,
    alloc.vec.Vec.index, alloc.vec.Vec.from, Slice.index_usize, difference,
    sandproof_protocol.lifecycle.manifest.disk_bundle_path, literal_owned,
    core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
  split <;> rfl

theorem validate_enters_length_scan (state memory disk : U64) :
    manifest_validate (value state memory disk) original profile =
      fileScan state memory disk (.Ok 0#u64) 0#usize := by
  have fileCount : alloc.vec.Vec.len (lengthFiles state memory disk) = 3#usize := by simp [lengthFiles]; scalar_tac
  have driveCount : alloc.vec.Vec.len profile.writable_drives = 1#usize := by simp [profile]; scalar_tac
  have countSum : ((2#usize + 1#usize) : Result Usize) = .ok 3#usize :=
    ManifestLoop.add_exact _ _ _ (by scalar_tac)
  simp [manifest_validate, value, manifestWith, fileScan,
    sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate,
    sandproof_protocol.lifecycle.manifest.MANIFEST_FORMAT_VERSION,
    sandproof_protocol.lifecycle.PROTOCOL_VERSION, original,
    profile_eq_self, profile_valid, fileCount, driveCount, countSum,
    core.result.Result.Insts.CoreOpsTry.branch, Aeneas.Std.lift,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq, sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq, sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq, sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq, sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq, sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId.eq, sandproof_protocol.lifecycle.identity.SessionId.Insts.CoreCmpPartialEqSessionId.eq, sandproof_protocol.lifecycle.identity.IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq, sandproof_protocol.lifecycle.identity.CaptureId.Insts.CoreCmpPartialEqCaptureId.eq, sandproof_protocol.lifecycle.identity.RequestWindowId.Insts.CoreCmpPartialEqRequestWindowId.eq, sandproof_protocol.lifecycle.identity.RequestSequence.Insts.CoreCmpPartialEqRequestSequence.eq, sandproof_protocol.lifecycle.identity.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq, sandproof_protocol.lifecycle.identity.ExpectedSession.Insts.CoreCmpPartialEqExpectedSession.eq, sandproof_protocol.lifecycle.identity.OperationBinding.Insts.CoreCmpPartialEqOperationBinding.eq, sandproof_protocol.lifecycle.manifest.CaptureBinding.Insts.CoreCmpPartialEqCaptureBinding.eq]
end SandboxModel.Stage1Protocol.ManifestLength.Values

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.Values.state_length
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.Values.memory_length
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.Values.disk_length
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.Values.validate_enters_length_scan
