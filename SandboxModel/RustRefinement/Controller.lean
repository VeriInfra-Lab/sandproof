import SandboxModel.RustRefinement.Runtime
import SandboxModel.RustRefinement.Auxiliary
import SandboxModel.RustRefinement.LifecycleRequest
import SandboxModel.RustRefinement.EffectHandler
import SandboxModel.RustRefinement.EffectFinalize
import SandboxModel.RustRefinement.EffectRetry
import SandboxModel.RustRefinement.EffectRetryAdmission
import SandboxModel.RustRefinement.EffectProtocol
import SandboxModel.RustRefinement.RpcTimeout
import SandboxModel.RustRefinement.WorkerHandler
import SandboxModel.UnifiedController

/-!
Composition layer for the single public Rust reducer.  Component files prove
the substantive state transformations; this file proves that the generated
`controller::handle_controller` dispatches to those components without
changing their event, rejection, or successor.

Runtime, all requests, access, dispatch, and timeout arms are connected
below, together with the stale and unqueued worker prefixes.  The remaining
arms still require complete component refinements; defining the common result
relation does not discharge those obligations.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance controllerOperationKeyDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance controllerLogicalEffectIdDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance controllerEffectIdDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

abbrev ControllerWorldRep :=
  @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
    effectIdDecidableEq valueRelations

/-- Erase the Rust-only protocol-domain tag while translating the observable
    event into the authoritative model event. -/
def controllerEvent : SandboxControllerRust.controller.ControllerEvent → Event
  | .Protocol event => modelEvent event
  | .Worker event => workerEvent event
  | .Publication event => publicationEvent event
  | .Auxiliary event => auxiliaryEvent event
  | .Runtime event => runtimeEvent event

/-- Machine exhaustion has no corresponding unbounded model rejection.
    Individual complete handler theorems must rule it out explicitly. -/
def controllerReject? : SandboxControllerRust.controller.ControllerReject →
    Option SandboxModel.UnifiedController.Reject
  | .Request reason => (reject? reason).map .request
  | .Effect reason => (effectReject? reason).map .effect
  | .Scheduler reason => (schedulerReject? reason).map .scheduler
  | .Worker reason => some (.worker (workerReject reason))
  | .Publication reason => some (.publication (publicationReject reason))
  | .Auxiliary reason => some (.auxiliary (auxiliaryReject reason))
  | .Runtime reason => some (.runtime (runtimeReject reason))

/-- One result relation for all public controller inputs.  This definition
    alone is not a coverage theorem: every input still needs its own proof. -/
def ControllerResultRep (model : World)
    (rustResult : core.result.Result
      SandboxControllerRust.controller.ControllerDecision
      SandboxControllerRust.controller.ControllerReject)
    (modelResult : Except SandboxModel.UnifiedController.Reject
      (SandboxModel.UnifiedController.Decision model)) : Prop :=
  match rustResult, modelResult with
  | .Err rustReject, .error modelReject =>
      controllerReject? rustReject = some modelReject
  | .Ok rustDecision, .ok modelDecision =>
      controllerEvent rustDecision.event = modelDecision.event ∧
        ControllerWorldRep rustDecision.next modelDecision.next
  | _, _ => False

/-- Result relation specialized to the already-closed runtime arm of the
    public controller.  Other rejection tags are deliberately false here, so
    the theorem also checks correct outer tagging. -/
def ControllerRuntimeResultRep (model : World)
    (rustResult : core.result.Result
      SandboxControllerRust.controller.ControllerDecision
      SandboxControllerRust.controller.ControllerReject)
    (modelResult : Except SandboxModel.UnifiedController.Reject
      (SandboxModel.UnifiedController.Decision model)) : Prop :=
  match rustResult, modelResult with
  | .Err (.Runtime rustReject), .error (.runtime modelReject) =>
      runtimeReject rustReject = modelReject
  | .Ok rustDecision, .ok modelDecision =>
      controllerEvent rustDecision.event = modelDecision.event ∧
        ControllerWorldRep rustDecision.next modelDecision.next
  | _, _ => False

/-- The actual Aeneas-generated public reducer refines the single model-side
    reducer for every runtime input, including both successful and rejected
    executions. -/
