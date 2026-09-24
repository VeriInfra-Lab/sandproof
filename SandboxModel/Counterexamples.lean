import SandboxModel.Scenarios

/-!
Machine-checked mutants.  Each helper deliberately bypasses one `Step` guard;
the theorem records the concrete safety fact that then fails.
-/

namespace SandboxModel.Counterexamples

set_option linter.unusedSimpArgs false
set_option linter.unnecessarySimpa false

open Scenarios

/-- Mutant: publish directly, without an applied-policy receipt or exact ack. -/
def unsafePolicyBase : World := {
  readyToPublish with
  vms := put readyToPublish.vms firstVm firstExecutingVm
}

def unsafeRunningWithoutPolicy : World :=
  publishRunning unsafePolicyBase sid preparedRecord firstSlot firstConfiguredSlot

/-- Deleting the publish guard admits Running while the applied-policy store is empty. -/
theorem missing_policy_guard_breaks_running_contract :
    ¬ RunningRequiresAppliedFilePolicy unsafeRunningWithoutPolicy := by
  intro invariant
  have receipt := invariant sid
    { preparedRecord with
      phase := .running
      currentVm := some firstConfiguredSlot.vm
      restoreFrom := none
      desired := .running sid preparedRecord.generation firstConfiguredSlot.vm
        preparedRecord.workspace preparedRecord.workspaceAccess firstSlot
        preparedRecord.policy
      effective := .running sid preparedRecord.generation firstConfiguredSlot.vm
        preparedRecord.workspace preparedRecord.workspaceAccess firstSlot
        preparedRecord.policy
      currentOperation := preparedRecord.currentOperation.map
        (fun operation => { operation with status := .succeeded }) }
    (by simp [unsafeRunningWithoutPolicy, unsafePolicyBase, readyToPublish,
      preparedRecord, createdRecord, firstConfiguredSlot,
      publishRunning, setSandbox, setNetSlot, setVm, put, emptyWorld]) rfl
  simp [unsafeRunningWithoutPolicy, unsafePolicyBase, readyToPublish,
    publishRunning, setSandbox, setNetSlot, setVm, put, emptyWorld] at receipt

/-- Mutant: call the worker update helper with a stale controller record. -/
def generationTwoRecord : SandboxRecord :=
  { preparedRecord with generation := 2 }

def stalePolicyWorld : World := {
  emptyWorld with
  sandboxes := put emptyWorld.sandboxes sid generationTwoRecord
}

def stalePolicyApplied : World :=
  applyFilePolicyResult stalePolicyWorld sid preparedRecord

theorem deleting_policy_generation_guard_records_stale_generation :
    stalePolicyApplied.appliedFilePolicies sid =
      some ⟨sid, 1, workspace, filePolicy⟩ := by
  simp [stalePolicyApplied, stalePolicyWorld, applyFilePolicyResult,
    preparedRecord, createdRecord, setAppliedFilePolicy, put, emptyWorld]

/-- The real relation rejects exactly that stale generation. -/
theorem real_policy_step_rejects_stale_generation :
    ¬ Step stalePolicyWorld (.applyFilePolicy sid 1) stalePolicyApplied := by
  apply staleFilePolicyCommandRejected (record := generationTwoRecord)
  · simp [stalePolicyWorld, generationTwoRecord, put, emptyWorld]
  · decide

abbrev foreignWorkspace : FsId := ⟨99⟩

def wrongManifestSnapshot : Snapshot :=
  ⟨tenant, sid, 1, foreignWorkspace, manifest, .durable, true⟩

def wrongManifestRecord : SandboxRecord := {
  preparedRecord with
  phase := .paused
  currentVm := none
  durableSnapshot := some source
  netSlot := none
  desired := .paused sid source
  effective := .paused sid source
}

def wrongManifestWorld : World := {
  emptyWorld with
  sandboxes := put emptyWorld.sandboxes sid wrongManifestRecord
  snapshots := put emptyWorld.snapshots snapshot wrongManifestSnapshot
}

/-- Durability alone cannot authorize restore from a manifest bound to another workspace. -/
theorem durability_without_manifest_source_check_is_unsound :
    ¬ usableSnapshot wrongManifestWorld sid wrongManifestRecord source := by
  intro usable
  rcases usable with ⟨found, present, owner, sandbox, workspaceBound,
    manifestBound, durable, immutable⟩
  have exactSnapshot : found = wrongManifestSnapshot := by
    simpa [wrongManifestWorld, put, emptyWorld] using present.symm
  subst found
  have different : foreignWorkspace ≠ workspace := by decide
  exact different (by simpa [wrongManifestSnapshot, wrongManifestRecord,
    preparedRecord, createdRecord] using workspaceBound)

abbrev orphanEffect : EffectId := effectId (.activateVm sid 8 4 secondVm)

def unsafeDispatchedWithoutCommit : World := dispatchEffect emptyWorld orphanEffect

/-- An unguarded dispatch helper can manufacture `dispatched`; `Step.dispatchEffect` cannot. -/
theorem deleting_commit_before_dispatch_guard_is_visible :
    emptyWorld.effects orphanEffect = none ∧
      unsafeDispatchedWithoutCommit.effects orphanEffect = some .dispatched := by
  simp [unsafeDispatchedWithoutCommit, orphanEffect, dispatchEffect,
    setEffect, put, emptyWorld]

theorem real_relation_rejects_uncommitted_dispatch :
    ¬ Step emptyWorld (.dispatchEffect orphanEffect) unsafeDispatchedWithoutCommit := by
  apply nondispatchableEffectCannotDispatch <;> simp [orphanEffect, emptyWorld]

/-- Mutant: reclaim the VM that the generation-2 record still names as current. -/
def currentVm : Vm := ⟨tenant, sid, 2, .stopped⟩
def currentRecord : SandboxRecord :=
  { preparedRecord with generation := 2, currentVm := some secondVm }
def cleanupCurrentBefore : World := {
  emptyWorld with
  sandboxes := put emptyWorld.sandboxes sid currentRecord
  vms := put emptyWorld.vms secondVm currentVm
}
def cleanupCurrentAfter : World := reclaimVm cleanupCurrentBefore secondVm currentVm

theorem deleting_cleanup_notCurrent_guard_breaks_reclaimed_contract :
    ¬ ReclaimedResourcesNotCurrent cleanupCurrentAfter := by
  intro safe
  have notCurrent := safe.1 secondVm
    { currentVm with status := .reclaimed }
    (by simp [cleanupCurrentAfter, cleanupCurrentBefore, reclaimVm,
      currentVm, setVm, put, emptyWorld]) rfl currentRecord
    (by simp [cleanupCurrentAfter, cleanupCurrentBefore, reclaimVm,
      currentRecord, currentVm, preparedRecord, createdRecord,
      setVm, put, emptyWorld])
  exact notCurrent rfl

/-- Mutant: overwrite a terminal epoch-5 fence with a nonterminal epoch-4 fence. -/
def fenceBefore : World :=
  setFence emptyWorld sid ⟨5, true⟩
def fenceAfterRollback : World :=
  setFence fenceBefore sid ⟨4, false⟩

theorem deleting_fence_monotonicity_admits_rollback :
    ¬ WorkerFenceMonotone fenceBefore fenceAfterRollback := by
  intro monotone
  have result := monotone sid ⟨5, true⟩ ⟨4, false⟩
    (by simp [fenceBefore, setFence, put, emptyWorld])
    (by simp [fenceAfterRollback, fenceBefore, setFence, put, emptyWorld])
  have impossible : 5 ≤ 4 := by simpa using result.1
  omega

end SandboxModel.Counterexamples
