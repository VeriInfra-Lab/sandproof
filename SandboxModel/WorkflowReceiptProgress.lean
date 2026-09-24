import SandboxModel.WorkflowLiveness
import SandboxModel.HandlerCompleteness

/-! Progress from actual receipts for the current logical-effect attempt.
Normal create/pause/resume operations are covered; kill and compensation
require separate cleanup-stage arguments. These are local decrease lemmas,
not a closed eventual-completion theorem or a machine-capacity guarantee. -/

namespace SandboxModel
open Liveness

theorem acknowledgeEffect_expectedCommand_strict
    (outcome : EffectOutcome)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (awaiting : world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .dispatched ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .outcomeUnknown) :
    expectedCommandDebt
        (acknowledgeEffect world ⟨logicalEffectId command, record.currentAttempt⟩ outcome)
        key plan command < expectedCommandDebt world key plan command := by
  rcases awaiting with awaiting | awaiting <;>
    cases outcome <;>
    simp [expectedCommandDebt, acknowledgeEffect, put, found, unfinished, origin,
      awaiting, attemptPositionDebt]

theorem acknowledgeEffect_expectedCommand_other
    (outcome : EffectOutcome)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (different : other ≠ command) :
    expectedCommandDebt
        (acknowledgeEffect world ⟨logicalEffectId command, record.currentAttempt⟩ outcome)
        key plan other = expectedCommandDebt world key plan other := by
  have logicalDifferent : logicalEffectId other ≠ logicalEffectId command := by
    simpa [logicalEffectId] using different
  cases outcome <;>
    simp [expectedCommandDebt, acknowledgeEffect, put, found, logicalDifferent]

theorem acknowledgeEffect_expectedEffectSlot_le
    (outcome : EffectOutcome)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (awaiting : world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .dispatched ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .outcomeUnknown) :
    ∀ slot, expectedEffectSlotDebt
        (acknowledgeEffect world ⟨logicalEffectId command, record.currentAttempt⟩ outcome)
        key plan slot ≤ expectedEffectSlotDebt world key plan slot := by
  intro slot
  cases slot with
  | future => exact Nat.le_refl _
  | command other =>
      by_cases same : other = command
      · subst other
        exact Nat.le_of_lt (acknowledgeEffect_expectedCommand_strict outcome found unfinished origin awaiting)
      · rw [expectedEffectSlotDebt, expectedEffectSlotDebt,
          acknowledgeEffect_expectedCommand_other outcome found same]
        exact Nat.le_refl _

@[simp] theorem workflowExpectedEffects_acknowledgeEffect (id : EffectId) (outcome : EffectOutcome) :
    workflowExpectedEffects workflow (acknowledgeEffect world id outcome) key plan =
      workflowExpectedEffects workflow world key plan := by
  simp [workflowExpectedEffects, normalExpectedEffects, createExpectedEffects, resumeExpectedEffects,
    killExpectedEffects, compensationExpectedEffects, startupExpectedEffects,
    pauseExpectedEffects, cleanupExpectedEffects]

theorem operationEffectDebt_acknowledgeEffect_strict
    (outcome : EffectOutcome)
    (foundPlan : world.operationPlans key = some plan)
    (member : .command command ∈ workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinished : record.result = none)
    (origin : record.origin = .operation key)
    (awaiting : world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .dispatched ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .outcomeUnknown) :
    operationEffectDebt workflow
        (acknowledgeEffect world ⟨logicalEffectId command, record.currentAttempt⟩ outcome) key <
      operationEffectDebt workflow world key := by
  unfold operationEffectDebt
  simp only [acknowledgeEffect_operationPlans, foundPlan, workflowExpectedEffects_acknowledgeEffect]
  apply sum_map_strict_of_member _ _ _ (.command command) member
  · intro slot _
    exact acknowledgeEffect_expectedEffectSlot_le outcome found unfinished origin awaiting slot
  · exact acknowledgeEffect_expectedCommand_strict outcome found unfinished origin awaiting

@[simp] theorem createStageDebt_acknowledgeEffect (id : EffectId) (outcome : EffectOutcome) :
    createStageDebt (acknowledgeEffect world id outcome) key plan = createStageDebt world key plan := by
  simp [createStageDebt, startFenceCheck, vmPreparedCheck, slotConfiguredCheck,
    filePolicyAppliedCheck, vmExecutingCheck]

@[simp] theorem resumeStageDebt_acknowledgeEffect (id : EffectId) (outcome : EffectOutcome) :
    resumeStageDebt (acknowledgeEffect world id outcome) key plan = resumeStageDebt world key plan := by
  simp [resumeStageDebt]

@[simp] theorem pauseStageDebt_acknowledgeEffect (id : EffectId) (outcome : EffectOutcome) :
    pauseStageDebt (acknowledgeEffect world id outcome) plan = pauseStageDebt world plan := by
  have slotClosed : slotClosedCheck (acknowledgeEffect world id outcome) = slotClosedCheck world := by
    funext slotId
    simp [slotClosedCheck]
  simp [pauseStageDebt, startFenceCheck, vmFrozenOrClosedCheck, snapshotPresentCheck,
    snapshotDurableCheck, vmClosedCheck, slotClosed]

theorem workflowStageDebt_acknowledgeEffect_normal_nonKill
    (id : EffectId) (outcome : EffectOutcome)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown) :
    workflowStageDebt workflow (acknowledgeEffect world id outcome) key =
      workflowStageDebt workflow world key := by
  rcases normal with normal | normal <;> cases workflow <;>
    simp_all [workflowStageDebt, normalStageDebt]

