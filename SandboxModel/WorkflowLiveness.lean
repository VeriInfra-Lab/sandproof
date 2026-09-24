import SandboxModel.ControllerLiveness
import SandboxModel.SchedulerCompleteness
import SandboxModel.AllocationProofs

/-!
Workflow-specific completion contracts.

`ControllerLiveness` proves the generic well-founded argument.  This module
supplies an executable, finite rank for each lifecycle workflow and separates
the environmental premises needed to close that argument:

* the controller eventually remains available;
* every dispatched current attempt eventually receives an explicit receipt;
* retry counters remain bounded by the accepted operation plan;
* terminal cleanup eventually reaches the publication guard; and
* the workflow-specific allocator/handler proof exposes persistent work which
  strictly lowers the concrete rank.

The last item is deliberately kept separate from adapter reliability.  It is
the controller proof obligation which must be discharged by an implementation;
it is not renamed as an environmental assumption.
-/

namespace SandboxModel

open Liveness

/-! ### Four workflow classes -/

inductive WorkflowKind where
  | create
  | pause
  | resume
  | kill
  deriving DecidableEq, Repr

def WorkflowKind.ofInput : OperationInput → WorkflowKind
  | .create _ _ _ => .create
  | .pause _ => .pause
  | .resume _ => .resume
  | .kill _ => .kill

/-- Number of normal-path milestones before stable publication.  Retry and
    resource cleanup debt are added separately below. -/
def WorkflowKind.normalMilestones : WorkflowKind → Nat
  | .create => 6
  | .pause => 7
  | .resume => 7
  | .kill => 1

def WorkflowKind.operationKind : WorkflowKind → OperationKind
  | .create => .create
  | .pause => .pause
  | .resume => .resume
  | .kill => .kill

/-- Pins a named workflow theorem to the durable request accepted at `start`.
    This prevents, for example, a kill operation from being presented as an
    instance of the create theorem merely by supplying an arbitrary closure. -/
def OperationAcceptedAs (world : World) (key : OperationKey)
    (workflow : WorkflowKind) : Prop :=
  ∃ ledger,
    LedgerAt world key ledger ∧
    ledger.operation.kind = workflow.operationKind ∧
    WorkflowKind.ofInput ledger.input = workflow

theorem traceEdge_operationAcceptedAs
    (edge : TraceEdge before action after)
    (accepted : OperationAcceptedAs before key workflow) :
    OperationAcceptedAs after key workflow := by
  rcases accepted with
    ⟨oldLedger, ⟨oldFound, oldSandbox, oldId⟩, oldKind, oldInput⟩
  cases edge with
  | idle =>
      exact ⟨oldLedger, ⟨oldFound, oldSandbox, oldId⟩, oldKind, oldInput⟩
  | protocol transition =>
      rcases step_operationRecordPersistent transition key oldLedger oldFound with
        ⟨newLedger, newFound⟩
      rcases step_operationIdentityStable transition key oldLedger newLedger
          oldFound newFound with
        ⟨sameSandbox, sameId, sameKind, _, sameInput⟩
      exact ⟨newLedger,
        ⟨newFound, sameSandbox.symm.trans oldSandbox,
          sameId.symm.trans oldId⟩,
        sameKind.symm.trans oldKind,
        by rw [← sameInput]; exact oldInput⟩

theorem operationAcceptedAs_persistent_on_trace
    (trace : Trace) (time : Nat)
    (accepted : OperationAcceptedAs (trace.state time) key workflow) :
    ∀ later, time ≤ later →
      OperationAcceptedAs (trace.state later) key workflow := by
  intro later ordered
  rcases Nat.le.dest ordered with ⟨distance, rfl⟩
  induction distance with
  | zero => simpa using accepted
  | succ distance inductionHypothesis =>
      have advanced := traceEdge_operationAcceptedAs
        (trace.edge (time + distance)) (inductionHypothesis (by omega))
      simpa only [Nat.add_succ] using advanced

/-! ### Executable finite debt calculation -/

def Bool.debt (ready : Bool) : Nat :=
  if ready then 0 else 1

def vmClosedCheck (world : World) (vmId : VmId) : Bool :=
  match world.vms vmId with
  | some vm => decide (vm.status = .stopped ∨ vm.status = .reclaimed)
  | none => false

def vmPreparedCheck (world : World) (vmId : VmId) : Bool :=
  match world.vms vmId with
  | some vm => decide (vm.status = .prepared ∨ vm.status = .executing ∨
      vm.status = .frozen ∨ vm.status = .stopped ∨
      vm.status = .reclaimed)
  | none => false

def vmExecutingCheck (world : World) (vmId : VmId) : Bool :=
  match world.vms vmId with
  | some vm => decide (vm.status = .executing)
  | none => false

def vmFrozenOrClosedCheck (world : World) (vmId : VmId) : Bool :=
  match world.vms vmId with
  | some vm => decide (vm.status = .frozen ∨ vm.status = .stopped ∨
      vm.status = .reclaimed)
  | none => false

def slotConfiguredCheck (world : World) (slotId : NetSlotId) : Bool :=
  match world.netSlots slotId with
  | some slot => slot.configured
  | none => false

def slotClosedCheck (world : World) (slotId : NetSlotId) : Bool :=
  match world.netSlots slotId with
  | some slot => !slot.active
  | none => false

/-- A reserved resource which was never materialized needs no cleanup.  These
    variants are used only for a sealed plan's cleanup domain; normal-path
    readiness continues to require that the selected resource exists. -/
def plannedVmClosedCheck (world : World) (vmId : VmId) : Bool :=
  match world.vms vmId with
  | some vm => decide (vm.status = .stopped ∨ vm.status = .reclaimed)
  | none => true

def plannedSlotClosedCheck (world : World) (slotId : NetSlotId) : Bool :=
  match world.netSlots slotId with
  | some slot => !slot.active
  | none => true

def snapshotPresentCheck (world : World) (source : RestoreSource) : Bool :=
  match world.snapshots source.snapshot with
  | some snapshot => decide (snapshot.manifest = source.manifest)
  | none => false

def snapshotDurableCheck (world : World) (source : RestoreSource) : Bool :=
  match world.snapshots source.snapshot with
  | some snapshot => decide (snapshot.manifest = source.manifest ∧
      snapshot.stage = .durable ∧ snapshot.immutable = true)
  | none => false

def startFenceCheck (world : World) (plan : OperationPlan) : Bool :=
  match world.workerFences plan.key.sandbox with
  | some fence => decide (plan.epoch ≤ fence.epoch ∧ fence.terminal = false)
  | none => false

def terminalFenceCheck (world : World) (plan : OperationPlan) : Bool :=
  match world.workerFences plan.key.sandbox with
  | some fence => decide (plan.epoch ≤ fence.epoch ∧ fence.terminal = true)
  | none => false

def selectedResourcesCheck (plan : OperationPlan) : Bool :=
  plan.sealed && plan.vm.isSome && plan.slot.isSome

def selectedSnapshotCheck (plan : OperationPlan) : Bool :=
  plan.sealed && plan.targetSnapshot.isSome

def restoreSourceSelectedCheck (plan : OperationPlan) : Bool :=
  plan.sealed && plan.restoreFrom.isSome

def filePolicyAppliedCheck (world : World) (key : OperationKey)
    (plan : OperationPlan) : Bool :=
  match world.sandboxes key.sandbox, world.appliedFilePolicies key.sandbox with
  | some sandbox, some applied => decide (
      applied.sandbox = key.sandbox ∧ applied.generation = plan.generation ∧
      applied.workspace = sandbox.workspace ∧
      applied.policy = sandbox.filePolicy)
  | _, _ => false

def countDebt {id : Type} (ready : id → Bool) : List id → Nat
  | [] => 0
  | value :: rest => Bool.debt (ready value) + countDebt ready rest

/-- Finite resource debt is computed only from the immutable operation plan,
    never by quantifying over the unbounded identifier type. -/
def plannedResourceDebt (world : World) (plan : OperationPlan) : Nat :=
  countDebt (plannedVmClosedCheck world) plan.stopVms +
    countDebt (plannedSlotClosedCheck world) plan.closeSlots

def stopReceiptCheck (world : World) (plan : OperationPlan)
    (vmId : VmId) : Bool :=
  match world.vms vmId with
  | some vm => Controller.vmStopSucceededFiniteCheck world plan.key.sandbox
      vm.generation vmId
  | none => true

def plannedStopReceiptDebt (world : World) (plan : OperationPlan) : Nat :=
  countDebt (stopReceiptCheck world plan) plan.stopVms

/-- Protocol position of an exact current attempt.  The retry multiplier is
    larger than every local position, so consuming one retry strictly lowers
    the command component even though a fresh committed attempt is created. -/
def attemptPositionDebt : Option EffectStatus → Nat
  | none => 4
  | some .committed => 3
  | some .dispatched => 2
  | some .outcomeUnknown => 3
  | some (.acknowledged (.succeeded _)) => 0
  | some (.acknowledged (.failed _)) => 1

/-- With weight five, consuming one retry lowers debt even though the next
    exact attempt starts again in the committed position. -/
theorem retry_consumption_strictly_lowers_debt (remaining : Nat) :
    5 * remaining + attemptPositionDebt (some .committed) <
      5 * (remaining + 1) +
        attemptPositionDebt (some (.acknowledged (.failed .resourceFailure))) := by
  simp [attemptPositionDebt]
  omega

/-- Before a command is journaled, reserve the greatest possible debt for its
    initial retry budget and protocol position.  Turning that future slot into
    a committed attempt therefore consumes one unit instead of creating debt. -/
def futureCommandDebt (plan : OperationPlan) : Nat :=
  5 * plan.retryLimit + attemptPositionDebt none

def expectedCommandDebt (world : World) (key : OperationKey)
    (plan : OperationPlan) (command : Command) : Nat :=
  let logical := logicalEffectId command
  match world.logicalEffects logical with
  | some record =>
      if record.result.isSome then 0
      else if record.origin = .operation key then
        let current : EffectId := ⟨logical, record.currentAttempt⟩
        5 * record.remainingRetries +
          attemptPositionDebt (world.effects current)
      else futureCommandDebt plan
  | none => futureCommandDebt plan

/-- Some commands cannot be named until a durable reservation has selected
    their resource identities.  Such a slot still carries maximum debt, so
    reservation can reveal the command without increasing the rank. -/
inductive ExpectedEffectSlot where
  | command (value : Command)
  | future
  deriving DecidableEq, Repr

def expectedEffectSlotDebt (world : World) (key : OperationKey)
    (plan : OperationPlan) : ExpectedEffectSlot → Nat
  | .command command => expectedCommandDebt world key plan command
  | .future => futureCommandDebt plan

def startupExpectedEffects (world : World) (key : OperationKey)
    (plan : OperationPlan) : List ExpectedEffectSlot :=
  let sandbox := world.sandboxes key.sandbox
  [ .command (.installFence key.sandbox plan.epoch false)
  , match plan.vm, plan.slot with
    | some vm, some slot => .command (.prepareVm key.sandbox plan.epoch
        plan.generation vm slot plan.restoreFrom)
    | _, _ => .future
  , match plan.vm, plan.slot, sandbox with
    | some vm, some slot, some record =>
        .command (.configureNetwork key.sandbox plan.epoch plan.generation
          vm slot record.policy)
    | _, _, _ => .future
  , match plan.vm with
    | some vm => .command
        (.activateVm key.sandbox plan.epoch plan.generation vm)
    | none => .future
  , match sandbox with
    | some record => .command (.applyFilePolicy key.sandbox plan.epoch
        plan.generation record.workspace record.filePolicy)
    | none => .future ]

def pauseExpectedEffects (world : World) (key : OperationKey)
    (plan : OperationPlan) : List ExpectedEffectSlot :=
  let sandbox := world.sandboxes key.sandbox
  [ .command (.installFence key.sandbox plan.epoch false)
  , match plan.vm with
    | some vm => .command (.freezeVm key.sandbox plan.epoch plan.generation vm)
    | none => .future
  , match plan.vm, plan.targetSnapshot, sandbox with
    | some vm, some source, some record =>
        .command (.sealSnapshot key.sandbox plan.epoch plan.generation vm
          record.workspace source.snapshot source.manifest)
    | _, _, _ => .future
  , match plan.targetSnapshot with
    | some source => .command (.commitSnapshot key.sandbox plan.epoch
        plan.generation source.snapshot source.manifest)
    | none => .future
  , match plan.vm with
    | some vm => .command (.stopVm key.sandbox plan.epoch plan.generation vm)
    | none => .future ]

def cleanupExpectedEffects (world : World) (key : OperationKey)
    (plan : OperationPlan) : List ExpectedEffectSlot :=
  .command (.installFence key.sandbox plan.epoch true) ::
    plan.stopVms.map fun vmId =>
      match world.vms vmId with
      | some vm => .command
          (.stopVm key.sandbox plan.epoch vm.generation vmId)
      | none => .future

def createExpectedEffects (world : World) (key : OperationKey)
    (plan : OperationPlan) : List ExpectedEffectSlot :=
  startupExpectedEffects world key plan

def resumeExpectedEffects (world : World) (key : OperationKey)
    (plan : OperationPlan) : List ExpectedEffectSlot :=
  startupExpectedEffects world key plan

def killExpectedEffects (world : World) (key : OperationKey)
    (plan : OperationPlan) : List ExpectedEffectSlot :=
  cleanupExpectedEffects world key plan

def compensationExpectedEffects (world : World) (key : OperationKey)
    (plan : OperationPlan) : List ExpectedEffectSlot :=
  cleanupExpectedEffects world key plan

/-! The scheduler does not have to assume that the command it is about to
    journal belongs to the rank.  Once the sealed plan exposes the command's
    durable identifiers, membership follows by computation. -/

theorem prepareVm_mem_startupExpectedEffects
    (selectedVm : plan.vm = some vmId)
    (selectedSlot : plan.slot = some slotId) :
    .command (.prepareVm key.sandbox plan.epoch plan.generation vmId slotId
      plan.restoreFrom) ∈ startupExpectedEffects world key plan := by
  simp [startupExpectedEffects, selectedVm, selectedSlot]

theorem configureNetwork_mem_startupExpectedEffects
    (selectedVm : plan.vm = some vmId)
    (selectedSlot : plan.slot = some slotId)
    (foundSandbox : world.sandboxes key.sandbox = some sandbox) :
    .command (.configureNetwork key.sandbox plan.epoch plan.generation vmId
      slotId sandbox.policy) ∈ startupExpectedEffects world key plan := by
  simp [startupExpectedEffects, selectedVm, selectedSlot, foundSandbox]

theorem activateVm_mem_startupExpectedEffects
    (selectedVm : plan.vm = some vmId) :
    .command (.activateVm key.sandbox plan.epoch plan.generation vmId) ∈
      startupExpectedEffects world key plan := by
  simp [startupExpectedEffects, selectedVm]

theorem applyFilePolicy_mem_startupExpectedEffects
    (foundSandbox : world.sandboxes key.sandbox = some sandbox) :
    .command (.applyFilePolicy key.sandbox plan.epoch plan.generation
      sandbox.workspace sandbox.filePolicy) ∈
        startupExpectedEffects world key plan := by
  simp [startupExpectedEffects, foundSandbox]

theorem freezeVm_mem_pauseExpectedEffects
    (selectedVm : plan.vm = some vmId) :
    .command (.freezeVm key.sandbox plan.epoch plan.generation vmId) ∈
      pauseExpectedEffects world key plan := by
  simp [pauseExpectedEffects, selectedVm]

theorem sealSnapshot_mem_pauseExpectedEffects
    (selectedVm : plan.vm = some vmId)
    (selectedSnapshot : plan.targetSnapshot = some source)
    (foundSandbox : world.sandboxes key.sandbox = some sandbox) :
    .command (.sealSnapshot key.sandbox plan.epoch plan.generation vmId
      sandbox.workspace source.snapshot source.manifest) ∈
        pauseExpectedEffects world key plan := by
  simp [pauseExpectedEffects, selectedVm, selectedSnapshot, foundSandbox]

theorem commitSnapshot_mem_pauseExpectedEffects
    (selectedSnapshot : plan.targetSnapshot = some source) :
    .command (.commitSnapshot key.sandbox plan.epoch plan.generation
      source.snapshot source.manifest) ∈ pauseExpectedEffects world key plan := by
  simp [pauseExpectedEffects, selectedSnapshot]

theorem stopVm_mem_pauseExpectedEffects
    (selectedVm : plan.vm = some vmId) :
    .command (.stopVm key.sandbox plan.epoch plan.generation vmId) ∈
      pauseExpectedEffects world key plan := by
  simp [pauseExpectedEffects, selectedVm]

theorem stopVm_mem_cleanupExpectedEffects
    (planned : vmId ∈ plan.stopVms)
    (foundVm : world.vms vmId = some vm) :
    .command (.stopVm key.sandbox plan.epoch vm.generation vmId) ∈
      cleanupExpectedEffects world key plan := by
  simp [cleanupExpectedEffects]
  exact ⟨vmId, planned, by simp [foundVm]⟩

def normalExpectedEffects (workflow : WorkflowKind) (world : World)
    (key : OperationKey) (plan : OperationPlan) : List ExpectedEffectSlot :=
  match workflow with
  | .create => createExpectedEffects world key plan
  | .pause => pauseExpectedEffects world key plan
  | .resume => resumeExpectedEffects world key plan
  | .kill => killExpectedEffects world key plan

theorem installStartFence_mem_startupExpectedEffects :
    .command (.installFence key.sandbox plan.epoch false) ∈
      startupExpectedEffects world key plan := by
  simp [startupExpectedEffects]

theorem installStartFence_mem_pauseExpectedEffects :
    .command (.installFence key.sandbox plan.epoch false) ∈
      pauseExpectedEffects world key plan := by
  simp [pauseExpectedEffects]

theorem installTerminalFence_mem_cleanupExpectedEffects :
    .command (.installFence key.sandbox plan.epoch true) ∈
      cleanupExpectedEffects world key plan := by
  simp [cleanupExpectedEffects]

/-- The accepted plan fixes a finite collection of command slots.  The normal
    workflow and terminal compensation each enumerate their own exact set;
    completed operations retain no effect debt. -/
def workflowExpectedEffects (workflow : WorkflowKind) (world : World)
    (key : OperationKey) (plan : OperationPlan) : List ExpectedEffectSlot :=
  match world.operations key with
  | some ledger =>
      match ledger.operation.status with
      | .compensating _ => compensationExpectedEffects world key plan
      | .pending | .outcomeUnknown =>
          normalExpectedEffects workflow world key plan
      | .succeeded | .failed _ => []
  | none => normalExpectedEffects workflow world key plan

theorem normalExpectedEffect_mem_workflowExpectedEffects
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (member : slot ∈ normalExpectedEffects workflow world key plan) :
    slot ∈ workflowExpectedEffects workflow world key plan := by
  rcases normal with normal | normal <;>
    simp [workflowExpectedEffects, foundLedger, normal, member]

