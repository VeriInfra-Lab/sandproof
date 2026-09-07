import SandboxModel.Proofs

/-!
Constructive validation artifacts.

The safety theorem alone could be vacuous if the protocol admitted no useful
history.  This file builds a concrete create/pause/resume/kill history and
small counterexamples for deliberately weakened rules.
-/

namespace SandboxModel.Scenarios

set_option linter.unusedSimpArgs false

def tenant : TenantId := ⟨0⟩
def sid : SandboxId := ⟨0⟩
def workspace : FsId := ⟨0⟩
def firstVm : VmId := ⟨0⟩
def secondVm : VmId := ⟨1⟩
def firstSlot : NetSlotId := ⟨0⟩
def secondSlot : NetSlotId := ⟨1⟩
def snapshot : SnapshotId := ⟨0⟩
def createOperation : OperationId := ⟨0⟩
def pauseOperation : OperationId := ⟨1⟩
def resumeOperation : OperationId := ⟨2⟩
def killOperation : OperationId := ⟨3⟩
def policy : NetworkPolicy := .denyAll

def createdRecord : SandboxRecord := {
  owner := tenant
  phase := .starting
  terminal := false
  epoch := 1
  generation := 1
  currentVm := none
  workspace := workspace
  durableSnapshot := none
  restoreFrom := none
  netSlot := none
  policy := policy
  desired := .closed
  effective := .closed
  currentOperation := some ⟨createOperation, 1, .create, .pending⟩
}

def preparedRecord : SandboxRecord :=
  { createdRecord with currentVm := some firstVm, netSlot := some firstSlot }

def firstPreparedVm : Vm := ⟨tenant, sid, 1, .prepared⟩
def firstExecutingVm : Vm := ⟨tenant, sid, 1, .executing⟩
def firstFrozenVm : Vm := ⟨tenant, sid, 1, .frozen⟩
def firstStoppedVm : Vm := ⟨tenant, sid, 1, .stopped⟩

def firstUnconfiguredSlot : NetSlot := {
  owner := tenant
  sandbox := sid
  generation := 1
  vm := firstVm
  policy := policy
  configured := false
  active := false
}

def firstConfiguredSlot : NetSlot :=
  { firstUnconfiguredSlot with configured := true }

def firstActiveSlot : NetSlot :=
  { firstConfiguredSlot with active := true }

def runningRecord : SandboxRecord := {
  preparedRecord with
  phase := .running
  terminal := false
  restoreFrom := none
  desired := .running sid 1 firstVm workspace firstSlot policy
  effective := .running sid 1 firstVm workspace firstSlot policy
  currentOperation := some ⟨createOperation, 1, .create, .succeeded⟩
}

def wCreate : World :=
  beginCreate emptyWorld sid tenant workspace policy createOperation
def wStartFenced : World := applyStartFence wCreate sid createdRecord
def wPrepareDispatched : World := enqueue wStartFenced
  (.prepareVm sid 1 1 firstVm firstSlot none)
def wPrepared : World :=
  prepareResources wPrepareDispatched sid createdRecord firstVm firstSlot
def wNetworkConfigured : World :=
  configureSlot wPrepared firstSlot firstUnconfiguredSlot
def wActivated : World := activateResource wNetworkConfigured firstVm firstPreparedVm
def wRunning : World :=
  publishRunning wActivated sid preparedRecord firstSlot firstConfiguredSlot

theorem create_step : Step emptyWorld
    (.acceptCreate (.tenant tenant) tenant sid workspace policy createOperation) wCreate := by
  apply Step.acceptCreate <;>
    simp_all [wCreate, tenant, sid, workspace, policy, authorized, emptyWorld]

theorem start_fence_step : Step wCreate (.installFence sid) wStartFenced := by
  apply Step.installStartFence (record := createdRecord) <;>
    simp_all [wCreate, wStartFenced, createdRecord, beginCreate, tenant, sid,
      workspace, policy, createOperation, enqueue, addCommand, setSandbox,
      setFilesystem, put, applyStartFence]

theorem dispatch_first_prepare_step : Step wStartFenced
    (.dispatchPrepare sid firstVm firstSlot none) wPrepareDispatched := by
  apply Step.dispatchPrepare (record := createdRecord) <;>
    simp_all [wStartFenced, wPrepareDispatched, wCreate, createdRecord, beginCreate,
      applyStartFence, tenant, sid, workspace, policy, createOperation,
      firstVm, firstSlot, enqueue, addCommand, setFence, setSandbox,
      setFilesystem, put, usableSnapshot, emptyWorld]

theorem prepare_first_vm_step : Step wPrepareDispatched
    (.prepareVm sid firstVm firstSlot none) wPrepared := by
  apply Step.prepareVm (record := createdRecord) <;>
    simp_all [wPrepareDispatched, wPrepared, wStartFenced, wCreate, createdRecord,
      beginCreate, applyStartFence, prepareResources, tenant, sid, workspace,
      policy, createOperation, firstVm, firstSlot, enqueue, addCommand,
      setFence, setSandbox, setFilesystem, setVm, setNetSlot, put,
      usableSnapshot, emptyWorld]

