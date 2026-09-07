import SandboxModel.Proofs

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

abbrev policyCommand : Command := .applyFilePolicy sid 1 1 workspace filePolicy
abbrev activateCommand : Command := .activateVm sid 1 1 firstVm

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
    (put emptyWorld.effects (effectId policyCommand) .dispatched)
    (effectId activateCommand) .dispatched
  inflight := addCommand (addCommand emptyWorld.inflight policyCommand) activateCommand
}

def policyApplied : World := applyFilePolicyResult readyToPublish sid preparedRecord
def policyAcknowledged : World :=
  acknowledgeEffect policyApplied (effectId policyCommand) effectResult
def vmActivated : World := activateResource policyAcknowledged firstVm firstPreparedVm
def running : World :=
  publishRunning vmActivated sid preparedRecord firstSlot firstConfiguredSlot

theorem apply_policy_step :
    Step readyToPublish (.applyFilePolicy sid 1) policyApplied := by
  apply Step.applyFilePolicy (record := preparedRecord) <;>
    simp [readyToPublish, preparedRecord, createdRecord, policyCommand,
      effectDispatched, effectHasStatus, applyFilePolicyResult, setAppliedFilePolicy,
      addCommand, setEffect, put, emptyWorld]

theorem acknowledge_policy_step : Step policyApplied
    (.acknowledgeEffect (effectId policyCommand) effectResult)
    policyAcknowledged := by
  apply Step.acknowledgeEffect <;>
    simp [policyAcknowledged, policyApplied, readyToPublish, policyCommand,
      preparedRecord, createdRecord, commandResultPresent,
      applyFilePolicyResult, acknowledgeEffect,
      effectId, setAppliedFilePolicy, setEffect, addCommand, put, emptyWorld]

theorem activate_vm_step :
    Step policyAcknowledged (.activateVm sid firstVm) vmActivated := by
  apply Step.activateVm (record := preparedRecord) (vm := firstPreparedVm) <;>
    simp [vmActivated, policyAcknowledged, policyApplied, readyToPublish,
      preparedRecord, createdRecord, firstPreparedVm, firstConfiguredSlot,
      activateCommand, effectDispatched, effectHasStatus, activateResource,
      applyFilePolicyResult, acknowledgeEffect, setAppliedFilePolicy, setEffect,
      effectId, setVm, addCommand, put, noExecutingVm, emptyWorld] <;> grind

theorem publish_running_step : Step vmActivated (.publishRunning sid) running := by
  apply Step.publishRunning (record := preparedRecord) (vm := firstExecutingVm)
      (vmId := firstVm) (slotId := firstSlot) (slot := firstConfiguredSlot)
      (operation := createOpPending) (ledger := createLedgerPending)
  all_goals
    simp [running, vmActivated, policyAcknowledged, policyApplied,
      readyToPublish, preparedRecord, createdRecord, firstPreparedVm,
      firstExecutingVm, firstConfiguredSlot, policyCommand,
      createLedgerPending, createInput, createOpPending,
      activateResource, applyFilePolicyResult, acknowledgeEffect,
      effectAcknowledged, effectHasStatus, publishRunning,
      updateCurrentOperation, setAppliedFilePolicy, setEffect, setSandbox,
      effectId, setVm, setNetSlot, addCommand, operationKey, put,
      noActiveSlot, emptyWorld] <;> grind

/-- This fragment witnesses the critical policy guard: apply, exact ack, then Running. -/
theorem policy_to_running_trace :
    Step readyToPublish (.applyFilePolicy sid 1) policyApplied ∧
    Step policyApplied (.acknowledgeEffect (effectId policyCommand) effectResult)
      policyAcknowledged ∧
    Step policyAcknowledged (.activateVm sid firstVm) vmActivated ∧
    Step vmActivated (.publishRunning sid) running :=
  ⟨apply_policy_step, acknowledge_policy_step, activate_vm_step,
    publish_running_step⟩

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

def readyToSeal : World := {
  emptyWorld with
  sandboxes := put emptyWorld.sandboxes sid pausingRecord
  vms := put emptyWorld.vms firstVm frozenVm
  filesystems := put emptyWorld.filesystems workspace ⟨tenant, sid, 1⟩
  workerFences := put emptyWorld.workerFences sid ⟨2, false⟩
  operations := put emptyWorld.operations (operationKey sid pauseOperation)
    pauseLedgerPending
}

