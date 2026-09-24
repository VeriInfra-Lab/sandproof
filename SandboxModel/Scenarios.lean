import SandboxModel.Controller

/-!
Machine-checked normal protocol fragments.  The global theorems quantify over
all finite `Step` histories; these examples additionally show that the central
guards allow useful create, policy publication, manifest, resume, retry, crash,
and cleanup behavior.
-/

namespace SandboxModel.Scenarios

set_option linter.unusedSimpArgs false

abbrev tenant : TenantId := ⟨0⟩
abbrev sid : SandboxId := ⟨0⟩
abbrev workspace : FsId := ⟨0⟩
abbrev firstVm : VmId := ⟨0⟩
abbrev secondVm : VmId := ⟨1⟩
abbrev firstSlot : NetSlotId := ⟨0⟩
abbrev secondSlot : NetSlotId := ⟨1⟩
abbrev snapshot : SnapshotId := ⟨0⟩
abbrev createOperation : OperationId := ⟨0⟩
abbrev pauseOperation : OperationId := ⟨1⟩
abbrev resumeOperation : OperationId := ⟨2⟩
abbrev killOperation : OperationId := ⟨3⟩
abbrev filePolicy : FilePolicyRef := ⟨⟨7⟩⟩
abbrev manifest : SnapshotManifestRef := ⟨⟨9⟩⟩
abbrev source : RestoreSource := ⟨snapshot, manifest⟩
abbrev policy : NetworkPolicy := .denyAll
abbrev effectResult : ResultDigest := ⟨11⟩

abbrev createOpPending : Operation := ⟨createOperation, 1, .create, .pending⟩
abbrev createOpSucceeded : Operation := ⟨createOperation, 1, .create, .succeeded⟩
abbrev createInput : OperationInput := .create workspace policy filePolicy
abbrev createLedgerPending : OperationLedgerRecord :=
  ⟨sid, createOpPending, createInput, none⟩

def createdRecord : SandboxRecord := {
  owner := tenant
  phase := .starting
  terminal := false
  epoch := 1
  generation := 1
  currentVm := none
  workspace := workspace
  workspaceAccess := .managedReadWrite
  filePolicy := filePolicy
  durableSnapshot := none
  restoreFrom := none
  netSlot := none
  policy := policy
  desired := .closed
  effective := .closed
  currentOperation := some createOpPending
}

def wCreate : World :=
  beginCreate emptyWorld sid tenant workspace filePolicy policy createOperation

/-- Create atomically stores the sandbox decision, operation, and first effect. -/
theorem create_step : Step emptyWorld
    (.acceptCreate (.tenant tenant) tenant sid workspace filePolicy policy createOperation)
    wCreate := by
  apply Step.acceptCreate <;>
    simp [wCreate, authorized, operationKey, emptyWorld]

theorem create_is_reachable : Reachable wCreate :=
  Reachable.step Reachable.initial create_step

theorem controller_accepts_create :
    ∃ decision,
      Controller.handle emptyWorld
        (.create (.tenant tenant) tenant sid workspace filePolicy policy
          createOperation) = .ok decision ∧
      decision.next = wCreate := by
  simp [Controller.handle, Controller.handleCreate, wCreate, emptyWorld,
    authorized]

/-! A kill that replaces an unfinished create finalizes the old operation in
    the same durable transition.  Once terminal revocation has begun, a second
    kill with a different operation ID is rejected. -/

abbrev secondKillOperation : OperationId := ⟨4⟩
abbrev createOpCancelled : Operation :=
  ⟨createOperation, 1, .create, .failed .cancelled⟩
abbrev createLedgerCancelled : OperationLedgerRecord :=
  ⟨sid, createOpCancelled, createInput, some (.failed .cancelled)⟩
abbrev killOpPending : Operation := ⟨killOperation, 2, .kill, .pending⟩

def killDuringCreate : World :=
  beginKill wCreate sid createdRecord killOperation

theorem controller_kill_supersedes_create :
    ∃ decision,
      Controller.handle wCreate
        (.kill (.tenant tenant) sid killOperation) = .ok decision ∧
      decision.next = killDuringCreate := by
  simp [Controller.handle, Controller.handleKill,
    Controller.supersedeReadyCheck, killDuringCreate, wCreate,
    createdRecord, createOpPending, createLedgerPending, createInput,
    beginCreate, operationKey, storeNewOperation, setOperation, setSandbox,
    setFilesystem, enqueue, commitEffect, effectId, setEffect, put,
    authorized, emptyWorld]

theorem killed_create_is_finalized :
    killDuringCreate.operations (operationKey sid createOperation) =
      some createLedgerCancelled := by
  native_decide

theorem controller_rejects_second_kill :
    Controller.handle killDuringCreate
      (.kill (.tenant tenant) sid secondKillOperation) =
        .error .terminal := by
  change Controller.handleKill killDuringCreate (.tenant tenant) sid
      secondKillOperation = .error .terminal
  unfold Controller.handleKill
  split
  · split
    · rename_i missing
      have terminalAt :
          (killDuringCreate.sandboxes sid).map SandboxRecord.terminal =
            some true := by native_decide
      rw [missing] at terminalAt
      contradiction
    · rename_i record found
      have terminalAt :
          (killDuringCreate.sandboxes sid).map SandboxRecord.terminal =
            some true := by native_decide
      rw [found] at terminalAt
      simp at terminalAt
      simp [terminalAt]
  · rename_i notUp
    have up : killDuringCreate.controllerUp = true := by native_decide
    contradiction

/-! A failed create uses the same terminal protocol as kill; it is not an
    atomic resource deletion outside `Step`. -/

abbrev createFailure : FailureKind := .policyFailure
abbrev createStartCommand : Command := .installFence sid 1 false
abbrev createOpCompensating : Operation :=
  ⟨createOperation, 1, .create, .compensating createFailure⟩
abbrev createOpFailed : Operation :=
  ⟨createOperation, 1, .create, .failed createFailure⟩
abbrev createLedgerCompensating : OperationLedgerRecord :=
  ⟨sid, createOpCompensating, createInput, none⟩

def compensatingRecord : SandboxRecord := {
  createdRecord with
  phase := .revoking
  terminal := true
  desired := .closed
  effective := .closed
  currentOperation := some createOpCompensating
}

abbrev compensationFence : Command := .installFence sid 1 true

abbrev createFenceDispatched : World :=
  dispatchEffect wCreate (effectId createStartCommand)

abbrev createFenceFailed : World := acknowledgeEffect createFenceDispatched
  (effectId createStartCommand) (.failed createFailure)

abbrev createFenceFinalized : World := finalizeLogicalFailure createFenceFailed
  (logicalEffectId createStartCommand) createFailure

def compensating : World :=
  beginCompensation createFenceFinalized sid createdRecord createOpPending createFailure

def compensationDispatched : World :=
  dispatchEffect compensating (effectId compensationFence)

def compensationFenced : World :=
  applyRevocationFence compensationDispatched sid compensatingRecord

def compensationAcknowledged : World := acknowledgeEffect compensationFenced
  (effectId compensationFence) (.succeeded effectResult)

def createFailed : World :=
  publishFailure compensationAcknowledged sid compensatingRecord createFailure

theorem dispatch_create_fence_step :
    Step wCreate (.dispatchEffect (effectId createStartCommand))
      createFenceDispatched := by
  apply Step.dispatchEffect <;>
    simp [createFenceDispatched, wCreate, createStartCommand, beginCreate,
      storeNewOperation, setOperation, setSandbox, setFilesystem, enqueue,
      commitEffect, dispatchEffect, effectId, setEffect, put, emptyWorld]

theorem acknowledge_create_fence_failure_step :
    Step createFenceDispatched
      (.acknowledgeEffect (effectId createStartCommand) (.failed createFailure))
      createFenceFailed := by
  apply Step.acknowledgeEffect <;>
    simp [createFenceFailed, createFenceDispatched, wCreate,
      createStartCommand, createFailure, beginCreate, storeNewOperation,
      setOperation, setSandbox, setFilesystem, enqueue, commitEffect,
      dispatchEffect, acknowledgeEffect, effectOutcomeValid,
      commandResultPresent, effectId,
      setEffect, put, emptyWorld]

theorem finalize_create_fence_failure_step :
    Step createFenceFailed
      (.finalizeEffectFailure (effectId createStartCommand) createFailure)
      createFenceFinalized := by
  apply Step.finalizeEffectFailure
      (logical := logicalEffectId createStartCommand)
      (record := {
        origin := .operation (operationKey sid createOperation)
        currentAttempt := 0
        remainingRetries := defaultRetryLimit
        result := none }) <;> native_decide

