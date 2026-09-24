import SandboxModel.RustRefinement.HandlerValues
import SandboxModel.RustRefinement.Clone
import SandboxModel.SchedulerCompleteness

/-! Refinement boundary for stable-state publication.  Publication is where
the controller exposes Running, Paused, Destroyed, or Failed as durable truth;
the result relation therefore always compares the complete successor world. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance publicationHandlerOperationKeyDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance publicationHandlerLogicalEffectIdDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance publicationHandlerEffectIdDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

abbrev PublicationWorldRep :=
  @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
    effectIdDecidableEq valueRelations

def PublicationResultRep (model : World)
    (rustResult : core.result.Result
      SandboxControllerRust.publication.PublicationDecision
      SandboxControllerRust.publication.PublicationReject)
    (modelResult : Except SandboxModel.Controller.PublicationReject
      (SandboxModel.Controller.Decision model)) : Prop :=
  match rustResult, modelResult with
  | .Err rustReject, .error modelReject =>
      publicationReject rustReject = modelReject
  | .Ok rustDecision, .ok modelDecision =>
      publicationEvent rustDecision.event = modelDecision.event ∧
        PublicationWorldRep rustDecision.next modelDecision.next
  | _, _ => False

theorem generated_handlePublication_controllerDown
    (world : SandboxControllerRust.model.World)
    (input : SandboxControllerRust.publication.PublicationInput)
    (down : world.controller_up = false) :
    SandboxControllerRust.publication.handle_publication world input =
      ok (core.result.Result.Err
        SandboxControllerRust.publication.PublicationReject.ControllerDown) := by
  cases input <;>
    simp [SandboxControllerRust.publication.handle_publication,
      SandboxControllerRust.publication.handle_publish_running,
      SandboxControllerRust.publication.handle_publish_paused,
      SandboxControllerRust.publication.handle_publish_destroyed,
      SandboxControllerRust.publication.handle_publish_failure, down]

theorem model_handlePublication_controllerDown
    (world : World) (valid : ResourceIndexValid world)
    (input : SandboxModel.Controller.PublicationInput)
    (down : world.controllerUp = false) :
    SandboxModel.Controller.handlePublication world valid input =
      .error .controllerDown := by
  cases input <;>
    simp [SandboxModel.Controller.handlePublication,
      SandboxModel.Controller.handlePublishRunning,
      SandboxModel.Controller.handlePublishPaused,
      SandboxModel.Controller.handlePublishDestroyed,
      SandboxModel.Controller.handlePublishFailure, down]

/-- Stable-state publication is fail-closed at the common first guard in both
    executable implementations. -/
theorem handlePublication_controllerDown_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : PublicationWorldRep rust model)
    (valid : ResourceIndexValid model)
    (input : SandboxControllerRust.publication.PublicationInput)
    (down : rust.controller_up = false) :
    SandboxControllerRust.publication.handle_publication rust input =
        ok (core.result.Result.Err
          SandboxControllerRust.publication.PublicationReject.ControllerDown) ∧
      SandboxModel.Controller.handlePublication model valid
          (publicationInput input) = .error .controllerDown := by
  have modelDown : model.controllerUp = false := by
    rw [← represented.controllerUp]
    exact down
  exact ⟨generated_handlePublication_controllerDown rust input down,
    model_handlePublication_controllerDown model valid
      (publicationInput input) modelDown⟩

end SandboxModel.RustRefinement
