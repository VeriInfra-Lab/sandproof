import SandboxModel.RustRefinement.PublicationRunning
import SandboxModel.RustRefinement.ControllerPublication

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

/-- All four publication inputs, including every ordered rejection and the full successor. -/
theorem handlePublication_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : PublicationWorldRep rust model) (valid : ResourceIndexValid model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (slotRoom : StoreHeadroom rust.net_slots)
    (input : SandboxControllerRust.publication.PublicationInput) :
    ∃ result, SandboxControllerRust.publication.handle_publication rust input = ok result ∧
      PublicationResultRep model result (Controller.handlePublication model valid (publicationInput input)) := by
  cases input with
  | Running sid => exact handlePublishRunning_refines rust model rep valid sandboxRoom operationRoom slotRoom sid
  | Paused sid source => exact handlePublishPaused_refines rust model rep valid sandboxRoom operationRoom sid source
  | Destroyed sid => exact handlePublishDestroyed_refines rust model rep valid sandboxRoom operationRoom sid
  | Failed sid failure => exact handlePublishFailure_refines rust model rep valid sandboxRoom operationRoom sid failure

theorem handleController_publication_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (slotRoom : StoreHeadroom rust.net_slots)
    (input : SandboxControllerRust.publication.PublicationInput) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Publication input) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.publication (publicationInput input))) := by
  apply handleController_publication_lift
  exact handlePublication_refines rust model rep valid sandboxRoom operationRoom slotRoom input

theorem handleController_publishRunning_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (slotRoom : StoreHeadroom rust.net_slots) (sid : SandboxControllerRust.ids.SandboxId) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Publication (.Running sid)) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.publication (.running (sandboxId sid)))) := by
  exact handleController_publication_refines rust model rep valid sandboxRoom operationRoom slotRoom (.Running sid)

end SandboxModel.RustRefinement
