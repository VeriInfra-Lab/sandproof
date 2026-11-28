import SandboxModel.Lifecycle.Protocol.ManifestPath
import SandboxModel.Lifecycle.Protocol.ManifestProfileModel
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestProfile
open sandproof_protocol.lifecycle.manifest
open SandboxModel.Stage1Protocol.ManifestPath
open SandproofFoundationsRust.IdentifierLibrary

theorem different_refines (a b : Drive) :
    different_drive a b = .ok (by classical exact decide (Separate a b)) := by
  classical
  simp [different_drive, Separate, core.cmp.PartialEq.ne.trait_default,
    core.cmp.PartialEq.ne.default, sandproof_protocol.ResourceId.Insts.CoreCmpPartialEqResourceId.eq,
    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
    SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq]
  split <;> simp_all

theorem distinct_split (drives : List Drive) (drive : Drive) (start stop : Nat)
    (within : start < drives.length) (before : start < stop) :
    DistinctRange drives drive start stop ↔
      Separate drive drives[start] ∧ DistinctRange drives drive (start + 1) stop := by
  constructor
  · intro valid
    constructor
    · exact valid start (by omega) before _ (by simp)
    · intro j after upto previous at_previous
      exact valid j (by omega) upto previous at_previous
  · rintro ⟨head, rest⟩ j after upto previous at_previous
    by_cases same : j = start
    · subst j
      have eq : previous = drives[start] := by simpa [List.getElem?_eq_getElem within] using at_previous.symm
      simpa [eq] using head
    · exact rest j (by omega) upto previous at_previous

theorem distinct_unfold (drives : Slice Drive) (index earlier : Usize) (valid : Bool) :
    distinct_drive_loop drives index earlier valid = (do
      let step ← distinct_drive_loop.body drives index earlier valid
      match step with
      | .done value => .ok value
      | .cont (next, distinct) => distinct_drive_loop drives index next distinct) := by
  unfold distinct_drive_loop
  rw [loop]
  apply congrArg (Aeneas.Std.bind (distinct_drive_loop.body drives index earlier valid))
  funext step
  cases step <;> rfl

theorem distinct_false (drives : Slice Drive) (index earlier : Usize) :
    distinct_drive_loop drives index earlier false = .ok false := by
  rw [distinct_unfold]
  simp [distinct_drive_loop.body]

theorem distinct_refines (drives : Slice Drive) (index earlier : Usize)
    (within : index.val < drives.val.length) :
    distinct_drive_loop drives index earlier true =
      .ok (by classical exact decide (DistinctRange drives.val drives.val[index.val] earlier.val index.val)) := by
  classical
  rw [distinct_unfold]
  by_cases before : earlier.val < index.val
  · have earlier_valid : earlier.val < drives.val.length := by omega
    have lt : earlier < index := by scalar_tac
    have current : Slice.index_usize drives index = .ok drives.val[index.val] := by
      simp [Slice.index_usize, within]
    have previous : Slice.index_usize drives earlier = .ok drives.val[earlier.val] := by
      simp [Slice.index_usize, earlier_valid]
    obtain ⟨next, addition, next_val⟩ := next_counter earlier (by scalar_tac)
    simp only [distinct_drive_loop.body, lt, if_pos, current, previous, bind_tc_ok,
      different_refines, addition]
    rw [distinct_split drives.val drives.val[index.val] earlier.val index.val earlier_valid before]
    by_cases separate : Separate drives.val[index.val] drives.val[earlier.val]
    · simpa [separate, next_val] using distinct_refines drives index next within
    · simp [separate, distinct_false]
  · have ge : ¬ earlier < index := by scalar_tac
    have vacuous : DistinctRange drives.val drives.val[index.val] earlier.val index.val := by
      intro j after upto; omega
    simp [distinct_drive_loop.body, ge, vacuous]
termination_by index.val - earlier.val

theorem valid_split (drives : List Drive) (start : Nat) (within : start < drives.length) :
    ValidFrom drives start ↔
      (ValidPath (stringBytes drives[start].jail_relative_path) ∧
        DistinctRange drives drives[start] 0 start) ∧ ValidFrom drives (start + 1) := by
  constructor
  · intro valid
    constructor
    · exact valid start (by omega) within _ (by simp)
    · intro i after upto
      exact valid i (by omega) upto
  · rintro ⟨head, rest⟩ i after upto drive at_drive
    by_cases same : i = start
    · subst i
      have eq : drive = drives[start] := by
        simpa [List.getElem?_eq_getElem within] using at_drive.symm
      simpa [eq] using head
    · exact rest i (by omega) upto drive at_drive

theorem profile_unfold (drives : alloc.vec.Vec Drive) (index : Usize) (valid : Bool) :
    VmCompatibility.validate_loop drives index valid = (do
      let step ← VmCompatibility.validate_loop.body drives index valid
      match step with
      | .done value => .ok value
      | .cont (next, good) => VmCompatibility.validate_loop drives next good) := by
  unfold VmCompatibility.validate_loop
  rw [loop]
  apply congrArg (Aeneas.Std.bind (VmCompatibility.validate_loop.body drives index valid))
  funext step
  cases step <;> rfl