theorem configure_first_network_step : Step wPrepared
    (.configureNetwork sid firstSlot) wNetworkConfigured := by
  apply Step.configureNetwork (record := preparedRecord)
      (slot := firstUnconfiguredSlot) (vmId := firstVm) <;>
    simp_all [wPrepared, wNetworkConfigured, wPrepareDispatched, wStartFenced,
      wCreate, createdRecord, preparedRecord, firstUnconfiguredSlot,
      beginCreate, applyStartFence, prepareResources, configureSlot, tenant,
      sid, workspace, policy, createOperation, firstVm, firstSlot, enqueue,
      addCommand, setFence, setSandbox, setFilesystem, setVm, setNetSlot, put]

theorem activate_first_vm_step : Step wNetworkConfigured
    (.activateVm sid firstVm) wActivated := by
  apply Step.activateVm (record := preparedRecord) (vm := firstPreparedVm) <;>
    simp_all [wActivated, wNetworkConfigured, wPrepared, wPrepareDispatched,
      wStartFenced, wCreate, createdRecord, preparedRecord, firstPreparedVm,
      firstUnconfiguredSlot, firstConfiguredSlot, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource, tenant, sid,
      workspace, policy, createOperation, firstVm, firstSlot, enqueue,
      addCommand, setFence, setSandbox, setFilesystem, setVm, setNetSlot, put,
      noExecutingVm, emptyWorld] <;> grind

theorem publish_first_running_step : Step wActivated
    (.publishRunning sid) wRunning := by
  apply Step.publishRunning (record := preparedRecord) (vm := firstExecutingVm)
      (vmId := firstVm) (slotId := firstSlot) (slot := firstConfiguredSlot)
  all_goals
    simp [wRunning, wActivated, wNetworkConfigured, wPrepared,
      wPrepareDispatched, wStartFenced, wCreate, createdRecord, preparedRecord,
      firstPreparedVm, firstExecutingVm, firstUnconfiguredSlot,
      firstConfiguredSlot, beginCreate, applyStartFence, prepareResources,
      configureSlot, activateResource, publishRunning, tenant, sid, workspace,
      policy, createOperation, firstVm, firstSlot, enqueue, addCommand,
      setFence, setSandbox, setFilesystem, setVm, setNetSlot, put,
      noActiveSlot, emptyWorld] <;> grind

theorem running_is_reachable : Reachable wRunning := by
  exact Reachable.step
    (Reachable.step
      (Reachable.step
        (Reachable.step
          (Reachable.step
            (Reachable.step
              (Reachable.step Reachable.initial create_step)
              start_fence_step)
            dispatch_first_prepare_step)
          prepare_first_vm_step)
        configure_first_network_step)
      activate_first_vm_step)
    publish_first_running_step

def pausingRecord : SandboxRecord := {
  runningRecord with
  phase := .pausing
  epoch := 2
  desired := .closed
  currentOperation := some ⟨pauseOperation, 2, .pause, .pending⟩
}

def pausingClosedRecord : SandboxRecord :=
  { pausingRecord with effective := .closed }

def localSnapshot : Snapshot := {
  owner := tenant
  sandbox := sid
  sourceGeneration := 1
  workspace := workspace
  stage := .localSealed
  immutable := false
}

def durableSnapshot : Snapshot :=
  { localSnapshot with stage := .durable, immutable := true }

def pausedRecord : SandboxRecord := {
  pausingClosedRecord with
  phase := .paused
  currentVm := none
  durableSnapshot := some snapshot
  restoreFrom := none
  netSlot := none
  desired := .paused sid snapshot
  effective := .paused sid snapshot
  currentOperation := some ⟨pauseOperation, 2, .pause, .succeeded⟩
}

def wPauseAccepted : World :=
  beginPause wRunning sid runningRecord pauseOperation firstVm
def wPauseFenced : World := applyRevocationFence wPauseAccepted sid pausingRecord
def wFrozen : World := setVm wPauseFenced firstVm firstFrozenVm
def wSnapshotSealed : World := sealSnapshot wFrozen snapshot pausingClosedRecord sid
def wSnapshotDurable : World :=
  makeSnapshotDurable wSnapshotSealed snapshot localSnapshot
def wFirstStopped : World := stopResource wSnapshotDurable firstVm firstFrozenVm
def wPaused : World := publishPaused wFirstStopped sid pausingClosedRecord snapshot

theorem accept_pause_step : Step wRunning
    (.acceptPause (.tenant tenant) sid pauseOperation) wPauseAccepted := by
  apply Step.acceptPause (record := runningRecord) (vmId := firstVm) <;>
    simp_all [wPauseAccepted, wRunning, runningRecord, preparedRecord,
      createdRecord, firstActiveSlot, firstConfiguredSlot,
      firstUnconfiguredSlot, firstExecutingVm, beginPause, publishRunning,
      wActivated, wNetworkConfigured, wPrepared, wPrepareDispatched,
      wStartFenced, wCreate, beginCreate, applyStartFence, prepareResources,
      configureSlot, activateResource, tenant, sid, workspace, policy,
      createOperation, pauseOperation, firstVm, firstSlot, authorized,
      enqueue, addCommand, setFence, setSandbox, setFilesystem, setVm,
      setNetSlot, put, emptyWorld]

