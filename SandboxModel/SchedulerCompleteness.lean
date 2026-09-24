import SandboxModel.Controller

/-!
Backward coverage for the executable scheduler and stable-state publisher.

The scheduler theorems need no invariant beyond the premises of the matching
`Step` constructor.  Publication scans finite resource/effect indexes.  The
resource scan is complete under `ResourceIndexValid`; the terminal stop scan
also needs the effect-key list to contain every stored effect.  That latter
condition is stated explicitly instead of silently assuming it for arbitrary
`World` values.
-/

namespace SandboxModel.Controller

set_option maxHeartbeats 2000000

theorem handleReserveStartup_complete
    (transition : Step world (.reserveStartupResources key vmId slotId) next) :
    ∃ decision,
      handleScheduler world (.reserveStartup key) = .ok decision ∧
      decision.event = .reserveStartupResources key vmId slotId ∧
      decision.next = next := by
  let decision : Decision world :=
    ⟨.reserveStartupResources key vmId slotId, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case reserveStartupResources =>
    simp only [decision, handleScheduler]
    unfold handleReserveStartup
    simp only [operationKey_sandbox, operationKey_operation]
    grind

theorem handleReservePauseSnapshot_complete
    (transition : Step world
      (.reservePauseSnapshot key snapshotId manifest) next) :
    ∃ decision,
      handleScheduler world (.reservePauseSnapshot key) = .ok decision ∧
      decision.event = .reservePauseSnapshot key snapshotId manifest ∧
      decision.next = next := by
  let decision : Decision world :=
    ⟨.reservePauseSnapshot key snapshotId manifest, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case reservePauseSnapshot =>
    simp only [decision, handleScheduler]
    unfold handleReservePauseSnapshot
    simp only [operationKey_sandbox, operationKey_operation]
    grind

theorem handleDispatchPrepare_complete
    (transition : Step world (.dispatchPrepare sid vmId slotId source) next) :
    ∃ decision,
      handleScheduler world (.dispatchPrepare sid vmId slotId) = .ok decision ∧
      decision.event = .dispatchPrepare sid vmId slotId source ∧
      decision.next = next := by
  let decision : Decision world :=
    ⟨.dispatchPrepare sid vmId slotId source, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case dispatchPrepare record operation plan starting live currentOperation
      planEpoch planGeneration planSealed plannedVm plannedSlot up planKey found
      foundPlan fenced freshVm freshSlot planSource selectedSource resumeSource =>
    subst source
    have checkedSource : restoreSourceReadyCheck world sid record = true :=
      restoreSourceReadyCheck_spec.mpr fun selected selectedFrom =>
        resumeSource selected (selectedSource.trans selectedFrom)
    simp only [decision, handleScheduler]
    unfold handleDispatchPrepare
    grind

theorem handleScheduleSealSnapshot_complete
    (transition : Step world
      (.scheduleSealSnapshot sid snapshotId manifest) next) :
    ∃ decision,
      handleScheduler world (.sealSnapshot sid snapshotId manifest) =
        .ok decision ∧
      decision.event = .scheduleSealSnapshot sid snapshotId manifest ∧
      decision.next = next := by
  let decision : Decision world :=
    ⟨.scheduleSealSnapshot sid snapshotId manifest, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case scheduleSealSnapshot record operation plan vmId vm pausing
      currentOperation planEpoch planGeneration planSealed plannedVm current
      frozen up foundVm planKey vmIdentity foundSandbox foundPlan fenced
      freezeSucceeded fresh plannedSnapshot =>
    have checkedFreeze : effectSucceededCheck world
        (.freezeVm sid record.epoch record.generation vmId) = true :=
      effectSucceededCheck_spec.mpr freezeSucceeded
    simp only [decision, handleScheduler]
    unfold handleScheduleSealSnapshot
    simp only [up, ↓reduceDIte]
    split
    · rename_i missing
      simp [foundSandbox] at missing
    · rename_i matchedRecord matched
      have sameRecord : matchedRecord = record :=
        Option.some.inj (matched.symm.trans foundSandbox)
      subst matchedRecord
      simp only [pausing, ↓reduceDIte]
      split
      · rename_i missingOperation
        simp [currentOperation] at missingOperation
      · rename_i matchedOperation operationFound
        have sameOperation : matchedOperation = operation :=
          Option.some.inj (operationFound.symm.trans currentOperation)
        subst matchedOperation
        split
        · rename_i missingPlan
          simp [foundPlan] at missingPlan
        · rename_i matchedPlan planFound
          have samePlan : matchedPlan = plan :=
            Option.some.inj (planFound.symm.trans foundPlan)
          subst matchedPlan
          simp only [planKey, ↓reduceDIte, planEpoch, planGeneration,
            planSealed, plannedSnapshot]
          split
          · rename_i missingVm
            simp [current] at missingVm
          · rename_i matchedVm currentFound
            have sameVm : matchedVm = vmId :=
              Option.some.inj (currentFound.symm.trans current)
            subst matchedVm
            simp only [plannedVm, ↓reduceDIte]
            split
            · rename_i missingVmRecord
              simp [foundVm] at missingVmRecord
            · rename_i matchedVmRecord vmFound
              have sameVmRecord : matchedVmRecord = vm :=
                Option.some.inj (vmFound.symm.trans foundVm)
              subst matchedVmRecord
              simp [vmIdentity, frozen, fenced, checkedFreeze]
              split
              · rename_i snapshotValue snapshotFound
                simp [fresh] at snapshotFound
              · congr 2

