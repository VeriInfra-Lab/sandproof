import Aeneas

namespace SandboxModel.Lifecycle.PauseStop
open Aeneas.Std

/-- The same full command and attempt for the real correct and wrong versions. -/
structure StopInput where
  sandbox : U64
  epoch : U64
  generation : U64
  vm : U64
  attempt : U64

/-- One property instantiated with the actual generated acceptance relation
    and the actual stored handoff in each independently generated program. -/
def RequiresPermission {State Input : Type}
    (stops hasPermission : State → Input → Prop) : Prop :=
  ∀ state input, stops state input → hasPermission state input

end SandboxModel.Lifecycle.PauseStop