theorem pause_fence_step : Step wPauseAccepted
    (.installFence sid) wPauseFenced := by
  apply Step.installRevocationFence (record := pausingRecord) <;>
    simp_all [wPauseFenced, wPauseAccepted, wRunning, pausingRecord,
      runningRecord, preparedRecord, createdRecord, firstActiveSlot,
      firstConfiguredSlot, firstUnconfiguredSlot, firstExecutingVm,
      beginPause, applyRevocationFence, publishRunning, wActivated,
      wNetworkConfigured, wPrepared, wPrepareDispatched, wStartFenced,
      wCreate, beginCreate, applyStartFence, prepareResources, configureSlot,
      activateResource, tenant, sid, workspace, policy, createOperation,
      pauseOperation, firstVm, firstSlot, enqueue, addCommand, setFence,
      setSandbox, setFilesystem, setVm, setNetSlot, put, emptyWorld] <;> grind

theorem freeze_first_vm_step : Step wPauseFenced
    (.freezeVm sid firstVm) wFrozen := by
  apply Step.freezeVm (record := pausingClosedRecord) (vm := firstExecutingVm) <;>
    simp_all [wFrozen, wPauseFenced, wPauseAccepted, wRunning,
      pausingClosedRecord, pausingRecord, runningRecord, preparedRecord,
      createdRecord, firstActiveSlot, firstConfiguredSlot,
      firstUnconfiguredSlot, firstPreparedVm, firstExecutingVm, firstFrozenVm, beginPause,
      applyRevocationFence, publishRunning, wActivated, wNetworkConfigured,
      wPrepared, wPrepareDispatched, wStartFenced, wCreate, beginCreate,
      applyStartFence, prepareResources, configureSlot, activateResource,
      tenant, sid, workspace, policy, createOperation, pauseOperation,
      firstVm, firstSlot, enqueue, addCommand, setFence, setSandbox,
      setFilesystem, setVm, setNetSlot, put, emptyWorld]

theorem seal_snapshot_step : Step wFrozen
    (.sealSnapshot sid snapshot) wSnapshotSealed := by
  apply Step.sealSnapshot (record := pausingClosedRecord) (vmId := firstVm)
      (vm := firstFrozenVm) <;>
    simp_all [wSnapshotSealed, wFrozen, wPauseFenced, wPauseAccepted,
      wRunning, localSnapshot, pausingClosedRecord, pausingRecord,
      runningRecord, preparedRecord, createdRecord, firstActiveSlot,
      firstConfiguredSlot, firstUnconfiguredSlot, firstExecutingVm,
      firstFrozenVm, beginPause, applyRevocationFence, publishRunning,
      sealSnapshot, wActivated, wNetworkConfigured, wPrepared,
      wPrepareDispatched, wStartFenced, wCreate, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource, tenant, sid,
      workspace, policy, createOperation, pauseOperation, firstVm, firstSlot,
      snapshot, enqueue, addCommand, setFence, setSandbox, setFilesystem,
      setVm, setSnapshot, setNetSlot, put, emptyWorld]

theorem commit_snapshot_step : Step wSnapshotSealed
    (.commitSnapshot snapshot) wSnapshotDurable := by
  apply Step.commitSnapshot (snapshot := localSnapshot) <;>
    simp_all [wSnapshotDurable, wSnapshotSealed, localSnapshot,
      makeSnapshotDurable, sealSnapshot, wFrozen, wPauseFenced,
      wPauseAccepted, wRunning, pausingClosedRecord, pausingRecord,
      runningRecord, preparedRecord, createdRecord, firstActiveSlot,
      firstConfiguredSlot, firstUnconfiguredSlot, firstExecutingVm,
      firstFrozenVm, beginPause, applyRevocationFence, publishRunning,
      wActivated, wNetworkConfigured, wPrepared, wPrepareDispatched,
      wStartFenced, wCreate, beginCreate, applyStartFence, prepareResources,
      configureSlot, activateResource, tenant, sid, workspace, policy,
      createOperation, pauseOperation, firstVm, firstSlot, snapshot, enqueue,
      addCommand, setFence, setSandbox, setFilesystem, setVm, setSnapshot,
      setNetSlot, put, emptyWorld]