theorem handleScheduleCommitSnapshot_complete
    (transition : Step world (.scheduleCommitSnapshot sid snapshotId) next) :
    ∃ decision,
      handleScheduler world (.commitSnapshot sid snapshotId) = .ok decision ∧
      decision.event = .scheduleCommitSnapshot sid snapshotId ∧
      decision.next = next := by
  let decision : Decision world :=
    ⟨.scheduleCommitSnapshot sid snapshotId, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case scheduleCommitSnapshot snapshot record vmId pausing current
      snapshotIdentity sealed up snapshotSandbox foundSandbox foundSnapshot
      sealSucceeded =>
    have checkedSeal : effectSucceededCheck world
        (.sealSnapshot sid record.epoch record.generation vmId record.workspace
          snapshotId snapshot.manifest) = true :=
      effectSucceededCheck_spec.mpr sealSucceeded
    simp only [decision, handleScheduler]
    unfold handleScheduleCommitSnapshot
    grind

theorem handleScheduleStopVm_complete
    (transition : Step world (.scheduleStopVm sid generation vmId) next) :
    ∃ decision,
      handleScheduler world (.stopVm sid generation vmId) = .ok decision ∧
      decision.event = .scheduleStopVm sid generation vmId ∧
      decision.next = next := by
  let decision : Decision world :=
    ⟨.scheduleStopVm sid generation vmId, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case scheduleStopVm =>
    simp only [decision, handleScheduler]
    unfold handleScheduleStopVm
    grind [effectSucceededCheck_spec, stoppableVmStatusCheck_spec]

theorem handleScheduleCleanupVm_complete
    (transition : Step world (.scheduleCleanupVm sid generation vmId) next) :
    ∃ decision,
      handleScheduler world (.cleanupVm sid generation vmId) = .ok decision ∧
      decision.event = .scheduleCleanupVm sid generation vmId ∧
      decision.next = next := by
  let decision : Decision world :=
    ⟨.scheduleCleanupVm sid generation vmId, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case scheduleCleanupVm =>
    simp only [decision, handleScheduler]
    unfold handleScheduleCleanupVm
    grind [notCurrentVmCheck_spec]

theorem handleScheduleCleanupSlot_complete
    (transition : Step world (.scheduleCleanupSlot sid generation slotId) next) :
    ∃ decision,
      handleScheduler world (.cleanupSlot sid generation slotId) = .ok decision ∧
      decision.event = .scheduleCleanupSlot sid generation slotId ∧
      decision.next = next := by
  let decision : Decision world :=
    ⟨.scheduleCleanupSlot sid generation slotId, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case scheduleCleanupSlot =>
    simp only [decision, handleScheduler]
    unfold handleScheduleCleanupSlot
    grind [notCurrentSlotCheck_spec]

