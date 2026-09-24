import SandboxModel.WorkerController
import SandboxModel.AuxController
import SandboxModel.RuntimeController

/-!
One model-side entry point for every operation accepted by the Rust pure
controller.  The individual reducers remain small proof units; this module
gives the system a single typed boundary and prevents a deployment wrapper
from choosing a state-changing helper outside that boundary.

The worker case makes transport membership explicit.  Rust decides finite
vector membership at runtime; the model records either the corresponding
membership proof or its negation.  This is why an unqueued delivery is a
normal tagged rejection instead of an unmodelled precondition.
-/

namespace SandboxModel.UnifiedController

open SandboxModel.Controller

inductive WorkerEnvelope (world : World) where
  | queued (input : SandboxModel.Controller.WorkerInput world)
  /-- Rust checks attempt freshness before transport membership. -/
  | stale (effect : EffectId)
      (notHead : currentEffectId world effect.command ≠ effect)
  | unqueued (effect : EffectId)
      (head : currentEffectId world effect.command = effect)
      (absent : ¬ world.inflight effect)

inductive Input (world : World) where
  | request (input : SandboxModel.Controller.Request)
  | effect (input : SandboxModel.Controller.EffectInput)
  | scheduler (input : SandboxModel.Controller.SchedulerInput)
  | worker (input : WorkerEnvelope world)
  | publication (input : SandboxModel.Controller.PublicationInput)
  | rpcTimeout (sandbox : SandboxId)
  | access (input : SandboxModel.Controller.AccessInput)
  | runtime (input : SandboxModel.RuntimeController.Input)

inductive Reject where
  | request (reason : SandboxModel.Controller.Reject)
  | effect (reason : SandboxModel.Controller.EffectReject)
  | scheduler (reason : SandboxModel.Controller.SchedulerReject)
  | worker (reason : SandboxModel.Controller.WorkerReject)
  | publication (reason : SandboxModel.Controller.PublicationReject)
  | auxiliary (reason : SandboxModel.Controller.AuxiliaryReject)
  | runtime (reason : SandboxModel.RuntimeController.Reject)
  deriving DecidableEq, Repr

structure Decision (before : World) where
  event : Event
  next : World
  legal : Step before event next

def fromControllerDecision
    (decision : SandboxModel.Controller.Decision world) : Decision world :=
  ⟨decision.event, decision.next, decision.legal⟩

def fromRuntimeDecision
    (decision : SandboxModel.RuntimeController.Decision world) :
    Decision world :=
  ⟨decision.event, decision.next, decision.legal⟩

def liftController (tag : Error → Reject)
    (result : Except Error (SandboxModel.Controller.Decision world)) :
    Except Reject (Decision world) :=
  match result with
  | .error reason => .error (tag reason)
  | .ok decision => .ok (fromControllerDecision decision)

def liftRuntime
    (result : Except SandboxModel.RuntimeController.Reject
      (SandboxModel.RuntimeController.Decision world)) :
    Except Reject (Decision world) :=
  match result with
  | .error reason => .error (.runtime reason)
  | .ok decision => .ok (fromRuntimeDecision decision)

/-- The single executable model reducer.  `valid` is an erased proof of the
    invariant already established for every reachable world. -/
def handle (world : World) (valid : ResourceIndexValid world) :
    Input world → Except Reject (Decision world)
  | .request input =>
      liftController .request (SandboxModel.Controller.handle world input)
  | .effect input =>
      liftController .effect (SandboxModel.Controller.handleEffect world input)
  | .scheduler input =>
      liftController .scheduler
        (SandboxModel.Controller.handleScheduler world input)
  | .worker (.queued input) =>
      liftController .worker
        (SandboxModel.Controller.handleWorker world valid input)
  | .worker (.stale _ _) => .error (.worker .staleAttempt)
  | .worker (.unqueued _ _ _) => .error (.worker .notQueued)
  | .publication input =>
      liftController .publication
        (SandboxModel.Controller.handlePublication world valid input)
  | .rpcTimeout sandbox =>
      liftController .auxiliary
        (SandboxModel.Controller.handleRpcTimeout world sandbox)
  | .access input =>
      liftController .auxiliary
        (SandboxModel.Controller.handleAccess world input)
  | .runtime input =>
      liftRuntime (SandboxModel.RuntimeController.handle world input)