theorem compensationExpectedEffect_mem_workflowExpectedEffects
    (failure : FailureKind)
    (foundLedger : world.operations key = some ledger)
    (compensating : ledger.operation.status =
      OperationStatus.compensating failure)
    (member : slot ∈ compensationExpectedEffects world key plan) :
    slot ∈ workflowExpectedEffects workflow world key plan := by
  simp [workflowExpectedEffects, foundLedger, compensating, member]

def operationEffectDebt (workflow : WorkflowKind) (world : World)
    (key : OperationKey) : Nat :=
  match world.operationPlans key with
  | none => 0
  | some plan =>
      (workflowExpectedEffects workflow world key plan).map
        (expectedEffectSlotDebt world key plan) |>.sum

theorem expectedCommandDebt_missing_eq_future
    (missing : world.logicalEffects (logicalEffectId command) = none) :
    expectedCommandDebt world key plan command = futureCommandDebt plan := by
  simp [expectedCommandDebt, missing]

theorem committedPosition_strictly_below_future (retryLimit : Nat) :
    5 * retryLimit + attemptPositionDebt (some .committed) <
      5 * retryLimit + attemptPositionDebt none := by
  simp [attemptPositionDebt]

theorem attemptPositionDebt_le_futurePosition
    (status : Option EffectStatus) :
    attemptPositionDebt status ≤ attemptPositionDebt none := by
  cases status with
  | none => simp
  | some status =>
      cases status with
      | committed => simp [attemptPositionDebt]
      | dispatched => simp [attemptPositionDebt]
      | outcomeUnknown => simp [attemptPositionDebt]
      | acknowledged outcome =>
          cases outcome <;> simp [attemptPositionDebt]

/-- The placeholder is an actual upper bound, not merely a sentinel, whenever
    the persisted retry budget remains within the accepted plan. -/
theorem expectedCommandDebt_le_future
    (bounded : ∀ record,
      world.logicalEffects (logicalEffectId command) = some record →
      record.origin = .operation key →
      record.remainingRetries ≤ plan.retryLimit) :
    expectedCommandDebt world key plan command ≤ futureCommandDebt plan := by
  unfold expectedCommandDebt
  cases found : world.logicalEffects (logicalEffectId command) with
  | none => simp [found, futureCommandDebt]
  | some record =>
      simp only [found]
      by_cases finished : record.result.isSome
      · simp [finished, futureCommandDebt]
      · simp only [finished, Bool.false_eq_true, ↓reduceIte]
        by_cases owned : record.origin = .operation key
        · simp only [owned, ↓reduceIte]
          have fuel := bounded record found owned
          have position := attemptPositionDebt_le_futurePosition
            (world.effects {
              logical := logicalEffectId command
              attempt := record.currentAttempt })
          unfold futureCommandDebt
          omega
        · simp [owned]

theorem expectedCommandDebt_plan_congr
    (sameRetry : left.retryLimit = right.retryLimit) :
    expectedCommandDebt world key left command =
      expectedCommandDebt world key right command := by
  cases found : world.logicalEffects (logicalEffectId command) with
  | none => simp [expectedCommandDebt, found, futureCommandDebt, sameRetry]
  | some record =>
      simp [expectedCommandDebt, found, futureCommandDebt, sameRetry]

/-- Journaling a command preserves the accepted retry ceiling.  A newly
    created logical record starts with `defaultRetryLimit`; existing records
    are untouched. -/
theorem enqueue_preserves_retryBound
    (plan : OperationPlan)
    (planDefault : plan.retryLimit = defaultRetryLimit)
    (bounded : ∀ logical record,
      world.logicalEffects logical = some record →
      record.origin = .operation key →
      record.remainingRetries ≤ plan.retryLimit) :
    ∀ logical record,
      (enqueue world command).logicalEffects logical = some record →
      record.origin = .operation key →
      record.remainingRetries ≤ plan.retryLimit := by
  intro logical record found origin
  cases effectFound : world.effects (effectId command) with
  | some status =>
      have unchanged : (enqueue world command).logicalEffects =
          world.logicalEffects := by
        simp [enqueue, commitEffect, effectFound]
      exact bounded logical record (by simpa [unchanged] using found) origin
  | none =>
      cases logicalFound : world.logicalEffects (logicalEffectId command) with
      | some existing =>
          have unchanged : (enqueue world command).logicalEffects =
              world.logicalEffects := by
            simp [enqueue, commitEffect, effectFound, logicalFound, setEffect]
          exact bounded logical record (by simpa [unchanged] using found) origin
      | none =>
          by_cases same : logical = logicalEffectId command
          · subst logical
            have sameRecord : record = {
                origin := inferredEffectOrigin world command
                currentAttempt := 0
                remainingRetries := defaultRetryLimit
                result := none } := by
              symm
              simpa [enqueue, commitEffect, effectFound, logicalFound,
                setEffect, setLogicalEffect, put] using found
            subst record
            simp [planDefault]
          · have oldFound : world.logicalEffects logical = some record := by
              simpa [enqueue, commitEffect, effectFound, logicalFound,
                setEffect, setLogicalEffect, put, same] using found
            exact bounded logical record oldFound origin

/-- Journaling a previously absent expected command consumes its reserved
    future-position unit.  In particular, enqueue cannot introduce new debt. -/
theorem enqueue_missing_expectedCommand_strict
    (effectMissing : world.effects (effectId command) = none)
    (logicalMissing :
      world.logicalEffects (logicalEffectId command) = none)
    (origin : inferredEffectOrigin world command = .operation key)
    (retryLimit : plan.retryLimit = defaultRetryLimit) :
    expectedCommandDebt (enqueue world command) key plan command <
      expectedCommandDebt world key plan command := by
  rw [expectedCommandDebt_missing_eq_future logicalMissing]
  have effectMissing' : world.effects
      ⟨⟨command⟩, 0⟩ = none := by
    simpa [effectId, effectIdAt, logicalEffectId] using effectMissing
  have logicalMissing' : world.logicalEffects ⟨command⟩ = none := by
    simpa [logicalEffectId] using logicalMissing
  simp [expectedCommandDebt, enqueue, commitEffect, effectMissing',
    logicalMissing', setEffect, setLogicalEffect, put, origin,
    futureCommandDebt, retryLimit, attemptPositionDebt, effectId,
    effectIdAt, logicalEffectId]

theorem enqueue_missing_expectedCommand_other
    (effectMissing : world.effects (effectId command) = none)
    (logicalMissing :
      world.logicalEffects (logicalEffectId command) = none)
    (different : other ≠ command) :
    expectedCommandDebt (enqueue world command) key plan other =
      expectedCommandDebt world key plan other := by
  have effectMissing' : world.effects ⟨⟨command⟩, 0⟩ = none := by
    simpa [effectId, effectIdAt, logicalEffectId] using effectMissing
  have logicalMissing' : world.logicalEffects ⟨command⟩ = none := by
    simpa [logicalEffectId] using logicalMissing
  have logicalDifferent : (⟨other⟩ : LogicalEffectId) ≠ ⟨command⟩ := by
    simpa using different
  simp [expectedCommandDebt, enqueue, commitEffect, effectMissing',
    logicalMissing', setEffect, setLogicalEffect, put, logicalDifferent,
    effectId, effectIdAt, logicalEffectId]

theorem enqueue_missing_expectedEffectSlot_le
    (effectMissing : world.effects (effectId command) = none)
    (logicalMissing :
      world.logicalEffects (logicalEffectId command) = none)
    (origin : inferredEffectOrigin world command = .operation key)
    (retryLimit : plan.retryLimit = defaultRetryLimit) :
    ∀ slot,
      expectedEffectSlotDebt (enqueue world command) key plan slot ≤
        expectedEffectSlotDebt world key plan slot := by
  intro slot
  cases slot with
  | future => exact Nat.le_refl _
  | command other =>
      by_cases same : other = command
      · subst other
        exact Nat.le_of_lt (enqueue_missing_expectedCommand_strict
          effectMissing logicalMissing origin retryLimit)
      · rw [expectedEffectSlotDebt, expectedEffectSlotDebt,
          enqueue_missing_expectedCommand_other effectMissing logicalMissing same]
        exact Nat.le_refl _

theorem sum_map_le_sum_map
    (values : List α) (before after : α → Nat)
    (decreases : ∀ value, value ∈ values → after value ≤ before value) :
    (values.map after).sum ≤ (values.map before).sum := by
  induction values with
  | nil => simp
  | cons value rest inductionHypothesis =>
      have head := decreases value (by simp)
      have tail := inductionHypothesis fun candidate member =>
        decreases candidate (by simp [member])
      simp only [List.map_cons, List.sum_cons]
      omega

theorem sum_map_strict_of_member
    (values : List α) (before after : α → Nat) (target : α)
    (member : target ∈ values)
    (decreases : ∀ value, value ∈ values → after value ≤ before value)
    (strict : after target < before target) :
    (values.map after).sum < (values.map before).sum := by
  induction values with
  | nil => simp at member
  | cons value rest inductionHypothesis =>
      simp only [List.mem_cons] at member
      simp only [List.map_cons, List.sum_cons]
      rcases member with same | tailMember
      · subst value
        have tailLe := sum_map_le_sum_map rest before after
          (fun candidate candidateMember =>
            decreases candidate (by simp [candidateMember]))
        omega
      · have headLe := decreases value (by simp)
        have tailStrict := inductionHypothesis tailMember
          (fun candidate candidateMember =>
            decreases candidate (by simp [candidateMember]))
        omega

@[simp] theorem workflowExpectedEffects_enqueue
    (workflow : WorkflowKind) (world : World) (key : OperationKey)
    (plan : OperationPlan) (command : Command) :
    workflowExpectedEffects workflow (enqueue world command) key plan =
      workflowExpectedEffects workflow world key plan := by
  simp [workflowExpectedEffects, normalExpectedEffects,
    createExpectedEffects, resumeExpectedEffects, killExpectedEffects,
    compensationExpectedEffects, startupExpectedEffects,
    pauseExpectedEffects, cleanupExpectedEffects]

/-- Every scheduler branch which journals a still-missing command already
    present in the workflow's expected-command list strictly consumes total
    effect debt. -/
theorem operationEffectDebt_enqueue_strict
    (foundPlan : world.operationPlans key = some plan)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (effectMissing : world.effects (effectId command) = none)
    (logicalMissing :
      world.logicalEffects (logicalEffectId command) = none)
    (origin : inferredEffectOrigin world command = .operation key)
    (retryLimit : plan.retryLimit = defaultRetryLimit) :
    operationEffectDebt workflow (enqueue world command) key <
      operationEffectDebt workflow world key := by
  unfold operationEffectDebt
  simp only [enqueue_operationPlans, foundPlan, workflowExpectedEffects_enqueue]
  apply sum_map_strict_of_member _ _ _ (.command command) member
  · intro slot _
    exact enqueue_missing_expectedEffectSlot_le effectMissing logicalMissing
      origin retryLimit slot
  · exact enqueue_missing_expectedCommand_strict effectMissing logicalMissing
      origin retryLimit

theorem dispatchEffect_expectedCommand_strict
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (dispatchable : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ = some .committed ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ =
        some .outcomeUnknown) :
    expectedCommandDebt
        (dispatchEffect world
          ⟨logicalEffectId command, record.currentAttempt⟩)
        key plan command < expectedCommandDebt world key plan command := by
  rcases dispatchable with dispatchable | dispatchable <;>
    simp [expectedCommandDebt, dispatchEffect, setEffect, put, found,
      unfinished, origin, dispatchable, attemptPositionDebt]

theorem dispatchEffect_expectedCommand_other
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (different : other ≠ command) :
    expectedCommandDebt
        (dispatchEffect world
          ⟨logicalEffectId command, record.currentAttempt⟩)
        key plan other = expectedCommandDebt world key plan other := by
  have logicalDifferent : logicalEffectId other ≠ logicalEffectId command := by
    simpa [logicalEffectId] using different
  simp [expectedCommandDebt, dispatchEffect, setEffect, put, logicalDifferent]

theorem dispatchEffect_expectedEffectSlot_le
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (dispatchable : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ = some .committed ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ =
        some .outcomeUnknown) :
    ∀ slot,
      expectedEffectSlotDebt
          (dispatchEffect world
            ⟨logicalEffectId command, record.currentAttempt⟩)
          key plan slot ≤ expectedEffectSlotDebt world key plan slot := by
  intro slot
  cases slot with
  | future => exact Nat.le_refl _
  | command other =>
      by_cases same : other = command
      · subst other
        exact Nat.le_of_lt (dispatchEffect_expectedCommand_strict found
          unfinished origin dispatchable)
      · rw [expectedEffectSlotDebt, expectedEffectSlotDebt,
          dispatchEffect_expectedCommand_other found same]
        exact Nat.le_refl _

@[simp] theorem workflowExpectedEffects_dispatchEffect
    (id : EffectId) :
    workflowExpectedEffects workflow (dispatchEffect world id) key plan =
      workflowExpectedEffects workflow world key plan := by
  simp [workflowExpectedEffects, normalExpectedEffects,
    createExpectedEffects, resumeExpectedEffects, killExpectedEffects,
    compensationExpectedEffects, startupExpectedEffects,
    pauseExpectedEffects, cleanupExpectedEffects, dispatchEffect, setEffect]

theorem operationEffectDebt_dispatchEffect_strict
    (foundPlan : world.operationPlans key = some plan)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (dispatchable : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ = some .committed ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ =
        some .outcomeUnknown) :
    operationEffectDebt workflow
        (dispatchEffect world
          ⟨logicalEffectId command, record.currentAttempt⟩) key <
      operationEffectDebt workflow world key := by
  unfold operationEffectDebt
  simp only [dispatchEffect_operationPlans, foundPlan,
    workflowExpectedEffects_dispatchEffect]
  apply sum_map_strict_of_member _ _ _ (.command command) member
  · intro slot _
    exact dispatchEffect_expectedEffectSlot_le found unfinished origin
      dispatchable slot
  · exact dispatchEffect_expectedCommand_strict found unfinished origin
      dispatchable

/-- Advancing a retry consumes one unit of retry fuel.  The next committed
    attempt starts higher in the local protocol position, but the factor five
    makes the command's total debt strictly smaller. -/
theorem startNextAttempt_expectedCommand_strict
    (failure : FailureKind) (remaining : Nat)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure)))
    (retryable : failure.retryable = true)
    (budget : record.remainingRetries = remaining + 1)
    (fresh : world.effects
      ⟨logicalEffectId command, record.currentAttempt + 1⟩ = none) :
    expectedCommandDebt
        (startNextAttempt world (logicalEffectId command) failure)
        key plan command <
      expectedCommandDebt world key plan command := by
  simp [expectedCommandDebt, startNextAttempt, nextAttemptData, found,
    unfinished, origin, failed, retryable, budget, fresh, put,
    attemptPositionDebt]
  omega

theorem startNextAttempt_expectedCommand_other
    (failure : FailureKind) (remaining : Nat)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure)))
    (retryable : failure.retryable = true)
    (budget : record.remainingRetries = remaining + 1)
    (fresh : world.effects
      ⟨logicalEffectId command, record.currentAttempt + 1⟩ = none)
    (different : other ≠ command) :
    expectedCommandDebt
        (startNextAttempt world (logicalEffectId command) failure)
        key plan other = expectedCommandDebt world key plan other := by
  have found' : world.logicalEffects ⟨command⟩ = some record := by
    simpa [logicalEffectId] using found
  have failed' : world.effects ⟨⟨command⟩, record.currentAttempt⟩ =
      some (.acknowledged (.failed failure)) := by
    simpa [logicalEffectId] using failed
  have fresh' : world.effects
      ⟨⟨command⟩, record.currentAttempt + 1⟩ = none := by
    simpa [logicalEffectId] using fresh
  have logicalDifferent : (⟨other⟩ : LogicalEffectId) ≠ ⟨command⟩ := by
    simpa using different
  simp [expectedCommandDebt, startNextAttempt, nextAttemptData, found',
    unfinished, origin, failed', retryable, budget, fresh', put,
    logicalDifferent, logicalEffectId]

theorem startNextAttempt_expectedEffectSlot_le
    (failure : FailureKind) (remaining : Nat)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure)))
    (retryable : failure.retryable = true)
    (budget : record.remainingRetries = remaining + 1)
    (fresh : world.effects
      ⟨logicalEffectId command, record.currentAttempt + 1⟩ = none) :
    ∀ slot,
      expectedEffectSlotDebt
          (startNextAttempt world (logicalEffectId command) failure)
          key plan slot ≤
        expectedEffectSlotDebt world key plan slot := by
  intro slot
  cases slot with
  | future => exact Nat.le_refl _
  | command other =>
      by_cases same : other = command
      · subst other
        exact Nat.le_of_lt (startNextAttempt_expectedCommand_strict
          failure remaining found unfinished origin failed retryable budget fresh)
      · rw [expectedEffectSlotDebt, expectedEffectSlotDebt,
          startNextAttempt_expectedCommand_other failure remaining found
            unfinished origin failed retryable budget fresh same]
        exact Nat.le_refl _

@[simp] theorem workflowExpectedEffects_startNextAttempt
    (workflow : WorkflowKind) (world : World) (key : OperationKey)
    (plan : OperationPlan) (logical : LogicalEffectId)
    (failure : FailureKind) :
    workflowExpectedEffects workflow (startNextAttempt world logical failure)
        key plan = workflowExpectedEffects workflow world key plan := by
  simp [workflowExpectedEffects, normalExpectedEffects,
    createExpectedEffects, resumeExpectedEffects, killExpectedEffects,
    compensationExpectedEffects, startupExpectedEffects,
    pauseExpectedEffects, cleanupExpectedEffects]

theorem operationEffectDebt_startNextAttempt_strict
    (failure : FailureKind) (remaining : Nat)
    (foundPlan : world.operationPlans key = some plan)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure)))
    (retryable : failure.retryable = true)
    (budget : record.remainingRetries = remaining + 1)
    (fresh : world.effects
      ⟨logicalEffectId command, record.currentAttempt + 1⟩ = none) :
    operationEffectDebt workflow
        (startNextAttempt world (logicalEffectId command) failure) key <
      operationEffectDebt workflow world key := by
  unfold operationEffectDebt
  simp only [startNextAttempt_operationPlans, foundPlan,
    workflowExpectedEffects_startNextAttempt]
  apply sum_map_strict_of_member _ _ _ (.command command) member
  · intro slot _
    exact startNextAttempt_expectedEffectSlot_le failure remaining found
      unfinished origin failed retryable budget fresh slot
  · exact startNextAttempt_expectedCommand_strict failure remaining found
      unfinished origin failed retryable budget fresh

theorem finalizeLogicalFailure_expectedCommand_strict
    (failure : FailureKind)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure))) :
    expectedCommandDebt
        (finalizeLogicalFailure world (logicalEffectId command) failure)
        key plan command < expectedCommandDebt world key plan command := by
  simp [expectedCommandDebt, finalizeLogicalFailure, found, unfinished,
    origin, failed, put, attemptPositionDebt]

