import SandboxModel.Lifecycle.Protocol.ManifestFiles
import SandboxModel.Lifecycle.Protocol.ManifestFilesValues
import SandboxModel.Lifecycle.Protocol.Manifest
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestFiles.Examples
open SandboxModel.Stage1Protocol.Manifest
open SandboxModel.Stage1Protocol.ManifestLength
open SandboxModel.Stage1Protocol.ManifestFiles.Values

def withoutDisk : alloc.vec.Vec ManifestLength.File := alloc.vec.Vec.from (files.val.take 2)
  (by simp [files]; scalar_tac)
def withExtraDisk : alloc.vec.Vec ManifestLength.File := alloc.vec.Vec.from
  (files.val ++ [⟨.Disk "rootfs", "disk-0.img", 1#u64, digest⟩])
  (by simp [files]; scalar_tac)

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
