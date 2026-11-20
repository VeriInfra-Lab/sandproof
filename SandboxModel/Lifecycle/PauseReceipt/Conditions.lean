import SandboxModel.Lifecycle.PauseDispatch.Conditions

namespace SandboxModel.Lifecycle.PauseReceipt

/-- The same property is used for the actual correct and actual wrong entry.
It restricts first unresolved receipts; identical replay is a separate result. -/
def RequiresOutcomeEvidence {State Input Outcome Decision : Type}
    (first : State → Input → Prop)
    (accepts : State → Input → Outcome → Decision → Prop)
    (matchingOutcome : State → Input → Outcome → Prop) : Prop :=
  ∀ world input outcome decision,
    first world input → accepts world input outcome decision → matchingOutcome world input outcome

end SandboxModel.Lifecycle.PauseReceipt
