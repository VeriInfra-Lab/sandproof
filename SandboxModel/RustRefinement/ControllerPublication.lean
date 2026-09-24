import SandboxModel.RustRefinement.Controller
import SandboxModel.RustRefinement.PublicationFailure
import SandboxModel.RustRefinement.PublicationPaused

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

theorem handleController_publication_lift
    (rust : SandboxControllerRust.model.World) (model : World)
    (valid : ResourceIndexValid model)
    (input : SandboxControllerRust.publication.PublicationInput)
    (component : ∃ result, SandboxControllerRust.publication.handle_publication rust input = ok result ∧
      PublicationResultRep model result (Controller.handlePublication model valid (publicationInput input))) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Publication input) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.publication (publicationInput input))) := by
  obtain ⟨result, evaluated, related⟩ := component
  cases result with
  | Err reason =>
    cases abstractEval : Controller.handlePublication model valid (publicationInput input) with
    | error abstractReason =>
      have same : publicationReject reason = abstractReason := by
        simpa [PublicationResultRep, abstractEval] using related
      refine ⟨.Err (.Publication reason), ?_, ?_⟩
      · simp [SandboxControllerRust.controller.handle_controller, evaluated]
      · simp [ControllerResultRep, controllerReject?, UnifiedController.handle,
          UnifiedController.liftController, abstractEval, same]
    | ok decision => simp [PublicationResultRep, abstractEval] at related
  | Ok decision =>
    cases abstractEval : Controller.handlePublication model valid (publicationInput input) with
    | error reason => simp [PublicationResultRep, abstractEval] at related
    | ok abstractDecision =>
      have same : publicationEvent decision.event = abstractDecision.event ∧
          ControllerWorldRep decision.next abstractDecision.next := by
        simpa [PublicationResultRep, abstractEval] using related
      refine ⟨.Ok { event := .Publication decision.event, next := decision.next }, ?_, ?_⟩
      · simp [SandboxControllerRust.controller.handle_controller, evaluated,
          SandboxControllerRust.controller.publication_decision]
      · simpa [ControllerResultRep, controllerEvent, UnifiedController.handle,
          UnifiedController.liftController, abstractEval, UnifiedController.fromControllerDecision] using same

theorem handleController_publishDestroyed_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Publication (.Destroyed sid)) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.publication (.destroyed (sandboxId sid)))) := by
  apply handleController_publication_lift
  exact handlePublishDestroyed_refines rust model rep valid sandboxRoom operationRoom sid

theorem handleController_publishFailure_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId) (failure : SandboxControllerRust.model.FailureKind) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Publication (.Failed sid failure)) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.publication (.failed (sandboxId sid) (failureKind failure)))) := by
  apply handleController_publication_lift
  exact handlePublishFailure_refines rust model rep valid sandboxRoom operationRoom sid failure

theorem handleController_publishPaused_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId) (source : SandboxControllerRust.model.RestoreSource) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Publication (.Paused sid source)) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.publication (.paused (sandboxId sid) (restoreSource source)))) := by
  apply handleController_publication_lift
  exact handlePublishPaused_refines rust model rep valid sandboxRoom operationRoom sid source

end SandboxModel.RustRefinement
