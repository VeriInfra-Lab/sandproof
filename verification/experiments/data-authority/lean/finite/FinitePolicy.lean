import Aeneas

namespace LifetimeFiniteProofs
open Aeneas Aeneas.Std

/-- Same safety statement for independently extracted program variants. -/
def DeadlineSafety (check : I64 → Option L → Result (core.result.Result L E))
    (deadline : L → I64) (expired : E) : Prop :=
  ∀ lifetime, check (deadline lifetime) (some lifetime) = .ok (.Err expired)

end LifetimeFiniteProofs
