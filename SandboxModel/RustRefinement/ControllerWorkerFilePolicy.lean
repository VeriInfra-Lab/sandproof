import SandboxModel.RustRefinement.WorkerFilePolicy

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

/-- Proof-level input translation. The Rust membership decision is proved
against this envelope; no transport or membership fact is assumed by callers. -/
noncomputable def workerEnvelope (model : World) (id : SandboxControllerRust.model.EffectId) :
    UnifiedController.WorkerEnvelope model := by
  classical
  exact if head : currentEffectId model (effectId id).command = effectId id then
    if queued : model.inflight (effectId id) then .queued ⟨effectId id, queued⟩
    else .unqueued (effectId id) head queued
  else .stale (effectId id) head

theorem handleController_worker_queued_lift
    (rust : SandboxControllerRust.model.World) (model : World)
    (valid : ResourceIndexValid model) (id : SandboxControllerRust.model.EffectId)
    (queued : model.inflight (effectId id))
    (component : ∃ result, SandboxControllerRust.worker.handle_worker rust id = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId id, queued⟩)) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Worker id) = ok result ∧
      ControllerResultRep model result
        (UnifiedController.handle model valid (.worker (.queued ⟨effectId id, queued⟩))) := by
  obtain ⟨result, evaluated, related⟩ := component
  cases result with
  | Err reason =>
    cases abstractEval : Controller.handleWorker model valid ⟨effectId id, queued⟩ with
    | error abstractReason =>
      have same : workerReject reason = abstractReason := by
        simpa [WorkerResultRep, abstractEval] using related
      refine ⟨.Err (.Worker reason), ?_, ?_⟩
      · simp [SandboxControllerRust.controller.handle_controller, evaluated]
      · simp [ControllerResultRep, controllerReject?, UnifiedController.handle,
          UnifiedController.liftController, abstractEval, same]
    | ok decision => simp [WorkerResultRep, abstractEval] at related
  | Ok decision =>
    cases abstractEval : Controller.handleWorker model valid ⟨effectId id, queued⟩ with
    | error reason => simp [WorkerResultRep, abstractEval] at related
    | ok abstractDecision =>
      have same : workerEvent decision.event = abstractDecision.event ∧
          ControllerWorldRep decision.next abstractDecision.next := by
        simpa [WorkerResultRep, abstractEval] using related
      refine ⟨.Ok { event := .Worker decision.event, next := decision.next }, ?_, ?_⟩
      · simp [SandboxControllerRust.controller.handle_controller, evaluated,
          SandboxControllerRust.controller.worker_decision]
      · simpa [ControllerResultRep, controllerEvent, UnifiedController.handle,
          UnifiedController.liftController, abstractEval, UnifiedController.fromControllerDecision] using same

/-- Lift a queued component without assuming queue membership at the public
boundary. Old attempts are checked before the missing-queue rejection. -/
theorem handleController_worker_lift
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (id : SandboxControllerRust.model.EffectId)
    (component : ∀ queued : model.inflight (effectId id),
      ∃ result, SandboxControllerRust.worker.handle_worker rust id = ok result ∧
        WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId id, queued⟩)) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Worker id) = ok result ∧
      ControllerResultRep model result (UnifiedController.handle model valid (.worker (workerEnvelope model id))) := by
  classical
  by_cases head : currentEffectId model (effectId id).command = effectId id
  · by_cases queued : model.inflight (effectId id)
    · simpa only [workerEnvelope, dif_pos head, dif_pos queued] using
        handleController_worker_queued_lift rust model valid id queued (component queued)
    · obtain ⟨evaluated, related⟩ := handleController_worker_unqueued_rejected rust model rep valid id head queued
      exact ⟨.Err (.Worker .NotQueued), evaluated, by simpa only [workerEnvelope, dif_pos head, dif_neg queued] using related⟩
  · obtain ⟨evaluated, related⟩ := handleController_worker_stale_rejected rust model rep valid id head
    exact ⟨.Err (.Worker .StaleAttempt), evaluated, by simpa only [workerEnvelope, dif_neg head] using related⟩

theorem handleController_workerFilePolicy_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (rep : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : StoreHeadroom rust.applied_file_policies)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch generation : U64)
    (workspace : SandboxControllerRust.ids.FsId) (policy : SandboxControllerRust.model.FilePolicyRef)
    (attempt : U64) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust
        (.Worker ⟨⟨.ApplyFilePolicy sid epoch generation workspace policy⟩, attempt⟩) = ok result ∧
      ControllerResultRep model result (UnifiedController.handle model valid
        (.worker (workerEnvelope model ⟨⟨.ApplyFilePolicy sid epoch generation workspace policy⟩, attempt⟩))) := by
  apply handleController_worker_lift rust model rep valid
  intro queued
  exact handleWorker_filePolicy_refines rust model rep valid room sid epoch generation workspace policy attempt queued

end SandboxModel.RustRefinement
