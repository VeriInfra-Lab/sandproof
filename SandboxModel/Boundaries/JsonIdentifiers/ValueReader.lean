import SandboxModel.Boundaries.JsonIdentifiers.Calls
import SandboxModel.Boundaries.JsonCounter.Refinement

open Aeneas Aeneas.Std SandproofFoundationsRust
open SandboxModel.Foundations
open IdentifierLibrary
namespace SandboxModel.Boundaries.JsonIdentifiers

-- A constant value reader witnessing the generic contract, not JSON parsing.
def valueReader (text : String) : serde_core.de.Deserializer U64 Unit :=
  { SandboxModel.Boundaries.JsonCounter.Examples.valueReader with
    deserialize_string := fun {_ _} visitor _ state =>
      visitor.visit_string SandboxModel.Boundaries.JsonCounter.Examples.unitError state text }


end SandboxModel.Boundaries.JsonIdentifiers
