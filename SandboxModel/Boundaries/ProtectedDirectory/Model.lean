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

theorem allow_iff (metadata : Snapshot) (owner : U32) :
    allow metadata owner = true ↔ Allowed metadata owner := by
  simp [allow, ownerAllowed, writesAllowed, Allowed, and_assoc]

theorem private_owner_nonempty :
    allow ⟨1001#u32, 16832#u32, true⟩ 1001#u32 = true := by decide

theorem root_sticky_nonempty :
    allow ⟨0#u32, 17407#u32, true⟩ 9000#u32 = true := by decide

theorem group_writable_is_not_allowed :
    ¬ Allowed ⟨1001#u32, 16888#u32, true⟩ 1001#u32 := by
  unfold Allowed
  decide

theorem nonroot_sticky_is_not_allowed :
    ¬ Allowed ⟨1001#u32, 17407#u32, true⟩ 1001#u32 := by
  unfold Allowed
  decide

end SandboxModel.Boundaries.ProtectedDirectory