theorem handleController_runtime_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model)
    (valid : ResourceIndexValid model)
    (input : SandboxControllerRust.recovery_monitor.RuntimeInput) :
    ∃ rustResult,
      SandboxControllerRust.controller.handle_controller rust
          (.Runtime input) = ok rustResult ∧
      ControllerRuntimeResultRep model rustResult
        (SandboxModel.UnifiedController.handle model valid
          (.runtime (runtimeInput input))) := by
  rcases handleRuntime_refines rust model represented input with
    ⟨runtimeResult, runtimeEvaluates, related⟩
  cases runtimeResult with
  | Err rustReject =>
      cases modelEvaluation : SandboxModel.RuntimeController.handle model
          (runtimeInput input) with
      | error modelReject =>
          have rejectSame : runtimeReject rustReject = modelReject := by
            simpa [RuntimeResultRep, modelEvaluation] using related
          refine ⟨.Err (.Runtime rustReject), ?_, ?_⟩
          · simp [SandboxControllerRust.controller.handle_controller,
              runtimeEvaluates]
          · simp [ControllerRuntimeResultRep,
              SandboxModel.UnifiedController.handle,
              SandboxModel.UnifiedController.liftRuntime, modelEvaluation,
              rejectSame]
      | ok modelDecision =>
          simp [RuntimeResultRep, modelEvaluation] at related
  | Ok rustDecision =>
      cases modelEvaluation : SandboxModel.RuntimeController.handle model
          (runtimeInput input) with
      | error modelReject =>
          simp [RuntimeResultRep, modelEvaluation] at related
      | ok modelDecision =>
          have success : runtimeEvent rustDecision.event = modelDecision.event ∧
              ControllerWorldRep rustDecision.next modelDecision.next := by
            simpa [RuntimeResultRep, modelEvaluation] using related
          refine ⟨.Ok
            { event := .Runtime rustDecision.event
              next := rustDecision.next }, ?_, ?_⟩
          · simp [SandboxControllerRust.controller.handle_controller,
              runtimeEvaluates,
              SandboxControllerRust.controller.runtime_decision]
          · simpa [ControllerRuntimeResultRep, controllerEvent,
              SandboxModel.UnifiedController.handle,
              SandboxModel.UnifiedController.liftRuntime, modelEvaluation,
              SandboxModel.UnifiedController.fromRuntimeDecision] using success

/-- Composition only: the caller must supply a proved component refinement.
    This lemma cannot establish coverage for an unproved request branch. -/
private theorem handleController_request_lift
    (rust : SandboxControllerRust.model.World) (model : World)
    (valid : ResourceIndexValid model)
    (input : SandboxControllerRust.request.Request)
    (component : ∃ result, SandboxControllerRust.request.handle rust input = ok result ∧
      RequestResultRep model result (SandboxModel.Controller.handle model (requestInput input))) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Request input) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.request (requestInput input))) := by
  obtain ⟨result, evaluated, related⟩ := component
  cases result with
  | Err reason =>
      cases abstractEval : SandboxModel.Controller.handle model (requestInput input) with
      | error abstractReason =>
          have same : reject? reason = some abstractReason := by
            simpa [RequestResultRep, abstractEval] using related
          refine ⟨.Err (.Request reason), ?_, ?_⟩
          · simp [SandboxControllerRust.controller.handle_controller, evaluated]
          · simp [ControllerResultRep, controllerReject?, SandboxModel.UnifiedController.handle,
              SandboxModel.UnifiedController.liftController, abstractEval, same]
      | ok decision => simp [RequestResultRep, abstractEval] at related
  | Ok decision =>
      cases abstractEval : SandboxModel.Controller.handle model (requestInput input) with
      | error reason => simp [RequestResultRep, abstractEval] at related
      | ok abstractDecision =>
          have same : modelEvent decision.event = abstractDecision.event ∧
              ControllerWorldRep decision.next abstractDecision.next := by
            simpa [RequestResultRep, abstractEval] using related
          refine ⟨.Ok { event := .Protocol decision.event, next := decision.next }, ?_, ?_⟩
          · simp [SandboxControllerRust.controller.handle_controller, evaluated,
              SandboxControllerRust.controller.protocol_decision]
          · simpa [ControllerResultRep, controllerEvent, SandboxModel.UnifiedController.handle,
              SandboxModel.UnifiedController.liftController, abstractEval,
              SandboxModel.UnifiedController.fromControllerDecision] using same

/-- Complete request family, with command capacity explicit for each call. -/
theorem handleController_request_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust) (input : SandboxControllerRust.request.Request)
    (capacity : RequestCapacity rust input) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Request input) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.request (requestInput input))) := by
  apply handleController_request_lift
  exact handleRequest_refines rust model represented room input capacity

theorem handleController_replay_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (input : SandboxControllerRust.request.ReplayRequest) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Request (.Replay input)) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.request (requestInput (.Replay input)))) := by
  apply handleController_request_lift
  exact handleReplay_refines rust model represented input

