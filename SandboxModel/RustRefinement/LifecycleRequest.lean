import SandboxModel.RustRefinement.LifecycleAdmission
import SandboxModel.RustRefinement.LifecycleCompensation

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

/-- Additional queue headroom for the commands emitted by this request.
    This is required for each invocation, not assumed to hold forever. -/
def RequestCapacity (rust : SandboxControllerRust.model.World) : SandboxControllerRust.request.Request → Prop
  | .Pause _ => EffectCapacity rust 2
  | .Create _ | .Resume _ | .Kill _ | .Compensate _ => EffectCapacity rust 1
  | .Replay _ | .Query _ => True

/-- All request inputs, including every ordered rejection and full successors. -/
theorem handleRequest_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : RequestWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (input : SandboxControllerRust.request.Request)
    (capacity : RequestCapacity rust input) :
    ∃ result, SandboxControllerRust.request.handle rust input = ok result ∧
      RequestResultRep model result (SandboxModel.Controller.handle model (requestInput input)) := by
  cases input with
  | Create request => exact handleCreate_refines rust model rep room capacity request
  | Pause request => exact handlePause_refines rust model rep room capacity request
  | Resume request => exact handleResume_refines rust model rep room capacity request
  | Kill request => exact handleKill_refines rust model rep room capacity request
  | Replay request => exact handleReplay_refines rust model rep request
  | Query request => exact handleQuery_refines rust model rep request
  | Compensate request => exact handleCompensation_refines rust model rep room capacity request

end SandboxModel.RustRefinement
