import SandboxModel.Local.State.Model
import SandboxModel.Foundations.Model

/-! A full current-state runtime specification. This is pure flag switching
    and persisted-effect observation, not process or database crash recovery. -/
namespace SandboxModel.Local.CurrentRuntime
open SandboxModel.Local.State

def transition (world : model.World) (input : recovery_monitor.RuntimeInput) :
    Except recovery_monitor.RuntimeReject recovery_monitor.RuntimeDecision :=
  match input with
  | .Crash =>
      if world.controller_up then
        .ok ⟨.ControllerCrash, {world with controller_up := false}⟩
      else .error .ControllerAlreadyDown
  | .Recover =>
      if world.controller_up then .error .ControllerAlreadyUp
      else .ok ⟨.ControllerRecover, {world with controller_up := true}⟩
  | .DuplicateOrLateDelivery effect =>
      if (SandboxModel.Foundations.tableLookup effect world.effects.entries).isSome then
        .ok ⟨.DuplicateOrLateDelivery effect, world⟩
      else .error .EffectNotFound

end SandboxModel.Local.CurrentRuntime