theorem handleController_query_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (input : SandboxControllerRust.request.QueryRequest) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Request (.Query input)) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.request (requestInput (.Query input)))) := by
  apply handleController_request_lift
  exact handleQuery_refines rust model represented input

theorem handleController_access_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (authorityShape : AuthorityShape model)
    (input : SandboxControllerRust.auxiliary.AccessInput) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Access input) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.access (accessInput input))) := by
  obtain ⟨result, evaluated, related⟩ := handleAccess_refines rust model represented authorityShape input
  cases result with
  | Err reason =>
      cases abstractEval : SandboxModel.Controller.handleAccess model (accessInput input) with
      | error abstractReason =>
          have same : auxiliaryReject reason = abstractReason := by
            simpa [AccessResultRep, abstractEval] using related
          refine ⟨.Err (.Auxiliary reason), ?_, ?_⟩
          · simp [SandboxControllerRust.controller.handle_controller, evaluated]
          · simp [ControllerResultRep, controllerReject?, SandboxModel.UnifiedController.handle,
              SandboxModel.UnifiedController.liftController, abstractEval, same]
      | ok decision => simp [AccessResultRep, abstractEval] at related
  | Ok decision =>
      cases abstractEval : SandboxModel.Controller.handleAccess model (accessInput input) with
      | error reason => simp [AccessResultRep, abstractEval] at related
      | ok abstractDecision =>
          have same : auxiliaryEvent decision.event = abstractDecision.event ∧
              ControllerWorldRep decision.next abstractDecision.next := by
            simpa [AccessResultRep, abstractEval] using related
          refine ⟨.Ok { event := .Auxiliary decision.event, next := decision.next }, ?_, ?_⟩
          · simp [SandboxControllerRust.controller.handle_controller, evaluated,
              SandboxControllerRust.controller.auxiliary_decision]
          · simpa [ControllerResultRep, controllerEvent, SandboxModel.UnifiedController.handle,
              SandboxModel.UnifiedController.liftController, abstractEval,
              SandboxModel.UnifiedController.fromControllerDecision] using same

theorem handleController_rpcTimeout_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (sandboxRoom : StoreHeadroom rust.sandboxes)
    (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.RpcTimeout sid) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.rpcTimeout (sandboxId sid))) := by
  obtain ⟨result, evaluated, related⟩ :=
    handleRpcTimeout_refines rust model represented sandboxRoom operationRoom sid
  cases result with
  | Err reason =>
      cases abstractEval : SandboxModel.Controller.handleRpcTimeout model (sandboxId sid) with
      | error abstractReason =>
          have same : auxiliaryReject reason = abstractReason := by
            simpa [AccessResultRep, abstractEval] using related
          refine ⟨.Err (.Auxiliary reason), ?_, ?_⟩
          · simp [SandboxControllerRust.controller.handle_controller, evaluated]
          · simp [ControllerResultRep, controllerReject?, SandboxModel.UnifiedController.handle,
              SandboxModel.UnifiedController.liftController, abstractEval, same]
      | ok decision => simp [AccessResultRep, abstractEval] at related
  | Ok decision =>
      cases abstractEval : SandboxModel.Controller.handleRpcTimeout model (sandboxId sid) with
      | error reason => simp [AccessResultRep, abstractEval] at related
      | ok abstractDecision =>
          have same : auxiliaryEvent decision.event = abstractDecision.event ∧
              ControllerWorldRep decision.next abstractDecision.next := by
            simpa [AccessResultRep, abstractEval] using related
          refine ⟨.Ok { event := .Auxiliary decision.event, next := decision.next }, ?_, ?_⟩
          · simp [SandboxControllerRust.controller.handle_controller, evaluated,
              SandboxControllerRust.controller.auxiliary_decision]
          · simpa [ControllerResultRep, controllerEvent, SandboxModel.UnifiedController.handle,
              SandboxModel.UnifiedController.liftController, abstractEval,
              SandboxModel.UnifiedController.fromControllerDecision] using same

theorem handleController_worker_stale_rejected
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (id : SandboxControllerRust.model.EffectId)
    (stale : SandboxModel.currentEffectId model (effectId id).command ≠ effectId id) :
    SandboxControllerRust.controller.handle_controller rust (.Worker id) =
      ok (.Err (.Worker .StaleAttempt)) ∧
    ControllerResultRep model (.Err (.Worker .StaleAttempt))
      (SandboxModel.UnifiedController.handle model valid (.worker (.stale (effectId id) stale))) := by
  constructor
  · simp [SandboxControllerRust.controller.handle_controller,
      handleWorker_stale_rejected rust model represented id stale]
  · rfl