theorem stop_first_vm_step : Step wSnapshotDurable
    (.stopVm sid firstVm) wFirstStopped := by
  apply Step.stopVm (vm := firstFrozenVm) <;>
    simp_all [wFirstStopped, wSnapshotDurable, wSnapshotSealed, wFrozen,
      wPauseFenced, wPauseAccepted, wRunning, localSnapshot, durableSnapshot,
      pausingClosedRecord, pausingRecord, runningRecord, preparedRecord,
      createdRecord, firstActiveSlot, firstConfiguredSlot,
      firstUnconfiguredSlot, firstExecutingVm, firstFrozenVm, beginPause,
      applyRevocationFence, publishRunning, sealSnapshot,
      makeSnapshotDurable, stopResource, wActivated, wNetworkConfigured,
      wPrepared, wPrepareDispatched, wStartFenced, wCreate, beginCreate,
      applyStartFence, prepareResources, configureSlot, activateResource,
      tenant, sid, workspace, policy, createOperation, pauseOperation,
      firstVm, firstSlot, snapshot, enqueue, addCommand, setFence, setSandbox,
      setFilesystem, setVm, setSnapshot, setNetSlot, put, emptyWorld]

theorem publish_paused_step : Step wFirstStopped
    (.publishPaused sid snapshot) wPaused := by
  apply Step.publishPaused (record := pausingClosedRecord)
      (snapshot := durableSnapshot) <;>
    simp_all [wPaused, wFirstStopped, wSnapshotDurable, wSnapshotSealed,
      wFrozen, wPauseFenced, wPauseAccepted, wRunning, localSnapshot,
      durableSnapshot, pausedRecord, pausingClosedRecord, pausingRecord,
      runningRecord, preparedRecord, createdRecord, firstActiveSlot,
      firstConfiguredSlot, firstUnconfiguredSlot, firstExecutingVm,
      firstFrozenVm, firstStoppedVm, beginPause, applyRevocationFence,
      publishRunning, sealSnapshot, makeSnapshotDurable, stopResource,
      publishPaused, wActivated, wNetworkConfigured, wPrepared,
      wPrepareDispatched, wStartFenced, wCreate, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource, tenant, sid,
      workspace, policy, createOperation, pauseOperation, firstVm, firstSlot,
      snapshot, enqueue, addCommand, setFence, setSandbox, setFilesystem,
      setVm, setSnapshot, setNetSlot, put, noExecutingVm, noActiveSlot,
      emptyWorld] <;> grind

theorem paused_is_reachable : Reachable wPaused := by
  exact Reachable.step
    (Reachable.step
      (Reachable.step
        (Reachable.step
          (Reachable.step
            (Reachable.step
              (Reachable.step running_is_reachable accept_pause_step)
              pause_fence_step)
            freeze_first_vm_step)
          seal_snapshot_step)
        commit_snapshot_step)
      stop_first_vm_step)
    publish_paused_step

def resumingRecord : SandboxRecord := {
  pausedRecord with
  phase := .starting
  epoch := 3
  generation := 2
  currentVm := none
  restoreFrom := some snapshot
  netSlot := none
  desired := .closed
  effective := .closed
  currentOperation := some ⟨resumeOperation, 3, .resume, .pending⟩
}

def secondPreparedRecord : SandboxRecord :=
  { resumingRecord with currentVm := some secondVm, netSlot := some secondSlot }

def secondPreparedVm : Vm := ⟨tenant, sid, 2, .prepared⟩
def secondExecutingVm : Vm := ⟨tenant, sid, 2, .executing⟩
def secondStoppedVm : Vm := ⟨tenant, sid, 2, .stopped⟩

def secondUnconfiguredSlot : NetSlot := {
  owner := tenant
  sandbox := sid
  generation := 2
  vm := secondVm
  policy := policy
  configured := false
  active := false
}

def secondConfiguredSlot : NetSlot :=
  { secondUnconfiguredSlot with configured := true }

def secondActiveSlot : NetSlot :=
  { secondConfiguredSlot with active := true }

def resumedRunningRecord : SandboxRecord := {
  secondPreparedRecord with
  phase := .running
  terminal := false
  restoreFrom := none
  desired := .running sid 2 secondVm workspace secondSlot policy
  effective := .running sid 2 secondVm workspace secondSlot policy
  currentOperation := some ⟨resumeOperation, 3, .resume, .succeeded⟩
}

def wResumeAccepted : World :=
  beginResume wPaused sid pausedRecord resumeOperation snapshot
def wResumeFenced : World := applyStartFence wResumeAccepted sid resumingRecord
def wSecondPrepareDispatched : World := enqueue wResumeFenced
  (.prepareVm sid 3 2 secondVm secondSlot (some snapshot))
def wSecondPrepared : World :=
  prepareResources wSecondPrepareDispatched sid resumingRecord secondVm secondSlot
def wSecondNetworkConfigured : World :=
  configureSlot wSecondPrepared secondSlot secondUnconfiguredSlot
def wSecondActivated : World :=
  activateResource wSecondNetworkConfigured secondVm secondPreparedVm
def wResumedRunning : World :=
  publishRunning wSecondActivated sid secondPreparedRecord secondSlot
    secondConfiguredSlot

