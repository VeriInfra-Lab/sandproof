import SandboxModel.RustRefinement.PublicationComplete
import SandboxModel.RustRefinement.SchedulerComplete
import SandboxModel.RustRefinement.WorkerComplete
import SandboxModel.RustRefinement.Initialization

/-! Composition of every existing controller input, including all eleven
worker command families. Every invocation retains its own explicit
finite-machine capacity premise; no I/O or liveness claim is made here. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

/-- Inputs with complete component proofs, including their rejection paths. -/
inductive CoveredInput where
  | request (input : SandboxControllerRust.request.Request)
  | replay (input : SandboxControllerRust.request.ReplayRequest)
  | query (input : SandboxControllerRust.request.QueryRequest)
  | access (input : SandboxControllerRust.auxiliary.AccessInput)
  | rpcTimeout (sid : SandboxControllerRust.ids.SandboxId)
  | dispatch (id : SandboxControllerRust.model.EffectId)
  | timeout (id : SandboxControllerRust.model.EffectId)
  | receipt (id : SandboxControllerRust.model.EffectId)
      (outcome : SandboxControllerRust.model.EffectOutcome)
  | retry (id : SandboxControllerRust.model.LogicalEffectId)
      (failure : SandboxControllerRust.model.FailureKind)
  | finalizeFailure (id : SandboxControllerRust.model.LogicalEffectId)
      (failure : SandboxControllerRust.model.FailureKind)
  | publishRunning (sid : SandboxControllerRust.ids.SandboxId)
  | publishPaused (sid : SandboxControllerRust.ids.SandboxId) (source : SandboxControllerRust.model.RestoreSource)
  | publishDestroyed (sid : SandboxControllerRust.ids.SandboxId)
  | publishFailure (sid : SandboxControllerRust.ids.SandboxId) (failure : SandboxControllerRust.model.FailureKind)
  | runtime (input : SandboxControllerRust.recovery_monitor.RuntimeInput)
  | scheduler (input : SandboxControllerRust.scheduler.SchedulerInput)
  | worker (id : SandboxControllerRust.model.EffectId)

def CoveredInput.toRust : CoveredInput → SandboxControllerRust.controller.ControllerInput
  | .request input => .Request input
  | .replay input => .Request (.Replay input)
  | .query input => .Request (.Query input)
  | .access input => .Access input
  | .rpcTimeout sid => .RpcTimeout sid
  | .dispatch id => .Effect (.Dispatch id)
  | .timeout id => .Effect (.Timeout id)
  | .receipt id outcome => .Effect (.Receipt id outcome)
  | .retry id failure => .Effect (.Retry id failure)
  | .finalizeFailure id failure => .Effect (.FinalizeFailure id failure)
  | .publishRunning sid => .Publication (.Running sid)
  | .publishPaused sid source => .Publication (.Paused sid source)
  | .publishDestroyed sid => .Publication (.Destroyed sid)
  | .publishFailure sid failure => .Publication (.Failed sid failure)
  | .runtime input => .Runtime input
  | .scheduler input => .Scheduler input
  | .worker id => .Worker id

noncomputable def CoveredInput.toModel (model : World) : CoveredInput → SandboxModel.UnifiedController.Input model
  | .request input => .request (requestInput input)
  | .replay input => .request (requestInput (.Replay input))
  | .query input => .request (requestInput (.Query input))
  | .access input => .access (accessInput input)
  | .rpcTimeout sid => .rpcTimeout (sandboxId sid)
  | .dispatch id => .effect (.dispatch (effectId id))
  | .timeout id => .effect (.timeout (effectId id))
  | .receipt id outcome => .effect (.receipt (effectId id) (effectOutcome outcome))
  | .retry id failure => .effect (.retry (logicalEffectId id) (failureKind failure))
  | .finalizeFailure id failure => .effect (.finalizeFailure (logicalEffectId id) (failureKind failure))
  | .publishRunning sid => .publication (.running (sandboxId sid))
  | .publishPaused sid source => .publication (.paused (sandboxId sid) (restoreSource source))
  | .publishDestroyed sid => .publication (.destroyed (sandboxId sid))
  | .publishFailure sid failure => .publication (.failed (sandboxId sid) (failureKind failure))
  | .runtime input => .runtime (runtimeInput input)
  | .scheduler input => .scheduler (schedulerInput input)
  | .worker id => .worker (workerEnvelope model id)

/-- Input-specific command and index headroom; old input families retain their
    existing machine premises. -/
def CoveredInput.capacity (rust : SandboxControllerRust.model.World) : CoveredInput → Prop
  | .request input => RequestCapacity rust input
  | .scheduler input => SchedulerCapacity rust input
  | .worker id => WorkerCapacity rust id
  | _ => True

