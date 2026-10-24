import SandboxModel.Lifecycle.Protocol.ManifestValues
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.Manifest
open SandboxModel.Stage1Protocol.ManifestLoop
def filesBody (state : (core.result.Result U64 sandproof_protocol.lifecycle.manifest.ManifestError) × Usize) :=
  sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body
    files profile.snapshot_format profile.tool profile.cpu profile.devices
    profile.image_configuration profile.writable_drives state.1 state.2
theorem files_step_0 :
    filesBody (.Ok 0#u64, 0#usize) = .ok (.cont (.Ok 1#u64, 1#usize)) := by
  have add_one : ((0#usize + 1#usize) : Result Usize) = .ok 1#usize :=
    add_exact _ _ _ (by scalar_tac)
  have checked : U64.checked_add 0#u64 1#u64 = some 1#u64 := by decide
  have length := state_file_length
  simp only [profile, alloc.vec.Vec.from] at length
  simp [filesBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    files, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
    core.result.Result.is_ok, profile, length, Aeneas.Std.lift, checked, core.option.Option.ok_or, add_one]
theorem files_step_1 :
    filesBody (.Ok 1#u64, 1#usize) = .ok (.cont (.Ok 2#u64, 2#usize)) := by
  have add_one : ((1#usize + 1#usize) : Result Usize) = .ok 2#usize :=
    add_exact _ _ _ (by scalar_tac)
  have checked : U64.checked_add 1#u64 1#u64 = some 2#u64 := by decide
  have length := memory_file_length
  simp only [profile, alloc.vec.Vec.from] at length
  simp [filesBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    files, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
    core.result.Result.is_ok, profile, length, Aeneas.Std.lift, checked, core.option.Option.ok_or, add_one]
theorem files_step_2 :
    filesBody (.Ok 2#u64, 2#usize) = .ok (.cont (.Ok 3#u64, 3#usize)) := by
  have add_one : ((2#usize + 1#usize) : Result Usize) = .ok 3#usize :=
    add_exact _ _ _ (by scalar_tac)
  have checked : U64.checked_add 2#u64 1#u64 = some 3#u64 := by decide
  have length := disk_file_length
  simp only [profile, alloc.vec.Vec.from] at length
  simp [filesBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    files, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
    core.result.Result.is_ok, profile, length, Aeneas.Std.lift, checked, core.option.Option.ok_or, add_one]
theorem files_loop :
    sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop
      files profile.snapshot_format profile.tool profile.cpu profile.devices
      profile.image_configuration profile.writable_drives (.Ok 0#u64) 0#usize = .ok (.Ok 3#u64) := by
  change loop filesBody (.Ok 0#u64, 0#usize) = .ok (.Ok 3#u64)
  refine loop_cont _ _ (.Ok 1#u64, 1#usize) _ files_step_0 ?_
  refine loop_cont _ _ (.Ok 2#u64, 2#usize) _ files_step_1 ?_
  refine loop_cont _ _ (.Ok 3#u64, 3#usize) _ files_step_2 ?_
  apply loop_done
  simp [filesBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body, files]
end SandboxModel.Stage1Protocol.Manifest

#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.files_step_0
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.files_step_1
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.files_step_2
#audit_controller_proof SandboxModel.Stage1Protocol.Manifest.files_loop
