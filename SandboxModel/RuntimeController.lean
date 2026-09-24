import SandboxModel.Step

/-!
Executable controller-runtime transitions.

`Step` remains the authoritative relational specification.  This reducer
covers the process-lifecycle transitions and duplicate-delivery observation
which do not belong to request admission, effect scheduling, publication, or
worker execution.  A successful decision carries its `Step` witness, while
the completeness theorems below show that every matching relational step is
accepted by the deterministic handler.
-/

namespace SandboxModel.RuntimeController

inductive Input where
  | crash
  | recover
  | duplicateOrLateDelivery (effect : EffectId)
  deriving DecidableEq, Repr

def Input.event : Input → Event
  | .crash => .controllerCrash
  | .recover => .controllerRecover
  | .duplicateOrLateDelivery effect => .duplicateOrLateDelivery effect

inductive Reject where
  | controllerAlreadyDown
  | controllerAlreadyUp
  | effectNotFound
  deriving DecidableEq, Repr

structure Decision (before : World) where
  event : Event
  next : World
  legal : Step before event next

def handleCrash (world : World) : Except Reject (Decision world) :=
  if up : world.controllerUp = true then
    .ok ⟨.controllerCrash, { world with controllerUp := false },
      Step.controllerCrash up⟩
  else .error .controllerAlreadyDown

def handleRecover (world : World) : Except Reject (Decision world) :=
  if down : world.controllerUp = false then
    .ok ⟨.controllerRecover, { world with controllerUp := true },
      Step.controllerRecover down⟩
  else .error .controllerAlreadyUp

def handleDuplicateOrLateDelivery (world : World) (effect : EffectId) :
    Except Reject (Decision world) :=
  match found : world.effects effect with
  | none => .error .effectNotFound
  | some _ =>
      .ok ⟨.duplicateOrLateDelivery effect, world,
        Step.duplicateOrLateDelivery effect found⟩

def handle (world : World) : Input → Except Reject (Decision world)
  | .crash => handleCrash world
  | .recover => handleRecover world
  | .duplicateOrLateDelivery effect =>
      handleDuplicateOrLateDelivery world effect

theorem accepted_is_legal
    (_accepted : handle world input = .ok decision) :
    Step world decision.event decision.next :=
  decision.legal

/-! ## Backward coverage -/

theorem handleCrash_complete
    (transition : Step world .controllerCrash next) :
    ∃ decision,
      handle world .crash = .ok decision ∧
      decision.event = .controllerCrash ∧
      decision.next = next := by
  cases transition with
  | controllerCrash up =>
      let decision : Decision world :=
        ⟨.controllerCrash, { world with controllerUp := false },
          Step.controllerCrash up⟩
      refine ⟨decision, ?_, rfl, rfl⟩
      simp [handle, handleCrash, up, decision]

theorem handleRecover_complete
    (transition : Step world .controllerRecover next) :
    ∃ decision,
      handle world .recover = .ok decision ∧
      decision.event = .controllerRecover ∧
      decision.next = next := by
  cases transition with
  | controllerRecover down =>
      let decision : Decision world :=
        ⟨.controllerRecover, { world with controllerUp := true },
          Step.controllerRecover down⟩
      refine ⟨decision, ?_, rfl, rfl⟩
      simp [handle, handleRecover, down, decision]

theorem handleDuplicateOrLateDelivery_complete
    (transition : Step world (.duplicateOrLateDelivery effect) next) :
    ∃ decision,
      handle world (.duplicateOrLateDelivery effect) = .ok decision ∧
      decision.event = .duplicateOrLateDelivery effect ∧
      decision.next = next := by
  cases transition with
  | duplicateOrLateDelivery effect persisted =>
      let decision : Decision world :=
        ⟨.duplicateOrLateDelivery effect, world,
          Step.duplicateOrLateDelivery effect persisted⟩
      refine ⟨decision, ?_, rfl, rfl⟩
      simp only [handle]
      unfold handleDuplicateOrLateDelivery
      split
      · rename_i missing
        simp [persisted] at missing
      · rfl

/-- Every relational step selected by a runtime input is represented by the
    deterministic handler with the same event and successor world. -/
theorem handle_complete
    (transition : Step world input.event next) :
    ∃ decision,
      handle world input = .ok decision ∧
      decision.event = input.event ∧
      decision.next = next := by
  cases input with
  | crash => exact handleCrash_complete transition
  | recover => exact handleRecover_complete transition
  | duplicateOrLateDelivery effect =>
      exact handleDuplicateOrLateDelivery_complete transition

end SandboxModel.RuntimeController
