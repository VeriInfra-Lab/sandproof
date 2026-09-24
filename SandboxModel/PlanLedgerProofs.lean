import SandboxModel.ControllerLiveness

/-! The persisted operation plan and operation ledger cannot drift apart. -/

namespace SandboxModel

private theorem operationPlanLedgerValid_of_unchangedPlans
    (recordPersistent : OperationRecordPersistent before after)
    (identityStable : OperationIdentityStable before after)
    (invariant : OperationPlanLedgerValid before)
    (samePlans : after.operationPlans = before.operationPlans) :
    OperationPlanLedgerValid after := by
  intro key plan foundAfter
  have foundBefore : before.operationPlans key = some plan := by
    simpa [samePlans] using foundAfter
  rcases invariant key plan foundBefore with
    ⟨oldLedger, oldFound, planKey, planEpoch⟩
  rcases recordPersistent key oldLedger oldFound with
    ⟨newLedger, newFound⟩
  have identity := identityStable key oldLedger newLedger oldFound newFound
  exact ⟨newLedger, newFound, planKey,
    planEpoch.trans identity.2.2.2.1⟩

private theorem operationPlanLedgerValid_after_newOperation
    (recordPersistent : OperationRecordPersistent before after)
    (identityStable : OperationIdentityStable before after)
    (invariant : OperationPlanLedgerValid before)
    (newKey : OperationKey) (newPlan : OperationPlan)
    (newLedger : OperationLedgerRecord)
    (plans : after.operationPlans = put before.operationPlans newKey newPlan)
    (ledger : after.operations newKey = some newLedger)
    (planKey : newPlan.key = newKey)
    (planEpoch : newPlan.epoch = newLedger.operation.epoch) :
    OperationPlanLedgerValid after := by
  intro key plan foundAfter
  by_cases same : key = newKey
  · subst key
    have samePlan : plan = newPlan := by
      have : put before.operationPlans newKey newPlan newKey = some plan := by
        simpa [plans] using foundAfter
      simpa [put] using this.symm
    subst plan
    exact ⟨newLedger, ledger, planKey, planEpoch⟩
  · have foundBefore : before.operationPlans key = some plan := by
      have : put before.operationPlans newKey newPlan key = some plan := by
        simpa [plans] using foundAfter
      simpa [put, same] using this
    rcases invariant key plan foundBefore with
      ⟨oldLedger, oldFound, oldPlanKey, oldPlanEpoch⟩
    rcases recordPersistent key oldLedger oldFound with
      ⟨laterLedger, laterFound⟩
    have identity := identityStable key oldLedger laterLedger oldFound laterFound
    exact ⟨laterLedger, laterFound, oldPlanKey,
      oldPlanEpoch.trans identity.2.2.2.1⟩

private theorem operationPlanLedgerValid_of_maps_eq
    (plans : after.operationPlans = before.operationPlans)
    (operations : after.operations = before.operations)
    (invariant : OperationPlanLedgerValid before) :
    OperationPlanLedgerValid after := by
  intro key plan found
  have oldPlan : before.operationPlans key = some plan := by
    simpa [plans] using found
  rcases invariant key plan oldPlan with
    ⟨ledger, oldLedger, planKey, planEpoch⟩
  exact ⟨ledger, by simpa [operations] using oldLedger, planKey, planEpoch⟩

private theorem operationPlanLedgerValid_storeNewOperation
    (invariant : OperationPlanLedgerValid world) :
    OperationPlanLedgerValid (storeNewOperation world sid operation input) := by
  intro key plan found
  by_cases same : key = operationKey sid operation.id
  · subst key
    have planIdentity : plan = initialOperationPlan world sid operation input := by
      have : put world.operationPlans (operationKey sid operation.id)
          (initialOperationPlan world sid operation input)
          (operationKey sid operation.id) = some plan := by
        simpa [storeNewOperation] using found
      simpa [put] using this.symm
    subst plan
    exact ⟨⟨sid, operation, input, none⟩,
      by simp [storeNewOperation, setOperation, put],
      by simp [initialOperationPlan],
      by simp [initialOperationPlan]⟩
  · have oldPlan : world.operationPlans key = some plan := by
      have : put world.operationPlans (operationKey sid operation.id)
          (initialOperationPlan world sid operation input) key = some plan := by
        simpa [storeNewOperation] using found
      simpa [put, same] using this
    rcases invariant key plan oldPlan with
      ⟨ledger, oldLedger, planKey, planEpoch⟩
    exact ⟨ledger,
      by simpa [storeNewOperation, setOperation, put, same] using oldLedger,
      planKey, planEpoch⟩

private theorem operationPlanLedgerValid_enqueueStore
    (invariant : OperationPlanLedgerValid world) :
    OperationPlanLedgerValid
      (enqueue (storeNewOperation world sid operation input) command) := by
  simpa [OperationPlanLedgerValid] using
    (operationPlanLedgerValid_storeNewOperation invariant)

private theorem operationPlanLedgerValid_enqueue
    (invariant : OperationPlanLedgerValid world) :
    OperationPlanLedgerValid (enqueue world command) := by
  simpa [OperationPlanLedgerValid] using invariant

