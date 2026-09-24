import SandboxModel.WorkflowReceiptProgress

namespace SandboxModel
open Liveness

theorem currentEffectId_acknowledgeEffect_eq (world : World) (id : EffectId)
    (outcome : EffectOutcome) (command : Command) :
    currentEffectId (acknowledgeEffect world id outcome) command =
      currentEffectId world command := by
  by_cases same : logicalEffectId command = id.logical
  · cases found : world.logicalEffects id.logical with
    | none => simp [currentEffectId, acknowledgeEffect, same, found]
    | some record =>
        by_cases attempt : record.currentAttempt = id.attempt
        · cases outcome <;>
            simp [currentEffectId, acknowledgeEffect, same, found, attempt, put]
        · simp [currentEffectId, acknowledgeEffect, same, found, attempt]
  · cases found : world.logicalEffects id.logical with
    | none => simp [currentEffectId, acknowledgeEffect, same, found]
    | some record =>
        by_cases attempt : record.currentAttempt = id.attempt
        · cases outcome <;>
            simp [currentEffectId, acknowledgeEffect, same, found, attempt, put]
        · simp [currentEffectId, acknowledgeEffect, same, found, attempt]

theorem effectSucceededCheck_acknowledgeEffect_mono
    (world : World) (id : EffectId) (outcome : EffectOutcome)
    (awaiting : world.effects id = some .dispatched ∨
      world.effects id = some .outcomeUnknown)
    (command : Command)
    (succeeded : effectSucceededCheck world command = true) :
    effectSucceededCheck (acknowledgeEffect world id outcome) command = true := by
  have currentSame := currentEffectId_acknowledgeEffect_eq world id outcome command
  by_cases same : currentEffectId world command = id
  · rcases awaiting with awaiting | awaiting <;>
      simp [effectSucceededCheck, same, awaiting] at succeeded
  · unfold effectSucceededCheck at succeeded ⊢
    rw [currentSame]
    simpa [acknowledgeEffect, put, same] using succeeded

theorem stopReceiptCandidateCheck_acknowledgeEffect_mono
    (world : World) (id : EffectId) (outcome : EffectOutcome)
    (awaiting : world.effects id = some .dispatched ∨
      world.effects id = some .outcomeUnknown)
    (sid : SandboxId) (generation : Generation) (vmId : VmId)
    (candidate : EffectId)
    (checked : Controller.stopReceiptCandidateCheck world sid generation vmId candidate = true) :
    Controller.stopReceiptCandidateCheck (acknowledgeEffect world id outcome)
      sid generation vmId candidate = true := by
  cases command : candidate.command <;>
    simp [Controller.stopReceiptCandidateCheck, command] at checked ⊢
  case stopVm commandSid commandEpoch commandGeneration commandVm =>
    exact ⟨checked.1,
      effectSucceededCheck_acknowledgeEffect_mono world id outcome awaiting
        _ checked.2⟩

theorem any_stopReceiptCandidateCheck_acknowledgeEffect_mono
    (world : World) (id : EffectId) (outcome : EffectOutcome)
    (awaiting : world.effects id = some .dispatched ∨
      world.effects id = some .outcomeUnknown)
    (sid : SandboxId) (generation : Generation) (vmId : VmId)
    (checked : Controller.vmStopSucceededFiniteCheck world sid generation vmId = true) :
    Controller.vmStopSucceededFiniteCheck (acknowledgeEffect world id outcome)
      sid generation vmId = true := by
  unfold Controller.vmStopSucceededFiniteCheck at checked ⊢
  simp only [List.any_eq_true] at checked ⊢
  obtain ⟨candidate, member, passed⟩ := checked
  refine ⟨candidate, ?_, stopReceiptCandidateCheck_acknowledgeEffect_mono
    world id outcome awaiting sid generation vmId candidate passed⟩
  simp only [acknowledgeEffect, mem_insertUnique]
  exact Or.inr member

theorem stopReceiptCheck_acknowledgeEffect_mono
    (world : World) (id : EffectId) (outcome : EffectOutcome)
    (awaiting : world.effects id = some .dispatched ∨
      world.effects id = some .outcomeUnknown)
    (plan : OperationPlan) (vmId : VmId)
    (checked : stopReceiptCheck world plan vmId = true) :
    stopReceiptCheck (acknowledgeEffect world id outcome) plan vmId = true := by
  cases found : world.vms vmId with
  | none => simp [stopReceiptCheck, found]
  | some vm =>
      simp only [stopReceiptCheck, found] at checked
      simpa [stopReceiptCheck, found] using
        any_stopReceiptCandidateCheck_acknowledgeEffect_mono world id outcome
          awaiting plan.key.sandbox vm.generation vmId checked

