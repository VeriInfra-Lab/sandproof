import SandproofStage1Manifest.Types
open Aeneas Aeneas.Std SandproofStage1ManifestRust
namespace SandboxModel.Stage1Protocol.ManifestPath

/-- A component is nonempty; one or two dots alone are forbidden. -/
def component (part : List U8) : Bool :=
  decide (0 < part.length) && !(part.all (fun b => decide (b = 46#u8)) && decide (part.length ≤ 2))

/-- Declarative component traversal, with the current component retained as bytes.
No generated function or machine counter is used in this specification. -/
def tail (part : List U8) : List U8 → Bool
  | [] => component part
  | b :: rest =>
    if b = 0#u8 ∨ b = 92#u8 then false
    else if b = 47#u8 then component part && tail [] rest
    else tail (part ++ [b]) rest

def ValidPath (bytes : List U8) : Prop :=
  0 < bytes.length ∧ bytes.length ≤ 4096 ∧ tail [] bytes = true

instance (bytes : List U8) : Decidable (ValidPath bytes) :=
  inferInstanceAs (Decidable (0 < bytes.length ∧ bytes.length ≤ 4096 ∧ tail [] bytes = true))

end SandboxModel.Stage1Protocol.ManifestPath
