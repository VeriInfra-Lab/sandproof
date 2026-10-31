import SandboxModel.Lifecycle.Protocol.ManifestScanModel
import SandboxModel.Lifecycle.Protocol.ManifestProfileModel
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestValidation
open sandproof_protocol.lifecycle.manifest sandproof_protocol.lifecycle.identity
open SandboxModel.Stage1Protocol.ManifestLength
open SandboxModel.Stage1Protocol.ManifestProfile
open SandboxModel.Stage1Protocol.ManifestFiles

/-- Only the Version variant is required here. This is not an authorization,
durable-acceptance or actual-resource-existence predicate. -/
def HasVersion (binding : CaptureBinding) : Prop :=
  ∃ version, binding.operation.expected = .Version version

def BindingValid (binding : CaptureBinding) : Prop :=
  binding.operation.protocol_version = 4#u16 ∧
    binding.operation.key.deployment = binding.operation.node.deployment_id ∧ HasVersion binding

/-- Complete ordered result specification, starting from decoded values. -/
noncomputable def validationResult (value : SnapshotManifest) (expected : CaptureBinding)
    (profile : Profile) : core.result.Result Nat Err := by
  classical
  exact if value.format_version ≠ 1#u16 then .Err .UnsupportedVersion
    else if value.binding ≠ expected then .Err .BindingMismatch
    else if ¬ BindingValid value.binding then .Err .InvalidBinding
    else if value.compatibility ≠ profile then .Err .IncompatibleProfile
    else if ¬ ValidProfile profile then .Err .InvalidDriveProfile
    else if value.files.val.length ≠ 2 + profile.writable_drives.val.length then .Err .FileSetMismatch
    else scanResult profile 0 0 value.files.val

def ValidationContract
    (validate : SnapshotManifest → CaptureBinding → Profile → Result Checked) : Prop :=
  ∀ value expected profile, Representable profile →
    (do let result ← validate value expected profile; .ok (observe result)) =
      .ok (validationResult value expected profile)

/-- Full declarative acceptance condition, reusing the existing exact file-set
and length specifications. No generated validator occurs in this predicate. -/
def Accepted (value : SnapshotManifest) (expected : CaptureBinding) (profile : Profile)
    (total : Nat) : Prop :=
  value.format_version = 1#u16 ∧ value.binding = expected ∧ BindingValid value.binding ∧
    value.compatibility = profile ∧ ValidProfile profile ∧ CompleteFiles profile value.files.val ∧
    (∀ file ∈ value.files.val, file.length ≠ 0#u64) ∧ total = bytes value.files.val ∧ total ≤ U64.max

def GoodFiles (profile : Profile) (start : Nat) (files : List File) : Prop :=
  ∀ j (within : j < files.length), FileAt profile (start + j) files[j] ∧ files[j].length ≠ 0#u64

end SandboxModel.Stage1Protocol.ManifestValidation