theorem countDebt_le_of_ready_preserved (before after : α → Bool)
    (values : List α)
    (preserved : ∀ value, before value = true → after value = true) :
    countDebt after values ≤ countDebt before values := by
  induction values with
  | nil => simp [countDebt]
  | cons value rest ih =>
      have tail := ih
      have head : Bool.debt (after value) ≤ Bool.debt (before value) := by
        cases old : before value <;> cases new : after value <;>
          simp_all [Bool.debt]
      simp only [countDebt]
      omega

theorem plannedStopReceiptDebt_acknowledgeEffect_le
    (world : World) (id : EffectId) (outcome : EffectOutcome)
    (awaiting : world.effects id = some .dispatched ∨
      world.effects id = some .outcomeUnknown)
    (plan : OperationPlan) :
    plannedStopReceiptDebt (acknowledgeEffect world id outcome) plan ≤
      plannedStopReceiptDebt world plan := by
  unfold plannedStopReceiptDebt
  apply countDebt_le_of_ready_preserved
  intro vmId checked
  exact stopReceiptCheck_acknowledgeEffect_mono world id outcome awaiting
    plan vmId checked

@[simp] theorem plannedResourceDebt_acknowledgeEffect (world : World)
    (id : EffectId) (outcome : EffectOutcome) (plan : OperationPlan) :
    plannedResourceDebt (acknowledgeEffect world id outcome) plan =
      plannedResourceDebt world plan := by
  have vms : plannedVmClosedCheck (acknowledgeEffect world id outcome) =
      plannedVmClosedCheck world := by
    funext vmId
    simp [plannedVmClosedCheck]
  have slots : plannedSlotClosedCheck (acknowledgeEffect world id outcome) =
      plannedSlotClosedCheck world := by
    funext slotId
    simp [plannedSlotClosedCheck]
  simp [plannedResourceDebt, vms, slots]

@[simp] theorem terminalFenceCheck_acknowledgeEffect (world : World)
    (id : EffectId) (outcome : EffectOutcome) (plan : OperationPlan) :
    terminalFenceCheck (acknowledgeEffect world id outcome) plan =
      terminalFenceCheck world plan := by
  simp [terminalFenceCheck]

theorem cleanupStageDebt_acknowledgeEffect_le
    (world : World) (id : EffectId) (outcome : EffectOutcome)
    (awaiting : world.effects id = some .dispatched ∨
      world.effects id = some .outcomeUnknown)
    (plan : OperationPlan) :
    cleanupStageDebt (acknowledgeEffect world id outcome) plan ≤
      cleanupStageDebt world plan := by
  have receipts := plannedStopReceiptDebt_acknowledgeEffect_le world id outcome
    awaiting plan
  simp only [cleanupStageDebt, plannedResourceDebt_acknowledgeEffect,
    terminalFenceCheck_acknowledgeEffect]
  omega

theorem workflowStageDebt_acknowledgeEffect_compensating_le
    (world : World) (id : EffectId) (outcome : EffectOutcome)
    (awaiting : world.effects id = some .dispatched ∨
      world.effects id = some .outcomeUnknown)
    (workflow : WorkflowKind) (key : OperationKey) (plan : OperationPlan)
    (ledger : OperationLedgerRecord) (failure : FailureKind)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (compensating : ledger.operation.status = .compensating failure) :
    workflowStageDebt workflow (acknowledgeEffect world id outcome) key ≤
      workflowStageDebt workflow world key := by
  have cleanup := cleanupStageDebt_acknowledgeEffect_le world id outcome awaiting plan
  simp only [workflowStageDebt, acknowledgeEffect_operationPlans,
    acknowledgeEffect_operations, foundPlan, foundLedger, compensating]
  exact cleanup

theorem workflowStageDebt_acknowledgeEffect_killNormal_le
    (world : World) (id : EffectId) (outcome : EffectOutcome)
    (awaiting : world.effects id = some .dispatched ∨
      world.effects id = some .outcomeUnknown)
    (key : OperationKey) (plan : OperationPlan) (ledger : OperationLedgerRecord)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (normal : ledger.operation.status = .pending ∨
      ledger.operation.status = .outcomeUnknown) :
    workflowStageDebt .kill (acknowledgeEffect world id outcome) key ≤
      workflowStageDebt .kill world key := by
  have cleanup := cleanupStageDebt_acknowledgeEffect_le world id outcome awaiting plan
  rcases normal with normal | normal <;>
    simp only [workflowStageDebt, acknowledgeEffect_operationPlans,
      acknowledgeEffect_operations, foundPlan, foundLedger, normal,
      normalStageDebt] <;> omega

