import Aeneas

namespace SandboxModel.Lifecycle.PausePublication

/-- One property instantiated for both real generated implementations.
A successful publication requires its exact current Stop acknowledgement. -/
def RequiresStop {State Input : Type}
    (publishes acknowledged : State → Input → Prop) : Prop :=
  ∀ state input, publishes state input → acknowledged state input

end SandboxModel.Lifecycle.PausePublication