/-! ## Stable-state publication coverage -/

/-- The finite effect-key list contains every effect stored in the authoritative
    effect table.  This is the effect-side analogue of `ResourceIndexValid`. -/
def EffectKeyIndexComplete (world : World) : Prop :=
  ∀ id status, world.effects id = some status → id ∈ world.effectKeys

theorem effectKeyIndexComplete_emptyWorld :
    EffectKeyIndexComplete emptyWorld := by
  simp [EffectKeyIndexComplete, emptyWorld]

private theorem effectKeyIndexComplete_of_fields_eq
    (complete : EffectKeyIndexComplete before)
    (sameEffects : after.effects = before.effects)
    (sameKeys : after.effectKeys = before.effectKeys) :
    EffectKeyIndexComplete after := by
  intro id status found
  rw [sameEffects] at found
  rw [sameKeys]
  exact complete id status found

private theorem effectKeyIndexComplete_after_put
    (id : EffectId) (status : EffectStatus)
    (complete : EffectKeyIndexComplete before)
    (sameEffects : after.effects = put before.effects id status)
    (sameKeys : after.effectKeys = insertUnique id before.effectKeys) :
    EffectKeyIndexComplete after := by
  intro candidate candidateStatus found
  rw [sameEffects] at found
  rw [sameKeys, mem_insertUnique]
  by_cases same : candidate = id
  · exact Or.inl same
  · exact Or.inr (complete candidate candidateStatus (by
      simpa [put, same] using found))

private theorem effectKeyIndexComplete_setEffect
    (id : EffectId) (status : EffectStatus)
    (complete : EffectKeyIndexComplete world) :
    EffectKeyIndexComplete (setEffect world id status) :=
  effectKeyIndexComplete_after_put id status complete rfl rfl

private theorem effectKeyIndexComplete_commitEffect
    (command : Command)
    (complete : EffectKeyIndexComplete world) :
    EffectKeyIndexComplete (commitEffect world command) := by
  cases found : world.effects (effectId command) with
  | none =>
      cases logical : world.logicalEffects (logicalEffectId command) with
      | none =>
          simp only [commitEffect, found, logical]
          exact effectKeyIndexComplete_after_put (effectId command)
            .committed complete rfl rfl
      | some record =>
          simp only [commitEffect, found, logical]
          exact effectKeyIndexComplete_after_put (effectId command)
            .committed complete rfl rfl
  | some status =>
      simpa [commitEffect, found] using complete

private theorem effectKeyIndexComplete_enqueue
    (command : Command)
    (complete : EffectKeyIndexComplete world) :
    EffectKeyIndexComplete (enqueue world command) := by
  apply effectKeyIndexComplete_of_fields_eq
    (effectKeyIndexComplete_commitEffect command complete) <;> rfl

private theorem effectKeyIndexComplete_acknowledgeEffect
    (id : EffectId) (outcome : EffectOutcome)
    (complete : EffectKeyIndexComplete world) :
    EffectKeyIndexComplete (acknowledgeEffect world id outcome) :=
  effectKeyIndexComplete_after_put id (.acknowledged outcome) complete rfl rfl

private theorem effectKeyIndexComplete_startNextAttempt
    (logical : LogicalEffectId) (failure : FailureKind)
    (complete : EffectKeyIndexComplete world) :
    EffectKeyIndexComplete (startNextAttempt world logical failure) := by
  unfold startNextAttempt
  cases selected : nextAttemptData world logical failure with
  | none =>
      exact effectKeyIndexComplete_of_fields_eq complete
        (by rfl) (by rfl)
  | some value =>
      rcases value with ⟨next, record⟩
      exact effectKeyIndexComplete_after_put next .committed complete
        (by rfl) (by rfl)

private theorem supersedeCurrentOperation_effectKeys
    (world : World) (sid : SandboxId) (current : Option Operation) :
    (supersedeCurrentOperation world sid current).effectKeys =
      world.effectKeys := by
  cases current with
  | none => rfl
  | some operation =>
      cases status : operation.status <;>
        simp only [supersedeCurrentOperation, status]
      all_goals
        cases found : world.operations (operationKey sid operation.id) with
        | none => rfl
        | some ledger =>
            cases final : ledger.result with
            | none =>
                by_cases sameOperation : ledger.operation = operation
                · simp [final, sameOperation, updateOperation]
                · simp [final, sameOperation]
            | some result => simp [final]