theorem profile_false (drives : alloc.vec.Vec Drive) (index : Usize) :
    VmCompatibility.validate_loop drives index false = .ok false := by
  rw [profile_unfold]
  simp [VmCompatibility.validate_loop.body]

theorem profile_loop_refines (drives : alloc.vec.Vec Drive) (index : Usize)
    (represented : ∀ drive ∈ drives.val, StringRepresentable drive.jail_relative_path) :
    VmCompatibility.validate_loop drives index true =
      .ok (by classical exact decide (ValidFrom drives.val index.val)) := by
  classical
  rw [profile_unfold]
  by_cases within : index.val < drives.val.length
  · have inside : index < alloc.vec.Vec.len drives := by scalar_tac
    have current : alloc.vec.Vec.index (core.slice.index.SliceIndexUsizeSlice Drive) drives index =
        .ok drives.val[index.val] := by
      simp only [alloc.vec.Vec.val] at within ⊢
      simp [alloc.vec.Vec.index, Slice.index_usize, within]
    have path_represented := represented drives.val[index.val] (List.getElem_mem within)
    have borrowed : alloc.string.String.Insts.CoreOpsDerefDerefStr.deref
        drives.val[index.val].jail_relative_path =
        .ok (Slice.from (stringBytes drives.val[index.val].jail_relative_path) path_represented) := by
      simp [alloc.string.String.Insts.CoreOpsDerefDerefStr.deref,
        SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref, path_represented]
    obtain ⟨next, addition, next_val⟩ := next_counter index (by scalar_tac)
    have deref_eq : alloc.vec.Vec.deref drives = drives.slice := by
      simp [alloc.vec.Vec.deref, alloc.vec.Vec.val]
    have unique := distinct_refines drives.slice index 0#usize within
    change distinct_drive_loop drives.slice index 0#usize true =
      .ok (decide (DistinctRange drives.val drives.val[index.val] 0 index.val)) at unique
    simp only [VmCompatibility.validate_loop.body, inside, if_pos, current, borrowed,
      bind_tc_ok, path_refines, Slice.from_val, distinct_drive, deref_eq, unique, addition]
    rw [valid_split drives.val index.val within]
    have rest := profile_loop_refines drives next represented
    generalize current_drive : drives.val[index.val] = drive at *
    by_cases path : ValidPath (stringBytes drive.jail_relative_path)
    · by_cases distinct : DistinctRange drives.val drive 0 index.val
      · simpa [path, distinct, next_val] using rest
      · simp [path, distinct, profile_false]
    · simp [path, profile_false]
  · have outside : ¬ index < alloc.vec.Vec.len drives := by scalar_tac
    have vacuous : ValidFrom drives.val index.val := by
      intro i after upto; omega
    simp [VmCompatibility.validate_loop.body, outside, vacuous]
termination_by drives.val.length - index.val

theorem profile_refines (profile : VmCompatibility) (represented : Representable profile) :
    VmCompatibility.validate profile = .ok
      (by classical exact if ValidProfile profile then .Ok () else .Err .InvalidDriveProfile) := by
  classical
  unfold VmCompatibility.validate
  by_cases zero : profile.snapshot_format = 0#u16
  · simp [zero, ValidProfile]
  · simp only [zero, if_false, alloc.vec.Vec.is_empty,
      SandproofFoundationsRust.alloc.vec.Vec.is_empty, bind_tc_ok]
    by_cases empty : profile.writable_drives.val = []
    · simp [empty, ValidProfile]
    · have positive : 1 ≤ profile.writable_drives.val.length := by
        have : profile.writable_drives.val.length ≠ 0 := fun h => empty (List.eq_nil_of_length_eq_zero h)
        omega
      simp only [show profile.writable_drives.val.length ≠ 0 by omega,
        decide_false, Bool.false_eq_true, if_false]
      by_cases large : 16 < profile.writable_drives.val.length
      · have too_large : alloc.vec.Vec.len profile.writable_drives > MAX_WRITABLE_DRIVES := by
          simp only [MAX_WRITABLE_DRIVES]; scalar_tac
        simp [too_large, ValidProfile, Nat.not_le_of_gt large]
      · have small : ¬ alloc.vec.Vec.len profile.writable_drives > MAX_WRITABLE_DRIVES := by
          simp only [MAX_WRITABLE_DRIVES]; scalar_tac
        simp [small, profile_loop_refines profile.writable_drives 0#usize represented,
          ValidProfile, zero, positive, Nat.le_of_not_gt large]
        split <;> rfl

end SandboxModel.Stage1Protocol.ManifestProfile

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestProfile.different_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestProfile.distinct_split
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestProfile.distinct_unfold
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestProfile.distinct_false
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestProfile.distinct_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestProfile.valid_split
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestProfile.profile_unfold
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestProfile.profile_false
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestProfile.profile_loop_refines
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestProfile.profile_refines
