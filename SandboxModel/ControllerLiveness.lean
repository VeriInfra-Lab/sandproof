import SandboxModel.Liveness
import SandboxModel.WorkerController
import SandboxModel.PlanProofs

/-!
Connects the temporal liveness foundation to the actual executable reducers.

The theorem in this file deliberately separates scheduler fairness from
environmental progress.  Fairness can ensure that a continuously enabled
handler is run; it cannot manufacture a successful VM, storage, or network
response.  Those concrete progress facts appear as the rank-decrease
obligation of `FairCompletionContract`.
-/

namespace SandboxModel

open Liveness

/-! ### Durable operation-record persistence -/

/-- Once an operation ledger key exists, protocol steps may update its value
    but cannot delete the key. -/
def OperationRecordPersistent (before after : World) : Prop :=
  ∀ key oldRecord, before.operations key = some oldRecord →
    ∃ newRecord, after.operations key = some newRecord

private theorem operationRecordPersistent_of_operations_eq
    (same : after.operations = before.operations) :
    OperationRecordPersistent before after := by
  intro key oldRecord found
  exact ⟨oldRecord, by simpa [same] using found⟩

theorem OperationRecordPersistent.trans
    (first : OperationRecordPersistent before middle)
    (second : OperationRecordPersistent middle after) :
    OperationRecordPersistent before after := by
  intro key oldRecord found
  rcases first key oldRecord found with ⟨middleRecord, middleFound⟩
  exact second key middleRecord middleFound

private theorem operationRecordPersistent_updateOperation :
    OperationRecordPersistent world
      (updateOperation world sid operation result) := by
  intro key oldRecord found
  by_cases same : key = operationKey sid operation.id
  · subst key
    exact ⟨{ oldRecord with operation := operation, result := result },
      by simp [updateOperation, found]⟩
  · exact ⟨oldRecord, by simp [updateOperation, same, found]⟩

private theorem operationRecordPersistent_storeNewOperation :
    OperationRecordPersistent world
      (storeNewOperation world sid operation input) := by
  intro key oldRecord found
  by_cases same : key = operationKey sid operation.id
  · subst key
    exact ⟨⟨sid, operation, input, none⟩,
      by simp [storeNewOperation, setOperation, put]⟩
  · exact ⟨oldRecord,
      by simp [storeNewOperation, setOperation, put, same, found]⟩

private theorem operationRecordPersistent_enqueueStore
    (same : base.operations = before.operations) :
    OperationRecordPersistent before
      (enqueue (storeNewOperation base sid operation input) command) := by
  intro key oldRecord found
  have baseFound : base.operations key = some oldRecord := by
    simpa [same] using found
  rcases operationRecordPersistent_storeNewOperation
      (world := base) (sid := sid) (operation := operation) (input := input)
      key oldRecord baseFound with ⟨newRecord, stored⟩
  exact ⟨newRecord, by simpa using stored⟩

private theorem operationRecordPersistent_supersedeCurrentOperation :
    OperationRecordPersistent world
      (supersedeCurrentOperation world sid current) := by
  cases current with
  | none => exact operationRecordPersistent_of_operations_eq rfl
  | some operation =>
      cases status : operation.status with
      | pending =>
          simp only [supersedeCurrentOperation, status]
          cases found : world.operations (operationKey sid operation.id) with
          | none => exact operationRecordPersistent_of_operations_eq rfl
          | some ledger =>
              cases final : ledger.result with
              | none =>
                  by_cases sameOperation : ledger.operation = operation
                  · simpa [final, sameOperation] using
                      (operationRecordPersistent_updateOperation
                        (world := world) (sid := sid)
                        (operation := { operation with
                          status := .failed .cancelled })
                        (result := some (.failed .cancelled)))
                  · simpa [final, sameOperation] using
                      (operationRecordPersistent_of_operations_eq
                        (before := world) (after := world) rfl)
              | some result =>
                  simpa [final] using
                    (operationRecordPersistent_of_operations_eq
                      (before := world) (after := world) rfl)
      | outcomeUnknown =>
          simp only [supersedeCurrentOperation, status]
          cases found : world.operations (operationKey sid operation.id) with
          | none => exact operationRecordPersistent_of_operations_eq rfl
          | some ledger =>
              cases final : ledger.result with
              | none =>
                  by_cases sameOperation : ledger.operation = operation
                  · simpa [final, sameOperation] using
                      (operationRecordPersistent_updateOperation
                        (world := world) (sid := sid)
                        (operation := { operation with
                          status := .failed .cancelled })
                        (result := some (.failed .cancelled)))
                  · simpa [final, sameOperation] using
                      (operationRecordPersistent_of_operations_eq
                        (before := world) (after := world) rfl)
              | some result =>
                  simpa [final] using
                    (operationRecordPersistent_of_operations_eq
                      (before := world) (after := world) rfl)
      | compensating failure =>
          simpa [supersedeCurrentOperation, status] using
            (operationRecordPersistent_of_operations_eq
              (before := world) (after := world) rfl)
      | succeeded =>
          simpa [supersedeCurrentOperation, status] using
            (operationRecordPersistent_of_operations_eq
              (before := world) (after := world) rfl)
      | failed failure =>
          simpa [supersedeCurrentOperation, status] using
            (operationRecordPersistent_of_operations_eq
              (before := world) (after := world) rfl)