theorem step_preserves_effectKeyIndexComplete
    (complete : EffectKeyIndexComplete world)
    (transition : Step world event next) :
    EffectKeyIndexComplete next := by
  cases transition
  case acceptCreate =>
    unfold beginCreate
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_of_fields_eq complete <;> rfl
  case dispatchEffect =>
    exact effectKeyIndexComplete_setEffect _ _ complete
  case acknowledgeEffect =>
    exact effectKeyIndexComplete_acknowledgeEffect _ _ complete
  case effectTimeout =>
    exact effectKeyIndexComplete_setEffect _ _ complete
  case startNextAttempt =>
    exact effectKeyIndexComplete_startNextAttempt _ _ complete
  case dispatchPrepare =>
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_of_fields_eq complete <;> rfl
  case prepareVm =>
    unfold prepareResources
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_of_fields_eq complete <;> rfl
  case acceptPause =>
    unfold beginPause
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_of_fields_eq complete <;> rfl
  case scheduleSealSnapshot =>
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_of_fields_eq complete <;> rfl
  case scheduleCommitSnapshot =>
    exact effectKeyIndexComplete_enqueue _ complete
  case commitSnapshot =>
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_of_fields_eq complete <;> rfl
  case scheduleStopVm =>
    exact effectKeyIndexComplete_enqueue _ complete
  case scheduleCleanupVm =>
    exact effectKeyIndexComplete_enqueue _ complete
  case scheduleCleanupSlot =>
    exact effectKeyIndexComplete_enqueue _ complete
  case acceptResume =>
    unfold beginResume
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_of_fields_eq complete <;> rfl
  case acceptKill =>
    unfold beginKill
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_of_fields_eq complete
    · simp
    · simpa only [storeNewOperation, setOperationPlan, setOperation,
        setSandbox] using
        (supersedeCurrentOperation_effectKeys world _ _)
  case beginCompensation =>
    unfold SandboxModel.beginCompensation
    apply effectKeyIndexComplete_enqueue
    apply effectKeyIndexComplete_of_fields_eq complete <;> rfl
  case installRevocationFence =>
    rename_i sid record revoking queued dispatched found monotone slotIdentity
    apply effectKeyIndexComplete_of_fields_eq complete
    all_goals
      cases slotChoice : record.netSlot with
      | none => simp only [applyRevocationFence, slotChoice]; rfl
      | some slotId =>
          cases slotFound : world.netSlots slotId <;>
            simp only [applyRevocationFence, slotChoice, slotFound] <;> rfl
  all_goals
    apply effectKeyIndexComplete_of_fields_eq complete <;>
      simp [applyStartFence, configureSlot,
        applyFilePolicyResult, activateResource, publishRunning,
        sealSnapshot, stopResource, reclaimVm, reclaimSlot, publishPaused,
        publishDestroyed, publishFailure, markOutcomeUnknown,
        finalizeLogicalFailure, setSandbox, setVm, setSnapshot,
        setNetSlot, setAppliedFilePolicy, setFence, updateOperation,
        updateCurrentOperation]

theorem reachable_effectKeyIndexComplete
    (reachable : Reachable world) : EffectKeyIndexComplete world := by
  induction reachable with
  | initial => exact effectKeyIndexComplete_emptyWorld
  | step _ transition inherited =>
      exact step_preserves_effectKeyIndexComplete inherited transition

@[simp] theorem currentEffectId_command (world : World) (command : Command) :
    (currentEffectId world command).command = command := by
  change (match world.logicalEffects (logicalEffectId command) with
    | some record => effectIdAt command record.currentAttempt
    | none => effectId command).command = command
  cases world.logicalEffects (logicalEffectId command) <;> rfl

private theorem any_true_of_mem (check : α → Bool) (value : α)
    (values : List α) (member : value ∈ values)
    (checked : check value = true) : values.any check = true := by
  induction values with
  | nil => simp at member
  | cons head tail ih =>
      simp only [List.mem_cons] at member
      simp only [List.any_cons, Bool.or_eq_true]
      rcases member with rfl | member
      · exact Or.inl checked
      · exact Or.inr (ih member)

