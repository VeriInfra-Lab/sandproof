import SandboxModel.Local.Runtime.Model
import SandproofNetwork.Funs

namespace SandboxModel.Local.Runtime
open Aeneas Aeneas.Std SandproofNetworkRust SandboxModel.Foundations

def view (world : sandbox_controller_core.model.World) : Focused Bool sandbox_controller_core.model.World :=
  ⟨world.controller_up, { world with controller_up := false }⟩

def event (input : Input) : sandbox_controller_core.recovery_monitor.RuntimeEvent :=
  match input with | .crash => .ControllerCrash | .recover => .ControllerRecover

def rejectView : Reject → sandbox_controller_core.recovery_monitor.RuntimeReject
  | .alreadyDown => .ControllerAlreadyDown | .alreadyUp => .ControllerAlreadyUp

def execute (world : sandbox_controller_core.model.World) (input : Input) :=
  match input with
  | .crash => SandproofNetworkRust.runtime_crash world
  | .recover => SandproofNetworkRust.runtime_recover world

def ResultRep (input : Input) (model : Except Reject (Focused Bool sandbox_controller_core.model.World))
    (actual : Result (core.result.Result sandbox_controller_core.recovery_monitor.RuntimeDecision
      sandbox_controller_core.recovery_monitor.RuntimeReject)) : Prop :=
  match model with
  | .error rejection => actual = .ok (.Err (rejectView rejection))
  | .ok after => ∃ decision, actual = .ok (.Ok decision) ∧ decision.event = event input ∧
      view decision.next = after

end SandboxModel.Local.Runtime
