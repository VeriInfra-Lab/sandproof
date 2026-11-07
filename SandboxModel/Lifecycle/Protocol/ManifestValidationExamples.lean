import SandboxModel.Lifecycle.Protocol.ManifestValidation
import SandboxModel.Lifecycle.Protocol.ManifestValidationValues
import SandboxModel.Lifecycle.Manifest.ManifestFilesExamples
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestValidation.Examples
open sandproof_protocol.lifecycle.manifest
open SandboxModel.Stage1Protocol.ManifestChecks
open SandboxModel.Stage1Protocol.ManifestValidation.Values
open SandboxModel.Stage1Protocol.ManifestProfile
open SandboxModel.Stage1Protocol.ManifestPath
open SandproofFoundationsRust.IdentifierLibrary

theorem normal_complete_contract : Accepted manifest original profile 3 := by
  exact (accepted_iff manifest original profile represented_profile 3#u64).mp
    ManifestFiles.Examples.normal_manifest_accepted

theorem different_capture_precedes_rest (capture : CaptureBinding) (different : original ≠ capture)
    (p : VmCompatibility) (represented : Representable p) (values : alloc.vec.Vec SnapshotFile) :
    manifest_validate (manifestWith p values) capture p = .ok (.Err .BindingMismatch) := by
  apply (refused_iff _ _ _ represented _).mpr
  simp [validationResult, manifestWith, different]

theorem omitted_capture_number_refused :
    manifest_validate manifest { original with capture := 2#u64 } profile = .ok (.Err .BindingMismatch) := by
  apply different_capture_precedes_rest _ _ _ represented_profile
  intro same
  have capture_eq := congrArg CaptureBinding.capture same
  simp [original] at capture_eq

def badProfile : VmCompatibility := { profile with snapshot_format := 0#u16 }

theorem represented_bad_profile : Representable badProfile := represented_profile

theorem complete_profile_precedes_profile_validity :
    manifest_validate manifest original badProfile = .ok (.Err .IncompatibleProfile) := by
  apply (refused_iff _ _ _ represented_bad_profile _).mpr
  have different : profile ≠ badProfile := by
    intro same
    have format_eq := congrArg VmCompatibility.snapshot_format same
    simp [profile, badProfile] at format_eq
  simp [validationResult, manifest, manifestWith, original_binding_valid, different]

theorem invalid_profile_precedes_file_count :
    manifest_validate (manifestWith badProfile (alloc.vec.Vec.from [] (by simp))) original badProfile =
      .ok (.Err .InvalidDriveProfile) := by
  apply (refused_iff _ _ _ represented_bad_profile _).mpr
  simp [validationResult, manifestWith, original_binding_valid, ValidProfile, badProfile]

def nestedPath : Str := Slice.from (stringBytes "dir/.../x")
  (by change (9 : Nat) ≤ Usize.max; scalar_tac)
def parentPath : Str := Slice.from (stringBytes "dir/../x")
  (by change (8 : Nat) ≤ Usize.max; scalar_tac)

theorem nested_three_dot_component_accepted : valid_jail_relative_path nestedPath = .ok true := by
  rw [path_refines]
  have good : ValidPath nestedPath.val := by decide
  simp [good]

theorem parent_component_refused : valid_jail_relative_path parentPath = .ok false := by
  rw [path_refines]
  have bad : ¬ ValidPath parentPath.val := by decide
  simp [bad]

def duplicateProfile : VmCompatibility := { profile with
  writable_drives := (alloc.vec.Vec.from [⟨"rootfs", "r"⟩, ⟨"rootfs", "s"⟩]
    (by simp only [List.length_cons, List.length_nil]; scalar_tac)) }

theorem represented_duplicate_profile : Representable duplicateProfile := by
  intro drive member
  simp [duplicateProfile] at member
  rcases member with rfl | rfl <;> simp [StringRepresentable, stringBytes] <;>
    change (1 : Nat) ≤ Usize.max <;> scalar_tac

theorem duplicate_id_with_different_path_refused :
    VmCompatibility.validate duplicateProfile = .ok (.Err .InvalidDriveProfile) := by
  rw [profile_refines duplicateProfile represented_duplicate_profile]
  have bad : ¬ ValidProfile duplicateProfile := by
    intro good
    have at_second := good.2.2.2 1 (by omega) (by simp [duplicateProfile])
      ⟨"rootfs", "s"⟩ (by simp [duplicateProfile])
    have distinct := at_second.2 0 (by omega) (by omega) ⟨"rootfs", "r"⟩
      (by simp [duplicateProfile])
    exact distinct.1 rfl
  simp [bad]

def duplicatePathProfile : VmCompatibility := { profile with
  writable_drives := (alloc.vec.Vec.from [⟨"rootfs", "r"⟩, ⟨"other", "r"⟩]
    (by simp only [List.length_cons, List.length_nil]; scalar_tac)) }

theorem represented_duplicate_path : Representable duplicatePathProfile := by
  intro drive member
  simp [duplicatePathProfile] at member
  rcases member with rfl | rfl <;> simp [StringRepresentable, stringBytes] <;>
    change (1 : Nat) ≤ Usize.max <;> scalar_tac

theorem duplicate_path_with_different_id_refused :
    VmCompatibility.validate duplicatePathProfile = .ok (.Err .InvalidDriveProfile) := by
  rw [profile_refines duplicatePathProfile represented_duplicate_path]
  have bad : ¬ ValidProfile duplicatePathProfile := by
    intro good
    have at_second := good.2.2.2 1 (by omega) (by simp [duplicatePathProfile])
      ⟨"other", "r"⟩ (by simp [duplicatePathProfile])
    have distinct := at_second.2 0 (by omega) (by omega) ⟨"rootfs", "r"⟩
      (by simp [duplicatePathProfile])
    exact distinct.2 rfl
  simp [bad]

def changedCpu : VmCompatibility := { profile with
  cpu := Array.from (List.replicate 32 (1#u8)) (by simp) }

theorem cpu_digest_field_refused :
    manifest_validate manifest original changedCpu = .ok (.Err .IncompatibleProfile) := by
  have represented : Representable changedCpu := represented_profile
  apply (refused_iff _ _ _ represented _).mpr
  have different : profile ≠ changedCpu := by
    intro same
    have head := congrArg (fun p : VmCompatibility => p.cpu.val.head?) same
    simp [profile, changedCpu, digest] at head
  simp [validationResult, manifest, manifestWith, original_binding_valid, different]

end SandboxModel.Stage1Protocol.ManifestValidation.Examples

#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.normal_complete_contract
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.different_capture_precedes_rest
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.omitted_capture_number_refused
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.represented_bad_profile
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.complete_profile_precedes_profile_validity
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.invalid_profile_precedes_file_count
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.nested_three_dot_component_accepted
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.parent_component_refused
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.represented_duplicate_profile
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.duplicate_id_with_different_path_refused
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.represented_duplicate_path
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.duplicate_path_with_different_id_refused
#audit_controller_proof SandboxModel.Stage1Protocol.ManifestValidation.Examples.cpu_digest_field_refused