theorem finalizeLogicalFailure_expectedCommand_other
    (failure : FailureKind)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure)))
    (different : other ≠ command) :
    expectedCommandDebt
        (finalizeLogicalFailure world (logicalEffectId command) failure)
        key plan other = expectedCommandDebt world key plan other := by
  have found' : world.logicalEffects ⟨command⟩ = some record := by
    simpa [logicalEffectId] using found
  have failed' : world.effects ⟨⟨command⟩, record.currentAttempt⟩ =
      some (.acknowledged (.failed failure)) := by
    simpa [logicalEffectId] using failed
  have logicalDifferent : (⟨other⟩ : LogicalEffectId) ≠ ⟨command⟩ := by
    simpa using different
  simp [expectedCommandDebt, finalizeLogicalFailure, found', origin, failed',
    put, logicalDifferent, logicalEffectId]

theorem finalizeLogicalFailure_expectedEffectSlot_le
    (failure : FailureKind)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure))) :
    ∀ slot,
      expectedEffectSlotDebt
          (finalizeLogicalFailure world (logicalEffectId command) failure)
          key plan slot ≤
        expectedEffectSlotDebt world key plan slot := by
  intro slot
  cases slot with
  | future => exact Nat.le_refl _
  | command other =>
      by_cases same : other = command
      · subst other
        exact Nat.le_of_lt (finalizeLogicalFailure_expectedCommand_strict
          failure found unfinished origin failed)
      · rw [expectedEffectSlotDebt, expectedEffectSlotDebt,
          finalizeLogicalFailure_expectedCommand_other failure found origin
            failed same]
        exact Nat.le_refl _

@[simp] theorem workflowExpectedEffects_finalizeLogicalFailure
    (workflow : WorkflowKind) (world : World) (key : OperationKey)
    (plan : OperationPlan) (logical : LogicalEffectId)
    (failure : FailureKind) :
    workflowExpectedEffects workflow
        (finalizeLogicalFailure world logical failure) key plan =
      workflowExpectedEffects workflow world key plan := by
  simp [workflowExpectedEffects, normalExpectedEffects,
    createExpectedEffects, resumeExpectedEffects, killExpectedEffects,
    compensationExpectedEffects, startupExpectedEffects,
    pauseExpectedEffects, cleanupExpectedEffects]

theorem operationEffectDebt_finalizeLogicalFailure_strict
    (failure : FailureKind)
    (foundPlan : world.operationPlans key = some plan)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure))) :
    operationEffectDebt workflow
        (finalizeLogicalFailure world (logicalEffectId command) failure) key <
      operationEffectDebt workflow world key := by
  unfold operationEffectDebt
  simp only [finalizeLogicalFailure_operationPlans, foundPlan,
    workflowExpectedEffects_finalizeLogicalFailure]
  apply sum_map_strict_of_member _ _ _ (.command command) member
  · intro slot _
    exact finalizeLogicalFailure_expectedEffectSlot_le failure found unfinished
      origin failed slot
  · exact finalizeLogicalFailure_expectedCommand_strict failure found
      unfinished origin failed

def createStageDebt (world : World) (key : OperationKey)
    (plan : OperationPlan) : Nat :=
  Bool.debt (startFenceCheck world plan) +
  Bool.debt (selectedResourcesCheck plan) +
  (match plan.vm with
    | some vm => Bool.debt (vmPreparedCheck world vm)
    | none => 1) +
  (match plan.slot with
    | some slot => Bool.debt (slotConfiguredCheck world slot)
    | none => 1) +
  Bool.debt (filePolicyAppliedCheck world key plan) +
  (match plan.vm with
    | some vm => Bool.debt (vmExecutingCheck world vm)
    | none => 1)

def resumeStageDebt (world : World) (key : OperationKey)
    (plan : OperationPlan) : Nat :=
  Bool.debt (restoreSourceSelectedCheck plan) + createStageDebt world key plan

def pauseStageDebt (world : World) (plan : OperationPlan) : Nat :=
  Bool.debt (startFenceCheck world plan) +
  (match plan.vm with
    | some vm => Bool.debt (vmFrozenOrClosedCheck world vm)
    | none => 1) +
  Bool.debt (selectedSnapshotCheck plan) +
  (match plan.targetSnapshot with
    | some source => Bool.debt (snapshotPresentCheck world source)
    | none => 1) +
  (match plan.targetSnapshot with
    | some source => Bool.debt (snapshotDurableCheck world source)
    | none => 1) +
  (match plan.vm with
    | some vm => Bool.debt (vmClosedCheck world vm)
    | none => 1) +
  countDebt (slotClosedCheck world) plan.closeSlots

def cleanupStageDebt (world : World) (plan : OperationPlan) : Nat :=
  Bool.debt (terminalFenceCheck world plan) +
    plannedResourceDebt world plan + plannedStopReceiptDebt world plan

def startupReservedPlan (world : World) (plan : OperationPlan) : OperationPlan :=
  { plan with
    vm := some ⟨world.nextVmId⟩
    slot := some ⟨world.nextSlotId⟩
    stopVms := insertUnique ⟨world.nextVmId⟩ plan.stopVms
    closeSlots := insertUnique ⟨world.nextSlotId⟩ plan.closeSlots
    sealed := true }

def pauseReservedPlan (world : World) (plan : OperationPlan)
    (vmId : VmId) : OperationPlan :=
  { plan with
    vm := some vmId
    targetSnapshot := some
      ⟨⟨world.nextSnapshotId⟩, ⟨⟨world.nextManifestDigest⟩⟩⟩
    sealed := true }

/-- An unsealed create/resume/pause plan reserves one future VM and slot in
    its cleanup capacity.  Sealing the startup reservation inserts at most one
    identifier into each list while consuming that spare capacity. -/
def cleanupVmCapacity (plan : OperationPlan) : Nat :=
  plan.stopVms.length + Bool.debt plan.sealed

def cleanupSlotCapacity (plan : OperationPlan) : Nat :=
  plan.closeSlots.length + Bool.debt plan.sealed

def cleanupStageUpperBound (plan : OperationPlan) : Nat :=
  1 + 2 * cleanupVmCapacity plan + cleanupSlotCapacity plan

def cleanupEffectUpperBound (plan : OperationPlan) : Nat :=
  (1 + cleanupVmCapacity plan) * futureCommandDebt plan

/-- This finite amount is reserved while an operation is on its normal path.
    Entering compensation removes the reservation and replaces it with actual
    cleanup debt, which is bounded by this value. -/
def compensationUpperBound (plan : OperationPlan) : Nat :=
  cleanupStageUpperBound plan + cleanupEffectUpperBound plan

theorem insertUnique_length_le_succ [DecidableEq α]
    (value : α) (values : List α) :
    (insertUnique value values).length ≤ values.length + 1 := by
  by_cases member : value ∈ values <;>
    simp [insertUnique, member]

theorem startupReservedPlan_vmCapacity_le
    (openPlan : plan.sealed = false) :
    cleanupVmCapacity (startupReservedPlan world plan) ≤
      cleanupVmCapacity plan := by
  have inserted := insertUnique_length_le_succ
    (⟨world.nextVmId⟩ : VmId) plan.stopVms
  simp only [cleanupVmCapacity, startupReservedPlan, Bool.debt, openPlan,
    Bool.false_eq_true, ↓reduceIte, List.length_cons]
  omega

theorem startupReservedPlan_slotCapacity_le
    (openPlan : plan.sealed = false) :
    cleanupSlotCapacity (startupReservedPlan world plan) ≤
      cleanupSlotCapacity plan := by
  have inserted := insertUnique_length_le_succ
    (⟨world.nextSlotId⟩ : NetSlotId) plan.closeSlots
  simp only [cleanupSlotCapacity, startupReservedPlan, Bool.debt, openPlan,
    Bool.false_eq_true, ↓reduceIte, List.length_cons]
  omega

theorem pauseReservedPlan_vmCapacity_le
    (openPlan : plan.sealed = false) :
    cleanupVmCapacity (pauseReservedPlan world plan vmId) <
      cleanupVmCapacity plan := by
  simp [cleanupVmCapacity, pauseReservedPlan, Bool.debt, openPlan]

theorem pauseReservedPlan_slotCapacity_le
    (openPlan : plan.sealed = false) :
    cleanupSlotCapacity (pauseReservedPlan world plan vmId) <
      cleanupSlotCapacity plan := by
  simp [cleanupSlotCapacity, pauseReservedPlan, Bool.debt, openPlan]

theorem compensationUpperBound_startupReservedPlan_le
    (openPlan : plan.sealed = false) :
    compensationUpperBound (startupReservedPlan world plan) ≤
      compensationUpperBound plan := by
  have vmLe := startupReservedPlan_vmCapacity_le
    (world := world) openPlan
  have slotLe := startupReservedPlan_slotCapacity_le
    (world := world) openPlan
  have effectLe := Nat.mul_le_mul_right (futureCommandDebt plan)
    (Nat.add_le_add_left vmLe 1)
  simp only [compensationUpperBound, cleanupStageUpperBound,
    cleanupEffectUpperBound]
  have sameFuture : futureCommandDebt (startupReservedPlan world plan) =
      futureCommandDebt plan := rfl
  rw [sameFuture]
  omega

theorem compensationUpperBound_pauseReservedPlan_lt
    (openPlan : plan.sealed = false) :
    compensationUpperBound (pauseReservedPlan world plan vmId) <
      compensationUpperBound plan := by
  have vmLt := pauseReservedPlan_vmCapacity_le
    (world := world) (vmId := vmId) openPlan
  have slotLt := pauseReservedPlan_slotCapacity_le
    (world := world) (vmId := vmId) openPlan
  have effectLe := Nat.mul_le_mul_right (futureCommandDebt plan)
    (Nat.add_le_add_left (Nat.le_of_lt vmLt) 1)
  simp only [compensationUpperBound, cleanupStageUpperBound,
    cleanupEffectUpperBound]
  have sameFuture : futureCommandDebt (pauseReservedPlan world plan vmId) =
      futureCommandDebt plan := rfl
  rw [sameFuture]
  omega

theorem startupExpectedEffectDebt_reservation_le
    (selectedVm : plan.vm = none)
    (selectedSlot : plan.slot = none)
    (foundSandbox : world.sandboxes key.sandbox = some sandbox)
    (bounded : ∀ logical logicalRecord,
      world.logicalEffects logical = some logicalRecord →
      logicalRecord.origin = .operation key →
      logicalRecord.remainingRetries ≤ plan.retryLimit) :
    ((startupExpectedEffects world key (startupReservedPlan world plan)).map
      (expectedEffectSlotDebt world key (startupReservedPlan world plan))).sum ≤
    ((startupExpectedEffects world key plan).map
      (expectedEffectSlotDebt world key plan)).sum := by
  let next := startupReservedPlan world plan
  let vmId : VmId := ⟨world.nextVmId⟩
  let slotId : NetSlotId := ⟨world.nextSlotId⟩
  have nextBounded : ∀ logical logicalRecord,
      world.logicalEffects logical = some logicalRecord →
      logicalRecord.origin = .operation key →
      logicalRecord.remainingRetries ≤ next.retryLimit := by
    simpa [next, startupReservedPlan] using bounded
  have prepareLe : expectedCommandDebt world key next
      (.prepareVm key.sandbox plan.epoch plan.generation vmId slotId
        plan.restoreFrom) ≤ futureCommandDebt plan := by
    have result := expectedCommandDebt_le_future
      (world := world) (key := key) (plan := next)
      (command := .prepareVm key.sandbox plan.epoch plan.generation vmId
        slotId plan.restoreFrom)
      (fun logicalRecord found origin =>
        nextBounded _ logicalRecord found origin)
    rwa [show futureCommandDebt next = futureCommandDebt plan from rfl] at result
  have networkLe : expectedCommandDebt world key next
      (.configureNetwork key.sandbox plan.epoch plan.generation vmId slotId
        sandbox.policy) ≤ futureCommandDebt plan := by
    have result := expectedCommandDebt_le_future
      (world := world) (key := key) (plan := next)
      (command := .configureNetwork key.sandbox plan.epoch plan.generation
        vmId slotId sandbox.policy)
      (fun logicalRecord found origin =>
        nextBounded _ logicalRecord found origin)
    rwa [show futureCommandDebt next = futureCommandDebt plan from rfl] at result
  have activateLe : expectedCommandDebt world key next
      (.activateVm key.sandbox plan.epoch plan.generation vmId) ≤
        futureCommandDebt plan := by
    have result := expectedCommandDebt_le_future
      (world := world) (key := key) (plan := next)
      (command := .activateVm key.sandbox plan.epoch plan.generation vmId)
      (fun logicalRecord found origin =>
        nextBounded _ logicalRecord found origin)
    rwa [show futureCommandDebt next = futureCommandDebt plan from rfl] at result
  have fenceEq : expectedCommandDebt world key next
      (.installFence key.sandbox plan.epoch false) =
      expectedCommandDebt world key plan
        (.installFence key.sandbox plan.epoch false) := by
    exact expectedCommandDebt_plan_congr rfl
  have policyEq : expectedCommandDebt world key next
      (.applyFilePolicy key.sandbox plan.epoch plan.generation
        sandbox.workspace sandbox.filePolicy) =
      expectedCommandDebt world key plan
        (.applyFilePolicy key.sandbox plan.epoch plan.generation
          sandbox.workspace sandbox.filePolicy) := by
    exact expectedCommandDebt_plan_congr rfl
  change ((startupExpectedEffects world key next).map
      (expectedEffectSlotDebt world key next)).sum ≤ _
  dsimp [next, startupReservedPlan, vmId, slotId] at prepareLe networkLe activateLe fenceEq policyEq
  simp [startupExpectedEffects, next, startupReservedPlan, vmId, slotId,
    selectedVm, selectedSlot, foundSandbox, expectedEffectSlotDebt]
  omega

theorem operationEffectDebt_reserveStartupResources_le
    (startupWorkflow : workflow = WorkflowKind.create ∨
      workflow = WorkflowKind.resume)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (selectedVm : plan.vm = none)
    (selectedSlot : plan.slot = none)
    (foundSandbox : world.sandboxes key.sandbox = some sandbox)
    (bounded : ∀ logical logicalRecord,
      world.logicalEffects logical = some logicalRecord →
      logicalRecord.origin = .operation key →
      logicalRecord.remainingRetries ≤ plan.retryLimit) :
    operationEffectDebt workflow
        (reserveStartupResources world key plan) key ≤
      operationEffectDebt workflow world key := by
  let next := startupReservedPlan world plan
  have afterPlan :
      (reserveStartupResources world key plan).operationPlans key = some next := by
    simp [next, startupReservedPlan, reserveStartupResources_operationPlans, put]
  have beforeExpected : workflowExpectedEffects workflow world key plan =
      startupExpectedEffects world key plan := by
    rcases startupWorkflow with rfl | rfl <;>
      rcases normal with normal | normal <;>
      simp [workflowExpectedEffects, normalExpectedEffects,
        createExpectedEffects, resumeExpectedEffects, foundLedger, normal,
        startupExpectedEffects]
  have debtEq : expectedEffectSlotDebt
      (reserveStartupResources world key plan) key next =
        expectedEffectSlotDebt world key next := by
    funext slot
    cases slot with
    | future => rfl
    | command command =>
        simp [expectedEffectSlotDebt, expectedCommandDebt]
  have afterExpected : workflowExpectedEffects workflow
      (reserveStartupResources world key plan) key next =
        startupExpectedEffects world key next := by
    rcases startupWorkflow with rfl | rfl <;>
      rcases normal with normal | normal <;>
      simp [workflowExpectedEffects, normalExpectedEffects,
        createExpectedEffects, resumeExpectedEffects, foundLedger, normal,
        startupExpectedEffects]
  unfold operationEffectDebt
  simp only [afterPlan, foundPlan, afterExpected, beforeExpected]
  rw [debtEq]
  simpa [next] using startupExpectedEffectDebt_reservation_le
    selectedVm selectedSlot foundSandbox bounded

theorem createStageDebt_reserveStartupResources_strict
    (openPlan : plan.sealed = false)
    (selectedVm : plan.vm = none)
    (selectedSlot : plan.slot = none)
    (freshVm : world.vms ⟨world.nextVmId⟩ = none)
    (freshSlot : world.netSlots ⟨world.nextSlotId⟩ = none) :
    createStageDebt (reserveStartupResources world key plan) key
        (startupReservedPlan world plan) <
      createStageDebt world key plan := by
  simp [createStageDebt, selectedResourcesCheck, startupReservedPlan,
    openPlan, selectedVm, selectedSlot, freshVm, freshSlot,
    startFenceCheck, vmPreparedCheck, slotConfiguredCheck,
    filePolicyAppliedCheck, vmExecutingCheck, Bool.debt]

theorem resumeStageDebt_reserveStartupResources_strict
    (openPlan : plan.sealed = false)
    (selectedVm : plan.vm = none)
    (selectedSlot : plan.slot = none)
    (freshVm : world.vms ⟨world.nextVmId⟩ = none)
    (freshSlot : world.netSlots ⟨world.nextSlotId⟩ = none) :
    resumeStageDebt (reserveStartupResources world key plan) key
        (startupReservedPlan world plan) <
      resumeStageDebt world key plan := by
  have createStrict := createStageDebt_reserveStartupResources_strict
    (world := world) (key := key) openPlan selectedVm selectedSlot freshVm freshSlot
  dsimp [startupReservedPlan] at createStrict
  unfold resumeStageDebt
  simp only [restoreSourceSelectedCheck, startupReservedPlan, Bool.true_and,
    Bool.false_and, openPlan]
  cases restore : plan.restoreFrom <;>
    simp [restore, Bool.debt] at createStrict ⊢ <;> omega

theorem boolDebt_le_one (value : Bool) : Bool.debt value ≤ 1 := by
  cases value <;> simp [Bool.debt]

theorem countDebt_le_length {identifier : Type} (ready : identifier → Bool)
    (values : List identifier) :
    countDebt ready values ≤ values.length := by
  induction values with
  | nil => simp [countDebt]
  | cons value rest ih =>
      have head := boolDebt_le_one (ready value)
      simp only [countDebt, List.length_cons]
      omega

private theorem sum_map_const (values : List α) (debt : Nat) :
    (values.map fun _ => debt).sum = values.length * debt := by
  induction values with
  | nil => simp
  | cons value rest ih =>
      simp [ih, Nat.add_mul, Nat.add_comm]

theorem cleanupStageDebt_le_upperBound :
    cleanupStageDebt world plan ≤ cleanupStageUpperBound plan := by
  have fence := boolDebt_le_one (terminalFenceCheck world plan)
  have stopResources := countDebt_le_length
    (plannedVmClosedCheck world) plan.stopVms
  have slotResources := countDebt_le_length
    (plannedSlotClosedCheck world) plan.closeSlots
  have receipts := countDebt_le_length (stopReceiptCheck world plan) plan.stopVms
  have sealedSpare : 0 ≤ Bool.debt plan.sealed := Nat.zero_le _
  simp only [cleanupStageDebt, plannedResourceDebt, plannedStopReceiptDebt,
    cleanupStageUpperBound, cleanupVmCapacity, cleanupSlotCapacity]
  omega