theorem vmStopSucceededFiniteCheck_complete
    (complete : EffectKeyIndexComplete world)
    (stopped : vmStopSucceeded world sid generation vmId) :
    vmStopSucceededFiniteCheck world sid generation vmId = true := by
  rcases stopped with ⟨epoch, result, acknowledged⟩
  let command : Command := .stopVm sid epoch generation vmId
  let id := currentEffectId world command
  have member : id ∈ world.effectKeys :=
    complete id (.acknowledged (.succeeded result)) acknowledged
  apply any_true_of_mem
    (stopReceiptCandidateCheck world sid generation vmId) id
    world.effectKeys member
  have succeeded : effectSucceeded world command := ⟨result, acknowledged⟩
  have checked : effectSucceededCheck world command = true :=
    effectSucceededCheck_spec.mpr succeeded
  simp [id, command, stopReceiptCandidateCheck, checked]

theorem indexedAnyStopsSucceededCheck_complete
    (resources : ResourceIndexValid world)
    (effects : EffectKeyIndexComplete world)
    (stopped : allVmStopsSucceeded world sid) :
    indexedAnyStopsSucceededCheck world sid = true := by
  rw [indexedAnyStopsSucceededCheck_spec]
  intro vmId member
  rcases resources.1 sid vmId member with ⟨vm, found, owned⟩
  exact ⟨vm, found,
    vmStopSucceededFiniteCheck_complete effects
      (stopped vmId vm found owned)⟩

theorem handlePublishRunning_complete
    (valid : ResourceIndexValid world)
    (transition : Step world (.publishRunning sid) next) :
    ∃ decision,
      handlePublication world valid (.running sid) = .ok decision ∧
      decision.event = .publishRunning sid ∧ decision.next = next := by
  let decision : Decision world := ⟨.publishRunning sid, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case publishRunning record vmId vm slot slotId operation ledger starting live
      currentVm executing currentSlot currentOperation up foundVm foundSlot
      vmIdentity slotReady operationMatches foundSandbox exclusiveNetwork
      appliedPolicy fenceSucceeded prepareSucceeded networkSucceeded
      activationSucceeded policyAcknowledged persistedOperation fenced =>
    have checkedNetwork : indexedSlotsClosedCheck world sid = true :=
      (indexedSlotsClosedCheck_global_spec valid).mpr exclusiveNetwork
    have checkedPolicy : appliedPolicyCheck world sid record.generation
        record.workspace record.filePolicy = true :=
      appliedPolicyCheck_spec.mpr appliedPolicy
    have checkedFence := effectSucceededCheck_spec.mpr fenceSucceeded
    have checkedPrepare := effectSucceededCheck_spec.mpr prepareSucceeded
    have checkedNetworkEffect := effectSucceededCheck_spec.mpr networkSucceeded
    have checkedActivation := effectSucceededCheck_spec.mpr activationSucceeded
    have checkedPolicyEffect := effectSucceededCheck_spec.mpr policyAcknowledged
    simp only [handlePublication]
    unfold handlePublishRunning
    rw [dif_pos up]
    split
    · rename_i heq
      have impossible : (none : Option SandboxRecord) = some record :=
        heq.symm.trans foundSandbox
      cases impossible
    · rename_i record' heq
      have same : record' = record :=
        Option.some.inj (heq.symm.trans foundSandbox)
      subst record'
      rw [dif_pos starting, dif_pos live]
      split
      · rename_i heq
        have impossible : (none : Option VmId) = some vmId :=
          heq.symm.trans currentVm
        cases impossible
      · rename_i vmId' heq
        have same : vmId' = vmId :=
          Option.some.inj (heq.symm.trans currentVm)
        subst vmId'
        split
        · rename_i heq
          have impossible : (none : Option Vm) = some vm :=
            heq.symm.trans foundVm
          cases impossible
        · rename_i vm' heq
          have same : vm' = vm :=
            Option.some.inj (heq.symm.trans foundVm)
          subst vm'
          rw [dif_pos vmIdentity, dif_pos executing]
          split
          · rename_i heq
            have impossible : (none : Option NetSlotId) = some slotId :=
              heq.symm.trans currentSlot
            cases impossible
          · rename_i slotId' heq
            have same : slotId' = slotId :=
              Option.some.inj (heq.symm.trans currentSlot)
            subst slotId'
            split
            · rename_i heq
              have impossible : (none : Option NetSlot) = some slot :=
                heq.symm.trans foundSlot
              cases impossible
            · rename_i slot' heq
              have same : slot' = slot :=
                Option.some.inj (heq.symm.trans foundSlot)
              subst slot'
              rw [dif_pos slotReady, dif_pos checkedNetwork,
                dif_pos checkedPolicy, dif_pos checkedFence,
                dif_pos checkedPrepare, dif_pos checkedNetworkEffect,
                dif_pos checkedActivation, dif_pos checkedPolicyEffect]
              split
              · rename_i heq
                have impossible : (none : Option Operation) = some operation :=
                  heq.symm.trans currentOperation
                cases impossible
              · rename_i operation' heq
                have same : operation' = operation :=
                  Option.some.inj (heq.symm.trans currentOperation)
                subst operation'
                split
                · rename_i heq
                  have impossible : (none : Option OperationLedgerRecord) =
                      some ledger := heq.symm.trans persistedOperation
                  cases impossible
                · rename_i ledger' heq
                  have same : ledger' = ledger :=
                    Option.some.inj (heq.symm.trans persistedOperation)
                  subst ledger'
                  rw [dif_pos operationMatches, dif_pos fenced]