theorem workflowRank_normalEffectReceipt_strict
    (outcome : EffectOutcome)
    (unfinishedOperation : ¬ Finalized key world)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (member : .command command ∈ workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinishedEffect : record.result = none)
    (origin : record.origin = .operation key)
    (awaiting : world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .dispatched ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .outcomeUnknown) :
    workflowRank workflow key
        (acknowledgeEffect world ⟨logicalEffectId command, record.currentAttempt⟩ outcome) <
      workflowRank workflow key world := by
  have beforeChecked : finalizedCheck world key = false := by
    cases checked : finalizedCheck world key
    · rfl
    · exact (unfinishedOperation (finalizedCheck_spec.mp checked)).elim
  have afterChecked : finalizedCheck
      (acknowledgeEffect world ⟨logicalEffectId command, record.currentAttempt⟩ outcome) key = false := by
    simpa [finalizedCheck] using beforeChecked
  have stageSame := workflowStageDebt_acknowledgeEffect_normal_nonKill
    ⟨logicalEffectId command, record.currentAttempt⟩ outcome notKill foundPlan foundLedger normal
  have effectStrict := operationEffectDebt_acknowledgeEffect_strict outcome foundPlan member found
    unfinishedEffect origin awaiting
  simp only [workflowRank, beforeChecked, afterChecked, Bool.false_eq_true, ↓reduceIte]
  omega


/-- A receipt for a superseded attempt is not progress of the current command.
It can still be recorded by the protocol without decreasing this debt. -/
theorem acknowledgeEffect_stale_expectedCommand_eq
    (outcome : EffectOutcome) (attempt : Nat)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (stale : record.currentAttempt ≠ attempt) :
    expectedCommandDebt
        (acknowledgeEffect world ⟨logicalEffectId command, attempt⟩ outcome) key plan command =
      expectedCommandDebt world key plan command := by
  simp [expectedCommandDebt, acknowledgeEffect, found, stale, put]

/-- The local decrease is realized by the actual executable receipt handler,
not by an arbitrary transformation of the effect map. Receipt evidence is
checked by that handler; no success is inferred merely from delivery. -/
theorem handleEffectReceipt_normal_progress
    (outcome : EffectOutcome)
    (unfinishedOperation : ¬ Finalized key world)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (member : .command command ∈ workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinishedEffect : record.result = none)
    (origin : record.origin = .operation key)
    (awaiting : world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .dispatched ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .outcomeUnknown)
    (validOutcome : effectOutcomeValid world command outcome) :
    ∃ decision, Controller.handleEffect world
        (.receipt ⟨logicalEffectId command, record.currentAttempt⟩ outcome) = .ok decision ∧
      decision.event = .acknowledgeEffect ⟨logicalEffectId command, record.currentAttempt⟩ outcome ∧
      workflowRank workflow key decision.next < workflowRank workflow key world := by
  have legal := Step.acknowledgeEffect awaiting validOutcome
  obtain ⟨decision, accepted, eventSame, nextSame⟩ := Controller.handleEffectReceipt_complete legal
  refine ⟨decision, accepted, eventSame, ?_⟩
  rw [nextSame]
  exact workflowRank_normalEffectReceipt_strict outcome unfinishedOperation notKill foundPlan
    foundLedger normal member found unfinishedEffect origin awaiting

theorem currentReceipt_event_next
    (trace : Trace) (time : Nat) (id : EffectId) (outcome : EffectOutcome)
    (observed : EventAt trace time (.acknowledgeEffect id outcome))
    (awaiting : (trace.state time).effects id = some .dispatched ∨
      (trace.state time).effects id = some .outcomeUnknown) :
    trace.state (time + 1) = acknowledgeEffect (trace.state time) id outcome := by
  have transition := observed.2
  generalize afterEq : trace.state (time + 1) = after at transition ⊢
  cases transition with
  | acknowledgeEffect _ _ => rfl
  | replayEffectReceipt recorded =>
      rcases awaiting with awaiting | awaiting <;> simp [recorded] at awaiting

theorem normalReceiptEvent_rank_strict
    (trace : Trace) (time : Nat) (world : World)
    (stateAt : trace.state time = world)
    (workflow : WorkflowKind) (key : OperationKey) (plan : OperationPlan)
    (ledger : OperationLedgerRecord) (command : Command)
    (record : LogicalEffectRecord) (outcome : EffectOutcome)
    (observed : EventAt trace time
      (.acknowledgeEffect ⟨logicalEffectId command, record.currentAttempt⟩ outcome))
    (unfinishedOperation : ¬ Finalized key world)
    (notKill : workflow ≠ WorkflowKind.kill)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = OperationStatus.pending ∨
      ledger.operation.status = OperationStatus.outcomeUnknown)
    (member : .command command ∈ workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinishedEffect : record.result = none)
    (origin : record.origin = .operation key)
    (awaiting : world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .dispatched ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .outcomeUnknown) :
    workflowRank workflow key (trace.state (time + 1)) < workflowRank workflow key world := by
  have awaitingAt : (trace.state time).effects
      ⟨logicalEffectId command, record.currentAttempt⟩ = some .dispatched ∨
      (trace.state time).effects
      ⟨logicalEffectId command, record.currentAttempt⟩ = some .outcomeUnknown := by
    simpa [stateAt] using awaiting
  rw [currentReceipt_event_next trace time _ outcome observed awaitingAt, stateAt]
  exact workflowRank_normalEffectReceipt_strict outcome unfinishedOperation notKill
    foundPlan foundLedger normal member found unfinishedEffect origin awaiting

end SandboxModel
