import SandboxModel.Lifecycle.Protocol.ManifestLength
import SandboxModel.Lifecycle.Protocol.ManifestLengthValues
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestLength.Examples
open SandboxModel.Stage1Protocol.Manifest
open SandboxModel.Stage1Protocol.ManifestLoop
open SandboxModel.Stage1Protocol.ManifestLength.Values

theorem maximum_total_accepted :
    manifest_validate (value 18446744073709551613#u64 1#u64 1#u64) original profile = .ok (.Ok 18446744073709551615#u64) := by
  rw [validate_enters_length_scan]
  change loop (fileBody 18446744073709551613#u64 1#u64 1#u64) (.Ok 0#u64, 0#usize) = .ok (.Ok 18446744073709551615#u64)
  refine loop_cont _ _ ((.Ok 18446744073709551613#u64), 1#usize) _ ?_ ?_
  · have add_one : ((0#usize + 1#usize) : Result Usize) = .ok 1#usize :=
      add_exact _ _ _ (by scalar_tac)
    have checked : U64.checked_add 0#u64 18446744073709551613#u64 = some 18446744073709551613#u64 := by decide
    have length := state_length 18446744073709551613#u64
    simp only [profile, alloc.vec.Vec.from] at length
    simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      lengthFiles, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
      core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]
  refine loop_cont _ _ ((.Ok 18446744073709551614#u64), 2#usize) _ ?_ ?_
  · have add_one : ((1#usize + 1#usize) : Result Usize) = .ok 2#usize :=
      add_exact _ _ _ (by scalar_tac)
    have checked : U64.checked_add 18446744073709551613#u64 1#u64 = some 18446744073709551614#u64 := by decide
    have length := memory_length 1#u64
    simp only [profile, alloc.vec.Vec.from] at length
    simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      lengthFiles, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
      core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]
  refine loop_cont _ _ ((.Ok 18446744073709551615#u64), 3#usize) _ ?_ ?_
  · have add_one : ((2#usize + 1#usize) : Result Usize) = .ok 3#usize :=
      add_exact _ _ _ (by scalar_tac)
    have checked : U64.checked_add 18446744073709551614#u64 1#u64 = some 18446744073709551615#u64 := by decide
    have length := disk_length 1#u64
    simp only [profile, alloc.vec.Vec.from] at length
    simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      lengthFiles, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
      core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]
  apply loop_done
  simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    lengthFiles]

theorem overflow_refused :
    manifest_validate (value 18446744073709551615#u64 1#u64 1#u64) original profile = .ok (.Err .LengthOverflow) := by
  rw [validate_enters_length_scan]
  change loop (fileBody 18446744073709551615#u64 1#u64 1#u64) (.Ok 0#u64, 0#usize) = .ok (.Err .LengthOverflow)
  refine loop_cont _ _ ((.Ok 18446744073709551615#u64), 1#usize) _ ?_ ?_
  · have add_one : ((0#usize + 1#usize) : Result Usize) = .ok 1#usize :=
      add_exact _ _ _ (by scalar_tac)
    have checked : U64.checked_add 0#u64 18446744073709551615#u64 = some 18446744073709551615#u64 := by decide
    have length := state_length 18446744073709551615#u64
    simp only [profile, alloc.vec.Vec.from] at length
    simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      lengthFiles, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
      core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]
  refine loop_cont _ _ ((.Err .LengthOverflow), 2#usize) _ ?_ ?_
  · have add_one : ((1#usize + 1#usize) : Result Usize) = .ok 2#usize :=
      add_exact _ _ _ (by scalar_tac)
    have checked : U64.checked_add 18446744073709551615#u64 1#u64 = none := by decide
    have length := memory_length 1#u64
    simp only [profile, alloc.vec.Vec.from] at length
    simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      lengthFiles, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
      core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]
  apply loop_done
  simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    lengthFiles, core.result.Result.is_ok]

theorem zero_precedes_addition :
    manifest_validate (value 18446744073709551615#u64 0#u64 1#u64) original profile = .ok (.Err .InvalidLength) := by
  rw [validate_enters_length_scan]
  change loop (fileBody 18446744073709551615#u64 0#u64 1#u64) (.Ok 0#u64, 0#usize) = .ok (.Err .InvalidLength)
  refine loop_cont _ _ ((.Ok 18446744073709551615#u64), 1#usize) _ ?_ ?_
  · have add_one : ((0#usize + 1#usize) : Result Usize) = .ok 1#usize :=
      add_exact _ _ _ (by scalar_tac)
    have checked : U64.checked_add 0#u64 18446744073709551615#u64 = some 18446744073709551615#u64 := by decide
    have length := state_length 18446744073709551615#u64
    simp only [profile, alloc.vec.Vec.from] at length
    simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      lengthFiles, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
      core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]
  refine loop_cont _ _ ((.Err .InvalidLength), 2#usize) _ ?_ ?_
  · have add_one : ((1#usize + 1#usize) : Result Usize) = .ok 2#usize :=
      add_exact _ _ _ (by scalar_tac)
    have length := memory_length 0#u64
    simp only [profile, alloc.vec.Vec.from] at length
    simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      lengthFiles, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
      core.result.Result.is_ok, profile, length, add_one]
  apply loop_done
  simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    lengthFiles, core.result.Result.is_ok]

theorem overflow_precedes_later_zero :
    manifest_validate (value 18446744073709551615#u64 1#u64 0#u64) original profile = .ok (.Err .LengthOverflow) := by
  rw [validate_enters_length_scan]
  change loop (fileBody 18446744073709551615#u64 1#u64 0#u64) (.Ok 0#u64, 0#usize) = .ok (.Err .LengthOverflow)
  refine loop_cont _ _ ((.Ok 18446744073709551615#u64), 1#usize) _ ?_ ?_
  · have add_one : ((0#usize + 1#usize) : Result Usize) = .ok 1#usize :=
      add_exact _ _ _ (by scalar_tac)
    have checked : U64.checked_add 0#u64 18446744073709551615#u64 = some 18446744073709551615#u64 := by decide
    have length := state_length 18446744073709551615#u64
    simp only [profile, alloc.vec.Vec.from] at length
    simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      lengthFiles, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
      core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]
  refine loop_cont _ _ ((.Err .LengthOverflow), 2#usize) _ ?_ ?_
  · have add_one : ((1#usize + 1#usize) : Result Usize) = .ok 2#usize :=
      add_exact _ _ _ (by scalar_tac)
    have checked : U64.checked_add 18446744073709551615#u64 1#u64 = none := by decide
    have length := memory_length 1#u64
    simp only [profile, alloc.vec.Vec.from] at length
    simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      lengthFiles, alloc.vec.Vec.index, alloc.vec.Vec.from, alloc.vec.Vec.val, Slice.index_usize,
      core.result.Result.is_ok, profile, length, lift, checked, core.option.Option.ok_or, add_one]
  apply loop_done
  simp [fileBody, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    lengthFiles, core.result.Result.is_ok]
end SandboxModel.Stage1Protocol.ManifestLength.Examples

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.Examples.maximum_total_accepted
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.Examples.overflow_refused
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.Examples.zero_precedes_addition
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.Examples.overflow_precedes_later_zero
