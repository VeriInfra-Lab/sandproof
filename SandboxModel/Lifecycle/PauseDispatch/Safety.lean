import SandboxModel.Lifecycle.PauseDispatch.Conditions
import SandboxModel.Lifecycle.PauseDispatch.Frames

namespace SandboxModel.Lifecycle.PauseDispatch
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

noncomputable section
local instance : DecidableEq model.EffectId := Classical.decEq _

def actualAccepts (world : model.World) (input : StopInput) (decision : effect.EffectDecision) : Prop :=
  effect.dispatch_stop world (stopId input.sandbox input.epoch input.generation input.vm input.attempt) =
    .ok (some (.Ok decision))

def bound (world : model.World) (input : StopInput) (decision : effect.EffectDecision) : Prop :=
  decision.event = .DispatchEffect (stopId input.sandbox input.epoch input.generation input.vm input.attempt) ∧
  decision.next = {world with effects := decision.next.effects} ∧
  tableLookup (stopId input.sandbox input.epoch input.generation input.vm input.attempt)
    decision.next.effects.entries.val = some .Dispatched

theorem same_property_accepts : PreservesBoundStop actualAccepts bound := by
  intro world input decision accepted
  exact selected_stop_bound world input.sandbox input.epoch input.generation input.vm input.attempt decision accepted

end

end SandboxModel.Lifecycle.PauseDispatch