theorem accept_resume_step : Step wPaused
    (.acceptResume (.tenant tenant) sid resumeOperation) wResumeAccepted := by
  apply Step.acceptResume (record := pausedRecord) (snapshotId := snapshot) <;>
    simp_all [wResumeAccepted, wPaused, pausedRecord, pausingClosedRecord,
      pausingRecord, runningRecord, preparedRecord, createdRecord,
      durableSnapshot, localSnapshot, wFirstStopped, wSnapshotDurable,
      wSnapshotSealed, wFrozen, wPauseFenced, wPauseAccepted, wRunning,
      beginResume, publishPaused, stopResource, makeSnapshotDurable,
      sealSnapshot, beginPause, applyRevocationFence, publishRunning,
      wActivated, wNetworkConfigured, wPrepared, wPrepareDispatched,
      wStartFenced, wCreate, beginCreate, applyStartFence, prepareResources,
      configureSlot, activateResource, tenant, sid, workspace, policy,
      createOperation, pauseOperation, resumeOperation, firstVm, firstSlot,
      snapshot, firstPreparedVm, firstExecutingVm, firstFrozenVm,
      firstUnconfiguredSlot, firstConfiguredSlot, authorized, usableSnapshot,
      enqueue, addCommand, setFence, setSandbox, setFilesystem, setVm,
      setSnapshot, setNetSlot, put, emptyWorld]

theorem resume_fence_step : Step wResumeAccepted
    (.installFence sid) wResumeFenced := by
  apply Step.installStartFence (record := resumingRecord) <;>
    simp_all [wResumeFenced, wResumeAccepted, resumingRecord, wPaused,
      pausedRecord, pausingClosedRecord, pausingRecord, runningRecord,
      preparedRecord, createdRecord, durableSnapshot, localSnapshot,
      wFirstStopped, wSnapshotDurable, wSnapshotSealed, wFrozen,
      wPauseFenced, wPauseAccepted, wRunning, beginResume, publishPaused,
      stopResource, makeSnapshotDurable, sealSnapshot, beginPause,
      applyRevocationFence, publishRunning, wActivated, wNetworkConfigured,
      wPrepared, wPrepareDispatched, wStartFenced, wCreate, beginCreate,
      applyStartFence, prepareResources, configureSlot, activateResource,
      tenant, sid, workspace, policy, createOperation, pauseOperation,
      resumeOperation, firstVm, firstSlot, snapshot, firstPreparedVm,
      firstExecutingVm, firstFrozenVm, firstUnconfiguredSlot,
      firstConfiguredSlot, enqueue, addCommand, setFence, setSandbox,
      setFilesystem, setVm, setSnapshot, setNetSlot, put, emptyWorld]

theorem dispatch_second_prepare_step : Step wResumeFenced
    (.dispatchPrepare sid secondVm secondSlot (some snapshot))
    wSecondPrepareDispatched := by
  apply Step.dispatchPrepare (record := resumingRecord) <;>
    simp_all [wSecondPrepareDispatched, wResumeFenced, wResumeAccepted,
      resumingRecord, wPaused, pausedRecord, pausingClosedRecord,
      pausingRecord, runningRecord, preparedRecord, createdRecord,
      durableSnapshot, localSnapshot, wFirstStopped, wSnapshotDurable,
      wSnapshotSealed, wFrozen, wPauseFenced, wPauseAccepted, wRunning,
      beginResume, publishPaused, stopResource, makeSnapshotDurable,
      sealSnapshot, beginPause, applyRevocationFence, publishRunning,
      wActivated, wNetworkConfigured, wPrepared, wPrepareDispatched,
      wStartFenced, wCreate, beginCreate, applyStartFence, prepareResources,
      configureSlot, activateResource, tenant, sid, workspace, policy,
      createOperation, pauseOperation, resumeOperation, firstVm, secondVm,
      firstSlot, secondSlot, snapshot, firstPreparedVm, firstExecutingVm,
      firstFrozenVm, firstUnconfiguredSlot, firstConfiguredSlot,
      usableSnapshot, enqueue, addCommand, setFence, setSandbox,
      setFilesystem, setVm, setSnapshot, setNetSlot, put, emptyWorld]

theorem prepare_second_vm_step : Step wSecondPrepareDispatched
    (.prepareVm sid secondVm secondSlot (some snapshot)) wSecondPrepared := by
  apply Step.prepareVm (record := resumingRecord) <;>
    simp_all [wSecondPrepared, wSecondPrepareDispatched, wResumeFenced,
      wResumeAccepted, resumingRecord, wPaused, pausedRecord,
      pausingClosedRecord, pausingRecord, runningRecord, preparedRecord,
      createdRecord, durableSnapshot, localSnapshot, wFirstStopped,
      wSnapshotDurable, wSnapshotSealed, wFrozen, wPauseFenced,
      wPauseAccepted, wRunning, beginResume, publishPaused, stopResource,
      makeSnapshotDurable, sealSnapshot, beginPause, applyRevocationFence,
      publishRunning, wActivated, wNetworkConfigured, wPrepared,
      wPrepareDispatched, wStartFenced, wCreate, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource, tenant, sid,
      workspace, policy, createOperation, pauseOperation, resumeOperation,
      firstVm, secondVm, firstSlot, secondSlot, snapshot, firstPreparedVm,
      firstExecutingVm, firstFrozenVm, firstUnconfiguredSlot,
      firstConfiguredSlot, usableSnapshot, enqueue, addCommand, setFence,
      setSandbox, setFilesystem, setVm, setSnapshot, setNetSlot, put,
      emptyWorld]