private theorem operationRecordPersistent_beginKill :
    OperationRecordPersistent world
      (beginKill world sid record operationId) := by
  unfold beginKill
  apply OperationRecordPersistent.trans
    (operationRecordPersistent_supersedeCurrentOperation
      (world := world) (sid := sid) (current := record.currentOperation))
  exact operationRecordPersistent_enqueueStore (by simp)

theorem step_operationRecordPersistent
    (transition : Step world event next) :
    OperationRecordPersistent world next := by
  cases transition
  case acceptKill =>
    exact operationRecordPersistent_beginKill
  all_goals
    simp_all [OperationRecordPersistent, beginCreate, beginPause, beginResume,
      beginCompensation, storeNewOperation, updateOperation,
      updateCurrentOperation, publishRunning,
      publishPaused, publishDestroyed, publishFailure, markOutcomeUnknown,
      setOperation, setSandbox, setFilesystem, enqueue, put] <;>
    grind

theorem traceEdge_operationRecordPersistent
    (edge : TraceEdge before action after) :
    OperationRecordPersistent before after := by
  cases edge with
  | idle =>
      intro key oldRecord found
      exact ⟨oldRecord, found⟩
  | protocol transition =>
      exact step_operationRecordPersistent transition

/-- An accepted operation ledger entry cannot disappear at a later trace
    position, even before it obtains a result. -/
theorem operationRecord_persistent_on_trace
    (trace : Trace) (time : Nat)
    (found : (trace.state time).operations key = some record) :
    ∀ later, time ≤ later →
      ∃ laterRecord, (trace.state later).operations key = some laterRecord := by
  intro later ordered
  rcases Nat.le.dest ordered with ⟨distance, rfl⟩
  induction distance with
  | zero => exact ⟨record, by simpa using found⟩
  | succ distance inductionHypothesis =>
      rcases inductionHypothesis (by omega) with
        ⟨currentRecord, currentFound⟩
      have persistent :=
        traceEdge_operationRecordPersistent (trace.edge (time + distance))
      rcases persistent key currentRecord currentFound with
        ⟨nextRecord, nextFound⟩
      exact ⟨nextRecord, by simpa only [Nat.add_succ] using nextFound⟩

/-- A finalized ledger result remains finalized across one clock tick,
    including idle ticks. -/
theorem traceEdge_finalized
    (edge : TraceEdge before action after)
    (finalized : Finalized key before) : Finalized key after := by
  rcases finalized with ⟨oldLedger, result, oldAt, oldResult⟩
  rcases oldAt with ⟨oldFound, oldSandbox, oldId⟩
  cases edge with
  | idle =>
      exact ⟨oldLedger, result,
        ⟨oldFound, oldSandbox, oldId⟩, oldResult⟩
  | protocol transition =>
      rcases step_operationRecordPersistent transition key oldLedger oldFound with
        ⟨newLedger, newFound⟩
      have resultStable := step_operationResultStable transition key oldLedger
        newLedger oldFound (by simp [oldResult]) newFound
      have identityStable := step_operationIdentityStable transition key oldLedger
        newLedger oldFound newFound
      exact ⟨newLedger, result,
        ⟨newFound, oldSandbox ▸ identityStable.1.symm,
          oldId ▸ identityStable.2.1.symm⟩,
        by simpa [oldResult] using resultStable⟩

/-- Finalization is not a transient observation: after it first appears it
    holds at every later point in the same trace. -/
theorem finalized_stable_on_trace
    (trace : Trace) (time : Nat)
    (finalized : Finalized key (trace.state time)) :
    ∀ later, time ≤ later → Finalized key (trace.state later) := by
  intro later ordered
  rcases Nat.le.dest ordered with ⟨distance, rfl⟩
  induction distance with
  | zero => simpa using finalized
  | succ distance inductionHypothesis =>
      have previous := inductionHypothesis (by omega)
      have advanced := traceEdge_finalized (trace.edge (time + distance))
        previous
      simpa only [Nat.add_succ] using advanced

