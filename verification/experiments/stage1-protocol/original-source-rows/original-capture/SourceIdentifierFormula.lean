import Aeneas
import SandboxModel.Foundations.IdentifierModel

open Aeneas Aeneas.Std
namespace SourceIdentifierFormula

-- Exact public identifier formula; no generated World/Types or business Funs.
def check (value : Str) : Result Bool :=
  .ok (decide (SandboxModel.Foundations.opaqueBytesValid value.val))

end SourceIdentifierFormula