theorem begin_compensation_step :
    Step createFenceFinalized (.beginCompensation sid createFailure) compensating := by
  apply Step.beginCompensation (record := createdRecord)
      (operation := createOpPending) (ledger := createLedgerPending)
  case evidence =>
    exact Or.inr (Or.inr ⟨effectId createStartCommand, by native_decide⟩)
  all_goals
    simp [compensating, createFenceFinalized, createFenceFailed,
      createFenceDispatched,
      createStartCommand, createFailure, CompensationEvidence,
      FinalFailedAttemptEvidence, commandBelongsToOperation,
      dispatchEffect, acknowledgeEffect, finalizeLogicalFailure,
      wCreate, createdRecord, createOpPending,
      createLedgerPending, createInput, operationKey, beginCreate,
      storeNewOperation, setOperation, setSandbox, setFilesystem, enqueue,
      commitEffect, effectId, setEffect, put, emptyWorld, authorized]

theorem controller_accepts_compensation :
    ∃ decision,
      Controller.handle createFenceFinalized
        (.compensate sid createFailure (some (effectId createStartCommand))) =
          .ok decision ∧
      decision.next = compensating := by
  let decision : Controller.Decision createFenceFinalized :=
    ⟨.beginCompensation sid createFailure, compensating,
      begin_compensation_step⟩
  refine ⟨decision, ?_, rfl⟩
  have up : createFenceFailed.controllerUp = true := by native_decide
  have found : createFenceFailed.sandboxes sid = some createdRecord := by
    simp [createFenceFailed, createFenceDispatched, wCreate, beginCreate,
      createdRecord, setSandbox, put, emptyWorld]
  have phase : createdRecord.phase = .starting ∨
      createdRecord.phase = .pausing := by native_decide
  have live : createdRecord.terminal = false := by native_decide
  have current : createdRecord.currentOperation = some createOpPending := by
    native_decide
  have kind : createOpPending.kind = .create ∨
      createOpPending.kind = .resume ∨ createOpPending.kind = .pause := by
    native_decide
  have unfinished : createOpPending.status = .pending ∨
      createOpPending.status = .outcomeUnknown := by native_decide
  have persisted : createFenceFailed.operations
      (operationKey sid createOpPending.id) = some createLedgerPending := by
    native_decide
  have matched : createLedgerPending.sandbox = sid ∧
      createLedgerPending.operation = createOpPending ∧
      createLedgerPending.result = none := by native_decide
  have justified : Controller.compensationEvidenceCheck createFenceFinalized
      (operationKey sid createOpPending.id) createOpPending.epoch createFailure
      (some (effectId createStartCommand)) = true := by native_decide
  have upFinal : createFenceFinalized.controllerUp = true := by
    simpa using up
  have foundFinal : createFenceFinalized.sandboxes sid =
      some createdRecord := by simpa using found
  have persistedFinal : createFenceFinalized.operations
      (operationKey sid createOpPending.id) = some createLedgerPending := by
    simpa using persisted
  change Controller.handleCompensation createFenceFinalized sid createFailure
      (some (effectId createStartCommand)) = .ok decision
  unfold Controller.handleCompensation
  simp only [upFinal, foundFinal, phase, live, current, kind, unfinished,
    persistedFinal, matched, justified]
  rfl

theorem dispatch_compensation_fence_step :
    Step compensating (.dispatchEffect (effectId compensationFence))
      compensationDispatched := by
  apply Step.dispatchEffect <;> native_decide

theorem install_compensation_fence_step :
    Step compensationDispatched (.installFence sid) compensationFenced := by
  have current : currentEffectId compensationDispatched compensationFence =
      effectId compensationFence := by native_decide
  have fenceCommand :
      Command.installFence sid compensatingRecord.epoch
        compensatingRecord.terminal = compensationFence := by native_decide
  apply Step.installRevocationFence (record := compensatingRecord)
  case queued =>
    unfold deliveryQueued
    rw [fenceCommand, current]
    change addCommand _ compensationFence (effectId compensationFence)
    exact Or.inl rfl
  case dispatched =>
    unfold effectAwaiting effectHasStatus
    rw [fenceCommand, current]
    left
    simp [compensationDispatched, dispatchEffect, setEffect, put]
  case found =>
    change (put createFenceFinalized.sandboxes sid compensatingRecord) sid =
      some compensatingRecord
    simp [put]
  case monotone =>
    change fenceCanAdvance none 1 true
    trivial
  all_goals
    simp [current, compensationFenced, compensationDispatched, compensating,
      compensatingRecord, createFenceFinalized, createFenceFailed,
      createFenceDispatched,
      createStartCommand, createFailure, wCreate, beginCompensation,
      beginCreate, compensationFence, createOpPending, createOpCompensating,
      createdRecord, createInput, updateOperation, storeNewOperation,
      setOperation, applyRevocationFence, dispatchEffect, acknowledgeEffect,
      setSandbox, setFilesystem, setFence, enqueue, commitEffect,
      effectDispatched, effectAwaiting, effectHasStatus, deliveryQueued,
      effectId, setEffect, addCommand, addEffect, put, emptyWorld,
      fenceCanAdvance]

theorem acknowledge_compensation_fence_step :
    Step compensationFenced
      (.acknowledgeEffect (effectId compensationFence) (.succeeded effectResult))
      compensationAcknowledged := by
  apply Step.acknowledgeEffect <;>
    simp [compensationAcknowledged, compensationFenced,
      compensationDispatched, compensating, compensatingRecord,
      createFenceFinalized, createFenceFailed, createFenceDispatched,
      createStartCommand,
      createFailure, wCreate,
      beginCompensation, beginCreate, compensationFence, createOpPending,
      createOpCompensating, createdRecord, createInput, updateOperation,
      storeNewOperation, setOperation, applyRevocationFence, dispatchEffect,
      acknowledgeEffect, effectOutcomeValid, commandResultPresent,
      setSandbox, setFilesystem, setFence, enqueue, commitEffect, effectId,
      setEffect, addCommand, put, emptyWorld]

theorem publish_create_failure_step :
    Step compensationAcknowledged (.publishFailure sid createFailure)
      createFailed := by
  apply Step.publishFailure (record := compensatingRecord)
      (operation := createOpCompensating) (ledger := createLedgerCompensating)
  case fenceSucceeded =>
    unfold effectSucceeded effectHasStatus
    refine ⟨effectResult, ?_⟩
    have current : currentEffectId compensationAcknowledged
        (.installFence sid compensatingRecord.epoch true) =
          effectId compensationFence := by native_decide
    rw [current]
    simp [compensationAcknowledged, acknowledgeEffect, setEffect, put]
  all_goals
    simp [createFailed, compensationAcknowledged, compensationFenced,
      compensationDispatched, compensating, compensatingRecord,
      createFenceFinalized, createFenceFailed, createFenceDispatched,
      createStartCommand,
      createFailure, wCreate,
      beginCompensation,
      beginCreate, publishFailure, compensationFence, createOpPending,
      createOpCompensating, createLedgerCompensating, createdRecord,
      createInput, updateOperation, updateCurrentOperation,
      storeNewOperation, setOperation, applyRevocationFence, dispatchEffect,
      acknowledgeEffect, effectSucceeded, effectHasStatus,
      setSandbox, setFilesystem, setFence, enqueue, commitEffect,
      effectId, setEffect, addCommand, operationKey, put,
      allVmsStoppedOrReclaimed, allVmStopsSucceeded, noActiveSlot,
      emptyWorld] <;> grind

theorem create_failure_trace :
    Step wCreate (.dispatchEffect (effectId createStartCommand))
      createFenceDispatched ∧
    Step createFenceDispatched
      (.acknowledgeEffect (effectId createStartCommand) (.failed createFailure))
      createFenceFailed ∧
    Step createFenceFailed
      (.finalizeEffectFailure (effectId createStartCommand) createFailure)
      createFenceFinalized ∧
    Step createFenceFinalized (.beginCompensation sid createFailure) compensating ∧
    Step compensating (.dispatchEffect (effectId compensationFence))
      compensationDispatched ∧
    Step compensationDispatched (.installFence sid) compensationFenced ∧
    Step compensationFenced
      (.acknowledgeEffect (effectId compensationFence) (.succeeded effectResult))
      compensationAcknowledged ∧
    Step compensationAcknowledged (.publishFailure sid createFailure)
      createFailed :=
  ⟨dispatch_create_fence_step, acknowledge_create_fence_failure_step,
    finalize_create_fence_failure_step, begin_compensation_step,
    dispatch_compensation_fence_step,
    install_compensation_fence_step, acknowledge_compensation_fence_step,
    publish_create_failure_step⟩

