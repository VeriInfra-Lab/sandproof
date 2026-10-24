import SandboxModel.Lifecycle.Protocol.ManifestLengthModel
/-! Declarative complete ordered file set. No generated functions are imported. -/
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestFiles
open SandboxModel.Stage1Protocol.ManifestLength
def diskName (index : Nat) : String := "disk-" ++ toString index ++ ".img"

def FileAt (profile : Profile) (index : Nat) (file : File) : Prop :=
  match index with
  | 0 => file.kind = .State ∧ file.relative_path = "state.bin"
  | 1 => file.kind = .Memory ∧ file.relative_path = "memory.bin"
  | n + 2 => ∃ drive, profile.writable_drives.val[n]? = some drive ∧
      file.kind = .Disk drive.drive_id ∧ file.relative_path = diskName n

def CompleteFiles (profile : Profile) (files : List File) : Prop :=
  files.length = 2 + profile.writable_drives.val.length ∧
    ∀ index (within : index < files.length), FileAt profile index files[index]

def FilesContract
    (validate : sandproof_protocol.lifecycle.manifest.SnapshotManifest →
      sandproof_protocol.lifecycle.manifest.CaptureBinding → Profile → Result Checked) : Prop :=
  ∀ value expected profile total, validate value expected profile = .ok (.Ok total) →
    1 ≤ profile.writable_drives.val.length ∧ profile.writable_drives.val.length ≤ 16 ∧
    CompleteFiles profile value.files.val

end SandboxModel.Stage1Protocol.ManifestFiles