theorem configure_second_network_step : Step wSecondPrepared
    (.configureNetwork sid secondSlot) wSecondNetworkConfigured := by
  apply Step.configureNetwork (record := secondPreparedRecord)
      (slot := secondUnconfiguredSlot) (vmId := secondVm) <;>
    simp_all [wSecondNetworkConfigured, wSecondPrepared,
      wSecondPrepareDispatched, wResumeFenced, wResumeAccepted,
      secondPreparedRecord, secondUnconfiguredSlot, resumingRecord, wPaused,
      pausedRecord, pausingClosedRecord, pausingRecord, runningRecord,
      preparedRecord, createdRecord, durableSnapshot, localSnapshot,
      wFirstStopped, wSnapshotDurable, wSnapshotSealed, wFrozen,
      wPauseFenced, wPauseAccepted, wRunning, beginResume, publishPaused,
      stopResource, makeSnapshotDurable, sealSnapshot, beginPause,
      applyRevocationFence, publishRunning, wActivated, wNetworkConfigured,
      wPrepared, wPrepareDispatched, wStartFenced, wCreate, beginCreate,
      applyStartFence, prepareResources, configureSlot, activateResource,
      tenant, sid, workspace, policy, createOperation, pauseOperation,
      resumeOperation, firstVm, secondVm, firstSlot, secondSlot, snapshot,
      firstPreparedVm, firstExecutingVm, firstFrozenVm,
      firstUnconfiguredSlot, firstConfiguredSlot, enqueue, addCommand,
      setFence, setSandbox, setFilesystem, setVm, setSnapshot, setNetSlot,
      put, emptyWorld]

theorem activate_second_vm_step : Step wSecondNetworkConfigured
    (.activateVm sid secondVm) wSecondActivated := by
  apply Step.activateVm (record := secondPreparedRecord) (vm := secondPreparedVm) <;>
    simp_all [wSecondActivated, wSecondNetworkConfigured, wSecondPrepared,
      wSecondPrepareDispatched, wResumeFenced, wResumeAccepted,
      secondPreparedRecord, secondPreparedVm, secondUnconfiguredSlot,
      secondConfiguredSlot, resumingRecord, wPaused, pausedRecord,
      pausingClosedRecord, pausingRecord, runningRecord, preparedRecord,
      createdRecord, durableSnapshot, localSnapshot, wFirstStopped,
      wSnapshotDurable, wSnapshotSealed, wFrozen, wPauseFenced,
      wPauseAccepted, wRunning, beginResume, publishPaused, stopResource,
      makeSnapshotDurable, sealSnapshot, beginPause, applyRevocationFence,
      publishRunning, wActivated, wNetworkConfigured, wPrepared,
      wPrepareDispatched, wStartFenced, wCreate, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource, tenant, sid,
      workspace, policy, createOperation, pauseOperation, resumeOperation,
      firstVm, secondVm, firstSlot, secondSlot, snapshot, firstPreparedVm,
      firstExecutingVm, firstFrozenVm, firstStoppedVm,
      firstUnconfiguredSlot, firstConfiguredSlot, enqueue, addCommand,
      setFence, setSandbox, setFilesystem, setVm, setSnapshot, setNetSlot,
      put, noExecutingVm, emptyWorld] <;> grind

theorem publish_resumed_running_step : Step wSecondActivated
    (.publishRunning sid) wResumedRunning := by
  apply Step.publishRunning (record := secondPreparedRecord)
      (vmId := secondVm) (vm := secondExecutingVm) (slotId := secondSlot)
      (slot := secondConfiguredSlot)
  all_goals
    simp [wResumedRunning, wSecondActivated, wSecondNetworkConfigured,
      wSecondPrepared, wSecondPrepareDispatched, wResumeFenced,
      wResumeAccepted, resumedRunningRecord, secondPreparedRecord,
      secondPreparedVm, secondExecutingVm, secondUnconfiguredSlot,
      secondConfiguredSlot, resumingRecord, wPaused, pausedRecord,
      pausingClosedRecord, pausingRecord, runningRecord, preparedRecord,
      createdRecord, durableSnapshot, localSnapshot, wFirstStopped,
      wSnapshotDurable, wSnapshotSealed, wFrozen, wPauseFenced,
      wPauseAccepted, wRunning, beginResume, publishPaused, stopResource,
      makeSnapshotDurable, sealSnapshot, beginPause, applyRevocationFence,
      publishRunning, wActivated, wNetworkConfigured, wPrepared,
      wPrepareDispatched, wStartFenced, wCreate, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource, tenant, sid,
      workspace, policy, createOperation, pauseOperation, resumeOperation,
      firstVm, secondVm, firstSlot, secondSlot, snapshot, firstPreparedVm,
      firstExecutingVm, firstFrozenVm, firstStoppedVm,
      firstUnconfiguredSlot, firstConfiguredSlot, enqueue, addCommand,
      setFence, setSandbox, setFilesystem, setVm, setSnapshot, setNetSlot,
      put, noActiveSlot, emptyWorld] <;> grind