abbrev policyCommand : Command := .applyFilePolicy sid 1 1 workspace filePolicy
abbrev activateCommand : Command := .activateVm sid 1 1 firstVm
abbrev startFenceCommand : Command := createStartCommand
abbrev prepareCommand : Command :=
  .prepareVm sid 1 1 firstVm firstSlot none
abbrev networkCommand : Command :=
  .configureNetwork sid 1 1 firstVm firstSlot policy

@[simp] theorem policyCommand_ne_activateCommand :
    effectId policyCommand ≠ effectId activateCommand := by decide

@[simp] theorem activateCommand_ne_policyCommand :
    effectId activateCommand ≠ effectId policyCommand := by decide

def preparedRecord : SandboxRecord :=
  { createdRecord with currentVm := some firstVm, netSlot := some firstSlot }

def firstPreparedVm : Vm := ⟨tenant, sid, 1, .prepared⟩
def firstExecutingVm : Vm := ⟨tenant, sid, 1, .executing⟩

def firstConfiguredSlot : NetSlot := {
  owner := tenant
  sandbox := sid
  generation := 1
  vm := firstVm
  policy := policy
  configured := true
  active := false
  reclaimed := false
}

def readyToPublish : World := {
  emptyWorld with
  sandboxes := put emptyWorld.sandboxes sid preparedRecord
  vms := put emptyWorld.vms firstVm firstPreparedVm
  filesystems := put emptyWorld.filesystems workspace ⟨tenant, sid, 1⟩
  netSlots := put emptyWorld.netSlots firstSlot firstConfiguredSlot
  workerFences := put emptyWorld.workerFences sid ⟨1, false⟩
  operations := put emptyWorld.operations (operationKey sid createOperation)
    createLedgerPending
  effects := put
    (put
      (put
        (put
          (put emptyWorld.effects (effectId startFenceCommand)
            (.acknowledged (.succeeded effectResult)))
          (effectId prepareCommand)
            (.acknowledged (.succeeded effectResult)))
        (effectId networkCommand)
          (.acknowledged (.succeeded effectResult)))
      (effectId policyCommand) .dispatched)
    (effectId activateCommand) .dispatched
  inflight := addCommand (addCommand emptyWorld.inflight policyCommand) activateCommand
}

def policyApplied : World := applyFilePolicyResult readyToPublish sid preparedRecord
def policyAcknowledged : World :=
  acknowledgeEffect policyApplied (effectId policyCommand) (.succeeded effectResult)
def vmActivated : World := activateResource policyAcknowledged firstVm firstPreparedVm
def activationAcknowledged : World := acknowledgeEffect vmActivated
  (effectId activateCommand) (.succeeded effectResult)
def running : World :=
  publishRunning activationAcknowledged sid preparedRecord firstSlot firstConfiguredSlot

/-! The executable effect handler distinguishes an unproved success from a
    timeout.  A timeout preserves the possibility that the worker already
    acted, so redispatch and a late result are both safe. -/

def policyTimedOut : World :=
  markEffectOutcomeUnknown readyToPublish (effectId policyCommand)

def policyRedispatched : World :=
  dispatchEffect policyTimedOut (effectId policyCommand)

def policyAppliedAfterTimeout : World :=
  applyFilePolicyResult policyTimedOut sid preparedRecord

def policyLateAcknowledged : World := acknowledgeEffect policyAppliedAfterTimeout
  (effectId policyCommand) (.succeeded effectResult)

theorem controller_rejects_unproved_policy_success :
    Controller.handleEffect readyToPublish
      (.receipt (effectId policyCommand) (.succeeded effectResult)) =
        .error .invalidOutcomeEvidence := by
  simp [Controller.handleEffect, Controller.handleEffectReceipt,
    Controller.effectOutcomeValidCheck,
    Controller.commandResultPresentCheck, readyToPublish, policyCommand,
    preparedRecord, createdRecord, effectId, put, emptyWorld]

theorem controller_rejects_failure_after_policy_success :
    Controller.handleEffect policyApplied
      (.receipt (effectId policyCommand) (.failed .policyFailure)) =
        .error .invalidOutcomeEvidence := by
  simp [Controller.handleEffect, Controller.handleEffectReceipt,
    Controller.effectOutcomeValidCheck,
    Controller.commandResultPresentCheck, policyApplied, readyToPublish,
    policyCommand, preparedRecord, createdRecord, applyFilePolicyResult,
    setAppliedFilePolicy, effectId, put, emptyWorld]

theorem controller_marks_policy_result_unknown :
    ∃ decision,
      Controller.handleEffect readyToPublish
        (.timeout (effectId policyCommand)) = .ok decision ∧
      decision.next = policyTimedOut := by
  simp [Controller.handleEffect, Controller.handleEffectTimeout,
    policyTimedOut, readyToPublish, policyCommand, markEffectOutcomeUnknown,
    setEffect, put, emptyWorld]

theorem controller_repeats_policy_timeout_idempotently :
    ∃ decision,
      Controller.handleEffect policyTimedOut
        (.timeout (effectId policyCommand)) = .ok decision ∧
      decision.next = policyTimedOut := by
  simp [Controller.handleEffect, Controller.handleEffectTimeout,
    policyTimedOut, readyToPublish, policyCommand, markEffectOutcomeUnknown,
    setEffect, put, emptyWorld]

theorem controller_redispatches_unknown_policy_result :
    ∃ decision,
      Controller.handleEffect policyTimedOut
        (.dispatch (effectId policyCommand)) = .ok decision ∧
      decision.next = policyRedispatched := by
  simp [Controller.handleEffect, Controller.handleEffectDispatch,
    policyRedispatched, policyTimedOut, readyToPublish, policyCommand,
    dispatchEffect, markEffectOutcomeUnknown, setEffect, put, emptyWorld]

theorem apply_policy_after_timeout_step :
    Step policyTimedOut (.applyFilePolicy sid 1) policyAppliedAfterTimeout := by
  have policyAt : Command.applyFilePolicy sid preparedRecord.epoch
      preparedRecord.generation preparedRecord.workspace
      preparedRecord.filePolicy = policyCommand := by native_decide
  have current : currentEffectId policyTimedOut policyCommand =
      effectId policyCommand := by native_decide
  apply Step.applyFilePolicy (record := preparedRecord)
  case queued =>
    unfold deliveryQueued
    rw [policyAt, current]
    change addCommand (addCommand (fun _ => False) policyCommand)
      activateCommand (effectId policyCommand)
    simp [addCommand, addEffect]
  case dispatched =>
    unfold effectAwaiting effectHasStatus
    rw [policyAt, current]
    right
    simp [policyTimedOut, markEffectOutcomeUnknown, setEffect, put]
  all_goals
    simp [policyAppliedAfterTimeout, policyTimedOut, readyToPublish,
      preparedRecord, createdRecord, policyCommand, effectAwaiting,
      effectHasStatus, applyFilePolicyResult, markEffectOutcomeUnknown,
      setAppliedFilePolicy, setEffect, addCommand, put, emptyWorld]

theorem controller_accepts_late_policy_success :
    ∃ decision,
      Controller.handleEffect policyAppliedAfterTimeout
        (.receipt (effectId policyCommand) (.succeeded effectResult)) =
          .ok decision ∧
      decision.next = policyLateAcknowledged := by
  simp [Controller.handleEffect, Controller.handleEffectReceipt,
    Controller.effectOutcomeValidCheck,
    Controller.commandResultPresentCheck, policyLateAcknowledged,
    policyAppliedAfterTimeout, policyTimedOut, readyToPublish, policyCommand,
    preparedRecord, createdRecord, applyFilePolicyResult,
    markEffectOutcomeUnknown, acknowledgeEffect, setAppliedFilePolicy,
    setEffect, effectId, put, emptyWorld]

theorem controller_replays_same_policy_result :
    ∃ decision,
      Controller.handleEffect policyLateAcknowledged
        (.receipt (effectId policyCommand) (.succeeded effectResult)) =
          .ok decision ∧
      decision.next = policyLateAcknowledged := by
  simp [Controller.handleEffect, Controller.handleEffectReceipt,
    policyLateAcknowledged, policyAppliedAfterTimeout, policyTimedOut,
    readyToPublish, policyCommand, preparedRecord, createdRecord,
    applyFilePolicyResult, markEffectOutcomeUnknown, acknowledgeEffect,
    setAppliedFilePolicy, setEffect, put, emptyWorld]

