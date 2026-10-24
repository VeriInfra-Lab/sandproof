import SandboxModel.Lifecycle.Protocol.ManifestFilesModel
import SandboxModel.Lifecycle.Protocol.ManifestLength
import SandboxModel.Lifecycle.Protocol.ManifestValues
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestFiles
open SandboxModel.Stage1Protocol.ManifestLength

theorem disk_path_exact (index : Usize) :
    sandproof_protocol.lifecycle.manifest.disk_bundle_path index =
      .ok (if index.val < 16 then some (diskName index.val) else none) := by
  by_cases within : index.val < 16
  · have alternatives : index.val = 0 ∨ index.val = 1 ∨ index.val = 2 ∨ index.val = 3 ∨ index.val = 4 ∨ index.val = 5 ∨ index.val = 6 ∨ index.val = 7 ∨ index.val = 8 ∨ index.val = 9 ∨ index.val = 10 ∨ index.val = 11 ∨ index.val = 12 ∨ index.val = 13 ∨ index.val = 14 ∨ index.val = 15 := by omega
    rcases alternatives with equation | equation | equation | equation | equation | equation | equation | equation | equation | equation | equation | equation | equation | equation | equation | equation <;>
      simp [sandproof_protocol.lifecycle.manifest.disk_bundle_path, equation, diskName,
        SandboxModel.Stage1Protocol.Manifest.literal_owned] <;> decide
  · unfold sandproof_protocol.lifecycle.manifest.disk_bundle_path
    split <;> simp_all

theorem vector_index_success {T : Type} (values : alloc.vec.Vec T) (index : Usize) (value : T)
    (run : alloc.vec.Vec.index (core.slice.index.SliceIndexUsizeSlice T) values index = .ok value) :
    values.val[index.val]? = some value := by
  simp only [alloc.vec.Vec.index, Slice.index_usize] at run
  split at run
  · simp at run
  · rename_i item at_index
    have same : item = value := by simpa using run
    simpa [alloc.vec.Vec.val, same] using at_index

theorem path_gate_success (file : File) (expected : String) (total : U64)
    (run : (do
      let bad ← core.cmp.PartialEq.ne.trait_default alloc.string.String.Insts.CoreCmpPartialEqString
        file.relative_path expected
      if bad then .ok (.Err .InvalidPath)
      else if file.length = 0#u64 then .ok (.Err .InvalidLength)
      else .ok (.Ok file.length) : Result Checked) = .ok (.Ok total)) : file.relative_path = expected := by
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    bind_tc_ok] at run
  split at run <;> simp_all

