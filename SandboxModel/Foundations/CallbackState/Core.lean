import SandproofCallbackState.Funs
import SandboxModel.Foundations.CallbackState.Contracts

namespace SandboxModel.Foundations.CallbackState
open Aeneas Aeneas.Std

theorem toggle_core_contract : ToggleContract SandproofCallbackStateRust.toggle_step := by
  intro state key value
  cases state <;> rfl

theorem trace_core_contract : TraceContract SandproofCallbackStateRust.trace_step := by
  intro visited key value floor capacity
  unfold SandproofCallbackStateRust.trace_step
  step as ⟨next, nextPost⟩
  constructor
  · rfl
  · exact nextPost

end SandboxModel.Foundations.CallbackState