theorem resumed_running_is_reachable : Reachable wResumedRunning := by
  exact Reachable.step
    (Reachable.step
      (Reachable.step
        (Reachable.step
          (Reachable.step
            (Reachable.step
              (Reachable.step paused_is_reachable accept_resume_step)
              resume_fence_step)
            dispatch_second_prepare_step)
          prepare_second_vm_step)
        configure_second_network_step)
      activate_second_vm_step)
    publish_resumed_running_step

def revokingRecord : SandboxRecord := {
  resumedRunningRecord with
  phase := .revoking
  terminal := true
  epoch := 4
  desired := .closed
  currentOperation := some ⟨killOperation, 4, .kill, .pending⟩
}

def revokingClosedRecord : SandboxRecord :=
  { revokingRecord with effective := .closed }

def destroyedRecord : SandboxRecord := {
  revokingClosedRecord with
  phase := .destroyed
  terminal := true
  currentVm := none
  netSlot := none
  desired := .closed
  effective := .closed
  currentOperation := some ⟨killOperation, 4, .kill, .succeeded⟩
}

def wKillAccepted : World :=
  beginKill wResumedRunning sid resumedRunningRecord killOperation
def wKillFenced : World := applyRevocationFence wKillAccepted sid revokingRecord
def wSecondStopped : World :=
  stopResource wKillFenced secondVm secondExecutingVm
def wDestroyed : World :=
  publishDestroyed wSecondStopped sid revokingClosedRecord

theorem accept_kill_step : Step wResumedRunning
    (.acceptKill (.tenant tenant) sid killOperation) wKillAccepted := by
  apply Step.acceptKill (record := resumedRunningRecord) <;>
    simp_all [wKillAccepted, wResumedRunning, resumedRunningRecord,
      secondPreparedRecord, secondPreparedVm, secondExecutingVm,
      secondUnconfiguredSlot, secondConfiguredSlot, resumingRecord,
      wSecondActivated, wSecondNetworkConfigured, wSecondPrepared,
      wSecondPrepareDispatched, wResumeFenced, wResumeAccepted, wPaused,
      pausedRecord, pausingClosedRecord, pausingRecord, runningRecord,
      preparedRecord, createdRecord, durableSnapshot, localSnapshot,
      wFirstStopped, wSnapshotDurable, wSnapshotSealed, wFrozen,
      wPauseFenced, wPauseAccepted, wRunning, beginKill, beginResume,
      publishPaused, stopResource, makeSnapshotDurable, sealSnapshot,
      beginPause, applyRevocationFence, publishRunning, wActivated,
      wNetworkConfigured, wPrepared, wPrepareDispatched, wStartFenced,
      wCreate, beginCreate, applyStartFence, prepareResources, configureSlot,
      activateResource, tenant, sid, workspace, policy, createOperation,
      pauseOperation, resumeOperation, killOperation, firstVm, secondVm,
      firstSlot, secondSlot, snapshot, firstPreparedVm, firstExecutingVm,
      firstFrozenVm, firstStoppedVm, firstUnconfiguredSlot,
      firstConfiguredSlot, authorized, Authority.withoutResume, enqueue,
      addCommand, setFence, setSandbox, setFilesystem, setVm, setSnapshot,
      setNetSlot, put, emptyWorld]

theorem kill_fence_step : Step wKillAccepted
    (.installFence sid) wKillFenced := by
  apply Step.installRevocationFence (record := revokingRecord) <;>
    simp_all [wKillFenced, wKillAccepted, revokingRecord, wResumedRunning,
      resumedRunningRecord, secondPreparedRecord, secondPreparedVm,
      secondExecutingVm, secondUnconfiguredSlot, secondConfiguredSlot,
      resumingRecord, wSecondActivated, wSecondNetworkConfigured,
      wSecondPrepared, wSecondPrepareDispatched, wResumeFenced,
      wResumeAccepted, wPaused, pausedRecord, pausingClosedRecord,
      pausingRecord, runningRecord, preparedRecord, createdRecord,
      durableSnapshot, localSnapshot, wFirstStopped, wSnapshotDurable,
      wSnapshotSealed, wFrozen, wPauseFenced, wPauseAccepted, wRunning,
      beginKill, beginResume, publishPaused, stopResource,
      makeSnapshotDurable, sealSnapshot, beginPause, applyRevocationFence,
      publishRunning, wActivated, wNetworkConfigured, wPrepared,
      wPrepareDispatched, wStartFenced, wCreate, beginCreate,
      applyStartFence, prepareResources, configureSlot, activateResource,
      tenant, sid, workspace, policy, createOperation, pauseOperation,
      resumeOperation, killOperation, firstVm, secondVm, firstSlot,
      secondSlot, snapshot, firstPreparedVm, firstExecutingVm,
      firstFrozenVm, firstStoppedVm, firstUnconfiguredSlot,
      firstConfiguredSlot, Authority.withoutResume, enqueue, addCommand,
      setFence, setSandbox, setFilesystem, setVm, setSnapshot, setNetSlot,
      put, emptyWorld] <;> grind