theorem controller_rejects_conflicting_policy_result :
    Controller.handleEffect policyLateAcknowledged
      (.receipt (effectId policyCommand) (.failed .resourceFailure)) =
        .error .conflictingReceipt := by
  simp [Controller.handleEffect, Controller.handleEffectReceipt,
    policyLateAcknowledged, policyAppliedAfterTimeout, policyTimedOut,
    readyToPublish, policyCommand, preparedRecord, createdRecord,
    applyFilePolicyResult, markEffectOutcomeUnknown, acknowledgeEffect,
    setAppliedFilePolicy, setEffect, put, emptyWorld]

theorem apply_policy_step :
    Step readyToPublish (.applyFilePolicy sid 1) policyApplied := by
  have policyAt : Command.applyFilePolicy sid preparedRecord.epoch
      preparedRecord.generation preparedRecord.workspace
      preparedRecord.filePolicy = policyCommand := by native_decide
  have current : currentEffectId readyToPublish policyCommand =
      effectId policyCommand := by native_decide
  apply Step.applyFilePolicy (record := preparedRecord)
  case queued =>
    unfold deliveryQueued
    rw [policyAt, current]
    change addCommand (addCommand (fun _ => False) policyCommand)
      activateCommand (effectId policyCommand)
    simp [addCommand, addEffect]
  case dispatched =>
    unfold effectAwaiting effectHasStatus
    rw [policyAt, current]
    left
    simp [readyToPublish, setEffect, put]
  all_goals
    simp [readyToPublish, preparedRecord, createdRecord, policyCommand,
      effectDispatched, effectAwaiting, effectHasStatus, applyFilePolicyResult,
      setAppliedFilePolicy,
      addCommand, setEffect, put, emptyWorld]

theorem acknowledge_policy_step : Step policyApplied
    (.acknowledgeEffect (effectId policyCommand) (.succeeded effectResult))
    policyAcknowledged := by
  apply Step.acknowledgeEffect <;>
    simp [policyAcknowledged, policyApplied, readyToPublish, policyCommand,
      preparedRecord, createdRecord, commandResultPresent, effectOutcomeValid,
      applyFilePolicyResult, acknowledgeEffect,
      effectId, setAppliedFilePolicy, setEffect, addCommand, put, emptyWorld]

theorem activate_vm_step :
    Step policyAcknowledged (.activateVm sid firstVm) vmActivated := by
  have commandAt : Command.activateVm sid preparedRecord.epoch
      preparedRecord.generation firstVm = activateCommand := by native_decide
  have current : currentEffectId policyAcknowledged activateCommand =
      effectId activateCommand := by native_decide
  apply Step.activateVm (record := preparedRecord) (vm := firstPreparedVm)
  case queued =>
    unfold deliveryQueued
    rw [commandAt, current]
    simp [policyAcknowledged, policyApplied, readyToPublish,
      applyFilePolicyResult, acknowledgeEffect, setAppliedFilePolicy,
      addCommand, addEffect]
  case dispatched =>
    unfold effectAwaiting effectHasStatus
    rw [commandAt, current]
    exact Or.inl (by native_decide)
  case foundSlot =>
    refine ⟨firstSlot, firstConfiguredSlot, rfl, ?_, rfl, rfl⟩
    simp [policyAcknowledged, policyApplied, readyToPublish,
      applyFilePolicyResult, acknowledgeEffect, setAppliedFilePolicy, put]
  all_goals
    simp [vmActivated, policyAcknowledged, policyApplied, readyToPublish,
      preparedRecord, createdRecord, firstPreparedVm, firstConfiguredSlot,
      activateCommand, activateResource, applyFilePolicyResult,
      acknowledgeEffect, setAppliedFilePolicy, setEffect, effectId, setVm,
      addCommand, put, noExecutingVm, emptyWorld]

theorem acknowledge_activation_step :
    Step vmActivated
      (.acknowledgeEffect (effectId activateCommand) (.succeeded effectResult))
      activationAcknowledged := by
  apply Step.acknowledgeEffect <;>
    simp [activationAcknowledged, vmActivated, policyAcknowledged,
      policyApplied, readyToPublish, preparedRecord, createdRecord,
      firstPreparedVm, firstExecutingVm, firstConfiguredSlot,
      startFenceCommand, prepareCommand, networkCommand, policyCommand,
      activateCommand, effectOutcomeValid, commandResultPresent,
      activateResource, applyFilePolicyResult, acknowledgeEffect,
      setAppliedFilePolicy, setEffect, effectId, setVm, addCommand, put,
      emptyWorld]

theorem publish_running_step :
    Step activationAcknowledged (.publishRunning sid) running := by
  apply Step.publishRunning (record := preparedRecord) (vm := firstExecutingVm)
      (vmId := firstVm) (slotId := firstSlot) (slot := firstConfiguredSlot)
      (operation := createOpPending) (ledger := createLedgerPending)
  case fenceSucceeded =>
    exact ⟨effectResult, by
      simp [effectHasStatus, currentEffectId, activationAcknowledged,
        vmActivated, policyAcknowledged, policyApplied, readyToPublish,
        preparedRecord, createdRecord,
        activateResource, applyFilePolicyResult, acknowledgeEffect,
        setAppliedFilePolicy, setEffect, setVm, put, emptyWorld]⟩
  case prepareSucceeded =>
    exact ⟨effectResult, by
      simp [effectHasStatus, currentEffectId, activationAcknowledged,
        vmActivated, policyAcknowledged, policyApplied, readyToPublish,
        preparedRecord, createdRecord,
        activateResource, applyFilePolicyResult, acknowledgeEffect,
        setAppliedFilePolicy, setEffect, setVm, put, emptyWorld]⟩
  case networkSucceeded =>
    exact ⟨effectResult, by
      simp [effectHasStatus, currentEffectId, activationAcknowledged,
        vmActivated, policyAcknowledged, policyApplied, readyToPublish,
        preparedRecord, createdRecord,
        activateResource, applyFilePolicyResult, acknowledgeEffect,
        setAppliedFilePolicy, setEffect, setVm, put, emptyWorld]⟩
  case activationSucceeded =>
    exact ⟨effectResult, by
      simp [effectHasStatus, currentEffectId, activationAcknowledged,
        vmActivated, policyAcknowledged, policyApplied, readyToPublish,
        preparedRecord, createdRecord,
        activateResource, applyFilePolicyResult, acknowledgeEffect,
        setAppliedFilePolicy, setEffect, setVm, put, emptyWorld]⟩
  case policyAcknowledged =>
    exact ⟨effectResult, by
      simp [effectHasStatus, currentEffectId, activationAcknowledged,
        vmActivated, policyAcknowledged, policyApplied, readyToPublish,
        preparedRecord, createdRecord,
        activateResource, applyFilePolicyResult, acknowledgeEffect,
        setAppliedFilePolicy, setEffect, setVm, put, emptyWorld]⟩
  all_goals
    simp [running, activationAcknowledged, vmActivated, policyAcknowledged,
      policyApplied,
      readyToPublish, preparedRecord, createdRecord, firstPreparedVm,
      firstExecutingVm, firstConfiguredSlot, policyCommand,
      startFenceCommand, prepareCommand, networkCommand, activateCommand,
      createLedgerPending, createInput, createOpPending,
      activateResource, applyFilePolicyResult, acknowledgeEffect,
      effectAcknowledged, effectSucceeded, effectHasStatus, publishRunning,
      updateCurrentOperation, setAppliedFilePolicy, setEffect, setSandbox,
      effectId, setVm, setNetSlot, addCommand, operationKey, put,
      noActiveSlot, emptyWorld] <;> grind

/-- This fragment witnesses the critical policy guard: apply, exact ack, then Running. -/
theorem policy_to_running_trace :
    Step readyToPublish (.applyFilePolicy sid 1) policyApplied ∧
    Step policyApplied
      (.acknowledgeEffect (effectId policyCommand) (.succeeded effectResult))
      policyAcknowledged ∧
    Step policyAcknowledged (.activateVm sid firstVm) vmActivated ∧
    Step vmActivated
      (.acknowledgeEffect (effectId activateCommand) (.succeeded effectResult))
      activationAcknowledged ∧
    Step activationAcknowledged (.publishRunning sid) running :=
  ⟨apply_policy_step, acknowledge_policy_step, activate_vm_step,
    acknowledge_activation_step, publish_running_step⟩