theorem workflowRank_acknowledgeEffect_strict_of_stage_le
    (world : World) (outcome : EffectOutcome)
    (workflow : WorkflowKind) (key : OperationKey) (plan : OperationPlan)
    (command : Command) (record : LogicalEffectRecord)
    (unfinishedOperation : ¬ Finalized key world)
    (foundPlan : world.operationPlans key = some plan)
    (member : .command command ∈ workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinishedEffect : record.result = none)
    (origin : record.origin = .operation key)
    (awaiting : world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .dispatched ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .outcomeUnknown)
    (stageLe : workflowStageDebt workflow
        (acknowledgeEffect world ⟨logicalEffectId command, record.currentAttempt⟩ outcome) key ≤
      workflowStageDebt workflow world key) :
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
  have effectStrict := operationEffectDebt_acknowledgeEffect_strict outcome foundPlan member
    found unfinishedEffect origin awaiting
  simp only [workflowRank, beforeChecked, afterChecked, Bool.false_eq_true, ↓reduceIte]
  omega

theorem workflowRank_cleanupEffectReceipt_strict
    (world : World) (outcome : EffectOutcome)
    (workflow : WorkflowKind) (key : OperationKey) (plan : OperationPlan)
    (ledger : OperationLedgerRecord) (command : Command)
    (record : LogicalEffectRecord)
    (unfinishedOperation : ¬ Finalized key world)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (cleanup : (∃ failure, ledger.operation.status = .compensating failure) ∨
      (workflow = .kill ∧
        (ledger.operation.status = .pending ∨ ledger.operation.status = .outcomeUnknown)))
    (member : .command command ∈ workflowExpectedEffects workflow world key plan)
    (found : world.logicalEffects (logicalEffectId command) = some record)
    (unfinishedEffect : record.result = none)
    (origin : record.origin = .operation key)
    (awaiting : world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .dispatched ∨
      world.effects ⟨logicalEffectId command, record.currentAttempt⟩ = some .outcomeUnknown) :
    workflowRank workflow key
        (acknowledgeEffect world ⟨logicalEffectId command, record.currentAttempt⟩ outcome) <
      workflowRank workflow key world := by
  apply workflowRank_acknowledgeEffect_strict_of_stage_le world outcome workflow key plan
    command record unfinishedOperation foundPlan member found unfinishedEffect origin awaiting
  rcases cleanup with ⟨failure, compensating⟩ | ⟨rfl, normal⟩
  · exact workflowStageDebt_acknowledgeEffect_compensating_le world _ outcome
      awaiting workflow key plan ledger failure foundPlan foundLedger compensating
  · exact workflowStageDebt_acknowledgeEffect_killNormal_le world _ outcome
      awaiting key plan ledger foundPlan foundLedger normal

/-- A valid current receipt in kill or compensation decreases the rank of
the actual executable handler, including the finite stop-receipt scan. -/
theorem handleEffectReceipt_cleanup_progress
    (world : World) (outcome : EffectOutcome)
    (workflow : WorkflowKind) (key : OperationKey) (plan : OperationPlan)
    (ledger : OperationLedgerRecord) (command : Command)
    (record : LogicalEffectRecord)
    (unfinishedOperation : ¬ Finalized key world)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (cleanup : (∃ failure, ledger.operation.status = .compensating failure) ∨
      (workflow = .kill ∧
        (ledger.operation.status = .pending ∨ ledger.operation.status = .outcomeUnknown)))
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
  obtain ⟨decision, accepted, eventSame, nextSame⟩ :=
    Controller.handleEffectReceipt_complete legal
  refine ⟨decision, accepted, eventSame, ?_⟩
  rw [nextSame]
  exact workflowRank_cleanupEffectReceipt_strict world outcome workflow key plan ledger
    command record unfinishedOperation foundPlan foundLedger cleanup member found
    unfinishedEffect origin awaiting

/-- A receipt step in the infinite trace has the same strict decrease.
The awaiting premise excludes a replay of an already acknowledged receipt. -/
theorem cleanupReceiptEvent_rank_strict
    (trace : Trace) (time : Nat) (world : World)
    (stateAt : trace.state time = world)
    (workflow : WorkflowKind) (key : OperationKey) (plan : OperationPlan)
    (ledger : OperationLedgerRecord) (command : Command)
    (record : LogicalEffectRecord) (outcome : EffectOutcome)
    (observed : EventAt trace time
      (.acknowledgeEffect ⟨logicalEffectId command, record.currentAttempt⟩ outcome))
    (unfinishedOperation : ¬ Finalized key world)
    (foundPlan : world.operationPlans key = some plan)
    (foundLedger : world.operations key = some ledger)
    (cleanup : (∃ failure, ledger.operation.status = .compensating failure) ∨
      (workflow = .kill ∧
        (ledger.operation.status = .pending ∨ ledger.operation.status = .outcomeUnknown)))
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
  exact workflowRank_cleanupEffectReceipt_strict world outcome workflow key plan ledger
    command record unfinishedOperation foundPlan foundLedger cleanup member found
    unfinishedEffect origin awaiting

end SandboxModel
