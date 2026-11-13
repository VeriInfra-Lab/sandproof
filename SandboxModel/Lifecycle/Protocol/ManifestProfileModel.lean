import SandboxModel.Lifecycle.Protocol.ManifestPathModel
import SandproofFoundations.IdentifierLibrary
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestProfile
open sandproof_protocol.lifecycle.manifest
open SandboxModel.Stage1Protocol.ManifestPath
open SandproofFoundationsRust.IdentifierLibrary

abbrev Drive := WritableDrive

def Separate (a b : Drive) : Prop :=
  a.drive_id ≠ b.drive_id ∧ a.jail_relative_path ≠ b.jail_relative_path

def DistinctRange (drives : List Drive) (drive : Drive) (start stop : Nat) : Prop :=
  ∀ j, start ≤ j → j < stop → ∀ previous, drives[j]? = some previous → Separate drive previous

def DriveAt (drives : List Drive) (i : Nat) : Prop :=
  ∀ drive, drives[i]? = some drive →
    ValidPath (stringBytes drive.jail_relative_path) ∧ DistinctRange drives drive 0 i

def ValidFrom (drives : List Drive) (start : Nat) : Prop :=
  ∀ i, start ≤ i → i < drives.length → DriveAt drives i

/-- Representation domain only: these String values can be borrowed as Rust str.
No path-validity or uniqueness condition is assumed. -/
def Representable (profile : VmCompatibility) : Prop :=
  ∀ drive ∈ profile.writable_drives.val, StringRepresentable drive.jail_relative_path

def ValidProfile (profile : VmCompatibility) : Prop :=
  profile.snapshot_format ≠ 0#u16 ∧
    1 ≤ profile.writable_drives.val.length ∧ profile.writable_drives.val.length ≤ 16 ∧
    ValidFrom profile.writable_drives.val 0

end SandboxModel.Stage1Protocol.ManifestProfile