theorem compensationEffectDebt_le_upperBound
    (bounded : ∀ logical record,
      world.logicalEffects logical = some record →
      record.origin = .operation key →
      record.remainingRetries ≤ plan.retryLimit) :
    ((compensationExpectedEffects world key plan).map
      (expectedEffectSlotDebt world key plan)).sum ≤
        cleanupEffectUpperBound plan := by
  have each : ∀ slot, slot ∈ compensationExpectedEffects world key plan →
      expectedEffectSlotDebt world key plan slot ≤ futureCommandDebt plan := by
    intro slot member
    cases slot with
    | future => exact Nat.le_refl _
    | command command =>
        exact expectedCommandDebt_le_future fun record found origin =>
          bounded (logicalEffectId command) record found origin
  have summed := sum_map_le_sum_map
    (compensationExpectedEffects world key plan)
    (fun _ => futureCommandDebt plan)
    (expectedEffectSlotDebt world key plan) each
  have lengthEq : (compensationExpectedEffects world key plan).length =
      1 + plan.stopVms.length := by
    simp [compensationExpectedEffects, cleanupExpectedEffects, Nat.add_comm]
  have capacity : plan.stopVms.length ≤ cleanupVmCapacity plan := by
    simp [cleanupVmCapacity]
  have lengthCapacity : 1 + plan.stopVms.length ≤
      1 + cleanupVmCapacity plan := Nat.add_le_add_left capacity 1
  calc
    ((compensationExpectedEffects world key plan).map
        (expectedEffectSlotDebt world key plan)).sum ≤
        (1 + plan.stopVms.length) * futureCommandDebt plan := by
          simpa [sum_map_const, lengthEq] using summed
    _ ≤ (1 + cleanupVmCapacity plan) * futureCommandDebt plan :=
      Nat.mul_le_mul_right (futureCommandDebt plan) lengthCapacity
    _ = cleanupEffectUpperBound plan := rfl

theorem operationEffectDebt_compensating_le_upperBound
    (failure : FailureKind)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (compensating : ledger.operation.status = .compensating failure)
    (bounded : ∀ logical record,
      world.logicalEffects logical = some record →
      record.origin = .operation key →
      record.remainingRetries ≤ plan.retryLimit) :
    operationEffectDebt workflow world key ≤ cleanupEffectUpperBound plan := by
  unfold operationEffectDebt
  simp only [foundPlan]
  rw [show workflowExpectedEffects workflow world key plan =
      compensationExpectedEffects world key plan by
    simp [workflowExpectedEffects, foundLedger, compensating]]
  exact compensationEffectDebt_le_upperBound bounded

def normalStageDebt (workflow : WorkflowKind) (world : World)
    (key : OperationKey) (plan : OperationPlan) : Nat :=
  match workflow with
  | .create => createStageDebt world key plan
  | .pause => pauseStageDebt world plan
  | .resume => resumeStageDebt world key plan
  | .kill => cleanupStageDebt world plan

def workflowStageDebt (workflow : WorkflowKind) (world : World)
    (key : OperationKey) : Nat :=
  match world.operationPlans key with
  | none => workflow.normalMilestones
  | some plan =>
      match world.operations key with
      | some ledger =>
          match ledger.operation.status with
          | .compensating _ => cleanupStageDebt world plan
          | .pending | .outcomeUnknown =>
              normalStageDebt workflow world key plan + 1 +
                compensationUpperBound plan
          | .succeeded | .failed _ => 0
      | none => normalStageDebt workflow world key plan

theorem workflowStageDebt_reserveStartupResources_strict
    (startupWorkflow : workflow = WorkflowKind.create ∨
      workflow = WorkflowKind.resume)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (openPlan : plan.sealed = false)
    (selectedVm : plan.vm = none)
    (selectedSlot : plan.slot = none)
    (freshVm : world.vms ⟨world.nextVmId⟩ = none)
    (freshSlot : world.netSlots ⟨world.nextSlotId⟩ = none) :
    workflowStageDebt workflow (reserveStartupResources world key plan) key <
      workflowStageDebt workflow world key := by
  let next := startupReservedPlan world plan
  have afterPlan :
      (reserveStartupResources world key plan).operationPlans key = some next := by
    simp [next, startupReservedPlan, reserveStartupResources_operationPlans, put]
  have compensationLe := compensationUpperBound_startupReservedPlan_le
    (world := world) openPlan
  rcases startupWorkflow with rfl | rfl
  · have stageStrict := createStageDebt_reserveStartupResources_strict
      (world := world) (key := key) openPlan selectedVm selectedSlot
      freshVm freshSlot
    rcases normal with normal | normal <;>
      simp [workflowStageDebt, normalStageDebt, afterPlan, foundPlan,
        foundLedger, normal, next] <;> omega
  · have stageStrict := resumeStageDebt_reserveStartupResources_strict
      (world := world) (key := key) openPlan selectedVm selectedSlot
      freshVm freshSlot
    rcases normal with normal | normal <;>
      simp [workflowStageDebt, normalStageDebt, afterPlan, foundPlan,
        foundLedger, normal, next] <;> omega

/-! ### Executable finalization check and concrete rank -/

def finalizedCheck (world : World) (key : OperationKey) : Bool :=
  match world.operations key with
  | none => false
  | some ledger => decide (ledger.sandbox = key.sandbox ∧
      ledger.operation.id = key.operation ∧ ledger.result.isSome)

theorem finalizedCheck_spec :
    finalizedCheck world key = true ↔ Finalized key world := by
  unfold finalizedCheck Finalized LedgerAt
  cases found : world.operations key with
  | none => simp
  | some ledger =>
      cases result : ledger.result with
      | none => simp [result]
      | some value => simp [result, and_assoc]

/-- A workflow rank is zero exactly at durable finalization.  Otherwise it is
    one publication token plus finite workflow, retry, and resource debt. -/
def workflowRank (workflow : WorkflowKind) (key : OperationKey)
    (world : World) : Nat :=
  if finalizedCheck world key then 0
  else 1 + workflowStageDebt workflow world key +
    operationEffectDebt workflow world key

theorem workflowRank_final
    (finalized : Finalized key world) :
    workflowRank workflow key world = 0 := by
  rw [← finalizedCheck_spec] at finalized
  simp [workflowRank, finalized]

theorem workflowRank_unfinished
    (unfinished : ¬ Finalized key world) :
    0 < workflowRank workflow key world := by
  have checked : finalizedCheck world key = false := by
    cases value : finalizedCheck world key
    · rfl
    · exact (unfinished (finalizedCheck_spec.mp value)).elim
  simp only [workflowRank, checked, Bool.false_eq_true, ↓reduceIte]
  omega

@[simp] theorem createStageDebt_enqueue :
    createStageDebt (enqueue world command) key plan =
      createStageDebt world key plan := by
  simp [createStageDebt, startFenceCheck, vmPreparedCheck,
    slotConfiguredCheck, filePolicyAppliedCheck, vmExecutingCheck]

@[simp] theorem resumeStageDebt_enqueue :
    resumeStageDebt (enqueue world command) key plan =
      resumeStageDebt world key plan := by
  simp [resumeStageDebt]

@[simp] theorem pauseStageDebt_enqueue :
    pauseStageDebt (enqueue world command) plan =
      pauseStageDebt world plan := by
  have slotClosed : slotClosedCheck (enqueue world command) =
      slotClosedCheck world := by
    funext slotId
    simp [slotClosedCheck]
  simp [pauseStageDebt, startFenceCheck, vmFrozenOrClosedCheck,
    snapshotPresentCheck, snapshotDurableCheck, vmClosedCheck,
    slotClosed]

theorem workflowStageDebt_enqueue_normal_nonKill
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown) :
    workflowStageDebt workflow (enqueue world command) key =
      workflowStageDebt workflow world key := by
  rcases normal with normal | normal <;>
    cases workflow <;>
    simp_all [workflowStageDebt, normalStageDebt]

theorem workflowRank_enqueue_strict
    (unfinished : ¬ Finalized key world)
    (stageSame : workflowStageDebt workflow (enqueue world command) key =
      workflowStageDebt workflow world key)
    (foundPlan : world.operationPlans key = some plan)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (effectMissing : world.effects (effectId command) = none)
    (logicalMissing :
      world.logicalEffects (logicalEffectId command) = none)
    (origin : inferredEffectOrigin world command = .operation key)
    (retryLimit : plan.retryLimit = defaultRetryLimit) :
    workflowRank workflow key (enqueue world command) <
      workflowRank workflow key world := by
  have beforeChecked : finalizedCheck world key = false := by
    cases checked : finalizedCheck world key
    · rfl
    · exact (unfinished (finalizedCheck_spec.mp checked)).elim
  have afterChecked : finalizedCheck (enqueue world command) key = false := by
    simpa [finalizedCheck] using beforeChecked
  have effectStrict := operationEffectDebt_enqueue_strict foundPlan member
    effectMissing logicalMissing origin retryLimit
  simp only [workflowRank, beforeChecked, afterChecked, Bool.false_eq_true,
    ↓reduceIte]
  omega

/-- This packages the common create/resume/pause scheduler case: a normal-path
    command is journaled for the first time, while stage state itself is
    unchanged. -/
theorem workflowRank_normalSchedulerEnqueue_strict
    (unfinished : ¬ Finalized key world)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (effectMissing : world.effects (effectId command) = none)
    (logicalMissing :
      world.logicalEffects (logicalEffectId command) = none)
    (origin : inferredEffectOrigin world command = .operation key)
    (retryLimit : plan.retryLimit = defaultRetryLimit) :
    workflowRank workflow key (enqueue world command) <
      workflowRank workflow key world := by
  apply workflowRank_enqueue_strict unfinished
    (workflowStageDebt_enqueue_normal_nonKill notKill foundPlan foundLedger normal)
    foundPlan member effectMissing logicalMissing origin retryLimit

@[simp] theorem createStageDebt_dispatchEffect (id : EffectId) :
    createStageDebt (dispatchEffect world id) key plan =
      createStageDebt world key plan := by
  simp [createStageDebt, startFenceCheck, vmPreparedCheck,
    slotConfiguredCheck, filePolicyAppliedCheck, vmExecutingCheck]

@[simp] theorem resumeStageDebt_dispatchEffect (id : EffectId) :
    resumeStageDebt (dispatchEffect world id) key plan =
      resumeStageDebt world key plan := by
  simp [resumeStageDebt]

@[simp] theorem pauseStageDebt_dispatchEffect (id : EffectId) :
    pauseStageDebt (dispatchEffect world id) plan =
      pauseStageDebt world plan := by
  have slotClosed : slotClosedCheck (dispatchEffect world id) =
      slotClosedCheck world := by
    funext slotId
    rfl
  simp [pauseStageDebt, startFenceCheck, vmFrozenOrClosedCheck,
    snapshotPresentCheck, snapshotDurableCheck, vmClosedCheck, slotClosed]

theorem workflowStageDebt_dispatchEffect_normal_nonKill
    (id : EffectId)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown) :
    workflowStageDebt workflow (dispatchEffect world id) key =
      workflowStageDebt workflow world key := by
  rcases normal with normal | normal <;>
    cases workflow <;>
    simp_all [workflowStageDebt, normalStageDebt]

theorem workflowRank_normalEffectDispatch_strict
    (unfinishedOperation : ¬ Finalized key world)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinishedEffect : record.result = none)
    (origin : record.origin = .operation key)
    (dispatchable : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ = some .committed ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ =
        some .outcomeUnknown) :
    workflowRank workflow key
        (dispatchEffect world
          ⟨logicalEffectId command, record.currentAttempt⟩) <
      workflowRank workflow key world := by
  have beforeChecked : finalizedCheck world key = false := by
    cases checked : finalizedCheck world key
    · rfl
    · exact (unfinishedOperation (finalizedCheck_spec.mp checked)).elim
  have afterChecked : finalizedCheck
      (dispatchEffect world
        ⟨logicalEffectId command, record.currentAttempt⟩) key = false := by
    simpa [finalizedCheck] using beforeChecked
  have stageSame := workflowStageDebt_dispatchEffect_normal_nonKill
    ⟨logicalEffectId command, record.currentAttempt⟩ notKill foundPlan
    foundLedger normal
  have effectStrict := operationEffectDebt_dispatchEffect_strict foundPlan
    member found unfinishedEffect origin dispatchable
  simp only [workflowRank, beforeChecked, afterChecked, Bool.false_eq_true,
    ↓reduceIte]
  omega

@[simp] theorem createStageDebt_startNextAttempt
    (logical : LogicalEffectId) (failure : FailureKind) :
    createStageDebt (startNextAttempt world logical failure) key plan =
      createStageDebt world key plan := by
  simp [createStageDebt, startFenceCheck, vmPreparedCheck,
    slotConfiguredCheck, filePolicyAppliedCheck, vmExecutingCheck]

@[simp] theorem resumeStageDebt_startNextAttempt
    (logical : LogicalEffectId) (failure : FailureKind) :
    resumeStageDebt (startNextAttempt world logical failure) key plan =
      resumeStageDebt world key plan := by
  simp [resumeStageDebt]

@[simp] theorem pauseStageDebt_startNextAttempt
    (logical : LogicalEffectId) (failure : FailureKind) :
    pauseStageDebt (startNextAttempt world logical failure) plan =
      pauseStageDebt world plan := by
  have slotClosed :
      slotClosedCheck (startNextAttempt world logical failure) =
        slotClosedCheck world := by
    funext slotId
    rfl
  simp [pauseStageDebt, startFenceCheck, vmFrozenOrClosedCheck,
    snapshotPresentCheck, snapshotDurableCheck, vmClosedCheck, slotClosed]

theorem workflowStageDebt_startNextAttempt_normal_nonKill
    (logical : LogicalEffectId) (failure : FailureKind)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown) :
    workflowStageDebt workflow
        (startNextAttempt world logical failure) key =
      workflowStageDebt workflow world key := by
  rcases normal with normal | normal <;>
    cases workflow <;>
    simp_all [workflowStageDebt, normalStageDebt]

theorem workflowRank_startNextAttempt_strict
    (failure : FailureKind) (remaining : Nat)
    (unfinishedOperation : ¬ Finalized key world)
    (stageSame : workflowStageDebt workflow
        (startNextAttempt world (logicalEffectId command) failure) key =
      workflowStageDebt workflow world key)
    (foundPlan : world.operationPlans key = some plan)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinishedEffect : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure)))
    (retryable : failure.retryable = true)
    (budget : record.remainingRetries = remaining + 1)
    (fresh : world.effects
      ⟨logicalEffectId command, record.currentAttempt + 1⟩ = none) :
    workflowRank workflow key
        (startNextAttempt world (logicalEffectId command) failure) <
      workflowRank workflow key world := by
  have beforeChecked : finalizedCheck world key = false := by
    cases checked : finalizedCheck world key
    · rfl
    · exact (unfinishedOperation (finalizedCheck_spec.mp checked)).elim
  have afterChecked : finalizedCheck
      (startNextAttempt world (logicalEffectId command) failure) key = false := by
    simpa [finalizedCheck] using beforeChecked
  have effectStrict := operationEffectDebt_startNextAttempt_strict
    failure remaining foundPlan member found unfinishedEffect origin failed
    retryable budget fresh
  simp only [workflowRank, beforeChecked, afterChecked, Bool.false_eq_true,
    ↓reduceIte]
  omega

theorem workflowRank_normalEffectRetry_strict
    (failure : FailureKind) (remaining : Nat)
    (unfinishedOperation : ¬ Finalized key world)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinishedEffect : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure)))
    (retryable : failure.retryable = true)
    (budget : record.remainingRetries = remaining + 1)
    (fresh : world.effects
      ⟨logicalEffectId command, record.currentAttempt + 1⟩ = none) :
    workflowRank workflow key
        (startNextAttempt world (logicalEffectId command) failure) <
      workflowRank workflow key world := by
  apply workflowRank_startNextAttempt_strict failure remaining
    unfinishedOperation
    (workflowStageDebt_startNextAttempt_normal_nonKill
      (logicalEffectId command) failure notKill foundPlan foundLedger normal)
    foundPlan member found unfinishedEffect origin failed retryable budget fresh

@[simp] theorem createStageDebt_finalizeLogicalFailure
    (logical : LogicalEffectId) (failure : FailureKind) :
    createStageDebt (finalizeLogicalFailure world logical failure) key plan =
      createStageDebt world key plan := by
  simp [createStageDebt, startFenceCheck, vmPreparedCheck,
    slotConfiguredCheck, filePolicyAppliedCheck, vmExecutingCheck]

@[simp] theorem resumeStageDebt_finalizeLogicalFailure
    (logical : LogicalEffectId) (failure : FailureKind) :
    resumeStageDebt (finalizeLogicalFailure world logical failure) key plan =
      resumeStageDebt world key plan := by
  simp [resumeStageDebt]

@[simp] theorem pauseStageDebt_finalizeLogicalFailure
    (logical : LogicalEffectId) (failure : FailureKind) :
    pauseStageDebt (finalizeLogicalFailure world logical failure) plan =
      pauseStageDebt world plan := by
  have slotClosed :
      slotClosedCheck (finalizeLogicalFailure world logical failure) =
        slotClosedCheck world := by
    funext slotId
    rfl
  simp [pauseStageDebt, startFenceCheck, vmFrozenOrClosedCheck,
    snapshotPresentCheck, snapshotDurableCheck, vmClosedCheck, slotClosed]

theorem workflowStageDebt_finalizeLogicalFailure_normal_nonKill
    (logical : LogicalEffectId) (failure : FailureKind)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown) :
    workflowStageDebt workflow
        (finalizeLogicalFailure world logical failure) key =
      workflowStageDebt workflow world key := by
  rcases normal with normal | normal <;>
    cases workflow <;>
    simp_all [workflowStageDebt, normalStageDebt]

theorem workflowRank_finalizeLogicalFailure_strict
    (failure : FailureKind)
    (unfinishedOperation : ¬ Finalized key world)
    (stageSame : workflowStageDebt workflow
        (finalizeLogicalFailure world (logicalEffectId command) failure) key =
      workflowStageDebt workflow world key)
    (foundPlan : world.operationPlans key = some plan)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinishedEffect : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure))) :
    workflowRank workflow key
        (finalizeLogicalFailure world (logicalEffectId command) failure) <
      workflowRank workflow key world := by
  have beforeChecked : finalizedCheck world key = false := by
    cases checked : finalizedCheck world key
    · rfl
    · exact (unfinishedOperation (finalizedCheck_spec.mp checked)).elim
  have afterChecked : finalizedCheck
      (finalizeLogicalFailure world (logicalEffectId command) failure) key =
        false := by
    simpa [finalizedCheck] using beforeChecked
  have effectStrict := operationEffectDebt_finalizeLogicalFailure_strict
    failure foundPlan member found unfinishedEffect origin failed
  simp only [workflowRank, beforeChecked, afterChecked, Bool.false_eq_true,
    ↓reduceIte]
  omega

