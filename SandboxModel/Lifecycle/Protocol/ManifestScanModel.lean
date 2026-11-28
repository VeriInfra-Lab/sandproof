import SandboxModel.Lifecycle.Protocol.ManifestFileModel
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestValidation
open sandproof_protocol.lifecycle.manifest
open SandboxModel.Stage1Protocol.ManifestLength
open SandboxModel.Stage1Protocol.ManifestFiles

/-- Observe the mathematical byte count, retaining every concrete error. -/
def observe : Checked → core.result.Result Nat Err
  | .Ok total => .Ok total.val
  | .Err error => .Err error

/-- Earliest file error, or the first prefix exceeding the u64 bound. The
accumulator is an unbounded natural number, not machine addition. -/
noncomputable def scanResult (profile : Profile) (index total : Nat) :
    List File → core.result.Result Nat Err
  | [] => .Ok total
  | file :: rest =>
    match fileResult profile index file with
    | .Err error => .Err error
    | .Ok length =>
      if U64.max < total + length.val then .Err .LengthOverflow
      else scanResult profile (index + 1) (total + length.val) rest

end SandboxModel.Stage1Protocol.ManifestValidation
