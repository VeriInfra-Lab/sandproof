import SandboxModel.Scenarios

/-!
Small, machine-checked counterexamples for rules that are easy to omit in an
implementation.  These are not alternate specifications: each `unsafe*`
helper represents one deliberately weakened protocol action.
-/

namespace SandboxModel.Counterexamples

open Scenarios

set_option linter.unusedSimpArgs false

def executionBefore : World :=
  setVm (setVm emptyWorld firstVm firstExecutingVm) secondVm secondPreparedVm

def executionAfterUnsafeActivate : World :=
  activateResource executionBefore secondVm secondPreparedVm

theorem execution_before_is_unique : UniqueExecution executionBefore := by
  simp [UniqueExecution, executionBefore, firstVm, secondVm,
    firstExecutingVm, secondPreparedVm, setVm, put, emptyWorld] <;> grind

/-- Removing `noExecutingVm` from ActivateVm admits two executing generations. -/
theorem unsafe_activate_breaks_unique_execution :
    ¬ UniqueExecution executionAfterUnsafeActivate := by
  intro unique
  have equal := unique firstVm firstExecutingVm secondVm secondExecutingVm
    (by simp [executionAfterUnsafeActivate, executionBefore, activateResource,
      firstVm, secondVm, firstExecutingVm, secondPreparedVm, setVm, put,
      emptyWorld]) rfl
    (by simp [executionAfterUnsafeActivate, executionBefore, activateResource,
      firstVm, secondVm, firstExecutingVm, secondPreparedVm,
      secondExecutingVm, setVm, put, emptyWorld]) rfl rfl
  simp [firstVm, secondVm] at equal

def networkBefore : World :=
  setNetSlot (setNetSlot emptyWorld firstSlot firstActiveSlot)
    secondSlot secondConfiguredSlot

def networkAfterUnsafePublish : World :=
  setNetSlot networkBefore secondSlot secondActiveSlot

theorem network_before_is_unique : UniqueActiveSlot networkBefore := by
  simp [UniqueActiveSlot, networkBefore, firstSlot, secondSlot, firstActiveSlot,
    firstConfiguredSlot, firstUnconfiguredSlot, secondConfiguredSlot,
    secondUnconfiguredSlot, setNetSlot, put, emptyWorld] <;> grind

/-- Removing `noActiveSlot` from PublishRunning admits two live routes. -/
theorem unsafe_publish_breaks_unique_active_slot :
    ¬ UniqueActiveSlot networkAfterUnsafePublish := by
  intro unique
  have equal := unique firstSlot firstActiveSlot secondSlot secondActiveSlot
    (by simp [networkAfterUnsafePublish, networkBefore, firstSlot, secondSlot,
      firstActiveSlot, firstConfiguredSlot, firstUnconfiguredSlot,
      secondActiveSlot, secondConfiguredSlot, secondUnconfiguredSlot,
      setNetSlot, put, emptyWorld]) rfl
    (by simp [networkAfterUnsafePublish, networkBefore, firstSlot, secondSlot,
      firstActiveSlot, firstConfiguredSlot, firstUnconfiguredSlot,
      secondActiveSlot, secondConfiguredSlot, secondUnconfiguredSlot,
      setNetSlot, put, emptyWorld]) rfl rfl
  simp [firstSlot, secondSlot] at equal

def prematurePauseBefore : World :=
  setSandbox (setVm emptyWorld firstVm firstExecutingVm) sid pausingClosedRecord

def prematurePauseAfter : World :=
  publishPaused prematurePauseBefore sid pausingClosedRecord snapshot

theorem premature_pause_before_satisfies_stable_rule :
    StableStatesQuiescent prematurePauseBefore := by
  simp [StableStatesQuiescent, prematurePauseBefore, pausingClosedRecord,
    pausingRecord, runningRecord, preparedRecord, createdRecord, setSandbox,
    setVm, put, emptyWorld]

/-- Publishing Paused without `noExecutingVm` would label a running VM paused. -/
theorem unsafe_pause_publish_breaks_quiescence :
    ¬ StableStatesQuiescent prematurePauseAfter := by
  intro stable
  have quiet := stable sid
    { pausingClosedRecord with
      phase := .paused
      currentVm := none
      durableSnapshot := some snapshot
      restoreFrom := none
      netSlot := none
      currentOperation := pausingClosedRecord.currentOperation.map
        (fun operation => { operation with status := .succeeded })
      desired := .paused sid snapshot
      effective := .paused sid snapshot }
    (by simp [prematurePauseAfter, prematurePauseBefore, publishPaused,
      setSandbox, setVm, put, sid, firstVm, emptyWorld]) (Or.inl rfl)
  have stopped := quiet.1 firstVm firstExecutingVm
    (by simp [prematurePauseAfter, prematurePauseBefore, publishPaused,
      setSandbox, setVm, put, sid, firstVm, firstExecutingVm, emptyWorld]) rfl
  exact stopped rfl

def foreignTenant : TenantId := ⟨1⟩
def foreignSid : SandboxId := ⟨1⟩

def wrongRestoreRecord : SandboxRecord := {
  resumingRecord with restoreFrom := some snapshot
}

def wrongRestoreSnapshot : Snapshot := {
  owner := foreignTenant
  sandbox := foreignSid
  sourceGeneration := 1
  workspace := workspace
  stage := .durable
  immutable := true
}

def wrongRestoreWorld : World :=
  setSnapshot (setSandbox emptyWorld sid wrongRestoreRecord)
    snapshot wrongRestoreSnapshot

/-- Merely checking durability cannot replace owner/sandbox/source binding. -/
theorem durable_foreign_snapshot_breaks_restore_source_validity :
    ¬ RestoreSourceValid wrongRestoreWorld := by
  intro valid
  rcases valid sid wrongRestoreRecord snapshot
    (by simp [wrongRestoreWorld, setSnapshot, setSandbox, put, sid]) rfl rfl with
    ⟨foundSnapshot, present, owner, sandboxOwner, workspaceOwner, stage,
      immutable, generation⟩
  have same : foundSnapshot = wrongRestoreSnapshot := by
    simpa [wrongRestoreWorld, setSnapshot, setSandbox, put, snapshot]
      using present.symm
  subst foundSnapshot
  have different : foreignTenant ≠ tenant := by decide
  exact different (by simpa [wrongRestoreSnapshot, wrongRestoreRecord,
    resumingRecord, pausedRecord, pausingClosedRecord, pausingRecord,
    runningRecord, preparedRecord, createdRecord] using owner)

end SandboxModel.Counterexamples