theorem handlePublishPaused_complete
    (valid : ResourceIndexValid world)
    (transition : Step world (.publishPaused sid snapshotId) next) :
    ∃ source decision,
      source.snapshot = snapshotId ∧
      handlePublication world valid (.paused sid source) = .ok decision ∧
      decision.event = .publishPaused sid snapshotId ∧ decision.next = next := by
  cases transition
  case publishPaused record snapshot vmId operation ledger source pausing
      currentVm currentOperation up foundSnapshot snapshotReady operationMatches
      foundSandbox fenced fenceSucceeded freezeSucceeded sealSucceeded
      commitSucceeded stopSucceeded vmsClosed networkClosed persistedOperation =>
    have checkedVms : indexedVmsClosedCheck world sid = true :=
      (indexedVmsClosedCheck_global_spec valid).mpr vmsClosed
    have checkedNetwork : indexedSlotsClosedCheck world sid = true :=
      (indexedSlotsClosedCheck_global_spec valid).mpr networkClosed
    have checkedFence := effectSucceededCheck_spec.mpr fenceSucceeded
    have checkedFreeze := effectSucceededCheck_spec.mpr freezeSucceeded
    have checkedSeal := effectSucceededCheck_spec.mpr sealSucceeded
    have checkedCommit := effectSucceededCheck_spec.mpr commitSucceeded
    have checkedStop := effectSucceededCheck_spec.mpr stopSucceeded
    let decision : Decision world :=
      ⟨.publishPaused sid source.snapshot,
        publishPaused world sid record source,
        Step.publishPaused up foundSandbox pausing foundSnapshot snapshotReady
          fenced currentVm fenceSucceeded freezeSucceeded sealSucceeded
          commitSucceeded stopSucceeded vmsClosed networkClosed currentOperation
          persistedOperation operationMatches⟩
    refine ⟨source, decision, rfl, ?_, rfl, rfl⟩
    simp only [handlePublication]
    unfold handlePublishPaused
    rw [dif_pos up]
    split
    · rename_i heq
      have impossible : (none : Option SandboxRecord) = some record :=
        heq.symm.trans foundSandbox
      cases impossible
    · rename_i record' heq
      have same : record' = record :=
        Option.some.inj (heq.symm.trans foundSandbox)
      subst record'
      rw [dif_pos pausing]
      split
      · rename_i heq
        have impossible : (none : Option Snapshot) = some snapshot :=
          heq.symm.trans foundSnapshot
        cases impossible
      · rename_i snapshot' heq
        have same : snapshot' = snapshot :=
          Option.some.inj (heq.symm.trans foundSnapshot)
        subst snapshot'
        rw [dif_pos snapshotReady, dif_pos fenced]
        split
        · rename_i heq
          have impossible : (none : Option VmId) = some vmId :=
            heq.symm.trans currentVm
          cases impossible
        · rename_i vmId' heq
          have same : vmId' = vmId :=
            Option.some.inj (heq.symm.trans currentVm)
          subst vmId'
          rw [dif_pos checkedFence, dif_pos checkedFreeze, dif_pos checkedSeal,
            dif_pos checkedCommit, dif_pos checkedStop, dif_pos checkedVms,
            dif_pos checkedNetwork]
          split
          · rename_i heq
            have impossible : (none : Option Operation) = some operation :=
              heq.symm.trans currentOperation
            cases impossible
          · rename_i operation' heq
            have same : operation' = operation :=
              Option.some.inj (heq.symm.trans currentOperation)
            subst operation'
            split
            · rename_i heq
              have impossible : (none : Option OperationLedgerRecord) =
                  some ledger := heq.symm.trans persistedOperation
              cases impossible
            · rename_i ledger' heq
              have same : ledger' = ledger :=
                Option.some.inj (heq.symm.trans persistedOperation)
              subst ledger'
              rw [dif_pos operationMatches]

