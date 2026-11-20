import SandboxModel.Lifecycle.Manifest.ManifestFiles
import SandboxModel.Lifecycle.Protocol.ManifestFileModel
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestValidation
open sandproof_protocol.lifecycle.manifest
open SandboxModel.Stage1Protocol.ManifestLength
open SandboxModel.Stage1Protocol.ManifestFiles

theorem file_refines (file : File) (index : Usize) (profile : Profile)
    (within : index.val < 2 + profile.writable_drives.val.length)
    (bounded : profile.writable_drives.val.length ≤ 16) :
    file_length file index profile = .ok (fileResult profile index.val file) := by
  classical
  unfold file_length
  cases kind : file.kind with
  | State =>
    by_cases zero : index.val = 0
    · simp [zero, kind, fileResult, KindAt, expectedPath,
        ManifestChecks.literal_owned, core.cmp.PartialEq.ne.trait_default,
        core.cmp.PartialEq.ne.default, alloc.string.String.Insts.CoreCmpPartialEqString.eq,
        SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
      split <;> simp_all
      split <;> simp_all
    · simp [zero, kind, fileResult, KindAt]
  | Memory =>
    by_cases one : index.val = 1
    · simp [one, kind, fileResult, KindAt, expectedPath,
        ManifestChecks.literal_owned, core.cmp.PartialEq.ne.trait_default,
        core.cmp.PartialEq.ne.default, alloc.string.String.Insts.CoreCmpPartialEqString.eq,
        SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
      split <;> simp_all
      split <;> simp_all
    · simp [one, kind, fileResult, KindAt]
  | Disk id =>
    by_cases low : index.val < 2
    · have less : index < 2#usize := by scalar_tac
      simp [less, kind, fileResult, KindAt, Nat.not_le_of_gt low]
    · have ge : 2 ≤ index.val := by omega
      have not_less : ¬ index < 2#usize := by scalar_tac
      obtain ⟨i, sub, post⟩ := (WP.spec_equiv_exists _ _).mp
        (Usize.sub_bv_spec (x := index) (y := 2#usize) (by scalar_tac))
      have i_val : i.val = index.val - 2 := by simpa using post.1
      have i_valid : i.val < profile.writable_drives.val.length := by omega
      have at_drive : alloc.vec.Vec.index (core.slice.index.SliceIndexUsizeSlice WritableDrive)
          profile.writable_drives i = .ok profile.writable_drives.val[i.val] := by
        simp only [alloc.vec.Vec.val] at i_valid ⊢
        simp [alloc.vec.Vec.index, Slice.index_usize, i_valid]
      have disk_bound : i.val < 16 := by omega
      have nonzero : index.val ≠ 0 := by omega
      have not_one : index.val ≠ 1 := by omega
      simp only [not_less, if_false, sub, bind_tc_ok, at_drive,
        core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
        alloc.string.String.Insts.CoreCmpPartialEqString.eq,
        SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
        disk_path_exact, disk_bound, if_pos, core.option.Option.ok_or,
        core.result.Result.Insts.CoreOpsTry.branch]
      simp [fileResult, KindAt, kind, ge, expectedPath, nonzero, not_one, ← i_val,
        List.getElem?_eq_getElem i_valid]
      split <;> simp_all
      split <;> simp_all
      split <;> simp_all

end SandboxModel.Stage1Protocol.ManifestValidation

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.file_refines