theorem handleController_worker_unqueued_rejected
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (id : SandboxControllerRust.model.EffectId)
    (head : SandboxModel.currentEffectId model (effectId id).command = effectId id)
    (absent : ¬ model.inflight (effectId id)) :
    SandboxControllerRust.controller.handle_controller rust (.Worker id) =
      ok (.Err (.Worker .NotQueued)) ∧
    ControllerResultRep model (.Err (.Worker .NotQueued))
      (SandboxModel.UnifiedController.handle model valid (.worker (.unqueued (effectId id) head absent))) := by
  constructor
  · simp [SandboxControllerRust.controller.handle_controller,
      handleWorker_unqueued_rejected rust model represented id head absent]
  · rfl

private theorem handleController_effect_lift
    (rust : SandboxControllerRust.model.World) (model : World)
    (valid : ResourceIndexValid model)
    (input : SandboxControllerRust.effect.EffectInput)
    (component : ∃ result, SandboxControllerRust.effect.handle_effect rust input = ok result ∧
      EffectResultRep model result (SandboxModel.Controller.handleEffect model (effectInput input))) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Effect input) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.effect (effectInput input))) := by
  obtain ⟨result, evaluated, related⟩ := component
  cases result with
  | Err reason =>
      cases abstractEval : SandboxModel.Controller.handleEffect model (effectInput input) with
      | error abstractReason =>
          have same : effectReject? reason = some abstractReason := by
            simpa [EffectResultRep, abstractEval] using related
          refine ⟨.Err (.Effect reason), ?_, ?_⟩
          · simp [SandboxControllerRust.controller.handle_controller, evaluated]
          · simp [ControllerResultRep, controllerReject?, SandboxModel.UnifiedController.handle,
              SandboxModel.UnifiedController.liftController, abstractEval, same]
      | ok decision => simp [EffectResultRep, abstractEval] at related
  | Ok decision =>
      cases abstractEval : SandboxModel.Controller.handleEffect model (effectInput input) with
      | error reason => simp [EffectResultRep, abstractEval] at related
      | ok abstractDecision =>
          have same : modelEvent decision.event = abstractDecision.event ∧
              ControllerWorldRep decision.next abstractDecision.next := by
            simpa [EffectResultRep, abstractEval] using related
          refine ⟨.Ok { event := .Protocol decision.event, next := decision.next }, ?_, ?_⟩
          · simp [SandboxControllerRust.controller.handle_controller, evaluated,
              SandboxControllerRust.controller.effect_decision]
          · simpa [ControllerResultRep, controllerEvent, SandboxModel.UnifiedController.handle,
              SandboxModel.UnifiedController.liftController, abstractEval,
              SandboxModel.UnifiedController.fromControllerDecision] using same

theorem handleController_effectDispatch_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (headroom : HandlerMachineHeadroom rust)
    (complete : SandboxModel.Controller.EffectKeyIndexComplete model)
    (id : SandboxControllerRust.model.EffectId) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Effect (.Dispatch id)) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.effect (.dispatch (effectId id)))) := by
  apply handleController_effect_lift
  exact handleEffectDispatch_refines rust model represented headroom complete id

theorem handleController_effectTimeout_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (headroom : HandlerMachineHeadroom rust)
    (complete : SandboxModel.Controller.EffectKeyIndexComplete model)
    (id : SandboxControllerRust.model.EffectId) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Effect (.Timeout id)) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.effect (.timeout (effectId id)))) := by
  apply handleController_effect_lift
  exact handleEffectTimeout_refines rust model represented headroom complete id

theorem controllerRuntimeResultRep_implies_resultRep
    (related : ControllerRuntimeResultRep model rustResult modelResult) :
    ControllerResultRep model rustResult modelResult := by
  cases rustResult with
  | Err reason =>
      cases reason <;> cases modelResult <;>
        simp_all [ControllerRuntimeResultRep, ControllerResultRep, controllerReject?]
      all_goals rename_i reject
      all_goals cases reject <;>
        simp_all [ControllerRuntimeResultRep, ControllerResultRep, controllerReject?]
  | Ok decision =>
      cases modelResult <;> simpa [ControllerRuntimeResultRep, ControllerResultRep] using related

