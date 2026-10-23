import SandboxModel.Boundaries.ProtectedDirectory.Model
import SandproofProtectedDirectoryBad.Funs
import SandboxModel.ProofAuditCommand
open Aeneas Aeneas.Std Result ControlFlow Error
namespace SandboxModel.Boundaries.ProtectedDirectory

def forbiddenSnapshot : Snapshot := ⟨1001#u32, 16888#u32, true⟩

theorem mutant_accepts_forbidden :
    SandproofProtectedDirectoryBadRust.protected_files.directory
      forbiddenSnapshot 1001#u32 = .ok (.Ok ()) := by
  have nonroot : (1001#u32 : U32) ≠ 0#u32 := by decide
  simp [SandproofProtectedDirectoryBadRust.protected_files.directory,
    forbiddenSnapshot, std.fs.Metadata.Insts.StdOsUnixFsMetadataExt.uid,
    std.fs.Metadata.Insts.StdOsUnixFsMetadataExt.mode, std.fs.Metadata.is_dir,
    Aeneas.Std.lift, nonroot]

theorem mutant_not_acceptance_safe :
    ¬ AcceptanceSafe SandproofProtectedDirectoryBadRust.protected_files.directory := by
  intro safe
  exact group_writable_is_not_allowed (safe forbiddenSnapshot 1001#u32 mutant_accepts_forbidden)

end SandboxModel.Boundaries.ProtectedDirectory