abbrev pauseOpPending : Operation := ⟨pauseOperation, 2, .pause, .pending⟩
abbrev pauseLedgerPending : OperationLedgerRecord :=
  ⟨sid, pauseOpPending, .pause 1, none⟩

def pausingRecord : SandboxRecord := {
  preparedRecord with
  phase := .pausing
  epoch := 2
  desired := .closed
  effective := .closed
  currentOperation := some pauseOpPending
}

def frozenVm : Vm := ⟨tenant, sid, 1, .frozen⟩
def stoppedVm : Vm := ⟨tenant, sid, 1, .stopped⟩
def sealedSnapshot : Snapshot :=
  ⟨tenant, sid, 1, workspace, manifest, .localSealed, false⟩
def durableSnapshot : Snapshot :=
  ⟨tenant, sid, 1, workspace, manifest, .durable, true⟩

abbrev freezeCommand : Command := .freezeVm sid 2 1 firstVm
abbrev pauseFenceCommand : Command := .installFence sid 2 false
abbrev sealCommand : Command :=
  .sealSnapshot sid 2 1 firstVm workspace snapshot manifest
abbrev commitCommand : Command := .commitSnapshot sid 2 1 snapshot manifest
abbrev stopCommand : Command := .stopVm sid 2 1 firstVm

@[simp] theorem sealCommand_ne_freezeCommand :
    effectId sealCommand ≠ effectId freezeCommand := by decide

@[simp] theorem freezeCommand_ne_sealCommand :
    effectId freezeCommand ≠ effectId sealCommand := by decide

@[simp] theorem commitCommand_ne_freezeCommand :
    effectId commitCommand ≠ effectId freezeCommand := by decide

@[simp] theorem freezeCommand_ne_commitCommand :
    effectId freezeCommand ≠ effectId commitCommand := by decide

@[simp] theorem commitCommand_ne_sealCommand :
    effectId commitCommand ≠ effectId sealCommand := by decide

@[simp] theorem sealCommand_ne_commitCommand :
    effectId sealCommand ≠ effectId commitCommand := by decide

@[simp] theorem stopCommand_ne_freezeCommand :
    effectId stopCommand ≠ effectId freezeCommand := by decide

@[simp] theorem freezeCommand_ne_stopCommand :
    effectId freezeCommand ≠ effectId stopCommand := by decide

@[simp] theorem stopCommand_ne_sealCommand :
    effectId stopCommand ≠ effectId sealCommand := by decide

@[simp] theorem sealCommand_ne_stopCommand :
    effectId sealCommand ≠ effectId stopCommand := by decide

@[simp] theorem stopCommand_ne_commitCommand :
    effectId stopCommand ≠ effectId commitCommand := by decide

@[simp] theorem commitCommand_ne_stopCommand :
    effectId commitCommand ≠ effectId stopCommand := by decide

@[simp] theorem sealCommand_ne_pauseFenceCommand :
    effectId sealCommand ≠ effectId pauseFenceCommand := by decide

@[simp] theorem pauseFenceCommand_ne_sealCommand :
    effectId pauseFenceCommand ≠ effectId sealCommand := by decide

@[simp] theorem commitCommand_ne_pauseFenceCommand :
    effectId commitCommand ≠ effectId pauseFenceCommand := by decide

@[simp] theorem pauseFenceCommand_ne_commitCommand :
    effectId pauseFenceCommand ≠ effectId commitCommand := by decide

@[simp] theorem stopCommand_ne_pauseFenceCommand :
    effectId stopCommand ≠ effectId pauseFenceCommand := by decide

@[simp] theorem pauseFenceCommand_ne_stopCommand :
    effectId pauseFenceCommand ≠ effectId stopCommand := by decide

@[simp] theorem freezeCommand_ne_pauseFenceCommand :
    effectId freezeCommand ≠ effectId pauseFenceCommand := by decide

@[simp] theorem pauseFenceCommand_ne_freezeCommand :
    effectId pauseFenceCommand ≠ effectId freezeCommand := by decide

def pauseResourceIndex : ResourceIndex := {
  vms := [firstVm]
  slots := [firstSlot]
  snapshots := []
}

def pausePlan : OperationPlan := {
  key := operationKey sid pauseOperation
  epoch := 2
  generation := 1
  vm := none
  slot := none
  restoreFrom := none
  targetSnapshot := none
  stopVms := [firstVm]
  closeSlots := [firstSlot]
  retryLimit := 3
  sealed := false
}

def sealedPausePlan : OperationPlan := {
  pausePlan with
  vm := some firstVm
  targetSnapshot := some source
  sealed := true
}

def readyToSeal : World := {
  emptyWorld with
  sandboxes := put emptyWorld.sandboxes sid pausingRecord
  vms := put emptyWorld.vms firstVm frozenVm
  filesystems := put emptyWorld.filesystems workspace ⟨tenant, sid, 1⟩
  workerFences := put emptyWorld.workerFences sid ⟨2, false⟩
  operations := put emptyWorld.operations (operationKey sid pauseOperation)
    pauseLedgerPending
  operationPlans := put emptyWorld.operationPlans
    (operationKey sid pauseOperation) sealedPausePlan
  resourceIndexes := fun candidate =>
    if candidate = sid then pauseResourceIndex
    else emptyWorld.resourceIndexes candidate
  effects := put
    (put emptyWorld.effects (effectId pauseFenceCommand)
      (.acknowledged (.succeeded effectResult)))
    (effectId freezeCommand) (.acknowledged (.succeeded effectResult))
  nextSnapshotId := 1
  nextManifestDigest := 10
}

def sealQueued : World :=
  enqueue readyToSeal sealCommand
def sealDispatched : World := dispatchEffect sealQueued (effectId sealCommand)
abbrev pauseFailure : FailureKind := .policyFailure
def sealFailed : World := acknowledgeEffect sealDispatched
  (effectId sealCommand) (.failed pauseFailure)
def sealFinalized : World := finalizeLogicalFailure sealFailed
  (logicalEffectId sealCommand) pauseFailure
def pauseCompensating : World :=
  beginCompensation sealFinalized sid pausingRecord pauseOpPending pauseFailure
def snapshotSealed : World :=
  sealSnapshot sealDispatched snapshot manifest pausingRecord sid
def sealAcknowledged : World := acknowledgeEffect snapshotSealed
  (effectId sealCommand) (.succeeded effectResult)
def commitQueued : World := enqueue sealAcknowledged commitCommand
def commitDispatched : World := dispatchEffect commitQueued (effectId commitCommand)
def snapshotDurable : World := enqueue
  (makeSnapshotDurable commitDispatched snapshot sealedSnapshot) stopCommand
def commitAcknowledged : World := acknowledgeEffect snapshotDurable
  (effectId commitCommand) (.succeeded effectResult)
def stopDispatched : World := dispatchEffect commitAcknowledged (effectId stopCommand)
def vmStopped : World := stopResource stopDispatched firstVm frozenVm
def stopAcknowledged : World := acknowledgeEffect vmStopped
  (effectId stopCommand) (.succeeded effectResult)
def paused : World := publishPaused stopAcknowledged sid pausingRecord source

theorem acknowledge_seal_failure_step :
    Step sealDispatched
      (.acknowledgeEffect (effectId sealCommand) (.failed pauseFailure))
      sealFailed := by
  apply Step.acknowledgeEffect <;>
    simp [sealFailed, sealDispatched, sealQueued, readyToSeal, sealCommand,
      pauseFailure, pausingRecord, preparedRecord, createdRecord,
      effectOutcomeValid, commandResultPresent, acknowledgeEffect, enqueue,
      commitEffect, dispatchEffect, effectId, setEffect, addCommand, put,
      emptyWorld]

theorem finalize_seal_failure_step :
    Step sealFailed (.finalizeEffectFailure (effectId sealCommand) pauseFailure)
      sealFinalized := by
  apply Step.finalizeEffectFailure
      (logical := logicalEffectId sealCommand)
      (record := {
        origin := .operation (operationKey sid pauseOperation)
        currentAttempt := 0
        remainingRetries := defaultRetryLimit
        result := none }) <;> native_decide

theorem begin_pause_compensation_step :
    Step sealFinalized (.beginCompensation sid pauseFailure)
      pauseCompensating := by
  apply Step.beginCompensation (record := pausingRecord)
      (operation := pauseOpPending) (ledger := pauseLedgerPending)
  case found =>
    simp only [sealFinalized, finalizeLogicalFailure_sandboxes]
    simp [sealFailed, sealDispatched, sealQueued, readyToSeal,
      pausingRecord, enqueue, put]
  case evidence =>
    exact Or.inr (Or.inr ⟨effectId sealCommand, by native_decide⟩)
  all_goals native_decide