theorem handleController_effectFinalize_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (headroom : StoreHeadroom rust.logical_effects)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (failure : SandboxControllerRust.model.FailureKind) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust
        (.Effect (.FinalizeFailure logical failure)) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid
          (.effect (.finalizeFailure (logicalEffectId logical) (failureKind failure)))) := by
  apply handleController_effect_lift
  exact handleEffectFinalize_refines rust model represented headroom logical failure

theorem handleController_effectRetry_accepted_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (record : SandboxControllerRust.model.LogicalEffectRecord)
    (failure : SandboxControllerRust.model.FailureKind)
    (up : rust.controller_up = true)
    (found : storeLookup rust.logical_effects logical = some record)
    (unfinished : record.result = none)
    (failed : storeLookup rust.effects ⟨logical, record.current_attempt⟩ =
      some (.Acknowledged (.Failed failure)))
    (retryable : (failureKind failure).retryable = true)
    (budget : 0 < record.remaining_retries.val)
    (attemptRoom : U64Headroom record.current_attempt)
    (fresh : model.effects ⟨logicalEffectId logical, record.current_attempt.val + 1⟩ = none) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Effect (.Retry logical failure)) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.effect (.retry (logicalEffectId logical) (failureKind failure)))) := by
  apply handleController_effect_lift
  obtain ⟨decision, evaluated, related⟩ := handleEffectRetry_accepted_refines rust model represented room
    logical record failure up found unfinished failed retryable budget attemptRoom fresh
  exact ⟨.Ok decision, evaluated, related⟩

theorem handleController_effectRetry_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (failure : SandboxControllerRust.model.FailureKind) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Effect (.Retry logical failure)) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.effect (.retry (logicalEffectId logical) (failureKind failure)))) := by
  apply handleController_effect_lift
  exact handleEffectRetry_refines rust model represented room logical failure

theorem handleController_effectReceipt_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust)
    (id : SandboxControllerRust.model.EffectId)
    (outcome : SandboxControllerRust.model.EffectOutcome) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Effect (.Receipt id outcome)) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.effect (.receipt (effectId id) (effectOutcome outcome)))) := by
  apply handleController_effect_lift
  exact handleEffectReceipt_refines rust model represented room id outcome

theorem handleController_effect_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model) (valid : ResourceIndexValid model)
    (room : HandlerMachineHeadroom rust)
    (complete : SandboxModel.Controller.EffectKeyIndexComplete model)
    (input : SandboxControllerRust.effect.EffectInput) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust (.Effect input) = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model valid (.effect (effectInput input))) := by
  apply handleController_effect_lift
  exact handleEffect_refines rust model represented room complete input

/-- Transfer an already-established result correspondence to an actual legal
    model step and its complete successor representation.  No component
    correctness is assumed implicitly: it must supply `related`. -/
theorem controllerResultRep_accepted_is_legal
    (valid : ResourceIndexValid model)
    (input : SandboxModel.UnifiedController.Input model)
    (rustDecision : SandboxControllerRust.controller.ControllerDecision)
    (related : ControllerResultRep model (.Ok rustDecision)
      (SandboxModel.UnifiedController.handle model valid input)) :
    ∃ next, Step model (controllerEvent rustDecision.event) next ∧
      ControllerWorldRep rustDecision.next next := by
  cases evaluated : SandboxModel.UnifiedController.handle model valid input with
  | error reason => simp [ControllerResultRep, evaluated] at related
  | ok decision =>
      have same : controllerEvent rustDecision.event = decision.event ∧
          ControllerWorldRep rustDecision.next decision.next := by
        simpa [ControllerResultRep, evaluated] using related
      exact ⟨decision.next, same.1.symm ▸ decision.legal, same.2⟩

/-- Safety inheritance for any public input whose result correspondence has
    been proved.  This is deliberately conditional until all inputs are
    covered by the full controller refinement theorem. -/
theorem controllerResultRep_accepted_preserves_safe
    (safe : Safe model) (valid : ResourceIndexValid model)
    (input : SandboxModel.UnifiedController.Input model)
    (rustDecision : SandboxControllerRust.controller.ControllerDecision)
    (related : ControllerResultRep model (.Ok rustDecision)
      (SandboxModel.UnifiedController.handle model valid input)) :
    ∃ next, ControllerWorldRep rustDecision.next next ∧ Safe next ∧
      ProtocolStepSecurity model (controllerEvent rustDecision.event) next := by
  obtain ⟨next, legal, represented⟩ :=
    controllerResultRep_accepted_is_legal valid input rustDecision related
  exact ⟨next, represented, step_preserves_safe safe legal, step_protocol_secure legal⟩

end SandboxModel.RustRefinement
