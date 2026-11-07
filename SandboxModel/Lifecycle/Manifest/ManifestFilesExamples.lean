import SandboxModel.Lifecycle.Manifest.ManifestFiles
import SandboxModel.Lifecycle.Manifest.ManifestFilesValues
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestFiles.Examples
open SandboxModel.Stage1Protocol.ManifestChecks
open SandboxModel.Stage1Protocol.ManifestLength
open SandboxModel.Stage1Protocol.ManifestFiles.Values

def withoutDisk : alloc.vec.Vec ManifestLength.File := alloc.vec.Vec.from (files.val.take 2)
  (by simp [files]; scalar_tac)
def withExtraDisk : alloc.vec.Vec ManifestLength.File := alloc.vec.Vec.from
  (files.val ++ [⟨.Disk "rootfs", "disk-0.img", 1#u64, digest⟩])
  (by simp [files]; scalar_tac)

theorem normal_manifest_accepted :
    manifest_validate manifest original profile = .ok (.Ok 3#u64) := by
  change manifest_validate (manifestWith profile (filesWithDisk "rootfs" "disk-0.img")) original profile = .ok (.Ok 3#u64)
  apply finish_after_headers
  change loop (fileBody (filesWithDisk "rootfs" "disk-0.img")) (.Ok 2#u64, 2#usize) = .ok (.Ok 3#u64)
  refine ManifestLoop.loop_cont _ _ (.Ok 3#u64, 3#usize) _ ?_ ?_
  · have add_one : ((2#usize + 1#usize) : Result Usize) = .ok 3#usize :=
      ManifestLoop.add_exact _ _ _ (by scalar_tac)
    have checked : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
    have length := disk_file_length
    simp only [profile, alloc.vec.Vec.from] at length
    simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      filesWithDisk, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
      core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]
  apply ManifestLoop.loop_done
  simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    filesWithDisk]

theorem normal_manifest_complete : CompleteFiles profile manifest.files.val :=
  (validate_files manifest original profile 3#u64 normal_manifest_accepted).2.2

theorem missing_disk_refused :
    manifest_validate (manifestWith profile withoutDisk) original profile = .ok (.Err .FileSetMismatch) := by
  rw [validate_fixed_profile]
  simp [withoutDisk, files]

theorem extra_disk_refused :
    manifest_validate (manifestWith profile withExtraDisk) original profile = .ok (.Err .FileSetMismatch) := by
  rw [validate_fixed_profile]
  simp [withExtraDisk, files]

theorem wrong_disk_id_refused :
    manifest_validate (manifestWith profile (filesWithDisk "foreign" "disk-0.img")) original profile =
      .ok (.Err .FileSetMismatch) := by
  apply finish_after_headers
  apply scan_file_error _ _ _ _ (by simp [filesWithDisk])
  have difference : ((2#usize - 2#usize) : Result Usize) = .ok 0#usize :=
    sub_exact _ _ _ (by scalar_tac) (by scalar_tac)
  simp [filesWithDisk, sandproof_protocol.lifecycle.manifest.file_length, profile,
    alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize, difference,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]

theorem wrong_disk_path_refused :
    manifest_validate (manifestWith profile (filesWithDisk "rootfs" "disk-1.img")) original profile =
      .ok (.Err .InvalidPath) := by
  apply finish_after_headers
  apply scan_file_error _ _ _ _ (by simp [filesWithDisk])
  have difference : ((2#usize - 2#usize) : Result Usize) = .ok 0#usize :=
    sub_exact _ _ _ (by scalar_tac) (by scalar_tac)
  simp [filesWithDisk, sandproof_protocol.lifecycle.manifest.file_length, profile,
    alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize, difference,
    sandproof_protocol.lifecycle.manifest.disk_bundle_path, literal_owned,
    core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
end SandboxModel.Stage1Protocol.ManifestFiles.Examples

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.Examples.normal_manifest_complete
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.Examples.missing_disk_refused
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.Examples.extra_disk_refused
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.Examples.wrong_disk_id_refused
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.Examples.wrong_disk_path_refused

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.Examples.normal_manifest_accepted