theorem controller_accepts_pause_failure_compensation :
    ∃ decision,
      Controller.handle sealFinalized
        (.compensate sid pauseFailure (some (effectId sealCommand))) =
          .ok decision ∧
      decision.next = pauseCompensating := by
  let decision : Controller.Decision sealFinalized :=
    ⟨.beginCompensation sid pauseFailure, pauseCompensating,
      begin_pause_compensation_step⟩
  refine ⟨decision, ?_, rfl⟩
  have up : sealFinalized.controllerUp = true := by native_decide
  have found : sealFinalized.sandboxes sid = some pausingRecord := by
    simp [sealFinalized, sealFailed, sealDispatched, sealQueued, readyToSeal,
      pausingRecord, enqueue, put]
  have phase : pausingRecord.phase = .starting ∨
      pausingRecord.phase = .pausing := by native_decide
  have live : pausingRecord.terminal = false := by native_decide
  have current : pausingRecord.currentOperation = some pauseOpPending := by
    native_decide
  have kind : pauseOpPending.kind = .create ∨
      pauseOpPending.kind = .resume ∨ pauseOpPending.kind = .pause := by
    native_decide
  have unfinished : pauseOpPending.status = .pending ∨
      pauseOpPending.status = .outcomeUnknown := by native_decide
  have persistedFailed : sealFailed.operations
      (operationKey sid pauseOpPending.id) = some pauseLedgerPending := by
    native_decide
  have persisted : sealFinalized.operations
      (operationKey sid pauseOpPending.id) = some pauseLedgerPending := by
    simpa [sealFinalized] using persistedFailed
  have matched : pauseLedgerPending.sandbox = sid ∧
      pauseLedgerPending.operation = pauseOpPending ∧
      pauseLedgerPending.result = none := by native_decide
  have justified : Controller.compensationEvidenceCheck sealFinalized
      (operationKey sid pauseOpPending.id) pauseOpPending.epoch pauseFailure
      (some (effectId sealCommand)) = true := by native_decide
  change Controller.handleCompensation sealFinalized sid pauseFailure
      (some (effectId sealCommand)) = .ok decision
  unfold Controller.handleCompensation
  simp only [up, found, phase, live, current, kind, unfinished, persisted,
    matched, justified]
  rfl

theorem schedule_seal_step :
    Step readyToSeal (.scheduleSealSnapshot sid snapshot manifest) sealQueued := by
  apply Step.scheduleSealSnapshot (record := pausingRecord)
      (operation := pauseOpPending) (plan := sealedPausePlan) (vmId := firstVm)
      (vm := frozenVm)
  case freezeSucceeded =>
    unfold effectSucceeded effectHasStatus
    refine ⟨effectResult, ?_⟩
    have commandEq : Command.freezeVm sid pausingRecord.epoch
        pausingRecord.generation firstVm = freezeCommand := by native_decide
    have current : currentEffectId readyToSeal freezeCommand =
        effectId freezeCommand := by native_decide
    rw [commandEq, current]
    simp [readyToSeal, put]
  all_goals
    simp [readyToSeal, sealQueued, pausingRecord, preparedRecord,
      createdRecord, frozenVm, freezeCommand, sealCommand, pausePlan,
      sealedPausePlan, pauseResourceIndex, setOperationPlan, effectSucceeded,
      effectHasStatus, enqueue, commitEffect, setEffect, addCommand, put,
      emptyWorld]

theorem dispatch_seal_step :
    Step sealQueued (.dispatchEffect (effectId sealCommand)) sealDispatched := by
  apply Step.dispatchEffect <;>
    simp [sealDispatched, sealQueued, readyToSeal, sealCommand,
      enqueue, commitEffect, dispatchEffect, setEffect, put, emptyWorld]

theorem seal_manifest_step :
    Step sealDispatched (.sealSnapshot sid snapshot manifest) snapshotSealed := by
  have sealAt : Command.sealSnapshot sid pausingRecord.epoch
      pausingRecord.generation firstVm pausingRecord.workspace snapshot manifest =
        sealCommand := by native_decide
  have current : currentEffectId sealDispatched sealCommand =
      effectId sealCommand := by native_decide
  apply Step.sealSnapshot (record := pausingRecord) (vmId := firstVm)
      (vm := frozenVm) (operation := pauseOpPending)
      (plan := sealedPausePlan)
  case queued =>
    unfold deliveryQueued
    rw [sealAt, current]
    change addCommand _ sealCommand (effectId sealCommand)
    exact Or.inl rfl
  case dispatched =>
    unfold effectAwaiting effectHasStatus
    rw [sealAt, current]
    left
    simp [sealDispatched, dispatchEffect, setEffect, put]
  all_goals
    simp [sealDispatched, sealQueued, readyToSeal, snapshotSealed,
      pausingRecord, preparedRecord, createdRecord, frozenVm, sealCommand,
      pausePlan, sealedPausePlan, pauseResourceIndex, setOperationPlan,
      effectAwaiting, effectHasStatus, enqueue, commitEffect, dispatchEffect,
      sealSnapshot, setSnapshot, setEffect, addCommand, put, emptyWorld]

theorem acknowledge_seal_step :
    Step snapshotSealed
      (.acknowledgeEffect (effectId sealCommand) (.succeeded effectResult))
      sealAcknowledged := by
  apply Step.acknowledgeEffect <;>
    simp [sealAcknowledged, snapshotSealed, sealDispatched, sealQueued,
      readyToSeal, sealedSnapshot, pausingRecord, preparedRecord,
      createdRecord, sealCommand, effectOutcomeValid, commandResultPresent,
      sealSnapshot, acknowledgeEffect, enqueue, commitEffect, dispatchEffect,
      effectId, setSnapshot, setEffect, addCommand, put, emptyWorld]

theorem schedule_commit_step :
    Step sealAcknowledged (.scheduleCommitSnapshot sid snapshot)
      commitQueued := by
  apply Step.scheduleCommitSnapshot (snapshot := sealedSnapshot)
      (record := pausingRecord) (vmId := firstVm)
  case sealSucceeded =>
    unfold effectSucceeded effectHasStatus
    refine ⟨effectResult, ?_⟩
    have sealAt : Command.sealSnapshot sid pausingRecord.epoch
        pausingRecord.generation firstVm pausingRecord.workspace snapshot
        sealedSnapshot.manifest = sealCommand := by native_decide
    have current : currentEffectId sealAcknowledged sealCommand =
        effectId sealCommand := by native_decide
    rw [sealAt, current]
    simp [sealAcknowledged, acknowledgeEffect, setEffect, put]
  all_goals
    simp [commitQueued, sealAcknowledged, snapshotSealed, sealDispatched,
      sealQueued, readyToSeal, sealedSnapshot, pausingRecord, preparedRecord,
      createdRecord, sealCommand, commitCommand, effectSucceeded,
      effectHasStatus, sealSnapshot, acknowledgeEffect, enqueue, commitEffect,
      dispatchEffect, setSnapshot, setEffect, addCommand, put, emptyWorld]

theorem dispatch_commit_step :
    Step commitQueued (.dispatchEffect (effectId commitCommand))
      commitDispatched := by
  apply Step.dispatchEffect <;> native_decide

theorem commit_manifest_step :
    Step commitDispatched (.commitSnapshot snapshot) snapshotDurable := by
  have commitAt : Command.commitSnapshot sid pausingRecord.epoch
      pausingRecord.generation snapshot sealedSnapshot.manifest =
        commitCommand := by native_decide
  have current : currentEffectId commitDispatched commitCommand =
      effectId commitCommand := by native_decide
  apply Step.commitSnapshot (snapshot := sealedSnapshot)
      (record := pausingRecord) (sid := sid) (vmId := firstVm)
  case queued =>
    unfold deliveryQueued
    rw [commitAt, current]
    change addCommand _ commitCommand (effectId commitCommand)
    exact Or.inl rfl
  case dispatched =>
    unfold effectAwaiting effectHasStatus
    rw [commitAt, current]
    left
    simp [commitDispatched, dispatchEffect, setEffect, put]
  case found =>
    change
      (put (registerSnapshot sealDispatched sid snapshot).snapshots snapshot
        sealedSnapshot) snapshot = some sealedSnapshot
    simp [put]
  case foundSandbox =>
    change (put (fun _ => none) sid pausingRecord) sid = some pausingRecord
    simp [put]
  all_goals
    simp [commitDispatched, commitQueued, sealAcknowledged, snapshotSealed,
      sealDispatched, sealQueued, snapshotDurable, readyToSeal,
      sealedSnapshot, pausingRecord, preparedRecord, createdRecord,
      commitCommand, stopCommand, effectAwaiting, effectHasStatus,
      sealSnapshot, makeSnapshotDurable, acknowledgeEffect, enqueue,
      commitEffect, dispatchEffect, setSnapshot, setEffect, addCommand, put,
      emptyWorld]

