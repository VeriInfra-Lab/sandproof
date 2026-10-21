import SandboxModel.Boundaries.ProtectedDirectory.Metadata

namespace SandboxModel.Boundaries.ProtectedDirectory
open Aeneas Aeneas.Std

def ownerAllowed (metadata : Snapshot) (owner : U32) : Bool :=
  decide (metadata.uid = 0#u32) || decide (metadata.uid = owner)

def writesAllowed (metadata : Snapshot) : Bool :=
  decide ((metadata.mode &&& 18#u32) = 0#u32) ||
    (decide (metadata.uid = 0#u32) && decide ((metadata.mode &&& 512#u32) ≠ 0#u32))

def allow (metadata : Snapshot) (owner : U32) : Bool :=
  metadata.isDirectory && ownerAllowed metadata owner && writesAllowed metadata

/-- The policy classifies the supplied snapshot, not a future path or file. -/
def Allowed (metadata : Snapshot) (owner : U32) : Prop :=
  metadata.isDirectory = true ∧
  (metadata.uid = 0#u32 ∨ metadata.uid = owner) ∧
  ((metadata.mode &&& 18#u32) = 0#u32 ∨
    (metadata.uid = 0#u32 ∧ (metadata.mode &&& 512#u32) ≠ 0#u32))

/-- Shared by the real original function and the independently translated mutant. -/
def AcceptanceSafe {E : Type}
    (checker : std.fs.Metadata → U32 → Result (core.result.Result Unit E)) : Prop :=
  ∀ metadata owner, checker metadata owner = .ok (.Ok ()) → Allowed metadata owner

end SandboxModel.Boundaries.ProtectedDirectory