theorem file_shape_success (file : File) (index : Usize) (profile : Profile) (n : U64)
    (run : sandproof_protocol.lifecycle.manifest.file_length file index profile = .ok (.Ok n)) :
    FileAt profile index.val file := by
  unfold sandproof_protocol.lifecycle.manifest.file_length at run
  split at run
  · rename_i kind
    split at run
    · simp at run
    · rename_i correct_index
      have at_zero : index = 0#usize := by apply UScalar.eq_of_val_eq; simpa using correct_index
      subst index
      simp only [SandboxModel.Stage1Protocol.Manifest.literal_owned, bind_tc_ok] at run
      exact ⟨kind, path_gate_success file "state.bin" n run⟩
  · rename_i kind
    split at run
    · simp at run
    · rename_i correct_index
      have at_one : index = 1#usize := by apply UScalar.eq_of_val_eq; simpa using correct_index
      subst index
      simp only [SandboxModel.Stage1Protocol.Manifest.literal_owned, bind_tc_ok] at run
      exact ⟨kind, path_gate_success file "memory.bin" n run⟩
  · rename_i drive_id kind
    split at run
    · simp at run
    · rename_i at_least_two
      have lower : 2 ≤ index.val := by scalar_tac
      obtain ⟨i, sub_run, run⟩ := (bind_success _ _ _).mp run
      obtain ⟨drive, at_drive, run⟩ := (bind_success _ _ _).mp run
      have subtraction := Usize.sub_bv_spec (x := index) (y := 2#usize) (by scalar_tac)
      obtain ⟨difference, sub_equation, sub_post⟩ := (WP.spec_equiv_exists _ _).mp subtraction
      have same_difference : i = difference := by simpa [sub_run] using sub_equation
      subst difference
      have position : i.val = index.val - 2 := by simpa using sub_post.1
      have at_drive := vector_index_success _ _ _ at_drive
      simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
        alloc.string.String.Insts.CoreCmpPartialEqString.eq,
        SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq,
        bind_tc_ok] at run
      split at run
      · simp at run
      · rename_i same_id
        have same_id : drive_id = drive.drive_id := by simpa using same_id
        rw [disk_path_exact] at run
        split at run
        · simp only [core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at run
          have path : file.relative_path = diskName i.val := by
            split at run <;> simp_all
          have offset : index.val = (index.val - 2) + 2 := by omega
          rw [offset]
          exact ⟨drive, by simpa [position] using at_drive, by simpa [same_id] using kind,
            by simpa [position] using path⟩
        · simp [core.option.Option.ok_or, core.result.Result.Insts.CoreOpsTry.branch,
            core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run

theorem scan_checks_file (files : alloc.vec.Vec File) (profile : Profile)
    (current total : U64) (index target : Usize)
    (run : scan files profile (.Ok current) index = .ok (.Ok total))
    (after_start : index.val ≤ target.val) (target_valid : target.val < files.val.length) :
    ∃ length, sandproof_protocol.lifecycle.manifest.file_length
      files.val[target.val] target profile = .ok (.Ok length) := by
  have within : index.val < files.val.length := by omega
  obtain ⟨next, add, next_val⟩ := index_next files index within
  have inside : index < alloc.vec.Vec.len files := by scalar_tac
  rw [scan_unfold] at run
  simp only [body, sandproof_protocol.lifecycle.manifest.SnapshotManifest.validate_loop.body,
    inside, if_pos, core.result.Result.is_ok, bind_tc_ok, index_exact files index within,
    bind_assoc_eq] at run
  obtain ⟨checked, checked_run, run⟩ := (bind_success _ _ _).mp run
  cases checked with
  | Err error => simp [add, scan_error] at run
  | Ok length =>
    by_cases same : index = target
    · subst target
      exact ⟨length, checked_run⟩
    · simp only [lift, bind_tc_ok, core.option.Option.ok_or, add] at run
      cases sum_eq : U64.checked_add current length with
      | none => rw [sum_eq] at run; simp [scan_error] at run
      | some next_total =>
        rw [sum_eq] at run
        simp only [bind_tc_ok] at run
        have different : index.val ≠ target.val := by
          intro eq
          exact same (UScalar.eq_of_val_eq eq)
        exact scan_checks_file files profile next_total total next target run (by omega) target_valid
termination_by files.val.length - index.val

theorem scan_file_shapes (files : alloc.vec.Vec File) (profile : Profile) (total : U64)
    (run : scan files profile (.Ok 0#u64) 0#usize = .ok (.Ok total)) :
    ∀ index (within : index < files.val.length), FileAt profile index files.val[index] := by
  intro index within
  have bound : index < 2^UScalarTy.Usize.numBits := by scalar_tac
  let target := Usize.ofNatCore index bound
  have target_val : target.val = index := Usize.ofNatCore_val_eq bound
  obtain ⟨length, checked⟩ := scan_checks_file files profile 0#u64 total 0#usize target run
    (by scalar_tac) (by simpa [target_val] using within)
  have shape := file_shape_success _ _ _ _ checked
  simpa [target_val] using shape

theorem profile_count_bounds (profile : Profile)
    (run : sandproof_protocol.lifecycle.manifest.VmCompatibility.validate profile = .ok (.Ok ())) :
    1 ≤ profile.writable_drives.val.length ∧ profile.writable_drives.val.length ≤ 16 := by
  unfold sandproof_protocol.lifecycle.manifest.VmCompatibility.validate at run
  split at run
  · simp at run
  · simp only [alloc.vec.Vec.is_empty, SandproofFoundationsRust.alloc.vec.Vec.is_empty, bind_tc_ok] at run
    split at run
    · simp at run
    · rename_i nonempty
      split at run
      · simp at run
      · rename_i bounded
        constructor
        · simp at nonempty
          have length_ne : profile.writable_drives.val.length ≠ 0 := by
            intro empty
            exact nonempty (List.eq_nil_of_length_eq_zero empty)
          omega
        · simp only [sandproof_protocol.lifecycle.manifest.MAX_WRITABLE_DRIVES] at bounded
          scalar_tac

theorem validate_shape_preconditions
    (value : sandproof_protocol.lifecycle.manifest.SnapshotManifest)
    (expected : sandproof_protocol.lifecycle.manifest.CaptureBinding) (p : Profile) (total : U64)
    (run : manifest_validate value expected p = .ok (.Ok total)) :
    1 ≤ p.writable_drives.val.length ∧ p.writable_drives.val.length ≤ 16 ∧
      value.files.val.length = 2 + p.writable_drives.val.length := by
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
            · obtain ⟨profile_result, profile_run, run⟩ := (bind_success _ _ _).mp run
              cases profile_result with
              | Err error =>
                simp [core.result.Result.Insts.CoreOpsTry.branch,
                  core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual] at run
              | Ok unit_value =>
                cases unit_value
                have bounds := profile_count_bounds p profile_run
                simp only [core.result.Result.Insts.CoreOpsTry.branch, bind_tc_ok] at run
                obtain ⟨count, count_run, run⟩ := (bind_success _ _ _).mp run
                split at run
                · simp at run
                · rename_i count_matches
                  have sum_bound : (2#usize).val + (alloc.vec.Vec.len p.writable_drives).val ≤ Usize.max := by
                    scalar_tac
                  obtain ⟨sum, sum_run, sum_post⟩ := (WP.spec_equiv_exists _ _).mp
                    (Usize.add_bv_spec (x := 2#usize) (y := alloc.vec.Vec.len p.writable_drives) sum_bound)
                  have same : count = sum := by simpa [count_run] using sum_run
                  subst sum
                  exact ⟨bounds.1, bounds.2, by scalar_tac⟩
          · simp at run

theorem validate_files : FilesContract manifest_validate := by
  intro value expected profile total run
  obtain ⟨minimum, maximum, count⟩ := validate_shape_preconditions value expected profile total run
  exact ⟨minimum, maximum, count,
    scan_file_shapes value.files profile total (validate_reaches_scan value expected profile total run)⟩

theorem wrong_count_rejected
    (value : sandproof_protocol.lifecycle.manifest.SnapshotManifest)
    (expected : sandproof_protocol.lifecycle.manifest.CaptureBinding) (profile : Profile) (total : U64)
    (wrong : value.files.val.length ≠ 2 + profile.writable_drives.val.length) :
    manifest_validate value expected profile ≠ .ok (.Ok total) := by
  intro run
  exact wrong (validate_files value expected profile total run).2.2.1

theorem wrong_file_rejected
    (value : sandproof_protocol.lifecycle.manifest.SnapshotManifest)
    (expected : sandproof_protocol.lifecycle.manifest.CaptureBinding) (profile : Profile) (total : U64)
    (index : Nat) (within : index < value.files.val.length)
    (wrong : ¬ FileAt profile index value.files.val[index]) :
    manifest_validate value expected profile ≠ .ok (.Ok total) := by
  intro run
  exact wrong ((validate_files value expected profile total run).2.2.2 index within)

theorem validate_layout_and_lengths
    (value : sandproof_protocol.lifecycle.manifest.SnapshotManifest)
    (expected : sandproof_protocol.lifecycle.manifest.CaptureBinding) (profile : Profile) (total : U64)
    (run : manifest_validate value expected profile = .ok (.Ok total)) :
    CompleteFiles profile value.files.val ∧ total.val = bytes value.files.val ∧
      ∀ file ∈ value.files.val, file.length ≠ 0#u64 := by
  exact ⟨(validate_files value expected profile total run).2.2,
    validate_lengths value expected profile total run⟩

theorem file_roles_unique (profile : Profile) (index : Nat) (file : File)
    (shape : FileAt profile index file) :
    (file.kind = .State ↔ index = 0) ∧ (file.kind = .Memory ↔ index = 1) := by
  cases index with
  | zero => obtain ⟨kind, _⟩ := shape; simp [kind]
  | succ index =>
    cases index with
    | zero => obtain ⟨kind, _⟩ := shape; simp [kind]
    | succ index =>
      obtain ⟨drive, _, kind, _⟩ := shape
      simp [kind]

theorem state_memory_unique
    (value : sandproof_protocol.lifecycle.manifest.SnapshotManifest)
    (expected : sandproof_protocol.lifecycle.manifest.CaptureBinding) (profile : Profile) (total : U64)
    (run : manifest_validate value expected profile = .ok (.Ok total))
    (index : Nat) (within : index < value.files.val.length) :
    (value.files.val[index].kind = .State ↔ index = 0) ∧
      (value.files.val[index].kind = .Memory ↔ index = 1) := by
  exact file_roles_unique profile index _ ((validate_files value expected profile total run).2.2.2 index within)

theorem every_drive_present
    (value : sandproof_protocol.lifecycle.manifest.SnapshotManifest)
    (expected : sandproof_protocol.lifecycle.manifest.CaptureBinding) (profile : Profile) (total : U64)
    (run : manifest_validate value expected profile = .ok (.Ok total))
    (index : Nat) (within : index < profile.writable_drives.val.length) :
    ∃ file, value.files.val[index + 2]? = some file ∧
      file.kind = .Disk profile.writable_drives.val[index].drive_id ∧
      file.relative_path = diskName index := by
  have contract := (validate_files value expected profile total run).2.2
  have count := contract.1
  have file_exists : index + 2 < value.files.val.length := by omega
  obtain ⟨drive, drive_at, kind, path⟩ := contract.2 (index + 2) file_exists
  have actual : profile.writable_drives.val[index]? = some profile.writable_drives.val[index] :=
    List.getElem?_eq_getElem within
  have same : profile.writable_drives.val[index] = drive := by simpa [actual] using drive_at
  exact ⟨value.files.val[index + 2], List.getElem?_eq_getElem file_exists, by simpa [same] using kind, path⟩

end SandboxModel.Stage1Protocol.ManifestFiles

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.disk_path_exact
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.vector_index_success
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.path_gate_success
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.file_shape_success
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.scan_checks_file
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.scan_file_shapes
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.profile_count_bounds
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.validate_shape_preconditions
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.validate_files
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.wrong_count_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.wrong_file_rejected
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.validate_layout_and_lengths
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.file_roles_unique
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.state_memory_unique
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestFiles.every_drive_present
