import SandboxModel.Lifecycle.Protocol.ManifestLengthModel
import SandproofStage1Manifest.Funs
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestLength
/-! Successful-result soundness for arbitrary represented manifests. No assumption
that the input profile, path or file shape is valid is needed for this direction.
The one-step refusal lemmas retain the actual file-check result as an explicit
premise; they do not claim a complete path/profile acceptance characterization. -/
theorem bind_success {A B : Type} (value : Result A) (next : A → Result B) (out : B) :
    (do let x ← value; next x) = .ok out ↔ ∃ x, value = .ok x ∧ next x = .ok out := by
  cases value <;> simp

theorem file_length_success (file : File) (index : Usize) (profile : Profile) (n : U64)
    (run : sandproof_protocol.lifecycle.manifest.file_length file index profile = .ok (.Ok n)) :
    n = file.length ∧ file.length ≠ 0#u64 := by
  unfold sandproof_protocol.lifecycle.manifest.file_length at run
  split at run
  · split at run
    · simp at run
    · obtain ⟨path, _, run⟩ := (bind_success _ _ _).mp run
      obtain ⟨bad, _, run⟩ := (bind_success _ _ _).mp run
      split at run
      · simp at run
      · split at run <;> simp_all
  · split at run
    · simp at run
    · obtain ⟨path, _, run⟩ := (bind_success _ _ _).mp run
      obtain ⟨bad, _, run⟩ := (bind_success _ _ _).mp run
      split at run
      · simp at run
      · split at run <;> simp_all
  · split at run
    · simp at run
    · obtain ⟨i, _, run⟩ := (bind_success _ _ _).mp run
      obtain ⟨drive, _, run⟩ := (bind_success _ _ _).mp run
      obtain ⟨bad, _, run⟩ := (bind_success _ _ _).mp run
      split at run
      · simp at run
      · obtain ⟨path, _, run⟩ := (bind_success _ _ _).mp run
        cases path with
        | none => simp [core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
            core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run
        | some path =>
          simp only [core.option.Option.ok_or, bind_tc_ok,
            core.result.Result.Insts.CoreOpsTry.branch] at run
          obtain ⟨bad, _, run⟩ := (bind_success _ _ _).mp run
          split at run
          · simp at run
          · split at run <;> simp_all

def scan (files : alloc.vec.Vec File) (p : Profile) (total : Checked) (index : Usize) : Result Checked :=
  sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop files
    p.snapshot_format p.tool p.cpu p.devices p.image_configuration p.writable_drives total index

def body (files : alloc.vec.Vec File) (p : Profile) (state : Checked × Usize) :=
  sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body files
    p.snapshot_format p.tool p.cpu p.devices p.image_configuration p.writable_drives state.1 state.2

theorem scan_unfold (files : alloc.vec.Vec File) (p : Profile) (total : Checked) (index : Usize) :
    scan files p total index = (do
      let step ← body files p (total, index)
      match step with
      | .done out => .ok out
      | .cont next => scan files p next.1 next.2) := by
  unfold scan sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop
  rw [loop]
  apply congrArg (Aeneas.Std.bind (body files p (total, index)))
  funext step
  cases step <;> rfl

theorem scan_error (files : alloc.vec.Vec File) (p : Profile) (error : Err) (index : Usize) :
    scan files p (.Err error) index = .ok (.Err error) := by
  rw [scan_unfold]
  simp [body, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    core.result.Result.is_ok]

theorem scan_end (files : alloc.vec.Vec File) (p : Profile) (total : Checked) (index : Usize)
    (end_of_files : files.val.length ≤ index.val) :
    scan files p total index = .ok total := by
  rw [scan_unfold]
  have outside : ¬ index < alloc.vec.Vec.len files := by scalar_tac
  simp [body, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body, outside]

theorem index_exact (files : alloc.vec.Vec File) (index : Usize)
    (within : index.val < files.val.length) :
    alloc.vec.Vec.index (core.slice.index.SliceIndexUsizeSlice File) files index =
      .ok files.val[index.val] := by
  simp only [alloc.vec.Vec.val] at within ⊢
  simp [alloc.vec.Vec.index, Slice.index_usize, within]

theorem index_next (files : alloc.vec.Vec File) (index : Usize)
    (within : index.val < files.val.length) :
    ∃ next : Usize, ((index + 1#usize) : Result Usize) = .ok next ∧ next.val = index.val + 1 := by
  have bound : index.val + (1#usize).val ≤ Usize.max := by scalar_tac
  obtain ⟨next, equation, post⟩ := (WP.spec_equiv_exists _ _).mp (Usize.add_bv_spec (x := index) (y := 1#usize) bound)
  exact ⟨next, equation, by simpa using post.1⟩


theorem scan_success (files : alloc.vec.Vec File) (p : Profile) (current total : U64) (index : Usize)
    (run : scan files p (.Ok current) index = .ok (.Ok total)) :
    total.val = current.val + bytes (files.val.drop index.val) ∧
      ∀ file ∈ files.val.drop index.val, file.length ≠ 0#u64 := by
  by_cases within : index.val < files.val.length
  · obtain ⟨next, add, next_val⟩ := index_next files index within
    have inside : index < alloc.vec.Vec.len files := by scalar_tac
    rw [scan_unfold] at run
    simp only [body, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
      inside, if_pos, core.result.Result.is_ok, bind_tc_ok, index_exact files index within,
      bind_assoc_eq] at run
    obtain ⟨checked, checked_run, run⟩ := (bind_success _ _ _).mp run
    cases checked with
    | Err error => simp [add, scan_error] at run
    | Ok size =>
      obtain ⟨same, positive⟩ := file_length_success _ _ _ _ checked_run
      simp only [lift, bind_tc_ok, core.option.Option.ok_or, add] at run
      have sum_spec := U64.checked_add_bv_spec current size
      cases sum_eq : U64.checked_add current size with
      | none =>
        rw [sum_eq] at run
        simp [scan_error] at run
      | some next_total =>
        rw [sum_eq] at run
        simp only [bind_tc_ok] at run
        have rest := scan_success files p next_total total next run
        simp only [sum_eq] at sum_spec
        rw [same] at sum_spec
        rw [List.drop_eq_getElem_cons within]
        constructor
        · simp only [bytes, List.map_cons, List.sum_cons] at *
          rw [next_val] at rest
          omega
        · intro file member
          simp only [List.mem_cons] at member
          rcases member with rfl | member
          · exact positive
          · apply rest.2 file
            simpa [next_val] using member
  · have end_of_files : files.val.length ≤ index.val := by omega
    rw [scan_end files p (.Ok current) index end_of_files] at run
    have same : current = total := by simpa using run
    subst total
    simp [List.drop_eq_nil_of_le end_of_files, bytes]
termination_by files.val.length - index.val



theorem validate_reaches_scan
    (value : sandproof_protocol.lifecycle.manifest.SnapshotManifest)
    (expected : sandproof_protocol.lifecycle.manifest.CaptureBinding) (p : Profile) (total : U64)
    (run : manifest_validate value expected p = .ok (.Ok total)) :
    scan value.files p (.Ok 0#u64) 0#usize = .ok (.Ok total) := by
  unfold manifest_validate sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate at run
  split at run
  · simp at run
  · obtain ⟨bad, _, run⟩ := (bind_success _ _ _).mp run
    split at run
    · simp at run
    · split at run
      · simp at run
      · obtain ⟨bad, _, run⟩ := (bind_success _ _ _).mp run
        split at run
        · simp at run
        · obtain ⟨has_version, _, run⟩ := (bind_success _ _ _).mp run
          split at run
          · obtain ⟨bad, _, run⟩ := (bind_success _ _ _).mp run
            split at run
            · simp at run
            · obtain ⟨profile_result, _, run⟩ := (bind_success _ _ _).mp run
              cases profile_result with
              | Err error =>
                simp [core.result.Result.Insts.CoreOpsTry.branch,
                  core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run
              | Ok value =>
                simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at run
                obtain ⟨count, _, run⟩ := (bind_success _ _ _).mp run
                split at run
                · simp at run
                · exact run
          · simp at run

theorem validate_lengths : LengthContract manifest_validate := by
  intro value expected p total run
  have sound := scan_success value.files p 0#u64 total 0#usize
    (validate_reaches_scan value expected p total run)
  simpa using sound

theorem overflow_never_accepted
    (value : sandproof_protocol.lifecycle.manifest.SnapshotManifest)
    (expected : sandproof_protocol.lifecycle.manifest.CaptureBinding) (p : Profile)
    (too_large : U64.max < bytes value.files.val) (total : U64) :
    manifest_validate value expected p ≠ .ok (.Ok total) := by
  intro run
  have exact_sum := (validate_lengths value expected p total run).1
  have bounded : total.val ≤ U64.max := by scalar_tac
  omega

theorem zero_length_never_accepted
    (value : sandproof_protocol.lifecycle.manifest.SnapshotManifest)
    (expected : sandproof_protocol.lifecycle.manifest.CaptureBinding) (p : Profile)
    (file : File) (member : file ∈ value.files.val) (zero : file.length = 0#u64) (total : U64) :
    manifest_validate value expected p ≠ .ok (.Ok total) := by
  intro run
  exact (validate_lengths value expected p total run).2 file member zero

theorem scan_file_error (files : alloc.vec.Vec File) (p : Profile) (current : U64)
    (index : Usize) (within : index.val < files.val.length) (error : Err)
    (refused : sandproof_protocol.lifecycle.manifest.file_length files.val[index.val] index p = .ok (.Err error)) :
    scan files p (.Ok current) index = .ok (.Err error) := by
  obtain ⟨next, add, _⟩ := index_next files index within
  have inside : index < alloc.vec.Vec.len files := by scalar_tac
  rw [scan_unfold]
  simp [body, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    inside, core.result.Result.is_ok, index_exact files index within, refused, add, scan_error]

theorem scan_overflow (files : alloc.vec.Vec File) (p : Profile) (current size : U64)
    (index : Usize) (within : index.val < files.val.length)
    (accepted : sandproof_protocol.lifecycle.manifest.file_length files.val[index.val] index p = .ok (.Ok size))
    (overflow : U64.max < current.val + size.val) :
    scan files p (.Ok current) index = .ok (.Err .LengthOverflow) := by
  have checked : U64.checked_add current size = none := by
    have spec := U64.checked_add_bv_spec current size
    cases eq : U64.checked_add current size with
    | none => rfl
    | some next => simp only [eq] at spec; omega
  obtain ⟨next, add, _⟩ := index_next files index within
  have inside : index < alloc.vec.Vec.len files := by scalar_tac
  rw [scan_unfold]
  simp [body, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    inside, core.result.Result.is_ok, index_exact files index within, accepted,
    lift, checked, core.option.Option.ok_or, add, scan_error]

end SandboxModel.Stage1Protocol.ManifestLength

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.bind_success
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.file_length_success
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.scan_unfold
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.scan_error
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.scan_end
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.index_exact
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.index_next
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.scan_success
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.validate_reaches_scan
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.validate_lengths
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.overflow_never_accepted
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.zero_length_never_accepted
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.scan_file_error
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestLength.scan_overflow
