import SandboxModel.RustRefinement.CoveredExecution

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

/-- Total enumeration of the actual public Rust input type. No command
families are omitted; replay/query retain their legacy redundant aliases. -/
def CoveredInput.ofRust : SandboxControllerRust.controller.ControllerInput → CoveredInput
  | .Request input => .request input
  | .Effect (.Dispatch id) => .dispatch id
  | .Effect (.Timeout id) => .timeout id
  | .Effect (.Receipt id outcome) => .receipt id outcome
  | .Effect (.Retry id failure) => .retry id failure
  | .Effect (.FinalizeFailure id failure) => .finalizeFailure id failure
  | .Scheduler input => .scheduler input
  | .Worker id => .worker id
  | .Publication (.Running sid) => .publishRunning sid
  | .Publication (.Paused sid source) => .publishPaused sid source
  | .Publication (.Destroyed sid) => .publishDestroyed sid
  | .Publication (.Failed sid failure) => .publishFailure sid failure
  | .RpcTimeout sid => .rpcTimeout sid
  | .Access input => .access input
  | .Runtime input => .runtime input

@[simp] theorem CoveredInput.ofRust_toRust (input : SandboxControllerRust.controller.ControllerInput) :
    (CoveredInput.ofRust input).toRust = input := by
  cases input with
  | Effect input => cases input <;> rfl
  | Publication input => cases input <;> rfl
  | _ => rfl

def ControllerCapacity (rust : SandboxControllerRust.model.World)
    (input : SandboxControllerRust.controller.ControllerInput) : Prop :=
  (CoveredInput.ofRust input).capacity rust

noncomputable def controllerInput (model : World)
    (input : SandboxControllerRust.controller.ControllerInput) : UnifiedController.Input model :=
  (CoveredInput.ofRust input).toModel model

/-- Every current Rust controller input, including ordered rejections,
with finite-machine bounds explicit and semantic invariants supplied by
model reachability. This theorem says nothing about external I/O or liveness. -/
theorem handleController_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (reachable : Reachable model)
    (room : HandlerMachineHeadroom rust)
    (input : SandboxControllerRust.controller.ControllerInput)
    (capacity : ControllerCapacity rust input) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust input = ok result ∧
      ControllerResultRep model result (UnifiedController.handle model
        (reachable_resourceIndexValid reachable) (controllerInput model input)) := by
  simpa only [CoveredInput.ofRust_toRust, controllerInput] using
    handleCovered_refines rust model rep reachable room (CoveredInput.ofRust input) capacity

/-- A finite pure-reducer run over the real public input type. Each step
explicitly assumes machine headroom; rejection keeps the entire input state.
There is no implemented persistence or scheduling loop hidden in this relation. -/
inductive BoundedControllerExecution : SandboxControllerRust.model.World →
    List SandboxControllerRust.controller.ControllerInput →
    SandboxControllerRust.model.World → List Event → Prop where
  | nil (rust) : BoundedControllerExecution rust [] rust []
  | rejected {before after input inputs reason events}
      (room : HandlerMachineHeadroom before)
      (capacity : ControllerCapacity before input)
      (evaluated : SandboxControllerRust.controller.handle_controller before input = ok (.Err reason))
      (rest : BoundedControllerExecution before inputs after events) :
      BoundedControllerExecution before (input :: inputs) after events
  | accepted {before after input inputs decision events}
      (room : HandlerMachineHeadroom before)
      (capacity : ControllerCapacity before input)
      (evaluated : SandboxControllerRust.controller.handle_controller before input = ok (.Ok decision))
      (rest : BoundedControllerExecution decision.next inputs after events) :
      BoundedControllerExecution before (input :: inputs) after (controllerEvent decision.event :: events)

theorem controller_execution_toCovered
    (execution : BoundedControllerExecution before inputs after events) :
    BoundedCoveredExecution before (inputs.map CoveredInput.ofRust) after events := by
  induction execution with
  | nil rust => exact .nil rust
  | rejected room capacity evaluated _ ih =>
      exact .rejected room capacity (by simpa only [CoveredInput.ofRust_toRust] using evaluated) ih
  | accepted room capacity evaluated _ ih =>
      exact .accepted room capacity (by simpa only [CoveredInput.ofRust_toRust] using evaluated) ih

theorem controller_execution_refines
    (execution : BoundedControllerExecution before inputs after events)
    (rep : ControllerWorldRep before model) (reachable : Reachable model) :
    ∃ next, ControllerWorldRep after next ∧ Reachable next ∧ ModelEventTrace model events next :=
  covered_execution_refines (controller_execution_toCovered execution) rep reachable

theorem controller_execution_preserves_safe
    (execution : BoundedControllerExecution before inputs after events)
    (rep : ControllerWorldRep before model) (reachable : Reachable model) :
    ∃ next, ControllerWorldRep after next ∧ Safe next ∧ ModelEventTrace model events next :=
  covered_execution_preserves_safe (controller_execution_toCovered execution) rep reachable

end SandboxModel.RustRefinement
