import SandboxModel.Local.Runtime.Representation
import SandboxModel.Access.Cloning

namespace SandboxModel.Local.Runtime
open Aeneas Aeneas.Std SandproofNetworkRust SandboxModel.Foundations

theorem crash_exact (world : sandbox_controller_core.model.World) :
    SandproofNetworkRust.runtime_crash world =
      .ok (if world.controller_up then .Ok ⟨.ControllerCrash, {world with controller_up := false}⟩
        else .Err .ControllerAlreadyDown) := by
  simp [SandproofNetworkRust.runtime_crash,
    sandbox_controller_core.recovery_monitor.handle_controller_crash,
    SandboxModel.Access.model_World_clone_exact]
  split <;> rfl

theorem recover_exact (world : sandbox_controller_core.model.World) :
    SandproofNetworkRust.runtime_recover world =
      .ok (if world.controller_up then .Err .ControllerAlreadyUp
        else .Ok ⟨.ControllerRecover, {world with controller_up := true}⟩) := by
  simp [SandproofNetworkRust.runtime_recover,
    sandbox_controller_core.recovery_monitor.handle_controller_recover,
    SandboxModel.Access.model_World_clone_exact]
  split <;> rfl

theorem update_view (world : sandbox_controller_core.model.World) (up : Bool) :
    view {world with controller_up := up} = (view world).set up := by rfl

theorem roots_refine (world : sandbox_controller_core.model.World) (input : Input) :
    ResultRep input (transition (view world) input) (execute world input) := by
  cases input <;> cases flag : world.controller_up <;>
    simp [transition, Focused.checked, check, view, flag, execute, ResultRep,
      rejectView, crash_exact, recover_exact, event, Focused.set]

theorem execute_frame (world : sandbox_controller_core.model.World) (input : Input)
    (decision : sandbox_controller_core.recovery_monitor.RuntimeDecision)
    (accepted : execute world input = .ok (.Ok decision)) :
    (view decision.next).rest = (view world).rest := by
  cases input <;> cases flag : world.controller_up <;>
    simp [execute, crash_exact, recover_exact, flag] at accepted
  all_goals cases accepted; rfl

theorem execute_preserves_rest_invariant (invariant : sandbox_controller_core.model.World → Prop)
    (world : sandbox_controller_core.model.World) (input : Input)
    (decision : sandbox_controller_core.recovery_monitor.RuntimeDecision)
    (holds : invariant (view world).rest)
    (accepted : execute world input = .ok (.Ok decision)) : invariant (view decision.next).rest := by
  rw [execute_frame world input decision accepted]
  exact holds

end SandboxModel.Local.Runtime