theorem stop_second_vm_step : Step wKillFenced
    (.stopVm sid secondVm) wSecondStopped := by
  apply Step.stopVm (vm := secondExecutingVm) <;>
    simp_all [wSecondStopped, wKillFenced, wKillAccepted,
      revokingClosedRecord, revokingRecord, wResumedRunning,
      resumedRunningRecord, secondPreparedRecord, secondPreparedVm,
      secondExecutingVm, secondUnconfiguredSlot, secondConfiguredSlot,
      resumingRecord, wSecondActivated, wSecondNetworkConfigured,
      wSecondPrepared, wSecondPrepareDispatched, wResumeFenced,
      wResumeAccepted, wPaused, pausedRecord, pausingClosedRecord,
      pausingRecord, runningRecord, preparedRecord, createdRecord,
      durableSnapshot, localSnapshot, wFirstStopped, wSnapshotDurable,
      wSnapshotSealed, wFrozen, wPauseFenced, wPauseAccepted, wRunning,
      beginKill, beginResume, publishPaused, stopResource,
      makeSnapshotDurable, sealSnapshot, beginPause, applyRevocationFence,
      publishRunning, wActivated, wNetworkConfigured, wPrepared,
      wPrepareDispatched, wStartFenced, wCreate, beginCreate,
      applyStartFence, prepareResources, configureSlot, activateResource,
      tenant, sid, workspace, policy, createOperation, pauseOperation,
      resumeOperation, killOperation, firstVm, secondVm, firstSlot,
      secondSlot, snapshot, firstPreparedVm, firstExecutingVm,
      firstFrozenVm, firstStoppedVm, firstUnconfiguredSlot,
      firstConfiguredSlot, Authority.withoutResume, enqueue, addCommand,
      setFence, setSandbox, setFilesystem, setVm, setSnapshot, setNetSlot,
      put, emptyWorld]

theorem publish_destroyed_step : Step wSecondStopped
    (.publishDestroyed sid) wDestroyed := by
  apply Step.publishDestroyed (record := revokingClosedRecord) <;>
    simp_all [wDestroyed, wSecondStopped, wKillFenced, wKillAccepted,
      destroyedRecord, revokingClosedRecord, revokingRecord,
      wResumedRunning, resumedRunningRecord, secondPreparedRecord,
      secondPreparedVm, secondExecutingVm, secondStoppedVm,
      secondUnconfiguredSlot, secondConfiguredSlot, resumingRecord,
      wSecondActivated, wSecondNetworkConfigured, wSecondPrepared,
      wSecondPrepareDispatched, wResumeFenced, wResumeAccepted, wPaused,
      pausedRecord, pausingClosedRecord, pausingRecord, runningRecord,
      preparedRecord, createdRecord, durableSnapshot, localSnapshot,
      wFirstStopped, wSnapshotDurable, wSnapshotSealed, wFrozen,
      wPauseFenced, wPauseAccepted, wRunning, beginKill, beginResume,
      publishPaused, stopResource, makeSnapshotDurable, sealSnapshot,
      beginPause, applyRevocationFence, publishRunning, publishDestroyed,
      wActivated, wNetworkConfigured, wPrepared, wPrepareDispatched,
      wStartFenced, wCreate, beginCreate, applyStartFence, prepareResources,
      configureSlot, activateResource, tenant, sid, workspace, policy,
      createOperation, pauseOperation, resumeOperation, killOperation,
      firstVm, secondVm, firstSlot, secondSlot, snapshot, firstPreparedVm,
      firstExecutingVm, firstFrozenVm, firstStoppedVm,
      firstUnconfiguredSlot, firstConfiguredSlot, Authority.withoutResume,
      enqueue, addCommand, setFence, setSandbox, setFilesystem, setVm,
      setSnapshot, setNetSlot, put, noExecutingVm, noActiveSlot,
      emptyWorld] <;> grind

/-- The full useful lifecycle is admitted; safety is not obtained by rejecting
    every request. -/
theorem canonical_lifecycle_reaches_destroyed :
    Reachable wDestroyed ∧ wDestroyed.sandboxes sid = some destroyedRecord := by
  constructor
  · exact Reachable.step
      (Reachable.step
        (Reachable.step
          (Reachable.step resumed_running_is_reachable accept_kill_step)
          kill_fence_step)
        stop_second_vm_step)
      publish_destroyed_step
  · simp [wDestroyed, destroyedRecord, publishDestroyed, revokingClosedRecord,
      revokingRecord, setSandbox, put]

theorem canonical_lifecycle_is_safe : Safe wDestroyed :=
  reachable_safe canonical_lifecycle_reaches_destroyed.1

end SandboxModel.Scenarios