/-! ### Concrete executable work -/

/-- Every class of work owned by the modeled controller.  Worker execution is
    keyed by the exact effect attempt, never merely by its command. -/
inductive WorkItem where
  | request (input : Controller.Request)
  /-- Failure compensation is selected by the controller after a durable
      terminal effect result; it is not an external client request. -/
  | compensation (sid : SandboxId) (failure : FailureKind)
      (evidence : Option EffectId)
  | effect (input : Controller.EffectInput)
  | scheduler (input : Controller.SchedulerInput)
  | publication (input : Controller.PublicationInput)
  | worker (effect : EffectId)

/-- Running a concrete reducer in `before` accepts exactly `event` and
    produces exactly `after`.  The resource-index and transport-membership
    witnesses are explicit because they are invariants/evidence rather than
    freely decidable external facts. -/
def WorkAccepted (before : World) : WorkItem → Event → World → Prop
  | .request input, event, after =>
      ∃ decision, Controller.handle before input = .ok decision ∧
        decision.event = event ∧ decision.next = after
  | .compensation sid failure evidence, event, after =>
      ∃ decision,
        Controller.handleCompensation before sid failure evidence = .ok decision ∧
        decision.event = event ∧ decision.next = after
  | .effect input, event, after =>
      ∃ decision, Controller.handleEffect before input = .ok decision ∧
        decision.event = event ∧ decision.next = after
  | .scheduler input, event, after =>
      ∃ decision, Controller.handleScheduler before input = .ok decision ∧
        decision.event = event ∧ decision.next = after
  | .publication input, event, after =>
      ∃ valid : ResourceIndexValid before, ∃ decision,
        Controller.handlePublication before valid input = .ok decision ∧
        decision.event = event ∧ decision.next = after
  | .worker effect, event, after =>
      ∃ valid : ResourceIndexValid before,
        ∃ queued : before.inflight effect, ∃ decision,
        Controller.handleWorker before valid ⟨effect, queued⟩ = .ok decision ∧
        decision.event = event ∧ decision.next = after

def WorkEnabled (work : WorkItem) (world : World) : Prop :=
  ∃ event next, WorkAccepted world work event next

theorem workEnabled_compensation_iff (sid : SandboxId)
    (failure : FailureKind) (evidence : Option EffectId) :
    WorkEnabled (.compensation sid failure evidence) world ↔
      ∃ decision,
        Controller.handleCompensation world sid failure evidence = .ok decision := by
  constructor
  · rintro ⟨event, next, decision, accepted, rfl, rfl⟩
    exact ⟨decision, accepted⟩
  · rintro ⟨decision, accepted⟩
    exact ⟨decision.event, decision.next, decision, accepted, rfl, rfl⟩

def WorkServedAt (trace : Trace) (work : WorkItem) (time : Nat) : Prop :=
  ∃ event,
    EventAt trace time event ∧
    WorkAccepted (trace.state time) work event (trace.state (time + 1))

/-- Work whose invocation is owned by the controller scheduler.  Requests,
    adapter receipts, time observations, and worker execution are external
    inputs; controller fairness must not silently promise that they occur. -/
def ControllerScheduledWork : WorkItem → Prop
  | .compensation _ _ _
  | .effect (.dispatch _) | .effect (.retry _ _)
  | .effect (.finalizeFailure _ _) | .scheduler _ | .publication _ => True
  | .request _ | .effect (.receipt _ _) | .effect (.timeout _) | .worker _ => False

@[simp] theorem compensation_controllerScheduled (sid : SandboxId)
    (failure : FailureKind) (evidence : Option EffectId) :
    ControllerScheduledWork (.compensation sid failure evidence) := by
  simp [ControllerScheduledWork]

theorem accepted_compensation_is_scheduled_work
    (sid : SandboxId) (failure : FailureKind) (evidence : Option EffectId)
    (accepted : Controller.handleCompensation world sid failure evidence =
      .ok decision) :
    ControllerScheduledWork (.compensation sid failure evidence) ∧
      WorkAccepted world (.compensation sid failure evidence)
        decision.event decision.next := by
  exact ⟨compensation_controllerScheduled sid failure evidence,
    ⟨decision, accepted, rfl, rfl⟩⟩

theorem accepted_compensation_is_enabled
    (sid : SandboxId) (failure : FailureKind) (evidence : Option EffectId)
    (accepted : Controller.handleCompensation world sid failure evidence =
      .ok decision) :
    WorkEnabled (.compensation sid failure evidence) world := by
  exact workEnabled_compensation_iff sid failure evidence |>.2
    ⟨decision, accepted⟩

/-- Weak fairness only for work actually owned by the controller.  External
    worker/adapter progress is supplied by a separate deployment contract. -/