def snapshotSealed : World := sealSnapshot readyToSeal snapshot manifest pausingRecord sid
def snapshotDurable : World := makeSnapshotDurable snapshotSealed snapshot sealedSnapshot
def vmStopped : World := stopResource snapshotDurable firstVm frozenVm
def paused : World := publishPaused vmStopped sid pausingRecord source

theorem seal_manifest_step :
    Step readyToSeal (.sealSnapshot sid snapshot manifest) snapshotSealed := by
  apply Step.sealSnapshot (record := pausingRecord) (vmId := firstVm)
      (vm := frozenVm) <;>
    simp [readyToSeal, snapshotSealed, pausingRecord, preparedRecord,
      createdRecord, frozenVm, sealSnapshot, setSnapshot, put, emptyWorld]

theorem commit_manifest_step :
    Step snapshotSealed (.commitSnapshot snapshot) snapshotDurable := by
  apply Step.commitSnapshot (snapshot := sealedSnapshot) <;>
    simp [snapshotSealed, snapshotDurable, readyToSeal, sealedSnapshot,
      pausingRecord, preparedRecord, createdRecord, sealSnapshot,
      makeSnapshotDurable, setSnapshot, put, emptyWorld]

theorem stop_for_pause_step :
    Step snapshotDurable (.stopVm sid firstVm) vmStopped := by
  apply Step.stopVm (vm := frozenVm) <;>
    simp [snapshotDurable, snapshotSealed, readyToSeal, frozenVm,
      sealedSnapshot, pausingRecord, preparedRecord, createdRecord,
      sealSnapshot, makeSnapshotDurable, stopResource,
      setSnapshot, setVm, put, emptyWorld]

theorem publish_paused_step : Step vmStopped (.publishPaused sid snapshot) paused := by
  apply Step.publishPaused (record := pausingRecord) (source := source)
      (snapshot := durableSnapshot) (operation := pauseOpPending)
      (ledger := pauseLedgerPending) <;>
    simp [paused, vmStopped, snapshotDurable, snapshotSealed, readyToSeal,
      pausingRecord, preparedRecord, createdRecord, frozenVm, stoppedVm,
      sealedSnapshot, durableSnapshot, source, pauseOpPending,
      pauseLedgerPending, sealSnapshot,
      makeSnapshotDurable, stopResource, publishPaused,
      updateCurrentOperation, setSnapshot, setVm, setSandbox, operationKey,
      put, noExecutingVm, noActiveSlot, emptyWorld] <;> grind

/-- A committed manifest leads to Paused with the exact `(snapshot, manifest)` source. -/
theorem manifest_to_paused_trace :
    Step readyToSeal (.sealSnapshot sid snapshot manifest) snapshotSealed ∧
    Step snapshotSealed (.commitSnapshot snapshot) snapshotDurable ∧
    Step snapshotDurable (.stopVm sid firstVm) vmStopped ∧
    Step vmStopped (.publishPaused sid snapshot) paused :=
  ⟨seal_manifest_step, commit_manifest_step, stop_for_pause_step,
    publish_paused_step⟩

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
  apply Step.acceptResume <;>
    simp [paused, vmStopped, snapshotDurable, snapshotSealed, readyToSeal,
      pausingRecord, preparedRecord, createdRecord, pauseOpPending,
      sealedSnapshot, durableSnapshot, source, sealSnapshot,
      makeSnapshotDurable, stopResource, publishPaused,
      updateCurrentOperation, usableSnapshot, setSnapshot, setVm, setSandbox,
      operationKey, put, authorized, emptyWorld]

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
    (effectId (.cleanupVm sid 3 1 firstVm)) .dispatched
  inflight := addCommand emptyWorld.inflight (.cleanupVm sid 3 1 firstVm)
}

def cleanupDone : World := reclaimVm cleanupReady firstVm stoppedVm

theorem delayed_cleanup_step :
    Step cleanupReady (.cleanupVm sid 1 firstVm) cleanupDone := by
  apply Step.cleanupVm (epoch := 3) (vm := stoppedVm) <;>
    simp [cleanupReady, cleanupDone, preparedRecord, createdRecord, stoppedVm,
      effectDispatched, effectHasStatus, reclaimVm, addCommand, setVm,
      put, emptyWorld]

theorem delayed_cleanup_preserves_current_vm :
    cleanupDone.vms secondVm = some ⟨tenant, sid, 2, .executing⟩ := by
  simp [cleanupDone, cleanupReady, reclaimVm, stoppedVm, setVm, put, emptyWorld]

end SandboxModel.Scenarios
