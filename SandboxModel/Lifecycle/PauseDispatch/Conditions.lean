import SandboxModel.Lifecycle.PauseStop.Conditions

namespace SandboxModel.Lifecycle.PauseDispatch

/-- A full Stop input is shared by the independently extracted correct and
wrong implementations. It includes the attempt, without assuming it is zero. -/
abbrev StopInput := SandboxModel.Lifecycle.PauseStop.StopInput

/-- The same property is instantiated with each actual generated selector
and its exact event, effect-row and remaining-World observations. The binding
relation is a conclusion to prove, not an assumption on accepted inputs. -/
def PreservesBoundStop {State Decision : Type}
    (accepts bound : State → StopInput → Decision → Prop) : Prop :=
  ∀ state input decision, accepts state input decision → bound state input decision

end SandboxModel.Lifecycle.PauseDispatch