def HandlerWeaklyFair (trace : Trace) : Prop :=
  ∀ work, ControllerScheduledWork work → ∀ start,
    AlwaysFrom trace start (WorkEnabled work) →
      ∃ time, start ≤ time ∧ WorkServedAt trace work time

theorem workAccepted_is_legal
    (accepted : WorkAccepted before work event after) :
    Step before event after := by
  cases work with
  | request input =>
      rcases accepted with ⟨decision, _, rfl, rfl⟩
      exact decision.legal
  | compensation sid failure evidence =>
      rcases accepted with ⟨decision, _, rfl, rfl⟩
      exact decision.legal
  | effect input =>
      rcases accepted with ⟨decision, _, rfl, rfl⟩
      exact decision.legal
  | scheduler input =>
      rcases accepted with ⟨decision, _, rfl, rfl⟩
      exact decision.legal
  | publication input =>
      rcases accepted with ⟨valid, decision, _, rfl, rfl⟩
      exact decision.legal
  | worker effect =>
      rcases accepted with ⟨valid, queued, decision, _, rfl, rfl⟩
      exact decision.legal

/-! ### Fair completion theorem -/

/-- The non-circular progress contract for one operation.

`nextWork` says that every unfinished state has a concrete reducer invocation
which stays enabled until completion.  `rankDrops` says that executing that
invocation consumes finite work.  Adapter response, retry exhaustion, and
safety-cleanup convergence are discharged here by the caller; they are not
mislabelled as consequences of scheduler fairness. -/
structure FairCompletionContract (trace : Trace) (key : OperationKey)
    (start : Nat) where
  rank : World → Nat
  fair : HandlerWeaklyFair trace
  finalRank : ∀ world, Finalized key world → rank world = 0
  unfinishedRank : ∀ world, ¬ Finalized key world → 0 < rank world
  nextWork : ∀ time, start ≤ time → ¬ Finalized key (trace.state time) →
    ∃ work,
      ControllerScheduledWork work ∧
      (∀ later, time ≤ later → ¬ Finalized key (trace.state later) →
        WorkEnabled work (trace.state later)) ∧
      (∀ later, time ≤ later → ¬ Finalized key (trace.state later) →
        WorkServedAt trace work later →
        rank (trace.state (later + 1)) < rank (trace.state time))

theorem strictRankProgress_of_fairHandlers
    (contract : FairCompletionContract trace key start) :
    StrictRankProgress trace start (Finalized key) contract.rank := by
  intro time startLe unfinished
  by_cases reached : EventuallyFrom trace time (Finalized key)
  · rcases reached with ⟨later, timeLe, finalized⟩
    have different : later ≠ time := by
      intro same
      subst later
      exact unfinished finalized
    have laterTime : time < later := by omega
    have zero := contract.finalRank (trace.state later) finalized
    have positive := contract.unfinishedRank (trace.state time) unfinished
    exact ⟨later, laterTime, by omega⟩
  · rcases contract.nextWork time startLe unfinished with
      ⟨work, scheduled, staysEnabled, rankDrops⟩
    have neverFinished : ∀ later, time ≤ later →
        ¬ Finalized key (trace.state later) := by
      intro later timeLeLater finalized
      exact reached ⟨later, timeLeLater, finalized⟩
    have continuouslyEnabled :
        AlwaysFrom trace time (WorkEnabled work) := by
      intro later timeLeLater
      exact staysEnabled later timeLeLater
        (neverFinished later timeLeLater)
    rcases contract.fair work scheduled time continuouslyEnabled with
      ⟨servedAt, timeLeServed, served⟩
    exact ⟨servedAt + 1, by omega,
      rankDrops servedAt timeLeServed
        (neverFinished servedAt timeLeServed) served⟩

/-- Under the explicit finite-rank progress contract and weak fairness of the
    executable handlers, the operation eventually obtains a durable terminal
    result (success or explicit failure). -/
theorem operation_eventually_finalized_under_fair_handlers
    (contract : FairCompletionContract trace key start) :
    EventuallyFrom trace start (Finalized key) :=
  operation_eventually_finalized_of_rank
    (strictRankProgress_of_fairHandlers contract)

/-- Once the preceding theorem reaches a result, that result remains present
    at every later state. -/
theorem operation_eventually_always_finalized_under_fair_handlers
    (contract : FairCompletionContract trace key start) :
    EventuallyAlwaysFrom trace start (Finalized key) := by
  rcases operation_eventually_finalized_under_fair_handlers contract with
    ⟨time, startLe, finalized⟩
  exact ⟨time, startLe, finalized_stable_on_trace trace time finalized⟩

end SandboxModel