theorem workflowRank_normalEffectFinalize_strict
    (failure : FailureKind)
    (unfinishedOperation : ¬ Finalized key world)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (member : .command command ∈
      workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinishedEffect : record.result = none)
    (origin : record.origin = .operation key)
    (failed : world.effects
      ⟨logicalEffectId command, record.currentAttempt⟩ =
        some (.acknowledged (.failed failure))) :
    workflowRank workflow key
        (finalizeLogicalFailure world (logicalEffectId command) failure) <
      workflowRank workflow key world := by
  apply workflowRank_finalizeLogicalFailure_strict failure unfinishedOperation
    (workflowStageDebt_finalizeLogicalFailure_normal_nonKill
      (logicalEffectId command) failure notKill foundPlan foundLedger normal)
    foundPlan member found unfinishedEffect origin failed

theorem beginCompensation_operationLedger
    (failure : FailureKind)
    (found : world.operations (operationKey sid operation.id) = some ledger) :
    (beginCompensation world sid record operation failure).operations
        (operationKey sid operation.id) =
      some { ledger with
        operation := { operation with status := .compensating failure }
        result := none } := by
  simp [beginCompensation, updateOperation, found]

theorem beginCompensation_stageDebt_le_upperBound
    (failure : FailureKind)
    (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
    (foundLedger : world.operations (operationKey sid operation.id) = some ledger) :
    workflowStageDebt workflow
        (beginCompensation world sid record operation failure)
        (operationKey sid operation.id) ≤ cleanupStageUpperBound plan := by
  have afterLedger := beginCompensation_operationLedger
    (world := world) (sid := sid) (record := record) (operation := operation)
    (failure := failure) foundLedger
  simp only [workflowStageDebt, beginCompensation_operationPlans, foundPlan,
    afterLedger]
  exact cleanupStageDebt_le_upperBound

theorem beginCompensation_effectDebt_le_upperBound
    (failure : FailureKind)
    (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
    (foundLedger : world.operations (operationKey sid operation.id) = some ledger)
    (bounded : ∀ logical logicalRecord,
      (beginCompensation world sid record operation failure).logicalEffects
          logical = some logicalRecord →
      logicalRecord.origin = .operation (operationKey sid operation.id) →
      logicalRecord.remainingRetries ≤ plan.retryLimit) :
    operationEffectDebt workflow
        (beginCompensation world sid record operation failure)
        (operationKey sid operation.id) ≤ cleanupEffectUpperBound plan := by
  let after := beginCompensation world sid record operation failure
  let compensatingLedger : OperationLedgerRecord :=
    { ledger with
      operation := { operation with status := .compensating failure }
      result := none }
  have afterPlan : after.operationPlans (operationKey sid operation.id) =
      some plan := by
    simp [after, foundPlan]
  have afterLedger : after.operations (operationKey sid operation.id) =
      some compensatingLedger := by
    simpa [after, compensatingLedger] using
      (beginCompensation_operationLedger failure
        (world := world) (sid := sid) (record := record)
        (operation := operation) foundLedger)
  apply operationEffectDebt_compensating_le_upperBound failure afterPlan
    afterLedger
  · rfl
  · exact bounded

theorem beginCompensation_preserves_retryBound
    (failure : FailureKind) (plan : OperationPlan)
    (planDefault : plan.retryLimit = defaultRetryLimit)
    (bounded : ∀ logical logicalRecord,
      world.logicalEffects logical = some logicalRecord →
      logicalRecord.origin = .operation (operationKey sid operation.id) →
      logicalRecord.remainingRetries ≤ plan.retryLimit) :
    ∀ logical logicalRecord,
      (beginCompensation world sid record operation failure).logicalEffects
          logical = some logicalRecord →
      logicalRecord.origin = .operation (operationKey sid operation.id) →
      logicalRecord.remainingRetries ≤ plan.retryLimit := by
  let compensating := { operation with status := .compensating failure }
  let updated := { record with
    phase := .revoking
    terminal := true
    desired := .closed
    effective := .closed
    currentOperation := some compensating }
  let base := updateOperation (setSandbox world sid updated) sid
    compensating none
  have baseLogical : base.logicalEffects = world.logicalEffects := rfl
  have preserved := enqueue_preserves_retryBound
    (world := base)
    (command := .installFence sid record.epoch true)
    (key := operationKey sid operation.id) plan planDefault
    (fun logical logicalRecord found origin =>
      bounded logical logicalRecord (by simpa [baseLogical] using found) origin)
  simpa [beginCompensation, compensating, updated, base] using preserved

theorem workflowRank_beginCompensation_strict
    (failure : FailureKind)
    (unfinishedOperation :
      ¬ Finalized (operationKey sid operation.id) world)
    (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
    (foundLedger : world.operations (operationKey sid operation.id) = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (bounded : ∀ logical logicalRecord,
      (beginCompensation world sid record operation failure).logicalEffects
          logical = some logicalRecord →
      logicalRecord.origin = .operation (operationKey sid operation.id) →
      logicalRecord.remainingRetries ≤ plan.retryLimit) :
    workflowRank workflow (operationKey sid operation.id)
        (beginCompensation world sid record operation failure) <
      workflowRank workflow (operationKey sid operation.id) world := by
  let key := operationKey sid operation.id
  let after := beginCompensation world sid record operation failure
  have beforeChecked : finalizedCheck world key = false := by
    cases checked : finalizedCheck world key
    · rfl
    · exact (unfinishedOperation (finalizedCheck_spec.mp checked)).elim
  have afterLedger := beginCompensation_operationLedger failure
    (world := world) (sid := sid) (record := record) (operation := operation)
    foundLedger
  have afterChecked : finalizedCheck after key = false := by
    simp [after, key, finalizedCheck, afterLedger]
  have beforeStage : workflowStageDebt workflow world key =
      normalStageDebt workflow world key plan + 1 +
        compensationUpperBound plan := by
    rcases normal with normal | normal <;>
      simp [workflowStageDebt, key, foundPlan, foundLedger, normal]
  have afterStage : workflowStageDebt workflow after key ≤
      cleanupStageUpperBound plan := by
    simpa [after, key] using
      (beginCompensation_stageDebt_le_upperBound failure
        (workflow := workflow) foundPlan foundLedger)
  have afterEffects : operationEffectDebt workflow after key ≤
      cleanupEffectUpperBound plan := by
    simpa [after, key] using
      (beginCompensation_effectDebt_le_upperBound failure
        (workflow := workflow) foundPlan foundLedger bounded)
  change workflowRank workflow key after < workflowRank workflow key world
  simp only [workflowRank, beforeChecked, afterChecked, Bool.false_eq_true,
    ↓reduceIte]
  rw [beforeStage]
  unfold compensationUpperBound
  omega

theorem workflowRank_beginCompensation_strict_of_bounded
    (failure : FailureKind)
    (unfinishedOperation :
      ¬ Finalized (operationKey sid operation.id) world)
    (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
    (foundLedger : world.operations (operationKey sid operation.id) = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (planDefault : plan.retryLimit = defaultRetryLimit)
    (bounded : ∀ logical logicalRecord,
      world.logicalEffects logical = some logicalRecord →
      logicalRecord.origin = .operation (operationKey sid operation.id) →
      logicalRecord.remainingRetries ≤ plan.retryLimit) :
    workflowRank workflow (operationKey sid operation.id)
        (beginCompensation world sid record operation failure) <
      workflowRank workflow (operationKey sid operation.id) world := by
  apply workflowRank_beginCompensation_strict failure unfinishedOperation
    foundPlan foundLedger normal
  exact beginCompensation_preserves_retryBound failure plan planDefault bounded

theorem updateCurrentOperation_finalized
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (owned : ledger.sandbox = sid) :
    Finalized (operationKey sid operation.id)
      (updateCurrentOperation world sid (some operation) (some result)) := by
  refine ⟨{ ledger with operation := operation, result := some result },
    result, ?_, rfl⟩
  refine ⟨?_, owned, rfl⟩
  simp [updateCurrentOperation, found]

theorem publishRunning_finalized
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (owned : ledger.sandbox = sid) :
    Finalized (operationKey sid operation.id)
      (publishRunning world sid record slotId slot) := by
  unfold publishRunning
  dsimp only
  simp only [current, Option.map_some]
  apply updateCurrentOperation_finalized
  · simpa using found
  · exact owned

theorem publishPaused_finalized
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (owned : ledger.sandbox = sid) :
    Finalized (operationKey sid operation.id)
      (publishPaused world sid record source) := by
  unfold publishPaused
  dsimp only
  simp only [current, Option.map_some]
  apply updateCurrentOperation_finalized
  · simpa using found
  · exact owned

theorem publishDestroyed_finalized
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (owned : ledger.sandbox = sid) :
    Finalized (operationKey sid operation.id)
      (publishDestroyed world sid record) := by
  unfold publishDestroyed
  dsimp only
  simp only [current, Option.map_some]
  apply updateCurrentOperation_finalized
  · simpa using found
  · exact owned

theorem publishFailure_finalized
    (failure : FailureKind)
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (owned : ledger.sandbox = sid) :
    Finalized (operationKey sid operation.id)
      (publishFailure world sid record failure) := by
  unfold publishFailure
  dsimp only
  simp only [current, Option.map_some]
  apply updateCurrentOperation_finalized
  · simpa using found
  · exact owned

theorem workflowRank_publishRunning_zero
    (workflow : WorkflowKind)
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (owned : ledger.sandbox = sid) :
    workflowRank workflow (operationKey sid operation.id)
      (publishRunning world sid record slotId slot) = 0 :=
  workflowRank_final (publishRunning_finalized current found owned)

theorem workflowRank_publishPaused_zero
    (workflow : WorkflowKind)
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (owned : ledger.sandbox = sid) :
    workflowRank workflow (operationKey sid operation.id)
      (publishPaused world sid record source) = 0 :=
  workflowRank_final (publishPaused_finalized current found owned)

theorem workflowRank_publishDestroyed_zero
    (workflow : WorkflowKind)
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (owned : ledger.sandbox = sid) :
    workflowRank workflow (operationKey sid operation.id)
      (publishDestroyed world sid record) = 0 :=
  workflowRank_final (publishDestroyed_finalized current found owned)

theorem workflowRank_publishFailure_zero
    (workflow : WorkflowKind)
    (failure : FailureKind)
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (owned : ledger.sandbox = sid) :
    workflowRank workflow (operationKey sid operation.id)
      (publishFailure world sid record failure) = 0 :=
  workflowRank_final (publishFailure_finalized failure current found owned)

/-- Every legal running-publication step completes the exact operation whose
    identity is persisted in the sandbox record. -/
theorem step_publishRunning_finalized
    (transition : Step world (.publishRunning sid) next) :
    ∃ operation : Operation,
      Finalized (operationKey sid operation.id) next := by
  cases transition
  case publishRunning record vmId vm slot slotId operation ledger starting live
      currentVm executing currentSlot currentOperation up foundVm foundSlot
      vmIdentity slotReady operationMatches foundSandbox exclusiveNetwork
      appliedPolicy fenceSucceeded prepareSucceeded networkSucceeded
      activationSucceeded policyAcknowledged persistedOperation fenced =>
    exact ⟨operation, publishRunning_finalized currentOperation
      persistedOperation operationMatches.1⟩

theorem step_publishPaused_finalized
    (snapshotId : SnapshotId)
    (transition : Step world (.publishPaused sid snapshotId) next) :
    ∃ operation : Operation,
      Finalized (operationKey sid operation.id) next := by
  cases transition
  case publishPaused record snapshot vmId operation ledger source pausing
      currentVm currentOperation up foundSnapshot snapshotReady operationMatches
      foundSandbox fenced fenceSucceeded freezeSucceeded sealSucceeded
      commitSucceeded stopSucceeded vmsClosed networkClosed persistedOperation =>
    exact ⟨operation, publishPaused_finalized currentOperation
      persistedOperation operationMatches.1⟩

theorem step_publishDestroyed_finalized
    (transition : Step world (.publishDestroyed sid) next) :
    ∃ operation : Operation,
      Finalized (operationKey sid operation.id) next := by
  cases transition
  case publishDestroyed record operation ledger revoking terminal
      currentOperation killOperation unfinished up operationMatches found fenced
      fenceSucceeded vmsClosed stopsSucceeded networkClosed persistedOperation =>
    exact ⟨operation, publishDestroyed_finalized currentOperation
      persistedOperation operationMatches.1⟩

theorem step_publishFailure_finalized
    (failure : FailureKind)
    (transition : Step world (.publishFailure sid failure) next) :
    ∃ operation : Operation,
      Finalized (operationKey sid operation.id) next := by
  cases transition
  case publishFailure record operation ledger revoking terminal currentOperation
      recoverableKind up operationMatches found fenced fenceSucceeded vmsClosed
      stopsSucceeded networkClosed persistedOperation compensating =>
    exact ⟨operation, publishFailure_finalized failure currentOperation
      persistedOperation operationMatches.1⟩

/-! ### Controller-owned local progress

The constructors below enumerate the controller transformations whose rank
effect is now proved in this module.  Consequently the temporal closure no
longer needs to postulate that serving one of these handlers lowers the rank;
it only has to connect a selected invocation to one of these concrete cases
and bound interference before that invocation is served. -/

def ControllerRankOutcome (workflow : WorkflowKind) (key : OperationKey)
    (before after : World) : Prop :=
  Finalized key after ∨
    workflowRank workflow key after < workflowRank workflow key before

inductive ControllerOwnedTransition (workflow : WorkflowKind)
    (key : OperationKey) : World → World → Prop where
  | schedulerEnqueue
      (unfinished : ¬ Finalized key world)
      (notKill : workflow ≠ WorkflowKind.kill)
      (foundPlan : world.operationPlans key = some plan)
      (foundLedger : world.operations key = some ledger)
      (normal : ledger.operation.status = OperationStatus.pending ∨
        ledger.operation.status = OperationStatus.outcomeUnknown)
      (member : .command command ∈
        workflowExpectedEffects workflow world key plan)
      (effectMissing : world.effects (effectId command) = none)
      (logicalMissing :
        world.logicalEffects (logicalEffectId command) = none)
      (origin : inferredEffectOrigin world command = .operation key)
      (retryLimit : plan.retryLimit = defaultRetryLimit) :
      ControllerOwnedTransition workflow key world (enqueue world command)
  | effectDispatch
      (unfinishedOperation : ¬ Finalized key world)
      (notKill : workflow ≠ WorkflowKind.kill)
      (foundPlan : world.operationPlans key = some plan)
      (foundLedger : world.operations key = some ledger)
      (normal : ledger.operation.status = OperationStatus.pending ∨
        ledger.operation.status = OperationStatus.outcomeUnknown)
      (member : .command command ∈
        workflowExpectedEffects workflow world key plan)
      (found : world.logicalEffects (logicalEffectId command) = some record)
      (unfinishedEffect : record.result = none)
      (origin : record.origin = .operation key)
      (dispatchable : world.effects
        ⟨logicalEffectId command, record.currentAttempt⟩ = some .committed ∨
        world.effects ⟨logicalEffectId command, record.currentAttempt⟩ =
          some .outcomeUnknown) :
      ControllerOwnedTransition workflow key world
        (dispatchEffect world
          ⟨logicalEffectId command, record.currentAttempt⟩)
  | effectRetry
      (failure : FailureKind) (remaining : Nat)
      (unfinishedOperation : ¬ Finalized key world)
      (notKill : workflow ≠ WorkflowKind.kill)
      (foundPlan : world.operationPlans key = some plan)
      (foundLedger : world.operations key = some ledger)
      (normal : ledger.operation.status = OperationStatus.pending ∨
        ledger.operation.status = OperationStatus.outcomeUnknown)
      (member : .command command ∈
        workflowExpectedEffects workflow world key plan)
      (found : world.logicalEffects (logicalEffectId command) = some record)
      (unfinishedEffect : record.result = none)
      (origin : record.origin = .operation key)
      (failed : world.effects
        ⟨logicalEffectId command, record.currentAttempt⟩ =
          some (.acknowledged (.failed failure)))
      (retryable : failure.retryable = true)
      (budget : record.remainingRetries = remaining + 1)
      (fresh : world.effects
        ⟨logicalEffectId command, record.currentAttempt + 1⟩ = none) :
      ControllerOwnedTransition workflow key world
        (startNextAttempt world (logicalEffectId command) failure)
  | effectFinalize
      (failure : FailureKind)
      (unfinishedOperation : ¬ Finalized key world)
      (notKill : workflow ≠ WorkflowKind.kill)
      (foundPlan : world.operationPlans key = some plan)
      (foundLedger : world.operations key = some ledger)
      (normal : ledger.operation.status = OperationStatus.pending ∨
        ledger.operation.status = OperationStatus.outcomeUnknown)
      (member : .command command ∈
        workflowExpectedEffects workflow world key plan)
      (found : world.logicalEffects (logicalEffectId command) = some record)
      (unfinishedEffect : record.result = none)
      (origin : record.origin = .operation key)
      (failed : world.effects
        ⟨logicalEffectId command, record.currentAttempt⟩ =
          some (.acknowledged (.failed failure))) :
      ControllerOwnedTransition workflow key world
        (finalizeLogicalFailure world (logicalEffectId command) failure)
  | beginCompensation
      (failure : FailureKind)
      (keyIdentity : key = operationKey sid operation.id)
      (unfinishedOperation : ¬ Finalized key world)
      (foundPlan : world.operationPlans key = some plan)
      (foundLedger : world.operations key = some ledger)
      (normal : ledger.operation.status = OperationStatus.pending ∨
        ledger.operation.status = OperationStatus.outcomeUnknown)
      (planDefault : plan.retryLimit = defaultRetryLimit)
      (bounded : ∀ logical logicalRecord,
        world.logicalEffects logical = some logicalRecord →
        logicalRecord.origin = .operation key →
        logicalRecord.remainingRetries ≤ plan.retryLimit) :
      ControllerOwnedTransition workflow key world
        (SandboxModel.beginCompensation world sid record operation failure)
  | publishRunning
      (keyIdentity : key = operationKey sid operation.id)
      (current : record.currentOperation = some operation)
      (found : world.operations (operationKey sid operation.id) = some ledger)
      (owned : ledger.sandbox = sid) :
      ControllerOwnedTransition workflow key world
        (SandboxModel.publishRunning world sid record slotId slot)
  | publishPaused
      (keyIdentity : key = operationKey sid operation.id)
      (current : record.currentOperation = some operation)
      (found : world.operations (operationKey sid operation.id) = some ledger)
      (owned : ledger.sandbox = sid) :
      ControllerOwnedTransition workflow key world
        (SandboxModel.publishPaused world sid record source)
  | publishDestroyed
      (keyIdentity : key = operationKey sid operation.id)
      (current : record.currentOperation = some operation)
      (found : world.operations (operationKey sid operation.id) = some ledger)
      (owned : ledger.sandbox = sid) :
      ControllerOwnedTransition workflow key world
        (SandboxModel.publishDestroyed world sid record)
  | publishFailure
      (failure : FailureKind)
      (keyIdentity : key = operationKey sid operation.id)
      (current : record.currentOperation = some operation)
      (found : world.operations (operationKey sid operation.id) = some ledger)
      (owned : ledger.sandbox = sid) :
      ControllerOwnedTransition workflow key world
        (SandboxModel.publishFailure world sid record failure)

theorem controllerOwnedTransition_rankOutcome
    (transition : ControllerOwnedTransition workflow key before after) :
    ControllerRankOutcome workflow key before after := by
  cases transition with
  | schedulerEnqueue unfinished notKill foundPlan foundLedger normal member
      effectMissing logicalMissing origin retryLimit =>
      exact Or.inr (workflowRank_normalSchedulerEnqueue_strict unfinished
        notKill foundPlan foundLedger normal member effectMissing logicalMissing
        origin retryLimit)
  | effectDispatch unfinishedOperation notKill foundPlan foundLedger normal
      member found unfinishedEffect origin dispatchable =>
      exact Or.inr (workflowRank_normalEffectDispatch_strict
        unfinishedOperation notKill foundPlan foundLedger normal member found
        unfinishedEffect origin dispatchable)
  | effectRetry failure remaining unfinishedOperation notKill foundPlan
      foundLedger normal member found unfinishedEffect origin failed retryable
      budget fresh =>
      exact Or.inr (workflowRank_normalEffectRetry_strict failure remaining
        unfinishedOperation notKill foundPlan foundLedger normal member found
        unfinishedEffect origin failed retryable budget fresh)
  | effectFinalize failure unfinishedOperation notKill foundPlan foundLedger
      normal member found unfinishedEffect origin failed =>
      exact Or.inr (workflowRank_normalEffectFinalize_strict failure
        unfinishedOperation notKill foundPlan foundLedger normal member found
        unfinishedEffect origin failed)
  | beginCompensation failure keyIdentity unfinishedOperation foundPlan
      foundLedger normal planDefault bounded =>
      subst key
      exact Or.inr (workflowRank_beginCompensation_strict_of_bounded failure
        unfinishedOperation foundPlan foundLedger normal planDefault bounded)
  | publishRunning keyIdentity current found owned =>
      subst key
      exact Or.inl (publishRunning_finalized current found owned)
  | publishPaused keyIdentity current found owned =>
      subst key
      exact Or.inl (publishPaused_finalized current found owned)
  | publishDestroyed keyIdentity current found owned =>
      subst key
      exact Or.inl (publishDestroyed_finalized current found owned)
  | publishFailure failure keyIdentity current found owned =>
      subst key
      exact Or.inl (publishFailure_finalized failure current found owned)

/-- A served controller action is either one of the transformations whose
    local rank effect is proved above, or an explicitly uncovered residual
    branch.  The residual constructor is disjoint from all covered cases, so
    callers cannot silently replace an enqueue/retry/finalize/publication
    proof with the old blanket strict-progress assumption. -/
inductive WorkflowServiceProgress (workflow : WorkflowKind)
    (key : OperationKey) (baseline before after : World) : Prop where
  | controller
      (transition : ControllerOwnedTransition workflow key before after)
      (bridge : Finalized key after ∨
        workflowRank workflow key before ≤
          workflowRank workflow key baseline) :
      WorkflowServiceProgress workflow key baseline before after
  | residual
      (notOwned : ¬ ControllerOwnedTransition workflow key before after)
      (strict : workflowRank workflow key after <
        workflowRank workflow key baseline) :
      WorkflowServiceProgress workflow key baseline before after

theorem WorkflowServiceProgress.strict
    (progress : WorkflowServiceProgress workflow key baseline before after)
    (baselineUnfinished : ¬ Finalized key baseline) :
    workflowRank workflow key after <
      workflowRank workflow key baseline := by
  cases progress with
  | controller transition bridge =>
      rcases controllerOwnedTransition_rankOutcome transition with
        finalized | locallyDecreased
      · have zero := workflowRank_final (workflow := workflow) finalized
        have positive := workflowRank_unfinished (workflow := workflow)
          baselineUnfinished
        omega
      · rcases bridge with finalized | bounded
        · have zero := workflowRank_final (workflow := workflow) finalized
          have positive := workflowRank_unfinished (workflow := workflow)
            baselineUnfinished
          omega
        · omega
  | residual notOwned strict => exact strict


/-! ### Explicit environment boundary -/

/-- The controller and its durable journal stay available throughout the
    suffix on which weak fairness is applied. -/
def ControllerStableFrom (trace : Trace) (start : Nat) : Prop :=
  AlwaysFrom trace start (fun world => world.controllerUp = true)

/-- A deployment may enter the stable suffix later than request acceptance.
    Per-workflow contracts below carry that stabilization time explicitly. -/
def ControllerEventuallyStable (trace : Trace) (acceptedAt : Nat) : Prop :=
  ∃ stableAt, acceptedAt ≤ stableAt ∧ ControllerStableFrom trace stableAt

def AttemptReceiptPresent (world : World) (id : EffectId) : Prop :=
  ∃ outcome, world.effects id = some (.acknowledged outcome)

/-- Adapter response says only that an already-dispatched exact attempt obtains
    a structured receipt (success or explicit failure), unless the operation
    has meanwhile finalized.  It does not assume success. -/
def AdapterEventuallyReplies (trace : Trace) (key : OperationKey)
    (start : Nat) : Prop :=
  ∀ time logical record,
    start ≤ time →
    (trace.state time).logicalEffects logical = some record →
    record.origin = .operation key →
    let id : EffectId := ⟨logical, record.currentAttempt⟩
    ((trace.state time).effects id = some .dispatched ∨
      (trace.state time).effects id = some .outcomeUnknown) →
    EventuallyFrom trace time (fun world =>
      AttemptReceiptPresent world id ∨ Finalized key world)

/-- Retry fuel is finite protocol data and may never be replenished above the
    limit persisted at acceptance. -/
def OperationLedgerHasDefaultPlan (world : World) : Prop :=
  ∀ key ledger, world.operations key = some ledger →
    ∃ plan, world.operationPlans key = some plan ∧
      plan.retryLimit = defaultRetryLimit

theorem operationLedgerHasDefaultPlan_emptyWorld :
    OperationLedgerHasDefaultPlan emptyWorld := by
  simp [OperationLedgerHasDefaultPlan, emptyWorld]

private theorem operationLedgerHasDefaultPlan_updateOperation
    (invariant : OperationLedgerHasDefaultPlan world) :
    OperationLedgerHasDefaultPlan
      (updateOperation world sid operation result) := by
  intro key ledger found
  by_cases same : key = operationKey sid operation.id
  · subst key
    simp only [updateOperation, ↓reduceIte] at found
    cases oldFound : world.operations (operationKey sid operation.id) with
    | none => simp [oldFound] at found
    | some oldLedger =>
        rcases invariant _ oldLedger oldFound with ⟨plan, planFound, retry⟩
        exact ⟨plan, planFound, retry⟩
  · have oldFound : world.operations key = some ledger := by
      simpa [updateOperation, same] using found
    exact invariant key ledger oldFound

private theorem operationLedgerHasDefaultPlan_supersedeCurrentOperation
    (invariant : OperationLedgerHasDefaultPlan world) :
    OperationLedgerHasDefaultPlan
      (supersedeCurrentOperation world sid current) := by
  cases current with
  | none => simpa [supersedeCurrentOperation] using invariant
  | some operation =>
      cases status : operation.status with
      | pending =>
          simp only [supersedeCurrentOperation, status]
          cases found : world.operations (operationKey sid operation.id) with
          | none => exact invariant
          | some ledger =>
              cases final : ledger.result with
              | some result => simpa [final] using invariant
              | none =>
                  by_cases sameOperation : ledger.operation = operation
                  · simpa [final, sameOperation] using
                      (operationLedgerHasDefaultPlan_updateOperation
                        (world := world) (sid := sid)
                        (operation := { operation with
                          status := .failed .cancelled })
                        (result := some (.failed .cancelled)) invariant)
                  · simpa [final, sameOperation] using invariant
      | outcomeUnknown =>
          simp only [supersedeCurrentOperation, status]
          cases found : world.operations (operationKey sid operation.id) with
          | none => exact invariant
          | some ledger =>
              cases final : ledger.result with
              | some result => simpa [final] using invariant
              | none =>
                  by_cases sameOperation : ledger.operation = operation
                  · simpa [final, sameOperation] using
                      (operationLedgerHasDefaultPlan_updateOperation
                        (world := world) (sid := sid)
                        (operation := { operation with
                          status := .failed .cancelled })
                        (result := some (.failed .cancelled)) invariant)
                  · simpa [final, sameOperation] using invariant
      | compensating failure =>
          simpa [supersedeCurrentOperation, status] using invariant
      | succeeded =>
          simpa [supersedeCurrentOperation, status] using invariant
      | failed failure =>
          simpa [supersedeCurrentOperation, status] using invariant

private theorem operationLedgerHasDefaultPlan_storeNewOperation
    (invariant : OperationLedgerHasDefaultPlan world) :
    OperationLedgerHasDefaultPlan
      (storeNewOperation world sid operation input) := by
  intro key ledger found
  by_cases same : key = operationKey sid operation.id
  · subst key
    refine ⟨initialOperationPlan world sid operation input, ?_, ?_⟩
    · simp [storeNewOperation, setOperation, put]
    · simp [initialOperationPlan]
  · have oldFound : world.operations key = some ledger := by
      simpa [storeNewOperation, setOperation, put, same] using found
    rcases invariant key ledger oldFound with ⟨plan, planFound, retry⟩
    exact ⟨plan,
      by simpa [storeNewOperation, setOperationPlan, put, same] using planFound,
      retry⟩

private theorem operationLedgerHasDefaultPlan_enqueue
    (invariant : OperationLedgerHasDefaultPlan world) :
    OperationLedgerHasDefaultPlan (enqueue world command) := by
  simpa [OperationLedgerHasDefaultPlan] using invariant

set_option maxHeartbeats 1000000 in
theorem step_preserves_operationLedgerHasDefaultPlan
    (invariant : OperationLedgerHasDefaultPlan world)
    (transition : Step world event next) :
    OperationLedgerHasDefaultPlan next := by
  cases transition
  case acceptCreate =>
    simp only [beginCreate]
    apply operationLedgerHasDefaultPlan_enqueue
    apply operationLedgerHasDefaultPlan_storeNewOperation
    simpa [OperationLedgerHasDefaultPlan] using invariant
  case acceptPause =>
    simp only [beginPause]
    apply operationLedgerHasDefaultPlan_enqueue
    apply operationLedgerHasDefaultPlan_enqueue
    apply operationLedgerHasDefaultPlan_storeNewOperation
    simpa [OperationLedgerHasDefaultPlan] using invariant
  case acceptResume =>
    simp only [beginResume]
    apply operationLedgerHasDefaultPlan_enqueue
    apply operationLedgerHasDefaultPlan_storeNewOperation
    simpa [OperationLedgerHasDefaultPlan] using invariant
  case acceptKill =>
    simp only [beginKill]
    apply operationLedgerHasDefaultPlan_enqueue
    apply operationLedgerHasDefaultPlan_storeNewOperation
    simpa [OperationLedgerHasDefaultPlan] using
      (operationLedgerHasDefaultPlan_supersedeCurrentOperation invariant)
  all_goals
    simp_all [OperationLedgerHasDefaultPlan, beginCreate, beginPause,
      beginResume, beginKill, beginCompensation, storeNewOperation,
      initialOperationPlan, supersedeCurrentOperation, updateOperation,
      updateCurrentOperation, publishRunning, publishPaused,
      publishDestroyed, publishFailure, markOutcomeUnknown,
      reserveStartupResources, reservePauseSnapshot, setOperationPlan,
      setOperation, setSandbox, setFilesystem, enqueue, put] <;>
    grind

theorem reachable_operationLedgerHasDefaultPlan
    (reachable : Reachable world) : OperationLedgerHasDefaultPlan world := by
  induction reachable with
  | initial => exact operationLedgerHasDefaultPlan_emptyWorld
  | step prior transition inductionHypothesis =>
      exact step_preserves_operationLedgerHasDefaultPlan inductionHypothesis
        transition

/-- The retry counter is created at the shared default limit and the only
    transition which changes it consumes one unit. -/
def LogicalRetriesBoundedByDefault (world : World) : Prop :=
  ∀ logical record, world.logicalEffects logical = some record →
    record.remainingRetries ≤ defaultRetryLimit

theorem logicalRetriesBoundedByDefault_emptyWorld :
    LogicalRetriesBoundedByDefault emptyWorld := by
  simp [LogicalRetriesBoundedByDefault, emptyWorld]

private theorem logicalRetriesBoundedByDefault_of_logicalEffects_eq
    (same : after.logicalEffects = before.logicalEffects)
    (invariant : LogicalRetriesBoundedByDefault before) :
    LogicalRetriesBoundedByDefault after := by
  intro logical record found
  apply invariant logical record
  simpa [same] using found

private theorem logicalRetriesBoundedByDefault_commitEffect
    (invariant : LogicalRetriesBoundedByDefault world) :
    LogicalRetriesBoundedByDefault (commitEffect world command) := by
  intro logical record found
  cases effectFound : world.effects (effectId command) with
  | some status =>
      apply invariant logical record
      simpa [commitEffect, effectFound] using found
  | none =>
      cases logicalFound : world.logicalEffects (logicalEffectId command) with
      | some oldRecord =>
          apply invariant logical record
          simpa [commitEffect, effectFound, logicalFound, setEffect] using found
      | none =>
          by_cases same : logical = logicalEffectId command
          · subst logical
            simp [commitEffect, effectFound, logicalFound, setEffect,
              setLogicalEffect, put] at found
            subst record
            simp
          · apply invariant logical record
            simpa [commitEffect, effectFound, logicalFound, setEffect,
              setLogicalEffect, put, same] using found

private theorem logicalRetriesBoundedByDefault_enqueue
    (invariant : LogicalRetriesBoundedByDefault world) :
    LogicalRetriesBoundedByDefault (enqueue world command) := by
  simpa [LogicalRetriesBoundedByDefault, enqueue] using
    (logicalRetriesBoundedByDefault_commitEffect
      (world := world) (command := command) invariant)

private theorem supersedeCurrentOperation_logicalEffects :
    (supersedeCurrentOperation world sid current).logicalEffects =
      world.logicalEffects := by
  cases current with
  | none => rfl
  | some operation =>
      cases status : operation.status with
      | pending =>
          cases found : world.operations (operationKey sid operation.id) with
          | none => simp [supersedeCurrentOperation, status, found]
          | some ledger =>
              cases final : ledger.result with
              | some result =>
                  simp [supersedeCurrentOperation, status, found, final]
              | none =>
                  by_cases sameOperation : ledger.operation = operation <;>
                    simp [supersedeCurrentOperation, status, found, final,
                      sameOperation, updateOperation]
      | outcomeUnknown =>
          cases found : world.operations (operationKey sid operation.id) with
          | none => simp [supersedeCurrentOperation, status, found]
          | some ledger =>
              cases final : ledger.result with
              | some result =>
                  simp [supersedeCurrentOperation, status, found, final]
              | none =>
                  by_cases sameOperation : ledger.operation = operation <;>
                    simp [supersedeCurrentOperation, status, found, final,
                      sameOperation, updateOperation]
      | compensating failure => simp [supersedeCurrentOperation, status]
      | succeeded => simp [supersedeCurrentOperation, status]
      | failed failure => simp [supersedeCurrentOperation, status]

private theorem logicalRetriesBoundedByDefault_acknowledgeEffect
    (id : EffectId) (outcome : EffectOutcome)
    (invariant : LogicalRetriesBoundedByDefault world) :
    LogicalRetriesBoundedByDefault
      (acknowledgeEffect world id outcome) := by
  intro logical record found
  cases oldFound : world.logicalEffects id.logical with
  | none =>
      apply invariant logical record
      simpa [acknowledgeEffect, oldFound] using found
  | some oldRecord =>
      by_cases current : oldRecord.currentAttempt = id.attempt
      · cases outcome <;>
          simp [acknowledgeEffect, oldFound, current, put] at found
        · by_cases same : logical = id.logical
          · subst logical
            simp at found
            subst record
            exact invariant id.logical oldRecord oldFound
          · exact invariant logical record (by simpa [same] using found)
        · exact invariant logical record found
      · apply invariant logical record
        simpa [acknowledgeEffect, oldFound, current] using found

private theorem logicalRetriesBoundedByDefault_startNextAttempt
    (logical : LogicalEffectId) (failure : FailureKind)
    (invariant : LogicalRetriesBoundedByDefault world) :
    LogicalRetriesBoundedByDefault
      (startNextAttempt world logical failure) := by
  cases nextFound : nextAttemptData world logical failure with
  | none =>
      simpa [startNextAttempt, nextFound] using invariant
  | some nextAndRecord =>
      rcases nextAndRecord with ⟨next, nextRecord⟩
      have decreased : ∃ oldRecord,
          world.logicalEffects logical = some oldRecord ∧
            nextRecord.remainingRetries < oldRecord.remainingRetries := by
        unfold nextAttemptData at nextFound
        split at nextFound <;> simp_all
        split at nextFound <;> simp_all
        split at nextFound <;> simp_all
        split at nextFound <;> simp_all
        grind
      rcases decreased with ⟨oldRecord, oldFound, decreased⟩
      intro candidate record found
      by_cases same : candidate = logical
      · subst candidate
        simp [startNextAttempt, nextFound, put] at found
        subst record
        have oldBound := invariant logical oldRecord oldFound
        omega
      · apply invariant candidate record
        simpa [startNextAttempt, nextFound, put, same] using found

private theorem logicalRetriesBoundedByDefault_finalizeLogicalFailure
    (logical : LogicalEffectId) (failure : FailureKind)
    (invariant : LogicalRetriesBoundedByDefault world) :
    LogicalRetriesBoundedByDefault
      (finalizeLogicalFailure world logical failure) := by
  cases oldFound : world.logicalEffects logical with
  | none =>
      simpa [finalizeLogicalFailure, oldFound] using invariant
  | some oldRecord =>
      cases effectFound : world.effects ⟨logical, oldRecord.currentAttempt⟩ with
      | none =>
          simpa [finalizeLogicalFailure, oldFound, effectFound] using invariant
      | some status =>
          cases status with
          | committed =>
              simpa [finalizeLogicalFailure, oldFound, effectFound] using invariant
          | dispatched =>
              simpa [finalizeLogicalFailure, oldFound, effectFound] using invariant
          | outcomeUnknown =>
              simpa [finalizeLogicalFailure, oldFound, effectFound] using invariant
          | acknowledged outcome =>
              cases outcome with
              | succeeded result =>
                  simpa [finalizeLogicalFailure, oldFound, effectFound] using invariant
              | failed observed =>
                  by_cases sameFailure : observed = failure
                  · intro candidate record found
                    by_cases same : candidate = logical
                    · subst candidate
                      simp [finalizeLogicalFailure, oldFound, effectFound,
                        sameFailure, put] at found
                      subst record
                      exact invariant logical oldRecord oldFound
                    · apply invariant candidate record
                      simpa [finalizeLogicalFailure, oldFound, effectFound,
                        sameFailure, put, same] using found
                  · simpa [finalizeLogicalFailure, oldFound, effectFound,
                      sameFailure] using invariant

set_option maxHeartbeats 1000000 in
theorem step_preserves_logicalRetriesBoundedByDefault
    (invariant : LogicalRetriesBoundedByDefault world)
    (transition : Step world event next) :
    LogicalRetriesBoundedByDefault next := by
  cases transition
  case acceptCreate =>
    simp only [beginCreate]
    apply logicalRetriesBoundedByDefault_enqueue
    apply logicalRetriesBoundedByDefault_of_logicalEffects_eq
      (by simp [storeNewOperation, setOperation, setOperationPlan,
        setSandbox, setFilesystem]) invariant
  case acceptPause =>
    simp only [beginPause]
    apply logicalRetriesBoundedByDefault_enqueue
    apply logicalRetriesBoundedByDefault_enqueue
    apply logicalRetriesBoundedByDefault_of_logicalEffects_eq
      (by simp [storeNewOperation, setOperation, setOperationPlan,
        setSandbox]) invariant
  case acceptResume =>
    simp only [beginResume]
    apply logicalRetriesBoundedByDefault_enqueue
    apply logicalRetriesBoundedByDefault_of_logicalEffects_eq
      (by simp [storeNewOperation, setOperation, setOperationPlan,
        setSandbox]) invariant
  case acceptKill =>
    simp only [beginKill]
    apply logicalRetriesBoundedByDefault_enqueue
    apply logicalRetriesBoundedByDefault_of_logicalEffects_eq
      (by simpa [storeNewOperation, setOperation, setOperationPlan,
        setSandbox] using
          (supersedeCurrentOperation_logicalEffects
            (world := world) (sid := _) (current := _))) invariant
  case acknowledgeEffect =>
    exact logicalRetriesBoundedByDefault_acknowledgeEffect _ _ invariant
  case startNextAttempt =>
    exact logicalRetriesBoundedByDefault_startNextAttempt _ _ invariant
  case finalizeEffectFailure =>
    exact logicalRetriesBoundedByDefault_finalizeLogicalFailure _ _ invariant
  case dispatchPrepare =>
    exact logicalRetriesBoundedByDefault_enqueue invariant
  case prepareVm =>
    simp only [prepareResources]
    apply logicalRetriesBoundedByDefault_enqueue
    apply logicalRetriesBoundedByDefault_enqueue
    apply logicalRetriesBoundedByDefault_enqueue
    apply logicalRetriesBoundedByDefault_of_logicalEffects_eq
      (by simp [setSandbox, setNetSlot, setVm]) invariant
  case scheduleSealSnapshot =>
    exact logicalRetriesBoundedByDefault_enqueue invariant
  case scheduleCommitSnapshot =>
    exact logicalRetriesBoundedByDefault_enqueue invariant
  case commitSnapshot =>
    apply logicalRetriesBoundedByDefault_enqueue
    apply logicalRetriesBoundedByDefault_of_logicalEffects_eq
      (by simp [makeSnapshotDurable, setSnapshot]) invariant
  case scheduleStopVm =>
    exact logicalRetriesBoundedByDefault_enqueue invariant
  case scheduleCleanupVm =>
    exact logicalRetriesBoundedByDefault_enqueue invariant
  case scheduleCleanupSlot =>
    exact logicalRetriesBoundedByDefault_enqueue invariant
  case beginCompensation =>
    simp only [SandboxModel.beginCompensation]
    apply logicalRetriesBoundedByDefault_enqueue
    apply logicalRetriesBoundedByDefault_of_logicalEffects_eq
      (by simp [updateOperation, setSandbox]) invariant
  case installRevocationFence =>
    apply logicalRetriesBoundedByDefault_of_logicalEffects_eq ?_ invariant
    rename_i sid record _ _ _ _ _ _
    cases slotChoice : record.netSlot with
    | none => simp [applyRevocationFence, slotChoice, setFence, setSandbox]
    | some slotId =>
        cases slotFound : world.netSlots slotId <;>
          simp [applyRevocationFence, slotChoice, slotFound, setFence,
            setSandbox, setNetSlot]
  all_goals
    apply logicalRetriesBoundedByDefault_of_logicalEffects_eq
      (by first
          | rfl
          | simp [markOutcomeUnknown, dispatchEffect, applyStartFence,
              configureSlot, applyFilePolicyResult, activateResource,
              sealSnapshot, stopResource, reclaimVm, reclaimSlot,
              publishRunning, publishPaused, publishDestroyed,
              publishFailure, updateCurrentOperation, updateOperation,
              setSandbox, setVm, setNetSlot, setSnapshot, setFence,
              setEffect, setAppliedFilePolicy, reserveStartupResources,
              reservePauseSnapshot]) invariant

theorem reachable_logicalRetriesBoundedByDefault
    (reachable : Reachable world) : LogicalRetriesBoundedByDefault world := by
  induction reachable with
  | initial => exact logicalRetriesBoundedByDefault_emptyWorld
  | step prior transition inductionHypothesis =>
      exact step_preserves_logicalRetriesBoundedByDefault inductionHypothesis
        transition

/-- Every operation-owned logical effect continues to point at a durable
    operation ledger entry.  This is the missing ancestry fact needed to turn
    the global default retry bound into the bound stored in that operation's
    immutable plan. -/
def LogicalOperationOriginsHaveLedger (world : World) : Prop :=
  ∀ logical record key,
    world.logicalEffects logical = some record →
    record.origin = .operation key →
    ∃ ledger, world.operations key = some ledger

theorem logicalOperationOriginsHaveLedger_emptyWorld :
    LogicalOperationOriginsHaveLedger emptyWorld := by
  simp [LogicalOperationOriginsHaveLedger, emptyWorld]

private theorem inferredEffectOrigin_operation_hasLedger
    (currentValid : CurrentOperationInLedger world)
    (origin : inferredEffectOrigin world command = .operation key) :
    ∃ ledger, world.operations key = some ledger := by
  unfold inferredEffectOrigin at origin
  cases epochFound : command.epoch? with
  | none => simp [epochFound] at origin
  | some epoch =>
      cases sandboxFound : world.sandboxes command.sandbox with
      | none => simp [epochFound, sandboxFound] at origin
      | some record =>
          cases currentFound : record.currentOperation with
          | none => simp [epochFound, sandboxFound, currentFound] at origin
          | some operation =>
              by_cases epochMatches : operation.epoch = epoch
              · have origin' := origin
                simp [epochFound, sandboxFound, currentFound, epochMatches,
                  operationKey] at origin'
                have keyIdentity :
                    operationKey command.sandbox operation.id = key := by
                  simpa [operationKey] using origin'
                rcases currentValid command.sandbox record operation sandboxFound
                    currentFound with ⟨ledger, found⟩
                subst key
                exact ⟨ledger, found⟩
              · simp [epochFound, sandboxFound, currentFound, epochMatches] at origin

private theorem logicalOperationOriginsHaveLedger_enqueueFrom
    (logicalSame : base.logicalEffects = before.logicalEffects)
    (recordPersistent : OperationRecordPersistent before
      (enqueue base command))
    (newOriginLedger : ∀ key,
      inferredEffectOrigin base command = .operation key →
      ∃ ledger, (enqueue base command).operations key = some ledger)
    (invariant : LogicalOperationOriginsHaveLedger before) :
    LogicalOperationOriginsHaveLedger (enqueue base command) := by
  intro logical record key found origin
  cases effectFound : base.effects (effectId command) with
  | some status =>
      have oldFound : before.logicalEffects logical = some record := by
        simpa [enqueue, commitEffect, effectFound, logicalSame] using found
      rcases invariant logical record key oldFound origin with
        ⟨oldLedger, oldLedgerFound⟩
      exact recordPersistent key oldLedger oldLedgerFound
  | none =>
      cases logicalFound : base.logicalEffects (logicalEffectId command) with
      | some oldRecord =>
          have oldFound : before.logicalEffects logical = some record := by
            simpa [enqueue, commitEffect, effectFound, logicalFound, setEffect,
              logicalSame] using found
          rcases invariant logical record key oldFound origin with
            ⟨oldLedger, oldLedgerFound⟩
          exact recordPersistent key oldLedger oldLedgerFound
      | none =>
          by_cases same : logical = logicalEffectId command
          · subst logical
            simp [enqueue, commitEffect, effectFound, logicalFound, setEffect,
              setLogicalEffect, put] at found
            subst record
            exact newOriginLedger key origin
          · have oldFound : before.logicalEffects logical = some record := by
              simpa [enqueue, commitEffect, effectFound, logicalFound, setEffect,
                setLogicalEffect, put, same, logicalSame] using found
            rcases invariant logical record key oldFound origin with
              ⟨oldLedger, oldLedgerFound⟩
            exact recordPersistent key oldLedger oldLedgerFound

private theorem logicalOperationOriginsHaveLedger_enqueue
    (currentValid : CurrentOperationInLedger world)
    (invariant : LogicalOperationOriginsHaveLedger world) :
    LogicalOperationOriginsHaveLedger (enqueue world command) := by
  apply logicalOperationOriginsHaveLedger_enqueueFrom
    (before := world) (base := world) rfl
  · intro key oldLedger found
    exact ⟨oldLedger, by simpa using found⟩
  · intro key origin
    rcases inferredEffectOrigin_operation_hasLedger currentValid origin with
      ⟨ledger, found⟩
    exact ⟨ledger, by simpa using found⟩
  · exact invariant

def RetryBudgetBounded (trace : Trace) (key : OperationKey)
    (start : Nat) : Prop :=
  AlwaysFrom trace start fun world =>
    ∀ logical record,
      world.logicalEffects logical = some record →
      record.origin = .operation key →
      ∃ plan, world.operationPlans key = some plan ∧
        record.remainingRetries ≤ plan.retryLimit

theorem traceEdge_preserves_reachable
    (reachable : Reachable before)
    (edge : TraceEdge before action after) : Reachable after := by
  cases edge with
  | idle => exact reachable
  | protocol transition => exact .step reachable transition

theorem reachable_state_on_trace
    (trace : Trace) (start : Nat)
    (reachable : Reachable (trace.state start)) :
    ∀ later, start ≤ later → Reachable (trace.state later) := by
  intro later ordered
  rcases Nat.le.dest ordered with ⟨distance, rfl⟩
  induction distance with
  | zero => simpa using reachable
  | succ distance inductionHypothesis =>
      have advanced := traceEdge_preserves_reachable
        (inductionHypothesis (by omega)) (trace.edge (start + distance))
      simpa only [Nat.add_succ] using advanced

/-- Retry boundedness is a reachable-state theorem, not an adapter or
    deployment assumption.  The accepted operation ledger persists, every
    ledger has its default-limit plan, and retry transitions only consume the
    globally shared default budget. -/
theorem retryBudgetBounded_of_reachable
    (accepted : OperationAcceptedAs (trace.state start) key workflow)
    (reachable : Reachable (trace.state start)) :
    RetryBudgetBounded trace key start := by
  intro time startLe logical record found _
  have reachableLater := reachable_state_on_trace trace start reachable time
    startLe
  have acceptedLater := operationAcceptedAs_persistent_on_trace trace start
    accepted time startLe
  rcases acceptedLater with
    ⟨ledger, ⟨ledgerFound, _, _⟩, _, _⟩
  rcases reachable_operationLedgerHasDefaultPlan reachableLater key ledger
      ledgerFound with ⟨plan, planFound, planDefault⟩
  have retryBound := reachable_logicalRetriesBoundedByDefault reachableLater
    logical record found
  exact ⟨plan, planFound, by simpa [planDefault] using retryBound⟩

/-- This is the state required by `publishDestroyed` or `publishFailure` after
    terminal cleanup. -/
def CleanupPublicationReady (world : World) (key : OperationKey) : Prop :=
  allVmsStoppedOrReclaimed world key.sandbox ∧
    allVmStopsSucceeded world key.sandbox ∧ noActiveSlot world key.sandbox

/-- Stop/fence/network cleanup is a stronger boundary than ordinary adapter
    response: safety publication is forbidden until these actions really
    converge.  Permanent stop failure therefore violates this premise instead
    of being hidden by scheduler fairness. -/
def SafetyCleanupEventuallyConverges (trace : Trace) (key : OperationKey)
    (start : Nat) : Prop :=
  ∀ time record operation,
    start ≤ time →
    (trace.state time).sandboxes key.sandbox = some record →
    record.currentOperation = some operation →
    operation.id = key.operation →
    record.phase = .revoking → record.terminal = true →
    EventuallyFrom trace time (fun world =>
      CleanupPublicationReady world key ∨ Finalized key world)

/-! ### Durable allocation

Resource identities are selected by the controller from journaled monotone
cursors and written into the operation plan by one reservation step.  A work
item names only the operation key, not a transient candidate identity; the
handler reads the current cursor when it is actually served. -/

def StartupReservationReady (world : World) (key : OperationKey) : Prop :=
  ∃ record operation plan,
    world.sandboxes key.sandbox = some record ∧
    record.phase = .starting ∧
    record.terminal = false ∧
    record.currentOperation = some operation ∧
    operation.id = key.operation ∧
    world.operationPlans key = some plan ∧
    plan.key = key ∧
    plan.epoch = record.epoch ∧
    plan.generation = record.generation ∧
    plan.sealed = false ∧
    plan.vm = none ∧
    plan.slot = none ∧
    plan.restoreFrom = record.restoreFrom

def PauseReservationReady (world : World) (key : OperationKey) : Prop :=
  ∃ record operation plan vmId,
    world.sandboxes key.sandbox = some record ∧
    record.phase = .pausing ∧
    record.terminal = false ∧
    record.currentOperation = some operation ∧
    operation.id = key.operation ∧
    world.operationPlans key = some plan ∧
    plan.key = key ∧
    plan.epoch = record.epoch ∧
    plan.generation = record.generation ∧
    plan.sealed = false ∧
    plan.vm = none ∧
    plan.targetSnapshot = none ∧
    record.currentVm = some vmId

/-- Allocation is needed only while every durable guard of the corresponding
    reservation reducer is present.  In particular, an operation which has
    entered compensation no longer advertises startup allocation merely
    because its old plan remains unsealed. -/
def allocationNeeded (workflow : WorkflowKind) (world : World)
    (key : OperationKey) : Prop :=
  match workflow with
  | .create | .resume => StartupReservationReady world key
  | .pause => PauseReservationReady world key
  | .kill => False

def AllocationWorkFor (workflow : WorkflowKind) (key : OperationKey) :
    WorkItem → Prop
  | .scheduler (.reserveStartup candidate) =>
      (workflow = WorkflowKind.create ∨ workflow = WorkflowKind.resume) ∧
        candidate = key
  | .scheduler (.reservePauseSnapshot candidate) =>
      workflow = WorkflowKind.pause ∧ candidate = key
  | _ => False

def NotServedBefore (trace : Trace) (work : WorkItem)
    (beginAt endAt : Nat) : Prop :=
  ∀ time, beginAt ≤ time → time < endAt → ¬ WorkServedAt trace work time

/-- No state up to `endAt` has already consumed progress relative to the
    rank at `beginAt`.  A chosen handler is only required to stay enabled
    while this holds: a receipt or another controller action may legitimately
    make that handler obsolete by lowering the rank first. -/
def NoRankDropBefore (trace : Trace) (rank : World → Nat)
    (beginAt endAt : Nat) : Prop :=
  ∀ time, beginAt ≤ time → time ≤ endAt →
    rank (trace.state beginAt) ≤ rank (trace.state time)

/-- Until allocation is atomically written into the operation plan, one
    key-addressed reservation invocation remains enabled.  The theorem which
    derives this property from reachability and cursor freshness lives with the
    allocation invariants; this definition is the trace-level interface used
    by the generic completion proof. -/
def AllocatorProvidesStableWork (workflow : WorkflowKind) (trace : Trace)
    (key : OperationKey) (start : Nat) : Prop :=
  ∀ time, start ≤ time → ¬ Finalized key (trace.state time) →
    allocationNeeded workflow (trace.state time) key →
    ∃ work, AllocationWorkFor workflow key work ∧
      ∀ later, time ≤ later → ¬ Finalized key (trace.state later) →
        NoRankDropBefore trace (workflowRank workflow key) time later →
        WorkEnabled work (trace.state later)

theorem kill_needs_no_allocator_reservation :
    AllocatorProvidesStableWork .kill trace key start := by
  intro time startLe unfinished needed
  exact needed.elim

theorem allocationCursorValid_freshVm
    (valid : AllocationCursorValid world) :
    world.vms ⟨world.nextVmId⟩ = none := by
  cases found : world.vms ⟨world.nextVmId⟩ with
  | none => rfl
  | some vm =>
      have below := valid.vmStore ⟨world.nextVmId⟩ vm found
      simp at below

theorem allocationCursorValid_freshSlot
    (valid : AllocationCursorValid world) :
    world.netSlots ⟨world.nextSlotId⟩ = none := by
  cases found : world.netSlots ⟨world.nextSlotId⟩ with
  | none => rfl
  | some slot =>
      have below := (valid.slotStore ⟨world.nextSlotId⟩ slot found).1
      simp at below

theorem allocationCursorValid_freshSnapshot
    (valid : AllocationCursorValid world) :
    world.snapshots ⟨world.nextSnapshotId⟩ = none := by
  cases found : world.snapshots ⟨world.nextSnapshotId⟩ with
  | none => rfl
  | some snapshot =>
      have below :=
        (valid.snapshotStore ⟨world.nextSnapshotId⟩ snapshot found).1
      simp at below

theorem startupReservationReady_workEnabled
    (ready : StartupReservationReady world key)
    (up : world.controllerUp = true)
    (valid : AllocationCursorValid world) :
    WorkEnabled (.scheduler (.reserveStartup key)) world := by
  rcases ready with
    ⟨record, operation, plan, found, starting, live, currentOperation,
      operationId, foundPlan, planKey, planEpoch, planGeneration, planOpen,
      planVmOpen, planSlotOpen, planSource⟩
  have sameKey : operationKey key.sandbox operation.id = key := by
    cases key
    simp [operationKey, operationId]
  have freshVm := allocationCursorValid_freshVm valid
  have freshSlot := allocationCursorValid_freshSlot valid
  have transition : Step world
      (.reserveStartupResources key ⟨world.nextVmId⟩
        ⟨world.nextSlotId⟩)
      (reserveStartupResources world key plan) := by
    simpa [sameKey] using Step.reserveStartupResources up found starting live
      currentOperation (by simpa [sameKey] using foundPlan)
      (by simpa [sameKey] using planKey) planEpoch planGeneration planOpen
      planVmOpen planSlotOpen planSource freshVm freshSlot
  rcases Controller.handleReserveStartup_complete transition with
    ⟨decision, accepted, eventIdentity, nextIdentity⟩
  exact ⟨.reserveStartupResources key ⟨world.nextVmId⟩
      ⟨world.nextSlotId⟩,
    reserveStartupResources world key plan, decision, accepted,
    eventIdentity, nextIdentity⟩

theorem pauseReservationReady_workEnabled
    (ready : PauseReservationReady world key)
    (up : world.controllerUp = true)
    (valid : AllocationCursorValid world) :
    WorkEnabled (.scheduler (.reservePauseSnapshot key)) world := by
  rcases ready with
    ⟨record, operation, plan, vmId, found, pausing, live, currentOperation,
      operationId, foundPlan, planKey, planEpoch, planGeneration, planOpen,
      planVmOpen, targetOpen, currentVm⟩
  have sameKey : operationKey key.sandbox operation.id = key := by
    cases key
    simp [operationKey, operationId]
  have freshSnapshot := allocationCursorValid_freshSnapshot valid
  have transition : Step world
      (.reservePauseSnapshot key ⟨world.nextSnapshotId⟩
        ⟨⟨world.nextManifestDigest⟩⟩)
      (reservePauseSnapshot world key plan vmId) := by
    simpa [sameKey] using Step.reservePauseSnapshot up found pausing live
      currentOperation (by simpa [sameKey] using foundPlan)
      (by simpa [sameKey] using planKey) planEpoch planGeneration planOpen
      planVmOpen targetOpen currentVm freshSnapshot
  rcases Controller.handleReservePauseSnapshot_complete transition with
    ⟨decision, accepted, eventIdentity, nextIdentity⟩
  exact ⟨.reservePauseSnapshot key ⟨world.nextSnapshotId⟩
      ⟨⟨world.nextManifestDigest⟩⟩,
    reservePauseSnapshot world key plan vmId, decision, accepted,
    eventIdentity, nextIdentity⟩

/-- Exact remaining model obligation for durable allocation.  It says only
    that the reducer's persisted guards survive interference which has not
    already lowered this workflow's rank.  Controller availability and cursor
    freshness are deliberately absent: both are proved separately below. -/
def ReservationGuardsPersistUnderNoRankDrop (workflow : WorkflowKind)
    (trace : Trace) (key : OperationKey) (start : Nat) : Prop :=
  ∀ time, start ≤ time → allocationNeeded workflow (trace.state time) key →
    ∀ later, time ≤ later → ¬ Finalized key (trace.state later) →
      NoRankDropBefore trace (workflowRank workflow key) time later →
      allocationNeeded workflow (trace.state later) key

theorem startup_allocatorProvidesStableWork
    (startupWorkflow : workflow = WorkflowKind.create ∨
      workflow = WorkflowKind.resume)
    (reachable : Reachable (trace.state start))
    (controllerStable : ControllerStableFrom trace start)
    (guards : ReservationGuardsPersistUnderNoRankDrop workflow trace key start) :
    AllocatorProvidesStableWork workflow trace key start := by
  intro time startLe unfinished needed
  refine ⟨.scheduler (.reserveStartup key), ?_, ?_⟩
  · simp [AllocationWorkFor, startupWorkflow]
  · intro later timeLeLater unfinishedLater noDrop
    have readyLater := guards time startLe needed later timeLeLater
      unfinishedLater noDrop
    have up := controllerStable later (Nat.le_trans startLe timeLeLater)
    have reachableLater := reachable_state_on_trace trace start reachable later
      (Nat.le_trans startLe timeLeLater)
    have cursorValid := reachable_allocationCursorValid reachableLater
    rcases startupWorkflow with rfl | rfl <;>
      exact startupReservationReady_workEnabled readyLater up cursorValid

theorem create_allocatorProvidesStableWork
    (reachable : Reachable (trace.state start))
    (controllerStable : ControllerStableFrom trace start)
    (guards : ReservationGuardsPersistUnderNoRankDrop .create trace key start) :
    AllocatorProvidesStableWork .create trace key start :=
  startup_allocatorProvidesStableWork (Or.inl rfl) reachable controllerStable
    guards

theorem resume_allocatorProvidesStableWork
    (reachable : Reachable (trace.state start))
    (controllerStable : ControllerStableFrom trace start)
    (guards : ReservationGuardsPersistUnderNoRankDrop .resume trace key start) :
    AllocatorProvidesStableWork .resume trace key start :=
  startup_allocatorProvidesStableWork (Or.inr rfl) reachable controllerStable
    guards

theorem pause_allocatorProvidesStableWork
    (reachable : Reachable (trace.state start))
    (controllerStable : ControllerStableFrom trace start)
    (guards : ReservationGuardsPersistUnderNoRankDrop .pause trace key start) :
    AllocatorProvidesStableWork .pause trace key start := by
  intro time startLe unfinished needed
  refine ⟨.scheduler (.reservePauseSnapshot key), ?_, ?_⟩
  · simp [AllocationWorkFor]
  · intro later timeLeLater unfinishedLater noDrop
    have readyLater := guards time startLe needed later timeLeLater
      unfinishedLater noDrop
    have up := controllerStable later (Nat.le_trans startLe timeLeLater)
    have reachableLater := reachable_state_on_trace trace start reachable later
      (Nat.le_trans startLe timeLeLater)
    exact pauseReservationReady_workEnabled readyLater up
      (reachable_allocationCursorValid reachableLater)

structure WorkflowEnvironment (trace : Trace) (key : OperationKey)
    (start : Nat) where
  fair : HandlerWeaklyFair trace
  controllerStable : ControllerStableFrom trace start
  adapterReplies : AdapterEventuallyReplies trace key start
  cleanupConverges : SafetyCleanupEventuallyConverges trace key start

/-! ### Controller-side workflow closure -/

/-- Local rank-decreasing work required at every unfinished state.  This is the
    exact non-circular controller obligation: it names an actual executable
    reducer, requires that reducer to remain enabled until its first service or
    finalization, and requires service to lower the concrete workflow rank. -/
def WorkflowAdvance (workflow : WorkflowKind) (trace : Trace)
    (key : OperationKey) (start : Nat) : Prop :=
  ∀ time, start ≤ time → ¬ Finalized key (trace.state time) →
    ∃ work,
      ControllerScheduledWork work ∧
      (∀ later, time ≤ later → ¬ Finalized key (trace.state later) →
        NoRankDropBefore trace (workflowRank workflow key) time later →
        WorkEnabled work (trace.state later)) ∧
      (∀ later, time ≤ later → ¬ Finalized key (trace.state later) →
        WorkServedAt trace work later →
        workflowRank workflow key (trace.state (later + 1)) <
          workflowRank workflow key (trace.state time))

/-- A selected controller invocation and its service evidence.  Covered
    transformations carry only a local transition plus the explicit
    interference bridge; an uncovered action must use the disjoint `residual`
    branch of `WorkflowServiceProgress`. -/
structure ControllerAdvanceCandidate (workflow : WorkflowKind) (trace : Trace)
    (key : OperationKey) (time : Nat) where
  work : WorkItem
  scheduled : ControllerScheduledWork work
  staysEnabled : ∀ later, time ≤ later →
    ¬ Finalized key (trace.state later) →
    NoRankDropBefore trace (workflowRank workflow key) time later →
    WorkEnabled work (trace.state later)
  serviceProgress : ∀ later, time ≤ later →
    ¬ Finalized key (trace.state later) →
    WorkServedAt trace work later →
    WorkflowServiceProgress workflow key (trace.state time) (trace.state later)
      (trace.state (later + 1))

/-- The remaining proof connects durable guards to a concrete controller
    invocation and controls unrelated transitions before it is served.  The
    rank effect of enqueue/retry/finalize/publication is no longer a field of
    this contract: `controllerOwnedTransition_rankOutcome` supplies it.  Only
    actions outside that enumeration retain an explicit residual strictness
    proof. -/
structure WorkflowProtocolClosure (workflow : WorkflowKind) (trace : Trace)
    (key : OperationKey) (start : Nat) where
  select : OperationAcceptedAs (trace.state start) key workflow →
    Reachable (trace.state start) →
    AllocatorProvidesStableWork workflow trace key start →
    ControllerStableFrom trace start →
    AdapterEventuallyReplies trace key start →
    SafetyCleanupEventuallyConverges trace key start →
    ∀ time, start ≤ time → ¬ Finalized key (trace.state time) →
      ControllerAdvanceCandidate workflow trace key time

theorem workflowAdvance_of_protocolClosure
    (accepted : OperationAcceptedAs (trace.state start) key workflow)
    (reachable : Reachable (trace.state start))
    (allocator : AllocatorProvidesStableWork workflow trace key start)
    (environment : WorkflowEnvironment trace key start)
    (closure : WorkflowProtocolClosure workflow trace key start) :
    WorkflowAdvance workflow trace key start := by
  intro time startLe unfinished
  let candidate := closure.select accepted reachable allocator
    environment.controllerStable environment.adapterReplies
    environment.cleanupConverges time startLe unfinished
  refine ⟨candidate.work, candidate.scheduled, candidate.staysEnabled, ?_⟩
  intro later timeLeLater unfinishedLater served
  exact (candidate.serviceProgress later timeLeLater unfinishedLater served).strict
    unfinished

theorem workflow_strictRankProgress
    (accepted : OperationAcceptedAs (trace.state start) key workflow)
    (reachable : Reachable (trace.state start))
    (allocator : AllocatorProvidesStableWork workflow trace key start)
    (environment : WorkflowEnvironment trace key start)
    (closure : WorkflowProtocolClosure workflow trace key start) :
    StrictRankProgress trace start (Finalized key)
      (workflowRank workflow key) := by
  intro time startLe unfinished
  by_cases reached : EventuallyFrom trace time (Finalized key)
  · rcases reached with ⟨later, timeLe, finalized⟩
    have different : later ≠ time := by
      intro same
      subst later
      exact unfinished finalized
    have zero := workflowRank_final (workflow := workflow) finalized
    have positive := workflowRank_unfinished (workflow := workflow) unfinished
    exact ⟨later, by omega, by omega⟩
  · by_cases lowered : ∃ later, time < later ∧
        workflowRank workflow key (trace.state later) <
          workflowRank workflow key (trace.state time)
    · exact lowered
    · have neverFinished : ∀ later, time ≤ later →
        ¬ Finalized key (trace.state later) := by
        intro later timeLeLater finalized
        exact reached ⟨later, timeLeLater, finalized⟩
      have neverLowered : ∀ later, time ≤ later →
          NoRankDropBefore trace (workflowRank workflow key) time later := by
        intro later timeLeLater middle timeLeMiddle middleLeLater
        have notStrict : ¬ workflowRank workflow key (trace.state middle) <
            workflowRank workflow key (trace.state time) := by
          intro strict
          by_cases same : middle = time
          · subst middle
            omega
          · exact lowered ⟨middle, by omega, strict⟩
        omega
      rcases (workflowAdvance_of_protocolClosure accepted reachable allocator
          environment closure) time startLe unfinished with
        ⟨work, scheduled, staysEnabled, rankDrops⟩
      have continuouslyEnabled :
          AlwaysFrom trace time (WorkEnabled work) := by
        intro later timeLeLater
        exact staysEnabled later timeLeLater
          (neverFinished later timeLeLater)
          (neverLowered later timeLeLater)
      rcases environment.fair work scheduled time continuouslyEnabled with
        ⟨later, timeLeLater, service⟩
      exact ⟨later + 1, by omega,
        rankDrops later timeLeLater
          (neverFinished later timeLeLater) service⟩

theorem workflow_eventually_always_finalized
    (accepted : OperationAcceptedAs (trace.state start) key workflow)
    (reachable : Reachable (trace.state start))
    (allocator : AllocatorProvidesStableWork workflow trace key start)
    (environment : WorkflowEnvironment trace key start)
    (closure : WorkflowProtocolClosure workflow trace key start) :
    EventuallyAlwaysFrom trace start (Finalized key) := by
  have progress := workflow_strictRankProgress accepted reachable allocator
    environment closure
  rcases operation_eventually_finalized_of_rank progress with
    ⟨time, startLe, finalized⟩
  exact ⟨time, startLe, finalized_stable_on_trace trace time finalized⟩

/-! ### Per-workflow completion theorems -/

structure CreateCompletionContract (trace : Trace) (key : OperationKey)
    (acceptedAt : Nat) where
  stableAt : Nat
  acceptedBeforeStable : acceptedAt ≤ stableAt
  accepted : OperationAcceptedAs (trace.state acceptedAt) key .create
  reachable : Reachable (trace.state acceptedAt)
  allocator : AllocatorProvidesStableWork .create trace key stableAt
  environment : WorkflowEnvironment trace key stableAt
  protocol : WorkflowProtocolClosure .create trace key stableAt

structure PauseCompletionContract (trace : Trace) (key : OperationKey)
    (acceptedAt : Nat) where
  stableAt : Nat
  acceptedBeforeStable : acceptedAt ≤ stableAt
  accepted : OperationAcceptedAs (trace.state acceptedAt) key .pause
  reachable : Reachable (trace.state acceptedAt)
  allocator : AllocatorProvidesStableWork .pause trace key stableAt
  environment : WorkflowEnvironment trace key stableAt
  protocol : WorkflowProtocolClosure .pause trace key stableAt

structure ResumeCompletionContract (trace : Trace) (key : OperationKey)
    (acceptedAt : Nat) where
  stableAt : Nat
  acceptedBeforeStable : acceptedAt ≤ stableAt
  accepted : OperationAcceptedAs (trace.state acceptedAt) key .resume
  reachable : Reachable (trace.state acceptedAt)
  allocator : AllocatorProvidesStableWork .resume trace key stableAt
  environment : WorkflowEnvironment trace key stableAt
  protocol : WorkflowProtocolClosure .resume trace key stableAt

structure KillCompletionContract (trace : Trace) (key : OperationKey)
    (acceptedAt : Nat) where
  stableAt : Nat
  acceptedBeforeStable : acceptedAt ≤ stableAt
  accepted : OperationAcceptedAs (trace.state acceptedAt) key .kill
  reachable : Reachable (trace.state acceptedAt)
  environment : WorkflowEnvironment trace key stableAt
  protocol : WorkflowProtocolClosure .kill trace key stableAt

def CreateCompletionContract.acceptedAtStable
    (contract : CreateCompletionContract trace key acceptedAt) :
    OperationAcceptedAs (trace.state contract.stableAt) key .create :=
  operationAcceptedAs_persistent_on_trace trace acceptedAt contract.accepted
    contract.stableAt contract.acceptedBeforeStable

def PauseCompletionContract.acceptedAtStable
    (contract : PauseCompletionContract trace key acceptedAt) :
    OperationAcceptedAs (trace.state contract.stableAt) key .pause :=
  operationAcceptedAs_persistent_on_trace trace acceptedAt contract.accepted
    contract.stableAt contract.acceptedBeforeStable

def ResumeCompletionContract.acceptedAtStable
    (contract : ResumeCompletionContract trace key acceptedAt) :
    OperationAcceptedAs (trace.state contract.stableAt) key .resume :=
  operationAcceptedAs_persistent_on_trace trace acceptedAt contract.accepted
    contract.stableAt contract.acceptedBeforeStable

def KillCompletionContract.acceptedAtStable
    (contract : KillCompletionContract trace key acceptedAt) :
    OperationAcceptedAs (trace.state contract.stableAt) key .kill :=
  operationAcceptedAs_persistent_on_trace trace acceptedAt contract.accepted
    contract.stableAt contract.acceptedBeforeStable

def CreateCompletionContract.reachableAtStable
    (contract : CreateCompletionContract trace key acceptedAt) :
    Reachable (trace.state contract.stableAt) :=
  reachable_state_on_trace trace acceptedAt contract.reachable
    contract.stableAt contract.acceptedBeforeStable

def PauseCompletionContract.reachableAtStable
    (contract : PauseCompletionContract trace key acceptedAt) :
    Reachable (trace.state contract.stableAt) :=
  reachable_state_on_trace trace acceptedAt contract.reachable
    contract.stableAt contract.acceptedBeforeStable

def ResumeCompletionContract.reachableAtStable
    (contract : ResumeCompletionContract trace key acceptedAt) :
    Reachable (trace.state contract.stableAt) :=
  reachable_state_on_trace trace acceptedAt contract.reachable
    contract.stableAt contract.acceptedBeforeStable

def KillCompletionContract.reachableAtStable
    (contract : KillCompletionContract trace key acceptedAt) :
    Reachable (trace.state contract.stableAt) :=
  reachable_state_on_trace trace acceptedAt contract.reachable
    contract.stableAt contract.acceptedBeforeStable

theorem create_eventually_always_finalized
  (contract : CreateCompletionContract trace key acceptedAt) :
    EventuallyAlwaysFrom trace acceptedAt (Finalized key) := by
  rcases workflow_eventually_always_finalized contract.acceptedAtStable
      contract.reachableAtStable contract.allocator contract.environment
      contract.protocol with
    ⟨time, stableLe, forever⟩
  exact ⟨time, Nat.le_trans contract.acceptedBeforeStable stableLe, forever⟩

theorem pause_eventually_always_finalized
  (contract : PauseCompletionContract trace key acceptedAt) :
    EventuallyAlwaysFrom trace acceptedAt (Finalized key) := by
  rcases workflow_eventually_always_finalized contract.acceptedAtStable
      contract.reachableAtStable contract.allocator contract.environment
      contract.protocol with
    ⟨time, stableLe, forever⟩
  exact ⟨time, Nat.le_trans contract.acceptedBeforeStable stableLe, forever⟩

theorem resume_eventually_always_finalized
  (contract : ResumeCompletionContract trace key acceptedAt) :
    EventuallyAlwaysFrom trace acceptedAt (Finalized key) := by
  rcases workflow_eventually_always_finalized contract.acceptedAtStable
      contract.reachableAtStable contract.allocator contract.environment
      contract.protocol with
    ⟨time, stableLe, forever⟩
  exact ⟨time, Nat.le_trans contract.acceptedBeforeStable stableLe, forever⟩

theorem kill_eventually_always_finalized
  (contract : KillCompletionContract trace key acceptedAt) :
    EventuallyAlwaysFrom trace acceptedAt (Finalized key) := by
  rcases workflow_eventually_always_finalized contract.acceptedAtStable
      contract.reachableAtStable kill_needs_no_allocator_reservation
      contract.environment
      contract.protocol with ⟨time, stableLe, forever⟩
  exact ⟨time, Nat.le_trans contract.acceptedBeforeStable stableLe, forever⟩

end SandboxModel
