import SandboxModel.Lifecycle.Protocol.ManifestValidationModel
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestValidation
open sandproof_protocol.lifecycle.manifest
open SandboxModel.Stage1Protocol.ManifestLength
open SandboxModel.Stage1Protocol.ManifestFiles
open SandboxModel.Stage1Protocol.ManifestProfile

theorem kind_path_iff (profile : Profile) (index : Nat) (file : File) :
    KindAt profile index file ∧ file.relative_path = expectedPath index ↔ FileAt profile index file := by
  cases index with
  | zero => cases kind : file.kind <;> simp [KindAt, FileAt, expectedPath, kind]
  | succ index =>
    cases index with
    | zero => cases kind : file.kind <;> simp [KindAt, FileAt, expectedPath, kind]
    | succ index =>
      cases kind : file.kind <;>
        simp [KindAt, FileAt, expectedPath, kind]
      constructor
      · rintro ⟨⟨drive, at_drive, same⟩, path⟩
        exact ⟨drive, at_drive, same, path⟩
      · rintro ⟨drive, at_drive, same, path⟩
        exact ⟨⟨drive, at_drive, same⟩, path⟩

theorem file_result_ok (profile : Profile) (index : Nat) (file : File) (length : U64) :
    fileResult profile index file = .Ok length ↔
      length = file.length ∧ FileAt profile index file ∧ file.length ≠ 0#u64 := by
  classical
  rw [← kind_path_iff]
  unfold fileResult
  split <;> simp_all
  split <;> simp_all
  split <;> simp_all
  exact eq_comm

theorem good_files_cons (profile : Profile) (start : Nat) (file : File) (rest : List File) :
    GoodFiles profile start (file :: rest) ↔
      (FileAt profile start file ∧ file.length ≠ 0#u64) ∧ GoodFiles profile (start + 1) rest := by
  constructor
  · intro good
    constructor
    · have head := good 0 (by simp)
      change FileAt profile (start + 0) file ∧ file.length ≠ 0#u64 at head
      simpa only [Nat.add_zero] using head
    · intro j within
      have later := good (j + 1) (by simp; omega)
      change FileAt profile (start + (j + 1)) rest[j] ∧ rest[j].length ≠ 0#u64 at later
      simpa only [Nat.add_assoc, Nat.add_left_comm, Nat.add_comm] using later
  · rintro ⟨head, tail⟩ j within
    cases j with
    | zero => simpa using head
    | succ j => simpa [Nat.add_assoc, Nat.add_left_comm, Nat.add_comm] using tail j (by simpa using within)

theorem scan_result_ok (profile : Profile) (start current total : Nat) (files : List File)
    (bounded : current ≤ U64.max) :
    scanResult profile start current files = .Ok total ↔
      GoodFiles profile start files ∧ total = current + bytes files ∧ total ≤ U64.max := by
  classical
  induction files generalizing start current with
  | nil =>
    have empty : GoodFiles profile start [] := by
      intro j within; simp at within
    simp only [scanResult, empty, bytes, List.map_nil, List.sum_nil, Nat.add_zero, true_and,
      core.result.Result.Ok.injEq]
    constructor
    · intro h; subst total; exact ⟨rfl, bounded⟩
    · intro h; exact h.1.symm
  | cons file rest ih =>
    rw [scanResult, good_files_cons]
    cases checked : fileResult profile start file with
    | Err error =>
      have invalid : ¬ (FileAt profile start file ∧ file.length ≠ 0#u64) := by
        intro good
        have run := (file_result_ok profile start file file.length).mpr ⟨rfl, good⟩
        simp [checked] at run
      constructor
      · intro impossible; cases impossible
      · rintro ⟨head, _⟩
        exact False.elim (invalid head.1)
    | Ok length =>
      obtain ⟨same, shape, positive⟩ := (file_result_ok profile start file length).mp checked
      subst length
      by_cases overflow : U64.max < current + file.length.val
      · simp only [overflow, if_pos]
        constructor
        · intro impossible; cases impossible
        · rintro ⟨_, total_eq, bound⟩
          simp only [bytes, List.map_cons, List.sum_cons] at total_eq
          omega
      · have safe : current + file.length.val ≤ U64.max := by omega
        simp [overflow, ih (start + 1) (current + file.length.val) safe, shape, positive, bytes,
          Nat.add_assoc]

theorem good_files_zero (profile : Profile) (files : List File) :
    GoodFiles profile 0 files ↔
      (∀ i (within : i < files.length), FileAt profile i files[i]) ∧
      (∀ file ∈ files, file.length ≠ 0#u64) := by
  constructor
  · intro good
    constructor
    · intro i within; simpa using (good i within).1
    · intro file member
      obtain ⟨i, within, at_file⟩ := List.mem_iff_getElem.mp member
      simpa [at_file] using (good i within).2
  · rintro ⟨shape, positive⟩ i within
    exact ⟨by simpa using shape i within, positive _ (List.getElem_mem within)⟩

theorem validation_result_ok (value : SnapshotManifest) (expected : CaptureBinding)
    (profile : Profile) (total : Nat) :
    validationResult value expected profile = .Ok total ↔ Accepted value expected profile total := by
  classical
  unfold validationResult Accepted
  by_cases format : value.format_version = 1#u16
  · simp only [format, ne_eq, not_true_eq_false, if_false, true_and]
    by_cases binding : value.binding = expected
    · simp only [binding, not_true_eq_false, if_false, true_and]
      by_cases valid_binding : BindingValid expected
      · simp only [valid_binding, not_true_eq_false, if_false, true_and]
        by_cases compatible : value.compatibility = profile
        · simp only [compatible, not_true_eq_false, if_false, true_and]
          by_cases valid_profile : ValidProfile profile
          · simp only [valid_profile, not_true_eq_false, if_false, true_and]
            by_cases count : value.files.val.length = 2 + profile.writable_drives.val.length
            · simp [count, scan_result_ok profile 0 0 total value.files.val (by scalar_tac),
                good_files_zero, CompleteFiles, and_assoc]
            · simp [count, CompleteFiles]
          · simp [valid_profile]
        · simp [compatible]
      · simp [valid_binding]
    · simp [binding]
  · simp [format]

theorem observe_injective : Function.Injective observe := by
  intro a b equal
  cases a with
  | Err error => cases b <;> simp_all [observe]
  | Ok total =>
    cases b with
    | Err error => simp [observe] at equal
    | Ok other =>
      have values : total.val = other.val := by simpa [observe] using equal
      exact congrArg core.result.Result.Ok (UScalar.eq_of_val_eq values)

theorem observe_run_iff (actual : Result Checked) (expected : Checked) :
    (do let result ← actual; .ok (observe result)) = .ok (observe expected) ↔ actual = .ok expected := by
  cases actual <;> simp [observe_injective.eq_iff]

theorem contract_acceptance
    (validate : SnapshotManifest → CaptureBinding → Profile → Result Checked)
    (contract : ValidationContract validate) (value : SnapshotManifest) (expected : CaptureBinding)
    (profile : Profile) (represented : Representable profile) (total : U64)
    (run : validate value expected profile = .ok (.Ok total)) :
    Accepted value expected profile total.val := by
  have spec := contract value expected profile represented
  rw [run] at spec
  apply (validation_result_ok value expected profile total.val).mp
  simpa [observe] using spec.symm

end SandboxModel.Stage1Protocol.ManifestValidation

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.kind_path_iff
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.file_result_ok
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.good_files_cons
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.scan_result_ok
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.good_files_zero
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.validation_result_ok
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.observe_injective
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.observe_run_iff
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.contract_acceptance