theorem handlePublishDestroyed_complete
    (valid : ResourceIndexValid world)
    (effectKeys : EffectKeyIndexComplete world)
    (transition : Step world (.publishDestroyed sid) next) :
    ∃ decision,
      handlePublication world valid (.destroyed sid) = .ok decision ∧
      decision.event = .publishDestroyed sid ∧ decision.next = next := by
  have stopped : allVmStopsSucceeded world sid := by
    cases transition
    assumption
  have checkedStops := indexedAnyStopsSucceededCheck_complete
    valid effectKeys stopped
  let decision : Decision world := ⟨.publishDestroyed sid, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case publishDestroyed record operation ledger revoking terminal
      currentOperation killOperation unfinished up operationMatches found
      fenced fenceSucceeded vmsClosed stopsSucceeded networkClosed
      persistedOperation =>
    have checkedVms : indexedVmsClosedCheck world sid = true :=
      (indexedVmsClosedCheck_global_spec valid).mpr vmsClosed
    have checkedNetwork : indexedSlotsClosedCheck world sid = true :=
      (indexedSlotsClosedCheck_global_spec valid).mpr networkClosed
    have checkedFence := effectSucceededCheck_spec.mpr fenceSucceeded
    simp_all (config := { maxSteps := 1000000 })
      [decision, handlePublication, handlePublishDestroyed]
    grind only

theorem handlePublishFailure_complete
    (valid : ResourceIndexValid world)
    (effectKeys : EffectKeyIndexComplete world)
    (failure : FailureKind)
    (transition : Step world (.publishFailure sid failure) next) :
    ∃ decision,
      handlePublication world valid (.failed sid failure) = .ok decision ∧
      decision.event = .publishFailure sid failure ∧ decision.next = next := by
  have stopped : allVmStopsSucceeded world sid := by
    cases transition
    assumption
  have checkedStops := indexedAnyStopsSucceededCheck_complete
    valid effectKeys stopped
  let decision : Decision world :=
    ⟨.publishFailure sid failure, next, transition⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  cases transition
  case publishFailure record operation ledger revoking terminal
      currentOperation recoverableKind up operationMatches found fenced
      fenceSucceeded vmsClosed stopsSucceeded networkClosed persistedOperation
      compensating =>
    have checkedVms : indexedVmsClosedCheck world sid = true :=
      (indexedVmsClosedCheck_global_spec valid).mpr vmsClosed
    have checkedNetwork : indexedSlotsClosedCheck world sid = true :=
      (indexedSlotsClosedCheck_global_spec valid).mpr networkClosed
    have checkedFence := effectSucceededCheck_spec.mpr fenceSucceeded
    simp_all (config := { maxSteps := 1000000 })
      [decision, handlePublication, handlePublishFailure]
    grind only

end SandboxModel.Controller
