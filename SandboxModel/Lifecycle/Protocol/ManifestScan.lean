import SandboxModel.Lifecycle.Protocol.ManifestScanModel
import SandboxModel.Lifecycle.Protocol.ManifestFile
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestValidation
open sandproof_protocol.lifecycle.manifest
open SandboxModel.Stage1Protocol.ManifestLength
open SandboxModel.Stage1Protocol.ManifestFiles

theorem scan_exists (files : alloc.vec.Vec File) (profile : Profile) (current : U64) (index : Usize)
    (count : files.val.length = 2 + profile.writable_drives.val.length)
    (bounded : profile.writable_drives.val.length ≤ 16) :
    ∃ output, scan files profile (.Ok current) index = .ok output ∧
      observe output = scanResult profile index.val current.val (files.val.drop index.val) := by
  classical
  by_cases within : index.val < files.val.length
  · have file_run := file_refines files.val[index.val] index profile (by omega) bounded
    have dropped := List.drop_eq_getElem_cons within
    cases checked : fileResult profile index.val files.val[index.val] with
    | Err error =>
      have refused : file_length files.val[index.val] index profile = .ok (.Err error) := by
        simpa [checked] using file_run
      refine ⟨.Err error, scan_file_error files profile current index within error refused, ?_⟩
      rw [dropped]
      simp [scanResult, checked, observe]
    | Ok length =>
      have accepted : file_length files.val[index.val] index profile = .ok (.Ok length) := by
        simpa [checked] using file_run
      have sum_spec := U64.checked_add_bv_spec current length
      cases sum_eq : U64.checked_add current length with
      | none =>
        simp only [sum_eq] at sum_spec
        have overflow : U64.max < current.val + length.val := by omega
        refine ⟨.Err .LengthOverflow,
          scan_overflow files profile current length index within accepted overflow, ?_⟩
        rw [dropped]
        simp [scanResult, checked, overflow, observe]
      | some next_total =>
        simp only [sum_eq] at sum_spec
        have total_val : next_total.val = current.val + length.val := sum_spec.2.1
        have safe : ¬ U64.max < current.val + length.val := by scalar_tac
        obtain ⟨next, addition, next_val⟩ := index_next files index within
        obtain ⟨output, rest_run, rest_spec⟩ := scan_exists files profile next_total next count bounded
        refine ⟨output, ?_, ?_⟩
        · rw [scan_unfold]
          have inside : index < alloc.vec.Vec.len files := by scalar_tac
          simpa [body, SnapshotManifest.validate_loop.body, inside, core.result.Result.is_ok,
            index_exact files index within, accepted, lift, sum_eq, core.option.Option.ok_or, addition]
            using rest_run
        · rw [dropped]
          simpa [scanResult, checked, safe, next_val, total_val] using rest_spec
  · refine ⟨.Ok current, scan_end files profile (.Ok current) index (by omega), ?_⟩
    simp [observe, List.drop_eq_nil_of_le (by omega : files.val.length ≤ index.val), scanResult]
termination_by files.val.length - index.val

theorem scan_refines (files : alloc.vec.Vec File) (profile : Profile) (current : U64) (index : Usize)
    (count : files.val.length = 2 + profile.writable_drives.val.length)
    (bounded : profile.writable_drives.val.length ≤ 16) :
    (do let result ← scan files profile (.Ok current) index; .ok (observe result)) =
      .ok (scanResult profile index.val current.val (files.val.drop index.val)) := by
  obtain ⟨output, run, spec⟩ := scan_exists files profile current index count bounded
  simp [run, spec]

end SandboxModel.Stage1Protocol.ManifestValidation

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.scan_exists
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.scan_refines