theorem acknowledge_commit_step :
    Step snapshotDurable
      (.acknowledgeEffect (effectId commitCommand) (.succeeded effectResult))
      commitAcknowledged := by
  have current : currentEffectId snapshotDurable commitCommand =
      effectId commitCommand := by native_decide
  apply Step.acknowledgeEffect
  case awaiting =>
    exact Or.inl (by native_decide)
  case outcomeValid =>
    unfold effectOutcomeValid commandResultPresent
    refine ⟨durableSnapshot, ?_⟩
    constructor
    · change
        (put (registerSnapshot commitDispatched sid snapshot).snapshots snapshot
          durableSnapshot) snapshot = some durableSnapshot
      simp [put]
    · simp [durableSnapshot]
  all_goals
    simp [commitAcknowledged, snapshotDurable, commitDispatched, commitQueued,
      sealAcknowledged, snapshotSealed, sealDispatched, sealQueued,
      readyToSeal, sealedSnapshot, durableSnapshot, pausingRecord,
      preparedRecord, createdRecord, commitCommand, stopCommand,
      effectOutcomeValid, commandResultPresent, sealSnapshot,
      makeSnapshotDurable, acknowledgeEffect, enqueue, commitEffect,
      dispatchEffect, effectId, setSnapshot, setEffect, addCommand, put,
      emptyWorld]

theorem dispatch_stop_step :
    Step commitAcknowledged (.dispatchEffect (effectId stopCommand))
      stopDispatched := by
  apply Step.dispatchEffect <;> native_decide

theorem stop_for_pause_step :
    Step stopDispatched (.stopVm sid firstVm) vmStopped := by
  have stopAt : Command.stopVm sid pausingRecord.epoch frozenVm.generation
      firstVm = stopCommand := by native_decide
  have current : currentEffectId stopDispatched stopCommand =
      effectId stopCommand := by native_decide
  apply Step.stopVm (record := pausingRecord) (vm := frozenVm)
  case queued =>
    unfold deliveryQueued
    rw [stopAt, current]
    change addCommand _ stopCommand (effectId stopCommand)
    exact Or.inl rfl
  case dispatched =>
    unfold effectAwaiting effectHasStatus
    rw [stopAt, current]
    exact Or.inl (by native_decide)
  case foundSandbox =>
    change (put (fun _ => none) sid pausingRecord) sid = some pausingRecord
    simp [put]
  case found =>
    change (put (fun _ => none) firstVm frozenVm) firstVm = some frozenVm
    simp [put]
  case owned => rfl
  case stoppable => trivial
  case authorizedStop =>
    apply Or.inl
    refine ⟨rfl, rfl, rfl, rfl, ?_, ?_⟩
    · change
        (put (fun _ => none) sid
          ({ epoch := 2, terminal := false } : WorkerFence)) sid =
            some ({ epoch := 2, terminal := false } : WorkerFence)
      simp [put]
    · refine ⟨pauseOpPending, sealedPausePlan, source, durableSnapshot,
        rfl, ?_, rfl, ?_, rfl, rfl, rfl, rfl⟩
      · change
          (put (put (fun _ => none) (operationKey sid pauseOperation)
              pausePlan) (operationKey sid pauseOperation) sealedPausePlan)
            (operationKey sid pauseOperation) = some sealedPausePlan
        simp [put]
      · change
          (put (registerSnapshot commitDispatched sid snapshot).snapshots
            snapshot durableSnapshot) snapshot = some durableSnapshot
        simp [put]

theorem acknowledge_stop_step :
    Step vmStopped
      (.acknowledgeEffect (effectId stopCommand) (.succeeded effectResult))
      stopAcknowledged := by
  apply Step.acknowledgeEffect <;>
    simp [stopAcknowledged, vmStopped, stopDispatched, commitAcknowledged,
      snapshotDurable, commitDispatched, commitQueued, sealAcknowledged,
      snapshotSealed, sealDispatched, sealQueued, readyToSeal, frozenVm,
      stoppedVm, stopCommand, effectOutcomeValid, commandResultPresent,
      stopResource, acknowledgeEffect, enqueue, commitEffect, dispatchEffect,
      makeSnapshotDurable, sealSnapshot, effectId, setSnapshot, setVm, setEffect,
      addCommand, put, emptyWorld]

theorem publish_paused_step :
    Step stopAcknowledged (.publishPaused sid snapshot) paused := by
  have fenceDone : effectSucceeded stopAcknowledged pauseFenceCommand := by
    unfold effectSucceeded effectHasStatus
    refine ⟨effectResult, ?_⟩
    have current : currentEffectId stopAcknowledged pauseFenceCommand =
        effectId pauseFenceCommand := by native_decide
    rw [current]
    native_decide
  have freezeDone : effectSucceeded stopAcknowledged freezeCommand := by
    unfold effectSucceeded effectHasStatus
    refine ⟨effectResult, ?_⟩
    have current : currentEffectId stopAcknowledged freezeCommand =
        effectId freezeCommand := by native_decide
    rw [current]
    native_decide
  have sealDone : effectSucceeded stopAcknowledged sealCommand := by
    unfold effectSucceeded effectHasStatus
    refine ⟨effectResult, ?_⟩
    have current : currentEffectId stopAcknowledged sealCommand =
        effectId sealCommand := by native_decide
    rw [current]
    native_decide
  have commitDone : effectSucceeded stopAcknowledged commitCommand := by
    unfold effectSucceeded effectHasStatus
    refine ⟨effectResult, ?_⟩
    have current : currentEffectId stopAcknowledged commitCommand =
        effectId commitCommand := by native_decide
    rw [current]
    native_decide
  have stopDone : effectSucceeded stopAcknowledged stopCommand := by
    unfold effectSucceeded effectHasStatus
    refine ⟨effectResult, ?_⟩
    have current : currentEffectId stopAcknowledged stopCommand =
        effectId stopCommand := by native_decide
    rw [current]
    native_decide
  apply Step.publishPaused (record := pausingRecord) (source := source)
      (snapshot := durableSnapshot) (operation := pauseOpPending)
      (ledger := pauseLedgerPending) (vmId := firstVm)
  case up => native_decide
  case foundSandbox =>
    change (put (fun _ => none) sid pausingRecord) sid = some pausingRecord
    simp [put]
  case pausing => rfl
  case foundSnapshot =>
    change
      (put (registerSnapshot commitDispatched sid snapshot).snapshots snapshot
        durableSnapshot) snapshot = some durableSnapshot
    simp [put]
  case snapshotReady =>
    exact ⟨rfl, rfl, rfl, rfl, rfl, rfl, rfl⟩
  case fenced =>
    change
      (put (fun _ => none) sid
        ({ epoch := 2, terminal := false } : WorkerFence)) sid =
          some ({ epoch := 2, terminal := false } : WorkerFence)
    simp [put]
  case currentVm => rfl
  case fenceSucceeded =>
    simpa [pausingRecord, preparedRecord, createdRecord] using fenceDone
  case freezeSucceeded =>
    simpa [pausingRecord, preparedRecord, createdRecord] using freezeDone
  case sealSucceeded =>
    simpa [pausingRecord, preparedRecord, createdRecord, sealedSnapshot]
      using sealDone
  case commitSucceeded =>
    simpa [pausingRecord, preparedRecord, createdRecord, sealedSnapshot]
      using commitDone
  case stopSucceeded =>
    simpa [pausingRecord, preparedRecord, createdRecord] using stopDone
  case vmsClosed =>
    intro vmId vm found _owned
    have vmsEq : stopAcknowledged.vms =
        put (fun _ => none) firstVm stoppedVm := by
      funext candidate
      by_cases same : candidate = firstVm <;>
      simp [stopAcknowledged, vmStopped, stopResource, setVm,
        stopDispatched, commitAcknowledged, snapshotDurable,
        commitDispatched, commitQueued, sealAcknowledged, snapshotSealed,
        sealDispatched, sealQueued, readyToSeal, stoppedVm, frozenVm,
        emptyWorld, put, same]
    rw [vmsEq] at found
    by_cases same : vmId = firstVm
    · subst vmId
      simp [put] at found
      subst vm
      exact Or.inl rfl
    · simp [put, same] at found
  case networkClosed =>
    intro slotId slot found _owned
    change (fun _ : NetSlotId => none) slotId = some slot at found
    simp at found
  case currentOperation => rfl
  case persistedOperation =>
    change
      (put (fun _ => none) (operationKey sid pauseOperation)
        pauseLedgerPending) (operationKey sid pauseOperation) =
          some pauseLedgerPending
    simp [put]
  case operationMatches => exact ⟨rfl, rfl, rfl⟩

