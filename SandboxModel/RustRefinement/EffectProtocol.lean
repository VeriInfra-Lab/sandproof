import SandboxModel.RustRefinement.EffectFinalize
import SandboxModel.RustRefinement.EffectRetryAdmission
import SandboxModel.RustRefinement.EffectReceipt

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

/-- Complete generated effect reducer: all five input constructors and all
their outcomes, within the explicitly stated finite-machine boundary. -/
theorem handleEffect_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (room : HandlerMachineHeadroom rust)
    (complete : SandboxModel.Controller.EffectKeyIndexComplete model)
    (input : SandboxControllerRust.effect.EffectInput) :
    ∃ result, SandboxControllerRust.effect.handle_effect rust input = ok result ∧
      EffectResultRep model result (SandboxModel.Controller.handleEffect model (effectInput input)) := by
  cases input with
  | Dispatch id => exact handleEffectDispatch_refines rust model represented room complete id
  | Timeout id => exact handleEffectTimeout_refines rust model represented room complete id
  | Receipt id outcome => exact handleEffectReceipt_refines rust model represented room id outcome
  | Retry id failure => exact handleEffectRetry_refines rust model represented room id failure
  | FinalizeFailure id failure => exact handleEffectFinalize_refines rust model represented room.logicalEffects id failure

end SandboxModel.RustRefinement