/-- Reachability supplies the model invariants; they are not new assumptions
about the caller or external adapters.  Capacity remains explicit. -/
theorem handleCovered_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : ControllerWorldRep rust model)
    (reachable : Reachable model) (headroom : HandlerMachineHeadroom rust)
    (input : CoveredInput) (capacity : input.capacity rust) :
    ∃ result, SandboxControllerRust.controller.handle_controller rust input.toRust = ok result ∧
      ControllerResultRep model result
        (SandboxModel.UnifiedController.handle model (reachable_resourceIndexValid reachable)
          (input.toModel model)) := by
  have valid := reachable_resourceIndexValid reachable
  have complete := SandboxModel.Controller.reachable_effectKeyIndexComplete reachable
  cases input with
  | worker id => exact handleController_worker_refines rust model represented valid headroom id capacity
  | scheduler input => exact handleController_scheduler_refines rust model represented valid headroom input capacity
  | request request => exact handleController_request_refines rust model represented valid headroom request capacity
  | replay request => exact handleController_replay_refines rust model represented valid request
  | query request => exact handleController_query_refines rust model represented valid request
  | access request =>
      exact handleController_access_refines rust model represented valid
        (reachable_authority_shape reachable) request
  | rpcTimeout sid =>
      exact handleController_rpcTimeout_refines rust model represented valid
        headroom.sandboxes headroom.operations sid
  | dispatch id => exact handleController_effectDispatch_refines rust model represented valid headroom complete id
  | timeout id => exact handleController_effectTimeout_refines rust model represented valid headroom complete id
  | receipt id outcome => exact handleController_effectReceipt_refines rust model represented valid headroom id outcome
  | retry id failure => exact handleController_effectRetry_refines rust model represented valid headroom id failure
  | finalizeFailure id failure =>
      exact handleController_effectFinalize_refines rust model represented valid
        headroom.logicalEffects id failure
  | publishRunning sid =>
      exact handleController_publishRunning_refines rust model represented valid headroom.sandboxes headroom.operations headroom.netSlots sid
  | publishPaused sid source =>
      exact handleController_publishPaused_refines rust model represented valid headroom.sandboxes headroom.operations sid source
  | publishDestroyed sid =>
      exact handleController_publishDestroyed_refines rust model represented valid headroom.sandboxes headroom.operations sid
  | publishFailure sid failure =>
      exact handleController_publishFailure_refines rust model represented valid headroom.sandboxes headroom.operations sid failure
  | runtime request =>
      obtain ⟨result, evaluated, related⟩ := handleController_runtime_refines rust model represented valid request
      exact ⟨result, evaluated, controllerRuntimeResultRep_implies_resultRep related⟩

/-- A reducer invocation either adopts its accepted decision or keeps its
input world after rejection.  This models no database or process loop.
Only actual generated Rust evaluations occur in the premises. -/
inductive BoundedCoveredExecution : SandboxControllerRust.model.World → List CoveredInput →
    SandboxControllerRust.model.World → List Event → Prop where
  | nil (rust) : BoundedCoveredExecution rust [] rust []
  | rejected {before after input inputs reason events}
      (headroom : HandlerMachineHeadroom before)
      (capacity : input.capacity before)
      (evaluated : SandboxControllerRust.controller.handle_controller before input.toRust = ok (.Err reason))
      (rest : BoundedCoveredExecution before inputs after events) :
      BoundedCoveredExecution before (input :: inputs) after events
  | accepted {before after input inputs decision events}
      (headroom : HandlerMachineHeadroom before)
      (capacity : input.capacity before)
      (evaluated : SandboxControllerRust.controller.handle_controller before input.toRust = ok (.Ok decision))
      (rest : BoundedCoveredExecution decision.next inputs after events) :
      BoundedCoveredExecution before (input :: inputs) after (controllerEvent decision.event :: events)

/-- Exact accepted event sequence in the authoritative state machine. -/
inductive ModelEventTrace : World → List Event → World → Prop where
  | nil (model) : ModelEventTrace model [] model
  | cons (step : Step before event middle) (rest : ModelEventTrace middle events after) :
      ModelEventTrace before (event :: events) after

/-- Any finite mixed execution of the covered inputs has an exact legal model
trace and related final state.  No component-correctness premise is supplied
by the caller: it is discharged by `handleCovered_refines` at each step. -/
theorem covered_execution_refines
    (execution : BoundedCoveredExecution before inputs after events)
    (represented : ControllerWorldRep before model) (reachable : Reachable model) :
    ∃ next, ControllerWorldRep after next ∧ Reachable next ∧ ModelEventTrace model events next := by
  induction execution generalizing model with
  | nil rust => exact ⟨model, represented, reachable, .nil model⟩
  | rejected headroom capacity evaluated rest induction =>
      exact induction represented reachable
  | @accepted before after input inputs decision events headroom capacity evaluated rest induction =>
      obtain ⟨result, resultEval, related⟩ := handleCovered_refines before model represented reachable headroom input capacity
      have same : result = .Ok decision := by simpa [evaluated] using resultEval.symm
      subst result
      obtain ⟨middle, legal, nextRep⟩ := controllerResultRep_accepted_is_legal
        (reachable_resourceIndexValid reachable) (input.toModel model) decision related
      obtain ⟨next, finalRep, finalReachable, trace⟩ := induction nextRep (.step reachable legal)
      exact ⟨next, finalRep, finalReachable, .cons legal trace⟩

/-- End-state safety, including executions interleaving rejected calls. -/
theorem covered_execution_preserves_safe
    (execution : BoundedCoveredExecution before inputs after events)
    (represented : ControllerWorldRep before model) (reachable : Reachable model) :
    ∃ next, ControllerWorldRep after next ∧ Safe next ∧ ModelEventTrace model events next := by
  obtain ⟨next, finalRep, finalReachable, trace⟩ := covered_execution_refines execution represented reachable
  exact ⟨next, finalRep, reachable_safe finalReachable, trace⟩

end SandboxModel.RustRefinement