private theorem operationPlanLedgerValid_updateStatus
    (invariant : OperationPlanLedgerValid world)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (sameOperation : ledger.operation = operation) :
    OperationPlanLedgerValid
      (updateOperation world sid { operation with status := status } result) := by
  intro key plan planFound
  have oldPlan : world.operationPlans key = some plan := by
    simpa using planFound
  rcases invariant key plan oldPlan with
    ⟨oldLedger, oldFound, planKey, planEpoch⟩
  by_cases same : key = operationKey sid operation.id
  · have oldFoundAtTarget :
        world.operations (operationKey sid operation.id) = some oldLedger := by
      simpa [same] using oldFound
    have ledgerIdentity : oldLedger = ledger :=
      Option.some.inj (oldFoundAtTarget.symm.trans found)
    subst oldLedger
    refine ⟨{ ledger with
        operation := { operation with status := status }
        result := result }, ?_, planKey, ?_⟩
    · simp [updateOperation, same, found]
    · simpa [sameOperation] using planEpoch
  · exact ⟨oldLedger,
      by simpa [updateOperation, same] using oldFound,
      planKey, planEpoch⟩

private theorem operationPlanLedgerValid_supersedeCurrentOperation
    (invariant : OperationPlanLedgerValid world) :
    OperationPlanLedgerValid (supersedeCurrentOperation world sid current) := by
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
              | none =>
                  by_cases sameOperation : ledger.operation = operation
                  · simpa [final, sameOperation] using
                      (operationPlanLedgerValid_updateStatus invariant found
                        sameOperation (status := .failed .cancelled)
                        (result := some (.failed .cancelled)))
                  · simpa [final, sameOperation] using invariant
              | some result => simpa [final] using invariant
      | outcomeUnknown =>
          simp only [supersedeCurrentOperation, status]
          cases found : world.operations (operationKey sid operation.id) with
          | none => exact invariant
          | some ledger =>
              cases final : ledger.result with
              | none =>
                  by_cases sameOperation : ledger.operation = operation
                  · simpa [final, sameOperation] using
                      (operationPlanLedgerValid_updateStatus invariant found
                        sameOperation (status := .failed .cancelled)
                        (result := some (.failed .cancelled)))
                  · simpa [final, sameOperation] using invariant
              | some result => simpa [final] using invariant
      | compensating failure => simpa [supersedeCurrentOperation, status] using invariant
      | succeeded => simpa [supersedeCurrentOperation, status] using invariant
      | failed failure => simpa [supersedeCurrentOperation, status] using invariant

theorem operationPlanLedgerValid_emptyWorld :
    OperationPlanLedgerValid emptyWorld := by
  simp [OperationPlanLedgerValid, emptyWorld]

set_option maxHeartbeats 1000000 in
theorem step_preserves_operationPlanLedgerValid
    (invariant : OperationPlanLedgerValid world)
    (transition : Step world event next) :
    OperationPlanLedgerValid next := by
  have recordPersistent := step_operationRecordPersistent transition
  have identityStable := step_operationIdentityStable transition
  cases transition
  case acceptCreate =>
    simp only [beginCreate]
    apply operationPlanLedgerValid_enqueue
    apply operationPlanLedgerValid_storeNewOperation
    simpa [OperationPlanLedgerValid] using invariant
  case acceptPause =>
    simp only [beginPause]
    apply operationPlanLedgerValid_enqueue
    apply operationPlanLedgerValid_enqueue
    apply operationPlanLedgerValid_storeNewOperation
    simpa [OperationPlanLedgerValid] using invariant
  case acceptResume =>
    simp only [beginResume]
    apply operationPlanLedgerValid_enqueue
    apply operationPlanLedgerValid_storeNewOperation
    simpa [OperationPlanLedgerValid] using invariant
  case acceptKill =>
    simp only [beginKill]
    apply operationPlanLedgerValid_enqueue
    apply operationPlanLedgerValid_storeNewOperation
    simpa [OperationPlanLedgerValid] using
      (operationPlanLedgerValid_supersedeCurrentOperation invariant)
  case reserveStartupResources =>
    simp_all [OperationPlanLedgerValid, reserveStartupResources,
      setOperationPlan, put] <;>
      grind
  case reservePauseSnapshot =>
    simp_all [OperationPlanLedgerValid, reservePauseSnapshot,
      setOperationPlan, put] <;>
      grind
  case dispatchPrepare =>
    simp_all [OperationPlanLedgerValid, setOperationPlan, enqueue, put] <;>
      grind
  case scheduleSealSnapshot =>
    simp_all [OperationPlanLedgerValid, setOperationPlan, enqueue, put] <;>
      grind
  all_goals
    apply operationPlanLedgerValid_of_unchangedPlans
      recordPersistent identityStable invariant
    simp

theorem reachable_operationPlanLedgerValid
    (reachable : Reachable world) : OperationPlanLedgerValid world := by
  induction reachable with
  | initial => exact operationPlanLedgerValid_emptyWorld
  | step prior transition inductionHypothesis =>
      exact step_preserves_operationPlanLedgerValid inductionHypothesis transition

end SandboxModel
