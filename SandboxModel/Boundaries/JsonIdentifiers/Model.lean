import SandboxModel.Foundations.IdentifierModel

open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
open IdentifierLibrary
namespace SandboxModel.Boundaries.JsonIdentifiers

def IdentifierSafe {E : Type} (result : Result (core.result.Result String E)) : Prop :=
  ∀ value, result = .ok (.Ok value) → opaqueValid value


end SandboxModel.Boundaries.JsonIdentifiers