/-- A committed manifest leads to Paused with the exact `(snapshot, manifest)` source. -/
theorem manifest_to_paused_trace :
    Step readyToSeal (.scheduleSealSnapshot sid snapshot manifest) sealQueued ∧
    Step sealQueued (.dispatchEffect (effectId sealCommand)) sealDispatched ∧
    Step sealDispatched (.sealSnapshot sid snapshot manifest) snapshotSealed ∧
    Step snapshotSealed
      (.acknowledgeEffect (effectId sealCommand) (.succeeded effectResult))
      sealAcknowledged ∧
    Step sealAcknowledged (.scheduleCommitSnapshot sid snapshot) commitQueued ∧
    Step commitQueued (.dispatchEffect (effectId commitCommand))
      commitDispatched ∧
    Step commitDispatched (.commitSnapshot snapshot) snapshotDurable ∧
    Step snapshotDurable
      (.acknowledgeEffect (effectId commitCommand) (.succeeded effectResult))
      commitAcknowledged ∧
    Step commitAcknowledged (.dispatchEffect (effectId stopCommand))
      stopDispatched ∧
    Step stopDispatched (.stopVm sid firstVm) vmStopped ∧
    Step vmStopped
      (.acknowledgeEffect (effectId stopCommand) (.succeeded effectResult))
      stopAcknowledged ∧
    Step stopAcknowledged (.publishPaused sid snapshot) paused :=
  ⟨schedule_seal_step, dispatch_seal_step, seal_manifest_step,
    acknowledge_seal_step, schedule_commit_step, dispatch_commit_step,
    commit_manifest_step, acknowledge_commit_step, dispatch_stop_step,
    stop_for_pause_step, acknowledge_stop_step, publish_paused_step⟩

theorem accept_resume_step :
    Step paused (.acceptResume (.tenant tenant) sid resumeOperation)
      (beginResume paused sid
        { pausingRecord with
          phase := .paused
          currentVm := none
          durableSnapshot := some source
          restoreFrom := none
          netSlot := none
          desired := .paused sid source
          effective := .paused sid source
          currentOperation := some { pauseOpPending with status := .succeeded } }
        resumeOperation source) := by
  apply Step.acceptResume
  case up => rfl
  case sourceValid =>
    unfold usableSnapshot
    refine ⟨durableSnapshot, ?_, rfl, rfl, rfl, rfl, rfl, rfl⟩
    change
      (put (registerSnapshot commitDispatched sid snapshot).snapshots snapshot
        durableSnapshot) snapshot = some durableSnapshot
    simp [put]
  case freshOperation => rfl
  case freshPlan =>
    change
      (put (put (fun _ => none) (operationKey sid pauseOperation) pausePlan)
        (operationKey sid pauseOperation) sealedPausePlan)
          (operationKey sid resumeOperation) = none
    have different :
        operationKey sid resumeOperation ≠ operationKey sid pauseOperation := by
      decide
    simp [put, different]
  all_goals
    simp [paused, stopAcknowledged, vmStopped, stopDispatched,
      commitAcknowledged, snapshotDurable, commitDispatched, commitQueued,
      sealAcknowledged, snapshotSealed, sealDispatched, sealQueued, readyToSeal,
      pausingRecord, preparedRecord, createdRecord, pauseOpPending,
      sealedSnapshot, durableSnapshot, source, freezeCommand, sealCommand,
      commitCommand, stopCommand, sealSnapshot, makeSnapshotDurable,
      stopResource, acknowledgeEffect, enqueue, commitEffect, dispatchEffect,
      publishPaused, updateCurrentOperation, usableSnapshot, setSnapshot,
      setVm, setEffect, setSandbox, addCommand, operationKey, put, authorized,
      emptyWorld]

/-- Retry and query are read-only views of the persisted operation decision. -/
def runningLedger : OperationLedgerRecord :=
  ⟨sid, createOpSucceeded, createInput, some (.running 1)⟩

def operationRecord : SandboxRecord := {
  preparedRecord with
  phase := .running
  desired := .running sid 1 firstVm workspace .managedReadWrite firstSlot policy
  effective := .running sid 1 firstVm workspace .managedReadWrite firstSlot policy
  currentOperation := some createOpSucceeded
}

def operationWorld : World := {
  emptyWorld with
  sandboxes := put emptyWorld.sandboxes sid operationRecord
  operations := put emptyWorld.operations (operationKey sid createOperation)
    runningLedger
}

theorem retry_is_idempotent : Step operationWorld
    (.retryOperation (.tenant tenant) sid createOperation .create createInput)
    operationWorld := by
  exact Step.retryOperation (record := operationRecord) (ledger := runningLedger)
    (by simp [operationWorld, operationRecord, put, emptyWorld])
    (by simp [operationRecord, preparedRecord, createdRecord, authorized])
    (by simp [operationWorld, runningLedger, operationKey, put, emptyWorld])
    rfl rfl

theorem query_returns_saved_result : Step operationWorld
    (.queryOperation (.tenant tenant) sid createOperation .succeeded
      (some (.running 1))) operationWorld := by
  exact Step.queryOperation (record := operationRecord) (ledger := runningLedger)
    (by simp [operationWorld, operationRecord, put, emptyWorld])
    (by simp [operationRecord, preparedRecord, createdRecord, authorized])
    (by simp [operationWorld, runningLedger, operationKey, put, emptyWorld])
    rfl rfl

/-- Journal commit survives a controller crash before dispatch. -/
def crashedAfterCommit : World := { wCreate with controllerUp := false }
def recoveredAfterCommit : World := { crashedAfterCommit with controllerUp := true }

theorem crash_after_commit_step : Step wCreate .controllerCrash crashedAfterCommit := by
  apply Step.controllerCrash
  simp [wCreate, beginCreate, enqueue, commitEffect, storeNewOperation,
    setOperation, setEffect, setSandbox, setFilesystem, operationKey,
    put, emptyWorld]

theorem recover_after_commit_step :
    Step crashedAfterCommit .controllerRecover recoveredAfterCommit := by
  exact Step.controllerRecover (by simp [crashedAfterCommit])

theorem committed_effect_survives_crash :
    recoveredAfterCommit.effects (effectId (.installFence sid 1 false)) =
      some .committed := by
  simp [recoveredAfterCommit, crashedAfterCommit, wCreate, beginCreate,
    enqueue, commitEffect, setEffect, storeNewOperation, setOperation,
    setSandbox, setFilesystem, operationKey, put, emptyWorld]

/-- A delayed generation-1 cleanup reclaims only its exact old VM. -/
def cleanupReady : World := {
  emptyWorld with
  sandboxes := put emptyWorld.sandboxes sid
    { preparedRecord with
      generation := 2
      currentVm := some secondVm
      netSlot := some secondSlot }
  vms := put (put emptyWorld.vms firstVm stoppedVm) secondVm
    ⟨tenant, sid, 2, .executing⟩
  effects := put emptyWorld.effects
    (effectId (.cleanupVm sid 1 firstVm)) .dispatched
  inflight := addCommand emptyWorld.inflight (.cleanupVm sid 1 firstVm)
}

def cleanupDone : World := reclaimVm cleanupReady firstVm stoppedVm

theorem delayed_cleanup_step :
    Step cleanupReady (.cleanupVm sid 1 firstVm) cleanupDone := by
  apply Step.cleanupVm (vm := stoppedVm) <;>
    simp [cleanupReady, cleanupDone, preparedRecord, createdRecord, stoppedVm,
      effectDispatched, effectAwaiting, effectHasStatus, deliveryQueued,
      currentEffectId, logicalEffectId, effectId, effectIdAt, reclaimVm,
      addCommand, addEffect, setVm, put, emptyWorld]

theorem delayed_cleanup_preserves_current_vm :
    cleanupDone.vms secondVm = some ⟨tenant, sid, 2, .executing⟩ := by
  simp [cleanupDone, cleanupReady, reclaimVm, stoppedVm, setVm, put, emptyWorld]

end SandboxModel.Scenarios