/-- The unified wrapper cannot manufacture a successor: every success still
    carries a witness of the authoritative `Step` relation. -/
theorem accepted_is_legal
    (_accepted : handle world valid input = .ok decision) :
    Step world decision.event decision.next :=
  decision.legal

theorem accepted_preserves_safe
    (safe : Safe world)
    (accepted : handle world valid input = .ok decision) :
    Safe decision.next :=
  step_preserves_safe safe (accepted_is_legal accepted)

theorem accepted_is_protocol_secure
    (accepted : handle world valid input = .ok decision) :
    ProtocolStepSecurity world decision.event decision.next :=
  step_protocol_secure (accepted_is_legal accepted)

theorem accepted_preserves_resourceIndexValid
    (accepted : handle world valid input = .ok decision) :
    ResourceIndexValid decision.next :=
  step_preserves_resourceIndexValid valid (accepted_is_legal accepted)

/-- The explicit unqueued branch is rejected before any worker reducer is
    called. -/
@[simp] theorem handle_worker_unqueued
    (effect : EffectId) (head : currentEffectId world effect.command = effect)
    (absent : ¬ world.inflight effect) :
    handle world valid (.worker (.unqueued effect head absent)) =
      .error (.worker .notQueued) := rfl

@[simp] theorem handle_worker_stale
    (effect : EffectId)
    (notHead : currentEffectId world effect.command ≠ effect) :
    handle world valid (.worker (.stale effect notHead)) =
      .error (.worker .staleAttempt) := rfl

/-- Component successes are preserved verbatim by the unified boundary. -/
theorem handle_request_success
    (accepted : SandboxModel.Controller.handle world input = .ok decision) :
    handle world valid (.request input) =
      .ok (fromControllerDecision decision) := by
  simp [handle, liftController, accepted]

theorem handle_effect_success
    (accepted : SandboxModel.Controller.handleEffect world input =
      .ok decision) :
    handle world valid (.effect input) =
      .ok (fromControllerDecision decision) := by
  simp [handle, liftController, accepted]

theorem handle_scheduler_success
    (accepted : SandboxModel.Controller.handleScheduler world input =
      .ok decision) :
    handle world valid (.scheduler input) =
      .ok (fromControllerDecision decision) := by
  simp [handle, liftController, accepted]

theorem handle_worker_success
    (accepted : SandboxModel.Controller.handleWorker world valid input =
      .ok decision) :
    handle world valid (.worker (.queued input)) =
      .ok (fromControllerDecision decision) := by
  simp [handle, liftController, accepted]

theorem handle_publication_success
    (accepted : SandboxModel.Controller.handlePublication world valid input =
      .ok decision) :
    handle world valid (.publication input) =
      .ok (fromControllerDecision decision) := by
  simp [handle, liftController, accepted]

theorem handle_rpcTimeout_success
    (accepted : SandboxModel.Controller.handleRpcTimeout world sandbox =
      .ok decision) :
    handle world valid (.rpcTimeout sandbox) =
      .ok (fromControllerDecision decision) := by
  simp [handle, liftController, accepted]

theorem handle_access_success
    (accepted : SandboxModel.Controller.handleAccess world input =
      .ok decision) :
    handle world valid (.access input) =
      .ok (fromControllerDecision decision) := by
  simp [handle, liftController, accepted]

theorem handle_runtime_success
    (accepted : SandboxModel.RuntimeController.handle world input =
      .ok decision) :
    handle world valid (.runtime input) =
      .ok (fromRuntimeDecision decision) := by
  simp [handle, liftRuntime, accepted]

end SandboxModel.UnifiedController
