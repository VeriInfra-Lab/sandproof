import SandboxModel.Lifecycle.Manifest.ManifestFilesModel
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestValidation
open sandproof_protocol.lifecycle.manifest
open SandboxModel.Stage1Protocol.ManifestLength
open SandboxModel.Stage1Protocol.ManifestFiles

/-- Kind and disk identity are checked before the file path or length. -/
def KindAt (profile : Profile) (index : Nat) (file : File) : Prop :=
  match file.kind with
  | .State => index = 0
  | .Memory => index = 1
  | .Disk id => 2 ≤ index ∧ ∃ drive, profile.writable_drives.val[index - 2]? = some drive ∧ id = drive.drive_id

def expectedPath (index : Nat) : String :=
  if index = 0 then "state.bin" else if index = 1 then "memory.bin" else diskName (index - 2)

noncomputable def fileResult (profile : Profile) (index : Nat) (file : File) : Checked := by
  classical
  exact if ¬ KindAt profile index file then .Err .FileSetMismatch
    else if file.relative_path ≠ expectedPath index then .Err .InvalidPath
    else if file.length = 0#u64 then .Err .InvalidLength
    else .Ok file.length

end SandboxModel.Stage1Protocol.ManifestValidation
