import SandproofStage1Manifest.Types

/-! Independent length contract: no generated functions or validation assumptions. -/
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestLength
abbrev File := sandproof_protocol.lifecycle.manifest.SnapshotFile
abbrev Profile := sandproof_protocol.lifecycle.manifest.VmCompatibility
abbrev Err := sandproof_protocol.lifecycle.manifest.ManifestError
abbrev Checked := core.result.Result U64 Err

def bytes (files : List File) : Nat := (files.map (fun file => file.length.val)).sum

def LengthContract
    (validate : sandproof_protocol.lifecycle.manifest.SnapshotManifest →
      sandproof_protocol.lifecycle.manifest.CaptureBinding → Profile → Result Checked) : Prop :=
  ∀ value expected p total, validate value expected p = .ok (.Ok total) →
    total.val = bytes value.files.val ∧ ∀ file ∈ value.files.val, file.length ≠ 0#u64

end SandboxModel.Stage1Protocol.ManifestLength
