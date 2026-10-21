import SandboxModel.Boundaries.ProtectedDirectory.Model
import SandproofProtectedDirectory.Funs
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std Result ControlFlow Error

namespace SandboxModel.Boundaries.ProtectedDirectory
theorem source_exact (metadata : Snapshot) (owner : U32) :
    SandproofProtectedDirectoryRust.protected_files.directory metadata owner =
      .ok (if allow metadata owner then .Ok () else .Err .Untrusted) := by
  by_cases root : metadata.uid = 0#u32 <;>
    by_cases same : metadata.uid = owner <;>
    by_cases clear : (metadata.mode &&& 18#u32) = 0#u32 <;>
    by_cases sticky : (metadata.mode &&& 512#u32) = 0#u32 <;>
    cases kind : metadata.isDirectory <;>
    simp [SandproofProtectedDirectoryRust.protected_files.directory,
      std.fs.Metadata.Insts.StdOsUnixFsMetadataExt.uid,
      std.fs.Metadata.Insts.StdOsUnixFsMetadataExt.mode,
      std.fs.Metadata.is_dir, Aeneas.Std.lift, allow, ownerAllowed, writesAllowed,
      root, same, clear, sticky, kind]
  all_goals
    have owner_not_root : owner ≠ 0#u32 := by
      intro owner_zero
      exact root (same.trans owner_zero)
    simp [owner_not_root]

theorem source_ok_iff (metadata : Snapshot) (owner : U32) :
    SandproofProtectedDirectoryRust.protected_files.directory metadata owner = .ok (.Ok ()) ↔
      Allowed metadata owner := by
  rw [source_exact]
  by_cases good : allow metadata owner = true
  · have valid := (allow_iff metadata owner).mp good
    simp [good, valid]
  · have invalid : ¬ Allowed metadata owner := fun h => good ((allow_iff metadata owner).mpr h)
    simp [good, invalid]

theorem source_untrusted_iff (metadata : Snapshot) (owner : U32) :
    SandproofProtectedDirectoryRust.protected_files.directory metadata owner = .ok (.Err .Untrusted) ↔
      ¬ Allowed metadata owner := by
  rw [source_exact]
  by_cases good : allow metadata owner = true
  · have valid := (allow_iff metadata owner).mp good
    simp [good, valid]
  · have invalid : ¬ Allowed metadata owner := fun h => good ((allow_iff metadata owner).mpr h)
    simp [good, invalid]

theorem source_acceptance_safe :
    AcceptanceSafe SandproofProtectedDirectoryRust.protected_files.directory := by
  intro metadata owner accepted
  exact (source_ok_iff metadata owner).mp accepted

theorem source_rejects_non_directory (metadata : Snapshot) (owner : U32)
    (notDirectory : metadata.isDirectory = false) :
    SandproofProtectedDirectoryRust.protected_files.directory metadata owner = .ok (.Err .Untrusted) := by
  apply (source_untrusted_iff metadata owner).mpr
  intro valid
  have kind := valid.1
  simp [notDirectory] at kind

theorem source_rejects_other_owner (metadata : Snapshot) (owner : U32)
    (notRoot : metadata.uid ≠ 0#u32) (notOwner : metadata.uid ≠ owner) :
    SandproofProtectedDirectoryRust.protected_files.directory metadata owner = .ok (.Err .Untrusted) := by
  apply (source_untrusted_iff metadata owner).mpr
  intro valid
  exact valid.2.1.elim notRoot notOwner

theorem source_rejects_nonroot_writes (metadata : Snapshot) (owner : U32)
    (notRoot : metadata.uid ≠ 0#u32) (writable : (metadata.mode &&& 18#u32) ≠ 0#u32) :
    SandproofProtectedDirectoryRust.protected_files.directory metadata owner = .ok (.Err .Untrusted) := by
  apply (source_untrusted_iff metadata owner).mpr
  intro valid
  exact valid.2.2.elim writable (fun h => notRoot h.1)

theorem source_private_owner_accepted :
    SandproofProtectedDirectoryRust.protected_files.directory
      ⟨1001#u32, 16832#u32, true⟩ 1001#u32 = .ok (.Ok ()) := by
  rw [source_exact, private_owner_nonempty]
  rfl

theorem source_root_sticky_accepted :
    SandproofProtectedDirectoryRust.protected_files.directory
      ⟨0#u32, 17407#u32, true⟩ 9000#u32 = .ok (.Ok ()) := by
  rw [source_exact, root_sticky_nonempty]
  rfl

end SandboxModel.Boundaries.ProtectedDirectory
