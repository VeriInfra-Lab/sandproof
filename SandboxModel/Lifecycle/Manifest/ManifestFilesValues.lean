import SandboxModel.Lifecycle.Manifest.ManifestFilesModel
import SandboxModel.Lifecycle.Manifest.ManifestValues
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestFiles.Values
open SandboxModel.Stage1Protocol.ManifestChecks
open SandboxModel.Stage1Protocol.ManifestLength

def filesWithDisk (drive_id path : String) : alloc.vec.Vec ManifestLength.File := alloc.vec.Vec.from [
  ⟨.State, "state.bin", 1#u64, digest⟩,
  ⟨.Memory, "memory.bin", 1#u64, digest⟩,
  ⟨.Disk drive_id, path, 1#u64, digest⟩]
  (by simp only [List.length_cons, List.length_nil]; scalar_tac)

def fileScan (values : alloc.vec.Vec ManifestLength.File) (total : Checked) (index : Usize) :=
  sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop values
    profile.snapshot_format profile.tool profile.cpu profile.devices profile.image_configuration
    profile.writable_drives total index

def fileBody (values : alloc.vec.Vec ManifestLength.File) (current : Checked × Usize) :=
  sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body values
    profile.snapshot_format profile.tool profile.cpu profile.devices profile.image_configuration
    profile.writable_drives current.1 current.2

theorem validate_fixed_profile (values : alloc.vec.Vec ManifestLength.File) :
    manifest_validate (manifestWith profile values) original profile =
      if values.val.length = 3 then fileScan values (.Ok 0#u64) 0#usize
      else .ok (.Err .FileSetMismatch) := by
  have driveCount : alloc.vec.Vec.len profile.writable_drives = 1#usize := by simp [profile]; scalar_tac
  have countSum : ((2#usize + 1#usize) : Result Usize) = .ok 3#usize :=
    ManifestLoop.add_exact _ _ _ (by scalar_tac)
  simp [manifest_validate, manifestWith, fileScan,
    sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate,
    sandproof_protocol.lifecycle.manifest.MANIFEST_FORMAT_VERSION,
    sandproof_protocol.lifecycle.PROTOCOL_VERSION, original,
    profile_eq_self, profile_valid, driveCount, countSum,
    core.result.Result.Insts.CoreOpsTry.branch, Aeneas.Std.lift,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    sandproof_protocol.internal.PositiveCounter.Insts.CoreCmpPartialEqPositiveCounter.eq, sandproof_protocol.NodeId.Insts.CoreCmpPartialEqNodeId.eq, sandproof_protocol.DeploymentId.Insts.CoreCmpPartialEqDeploymentId.eq, sandproof_protocol.internal.NodeIdentity.Insts.CoreCmpPartialEqNodeIdentity.eq, sandproof_protocol.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq, sandproof_protocol.TenantId.Insts.CoreCmpPartialEqTenantId.eq, sandproof_protocol.lifecycle.identity.SessionId.Insts.CoreCmpPartialEqSessionId.eq, sandproof_protocol.lifecycle.identity.IncarnationId.Insts.CoreCmpPartialEqIncarnationId.eq, sandproof_protocol.lifecycle.identity.CaptureId.Insts.CoreCmpPartialEqCaptureId.eq, sandproof_protocol.lifecycle.identity.RequestWindowId.Insts.CoreCmpPartialEqRequestWindowId.eq, sandproof_protocol.lifecycle.identity.RequestSequence.Insts.CoreCmpPartialEqRequestSequence.eq, sandproof_protocol.lifecycle.identity.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq, sandproof_protocol.lifecycle.identity.ExpectedSession.Insts.CoreCmpPartialEqExpectedSession.eq, sandproof_protocol.lifecycle.identity.OperationBinding.Insts.CoreCmpPartialEqOperationBinding.eq, sandproof_protocol.lifecycle.manifest.CaptureBinding.Insts.CoreCmpPartialEqCaptureBinding.eq]

open SandboxModel.Stage1Protocol.ManifestLoop

theorem state_step (drive_id path : String) :
    fileBody (filesWithDisk drive_id path) (.Ok 0#u64, 0#usize) =
      .ok (.cont (.Ok 1#u64, 1#usize)) := by
  have add_one : ((0#usize + 1#usize) : Result Usize) = .ok 1#usize :=
    add_exact _ _ _ (by scalar_tac)
  have checked : U64.checked_add 0#u64 1#u64 = some 1#u64 := by decide
  have length := state_file_length
  simp only [profile, alloc.vec.Vec.from] at length
  simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    filesWithDisk, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
    core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]

theorem memory_step (drive_id path : String) :
    fileBody (filesWithDisk drive_id path) (.Ok 1#u64, 1#usize) =
      .ok (.cont (.Ok 2#u64, 2#usize)) := by
  have add_one : ((1#usize + 1#usize) : Result Usize) = .ok 2#usize :=
    add_exact _ _ _ (by scalar_tac)
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  have length := memory_file_length
  simp only [profile, alloc.vec.Vec.from] at length
  simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    filesWithDisk, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
    core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]

theorem finish_after_headers (drive_id path : String) (result : Checked)
    (finish : fileScan (filesWithDisk drive_id path) (.Ok 2#u64) 2#usize = .ok result) :
    manifest_validate (manifestWith profile (filesWithDisk drive_id path)) original profile = .ok result := by
  rw [validate_fixed_profile, if_pos (by simp [filesWithDisk])]
  change loop (fileBody (filesWithDisk drive_id path)) (.Ok 0#u64, 0#usize) = .ok result
  refine loop_cont _ _ (.Ok 1#u64, 1#usize) _ (state_step drive_id path) ?_
  refine loop_cont _ _ (.Ok 2#u64, 2#usize) _ (memory_step drive_id path) ?_
  exact finish

end SandboxModel.Stage1Protocol.ManifestFiles.Values

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.Values.validate_fixed_profile
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.Values.state_step
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.Values.memory_step
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.Values.finish_after_headers
