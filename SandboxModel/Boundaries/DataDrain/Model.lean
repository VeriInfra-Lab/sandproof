import Std

namespace SandboxModel.Boundaries.DataDrain

/-- An ordered pair of call-local observations; it is not a stable heap snapshot. -/
structure Observation where
  strongCount : Nat
  unknown : Bool

def routesDropped (observation : Observation) : Bool :=
  decide (observation.strongCount = 1)

def drained (observation : Observation) : Bool :=
  routesDropped observation && !observation.unknown

/-- Safety of this read policy only; not a resource release certificate. -/
def ReadSafe (observation : Observation) (answer : Bool) : Prop :=
  answer = true → observation.strongCount = 1 ∧ observation.unknown = false

theorem policy_exact (observation : Observation) :
    drained observation = true ↔
      observation.strongCount = 1 ∧ observation.unknown = false := by
  simp [drained, routesDropped]

theorem policy_safe (observation : Observation) : ReadSafe observation (drained observation) :=
  (policy_exact observation).mp

theorem policy_nonempty : drained ⟨1, false⟩ = true := rfl

end SandboxModel.Boundaries.DataDrain
