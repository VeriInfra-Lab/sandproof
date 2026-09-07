import SandboxModel.Safety

/-! Machine-checked safety theorems for arbitrary reachable worlds. -/

namespace SandboxModel

@[simp] theorem put_eq_some_iff [DecidableEq α]
    (store : α → Option β) (key candidate : α) (value result : β) :
    put store key value candidate = some result ↔
      (candidate = key ∧ value = result) ∨
      (candidate ≠ key ∧ store candidate = some result) := by
  by_cases same : candidate = key <;> simp [put, same]

theorem emptyWorld_uniqueExecution : UniqueExecution emptyWorld := by
  simp [UniqueExecution, emptyWorld]

@[simp] theorem beginCreate_vms :
    (beginCreate world sid owner workspace policy operationId).vms = world.vms := by
  simp [beginCreate, enqueue, setSandbox, setFilesystem]

@[simp] theorem applyStartFence_vms :
    (applyStartFence world sid record).vms = world.vms := by
  simp [applyStartFence, setFence]

@[simp] theorem applyStartFence_sandboxes :
    (applyStartFence world sid record).sandboxes = world.sandboxes := by
  simp [applyStartFence, setFence]

@[simp] theorem applyStartFence_workerFences :
    (applyStartFence world sid record).workerFences =
      put world.workerFences sid ⟨record.epoch, false⟩ := by
  simp [applyStartFence, setFence]

@[simp] theorem applyRevocationFence_vms :
    (applyRevocationFence world sid record).vms = world.vms := by
  unfold applyRevocationFence
  cases slotChoice : record.netSlot with
  | none => simp [setFence, setSandbox]
  | some slotId =>
      cases slotFound : world.netSlots slotId <;>
        simp [slotFound, setFence, setSandbox, setNetSlot]

@[simp] theorem applyRevocationFence_sandboxes :
    (applyRevocationFence world sid record).sandboxes =
      put world.sandboxes sid { record with effective := .closed } := by
  unfold applyRevocationFence
  cases slotChoice : record.netSlot with
  | none => simp [setFence, setSandbox]
  | some slotId =>
      cases slotFound : world.netSlots slotId <;>
        simp [slotFound, setFence, setSandbox, setNetSlot]

@[simp] theorem applyRevocationFence_workerFences :
    (applyRevocationFence world sid record).workerFences =
      put world.workerFences sid ⟨record.epoch, record.terminal⟩ := by
  unfold applyRevocationFence
  cases slotChoice : record.netSlot with
  | none => simp [setFence, setSandbox]
  | some slotId =>
      cases slotFound : world.netSlots slotId <;>
        simp [slotFound, setFence, setSandbox, setNetSlot]

@[simp] theorem applyRevocationFence_netSlots :
    (applyRevocationFence world sid record).netSlots =
      match record.netSlot with
      | none => world.netSlots
      | some slotId =>
          match world.netSlots slotId with
          | none => world.netSlots
          | some slot => put world.netSlots slotId { slot with active := false } := by
  unfold applyRevocationFence
  cases slotChoice : record.netSlot with
  | none => simp [setFence, setSandbox]
  | some slotId =>
      cases slotFound : world.netSlots slotId <;>
        simp [slotFound, setFence, setSandbox, setNetSlot]

@[simp] theorem prepareResources_vms :
    (prepareResources world sid record vmId slotId).vms =
      put world.vms vmId ⟨record.owner, sid, record.generation, .prepared⟩ := by
  simp [prepareResources, enqueue, setSandbox, setNetSlot, setVm]

@[simp] theorem configureSlot_vms :
    (configureSlot world slotId slot).vms = world.vms := by
  simp [configureSlot, setNetSlot]

@[simp] theorem activateResource_vms :
    (activateResource world vmId vm).vms =
      put world.vms vmId { vm with status := .executing } := by
  simp [activateResource, setVm]

@[simp] theorem publishRunning_vms :
    (publishRunning world sid record slotId slot).vms = world.vms := by
  simp [publishRunning, setSandbox, setNetSlot]

@[simp] theorem beginPause_vms :
    (beginPause world sid record operationId vmId).vms = world.vms := by
  simp [beginPause, enqueue, setSandbox]

@[simp] theorem sealSnapshot_vms :
    (sealSnapshot world snapshotId record sid).vms = world.vms := by
  simp [sealSnapshot, setSnapshot]

@[simp] theorem makeSnapshotDurable_vms :
    (makeSnapshotDurable world snapshotId snapshot).vms = world.vms := by
  simp [makeSnapshotDurable, setSnapshot]

@[simp] theorem stopResource_vms :
    (stopResource world vmId vm).vms =
      put world.vms vmId { vm with status := .stopped } := by
  simp [stopResource, setVm]

@[simp] theorem publishPaused_vms :
    (publishPaused world sid record snapshotId).vms = world.vms := by
  simp [publishPaused, setSandbox]

@[simp] theorem beginResume_vms :
    (beginResume world sid record operationId snapshotId).vms = world.vms := by
  simp [beginResume, enqueue, setSandbox]

@[simp] theorem beginKill_vms :
    (beginKill world sid record operationId).vms = world.vms := by
  simp [beginKill, enqueue, setSandbox]

@[simp] theorem publishDestroyed_vms :
    (publishDestroyed world sid record).vms = world.vms := by
  simp [publishDestroyed, setSandbox]

@[simp] theorem markOutcomeUnknown_vms :
    (markOutcomeUnknown world sid record operation).vms = world.vms := by
  simp [markOutcomeUnknown, setSandbox]

@[simp] theorem applyStartFence_snapshots :
    (applyStartFence world sid record).snapshots = world.snapshots := by
  simp [applyStartFence, setFence]

@[simp] theorem applyRevocationFence_snapshots :
    (applyRevocationFence world sid record).snapshots = world.snapshots := by
  unfold applyRevocationFence
  cases slotChoice : record.netSlot with
  | none => simp [setFence, setSandbox]
  | some slotId =>
      cases slotFound : world.netSlots slotId <;>
        simp [slotFound, setFence, setSandbox, setNetSlot]

@[simp] theorem applyRevocationFence_filesystems :
    (applyRevocationFence world sid record).filesystems = world.filesystems := by
  unfold applyRevocationFence
  cases slotChoice : record.netSlot with
  | none => simp [setFence, setSandbox]
  | some slotId =>
      cases slotFound : world.netSlots slotId <;>
        simp [slotFound, setFence, setSandbox, setNetSlot]

@[simp] theorem beginCreate_snapshots :
    (beginCreate world sid owner workspace policy operationId).snapshots = world.snapshots := by
  simp [beginCreate, enqueue, setSandbox, setFilesystem]

@[simp] theorem prepareResources_snapshots :
    (prepareResources world sid record vmId slotId).snapshots = world.snapshots := by
  simp [prepareResources, enqueue, setSandbox, setNetSlot, setVm]

@[simp] theorem publishRunning_snapshots :
    (publishRunning world sid record slotId slot).snapshots = world.snapshots := by
  simp [publishRunning, setSandbox, setNetSlot]

@[simp] theorem beginPause_snapshots :
    (beginPause world sid record operationId vmId).snapshots = world.snapshots := by
  simp [beginPause, enqueue, setSandbox]

@[simp] theorem publishPaused_snapshots :
    (publishPaused world sid record snapshotId).snapshots = world.snapshots := by
  simp [publishPaused, setSandbox]

@[simp] theorem beginResume_snapshots :
    (beginResume world sid record operationId snapshotId).snapshots = world.snapshots := by
  simp [beginResume, enqueue, setSandbox]

@[simp] theorem beginKill_snapshots :
    (beginKill world sid record operationId).snapshots = world.snapshots := by
  simp [beginKill, enqueue, setSandbox]

@[simp] theorem publishDestroyed_snapshots :
    (publishDestroyed world sid record).snapshots = world.snapshots := by
  simp [publishDestroyed, setSandbox]

@[simp] theorem markOutcomeUnknown_snapshots :
    (markOutcomeUnknown world sid record operation).snapshots = world.snapshots := by
  simp [markOutcomeUnknown, setSandbox]

set_option maxHeartbeats 800000 in
theorem step_preserves_uniqueExecution {world next : World} {event : Event}
    (invariant : UniqueExecution world) (step : Step world event next) :
    UniqueExecution next := by
  cases step <;>
    simp_all [UniqueExecution, enqueue, setVm, put, noExecutingVm] <;>
    grind

theorem reachable_unique_execution {world : World} (reachable : Reachable world) :
    UniqueExecution world := by
  induction reachable with
  | initial => exact emptyWorld_uniqueExecution
  | step _ transition inductionHypothesis =>
      exact step_preserves_uniqueExecution inductionHypothesis transition

theorem emptyWorld_durableSnapshotsImmutable :
    DurableSnapshotsImmutable emptyWorld := by
  simp [DurableSnapshotsImmutable, emptyWorld]

set_option maxHeartbeats 500000 in
theorem step_preserves_durableSnapshotsImmutable {world next : World} {event : Event}
    (invariant : DurableSnapshotsImmutable world) (step : Step world event next) :
    DurableSnapshotsImmutable next := by
  cases step <;>
    simp_all [DurableSnapshotsImmutable, configureSlot, activateResource,
      sealSnapshot, makeSnapshotDurable, stopResource, setVm, setSnapshot,
      setNetSlot, enqueue, put] <;>
    grind

theorem reachable_durable_snapshots_are_immutable {world : World}
    (reachable : Reachable world) : DurableSnapshotsImmutable world := by
  induction reachable with
  | initial => exact emptyWorld_durableSnapshotsImmutable
  | step _ transition inductionHypothesis =>
      exact step_preserves_durableSnapshotsImmutable inductionHypothesis transition

theorem emptyWorld_terminalConsistent : TerminalConsistent emptyWorld := by
  simp [TerminalConsistent, emptyWorld]

set_option maxHeartbeats 800000 in
theorem step_preserves_terminalConsistent {world next : World} {event : Event}
    (invariant : TerminalConsistent world) (step : Step world event next) :
    TerminalConsistent next := by
  cases step <;>
    simp_all [TerminalConsistent, beginCreate, prepareResources,
      configureSlot, activateResource, publishRunning, beginPause,
      sealSnapshot, makeSnapshotDurable, stopResource, publishPaused,
      beginResume, beginKill, publishDestroyed, markOutcomeUnknown,
      setSandbox, setFilesystem, setVm, setSnapshot, setNetSlot,
      enqueue, put] <;>
    grind

theorem reachable_terminal_consistent {world : World} (reachable : Reachable world) :
    TerminalConsistent world := by
  induction reachable with
  | initial => exact emptyWorld_terminalConsistent
  | step _ transition inductionHypothesis =>
      exact step_preserves_terminalConsistent inductionHypothesis transition

/-! A terminal bit is a tombstone: no protocol step can clear it. -/
set_option maxHeartbeats 800000 in
theorem step_never_clears_terminal {world next : World} {event : Event}
    (step : Step world event next) :
    ∀ sid before after,
      world.sandboxes sid = some before → before.terminal = true →
      next.sandboxes sid = some after → after.terminal = true := by
  cases step <;>
    simp_all [beginCreate, prepareResources, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, setSandbox, setFilesystem, setVm, setSnapshot,
      setNetSlot, enqueue, put] <;>
    grind

theorem emptyWorld_resourceOwnership : ResourceOwnership emptyWorld := by
  simp [ResourceOwnership, VmOwnership, FilesystemOwnership,
    SnapshotOwnership, NetSlotOwnership, emptyWorld]

theorem filesystemOwnership_setSandbox
    (invariant : FilesystemOwnership world)
    (found : world.sandboxes sid = some before)
    (sameOwner : after.owner = before.owner)
    (sameWorkspace : after.workspace = before.workspace) :
    FilesystemOwnership (setSandbox world sid after) := by
  intro fsId fs present
  rcases invariant fsId fs present with ⟨old, oldFound, oldOwner, oldWorkspace⟩
  by_cases sameSid : fs.sandbox = sid
  · rw [sameSid] at oldFound
    have sameRecord : old = before := by
      exact Option.some.inj (oldFound.symm.trans found)
    subst old
    exact ⟨after, by simp [setSandbox, sameSid],
      sameOwner.trans oldOwner, sameWorkspace.trans oldWorkspace⟩
  · exact ⟨old, by simp [setSandbox, sameSid, oldFound], oldOwner, oldWorkspace⟩

theorem vmOwnership_setSandbox
    (invariant : VmOwnership world)
    (found : world.sandboxes sid = some before)
    (sameOwner : after.owner = before.owner) :
    VmOwnership (setSandbox world sid after) := by
  intro vmId vm present
  rcases invariant vmId vm present with ⟨old, oldFound, oldOwner⟩
  by_cases sameSid : vm.sandbox = sid
  · rw [sameSid] at oldFound
    have sameRecord : old = before := by
      exact Option.some.inj (oldFound.symm.trans found)
    subst old
    exact ⟨after, by simp [setSandbox, sameSid], sameOwner.trans oldOwner⟩
  · exact ⟨old, by simp [setSandbox, sameSid, oldFound], oldOwner⟩

theorem snapshotOwnership_setSandbox
    (invariant : SnapshotOwnership world)
    (found : world.sandboxes sid = some before)
    (sameOwner : after.owner = before.owner)
    (sameWorkspace : after.workspace = before.workspace) :
    SnapshotOwnership (setSandbox world sid after) := by
  intro snapshotId snapshot present
  rcases invariant snapshotId snapshot present with
    ⟨old, oldFound, oldOwner, oldWorkspace⟩
  by_cases sameSid : snapshot.sandbox = sid
  · rw [sameSid] at oldFound
    have sameRecord : old = before := by
      exact Option.some.inj (oldFound.symm.trans found)
    subst old
    exact ⟨after, by simp [setSandbox, sameSid],
      sameOwner.trans oldOwner, sameWorkspace.trans oldWorkspace⟩
  · exact ⟨old, by simp [setSandbox, sameSid, oldFound], oldOwner, oldWorkspace⟩

theorem netSlotOwnership_setSandbox
    (invariant : NetSlotOwnership world)
    (found : world.sandboxes sid = some before)
    (sameOwner : after.owner = before.owner) :
    NetSlotOwnership (setSandbox world sid after) := by
  intro slotId slot present
  rcases invariant slotId slot present with ⟨old, oldFound, oldOwner⟩
  by_cases sameSid : slot.sandbox = sid
  · rw [sameSid] at oldFound
    have sameRecord : old = before := by
      exact Option.some.inj (oldFound.symm.trans found)
    subst old
    exact ⟨after, by simp [setSandbox, sameSid], sameOwner.trans oldOwner⟩
  · exact ⟨old, by simp [setSandbox, sameSid, oldFound], oldOwner⟩

theorem vmOwnership_setVm
    (invariant : VmOwnership world)
    (found : world.vms vmId = some before)
    (sameOwner : after.owner = before.owner)
    (sameSandbox : after.sandbox = before.sandbox) :
    VmOwnership (setVm world vmId after) := by
  intro candidate vm present
  by_cases same : candidate = vmId
  · subst candidate
    simp [setVm, put] at present
    subst vm
    rcases invariant vmId before found with ⟨record, recordFound, owner⟩
    exact ⟨record, by simpa [sameSandbox] using recordFound, owner.trans sameOwner.symm⟩
  · exact invariant candidate vm (by simpa [setVm, put, same] using present)

theorem snapshotOwnership_setSnapshot
    (invariant : SnapshotOwnership world)
    (found : world.snapshots snapshotId = some before)
    (sameOwner : after.owner = before.owner)
    (sameSandbox : after.sandbox = before.sandbox)
    (sameWorkspace : after.workspace = before.workspace) :
    SnapshotOwnership (setSnapshot world snapshotId after) := by
  intro candidate snapshot present
  by_cases same : candidate = snapshotId
  · subst candidate
    simp [setSnapshot, put] at present
    subst snapshot
    rcases invariant snapshotId before found with
      ⟨record, recordFound, owner, workspace⟩
    exact ⟨record, by simpa [sameSandbox] using recordFound,
      owner.trans sameOwner.symm, workspace.trans sameWorkspace.symm⟩
  · exact invariant candidate snapshot
      (by simpa [setSnapshot, put, same] using present)

theorem netSlotOwnership_setNetSlot
    (invariant : NetSlotOwnership world)
    (found : world.netSlots slotId = some before)
    (sameOwner : after.owner = before.owner)
    (sameSandbox : after.sandbox = before.sandbox) :
    NetSlotOwnership (setNetSlot world slotId after) := by
  intro candidate slot present
  by_cases same : candidate = slotId
  · subst candidate
    simp [setNetSlot, put] at present
    subst slot
    rcases invariant slotId before found with ⟨record, recordFound, owner⟩
    exact ⟨record, by simpa [sameSandbox] using recordFound, owner.trans sameOwner.symm⟩
  · exact invariant candidate slot (by simpa [setNetSlot, put, same] using present)

theorem resourceOwnership_setSandbox
    (invariant : ResourceOwnership world)
    (found : world.sandboxes sid = some before)
    (sameOwner : after.owner = before.owner)
    (sameWorkspace : after.workspace = before.workspace) :
    ResourceOwnership (setSandbox world sid after) := by
  rcases invariant with ⟨vms, filesystems, snapshots, slots⟩
  exact ⟨vmOwnership_setSandbox vms found sameOwner,
    filesystemOwnership_setSandbox filesystems found sameOwner sameWorkspace,
    snapshotOwnership_setSandbox snapshots found sameOwner sameWorkspace,
    netSlotOwnership_setSandbox slots found sameOwner⟩

theorem resourceOwnership_setVm
    (invariant : ResourceOwnership world)
    (found : world.vms vmId = some before)
    (sameOwner : after.owner = before.owner)
    (sameSandbox : after.sandbox = before.sandbox) :
    ResourceOwnership (setVm world vmId after) := by
  rcases invariant with ⟨vms, filesystems, snapshots, slots⟩
  exact ⟨vmOwnership_setVm vms found sameOwner sameSandbox,
    by simpa [FilesystemOwnership, setVm] using filesystems,
    by simpa [SnapshotOwnership, setVm] using snapshots,
    by simpa [NetSlotOwnership, setVm] using slots⟩

theorem resourceOwnership_setSnapshot
    (invariant : ResourceOwnership world)
    (found : world.snapshots snapshotId = some before)
    (sameOwner : after.owner = before.owner)
    (sameSandbox : after.sandbox = before.sandbox)
    (sameWorkspace : after.workspace = before.workspace) :
    ResourceOwnership (setSnapshot world snapshotId after) := by
  rcases invariant with ⟨vms, filesystems, snapshots, slots⟩
  exact ⟨by simpa [VmOwnership, setSnapshot] using vms,
    by simpa [FilesystemOwnership, setSnapshot] using filesystems,
    snapshotOwnership_setSnapshot snapshots found sameOwner sameSandbox sameWorkspace,
    by simpa [NetSlotOwnership, setSnapshot] using slots⟩

theorem resourceOwnership_setNetSlot
    (invariant : ResourceOwnership world)
    (found : world.netSlots slotId = some before)
    (sameOwner : after.owner = before.owner)
    (sameSandbox : after.sandbox = before.sandbox) :
    ResourceOwnership (setNetSlot world slotId after) := by
  rcases invariant with ⟨vms, filesystems, snapshots, slots⟩
  exact ⟨by simpa [VmOwnership, setNetSlot] using vms,
    by simpa [FilesystemOwnership, setNetSlot] using filesystems,
    by simpa [SnapshotOwnership, setNetSlot] using snapshots,
    netSlotOwnership_setNetSlot slots found sameOwner sameSandbox⟩

theorem resourceOwnership_insertSandbox
    (invariant : ResourceOwnership world)
    (fresh : world.sandboxes sid = none) :
    ResourceOwnership (setSandbox world sid record) := by
  rcases invariant with ⟨vms, filesystems, snapshots, slots⟩
  constructor
  · intro vmId vm present
    rcases vms vmId vm present with ⟨old, oldFound, owner⟩
    have different : vm.sandbox ≠ sid := by
      intro same
      rw [same, fresh] at oldFound
      contradiction
    exact ⟨old, by simpa [setSandbox, put, different] using oldFound, owner⟩
  constructor
  · intro fsId fs present
    rcases filesystems fsId fs present with ⟨old, oldFound, owner, workspace⟩
    have different : fs.sandbox ≠ sid := by
      intro same
      rw [same, fresh] at oldFound
      contradiction
    exact ⟨old, by simpa [setSandbox, put, different] using oldFound,
      owner, workspace⟩
  constructor
  · intro snapshotId snapshot present
    rcases snapshots snapshotId snapshot present with
      ⟨old, oldFound, owner, workspace⟩
    have different : snapshot.sandbox ≠ sid := by
      intro same
      rw [same, fresh] at oldFound
      contradiction
    exact ⟨old, by simpa [setSandbox, put, different] using oldFound,
      owner, workspace⟩
  · intro slotId slot present
    rcases slots slotId slot present with ⟨old, oldFound, owner⟩
    have different : slot.sandbox ≠ sid := by
      intro same
      rw [same, fresh] at oldFound
      contradiction
    exact ⟨old, by simpa [setSandbox, put, different] using oldFound, owner⟩

theorem resourceOwnership_insertVm
    (invariant : ResourceOwnership world)
    (recordFound : world.sandboxes vm.sandbox = some record)
    (owner : record.owner = vm.owner) :
    ResourceOwnership (setVm world vmId vm) := by
  rcases invariant with ⟨vms, filesystems, snapshots, slots⟩
  constructor
  · intro candidate current present
    by_cases same : candidate = vmId
    · subst candidate
      simp [setVm, put] at present
      subst current
      exact ⟨record, recordFound, owner⟩
    · exact vms candidate current (by simpa [setVm, put, same] using present)
  exact ⟨by simpa [FilesystemOwnership, setVm] using filesystems,
    by simpa [SnapshotOwnership, setVm] using snapshots,
    by simpa [NetSlotOwnership, setVm] using slots⟩

theorem resourceOwnership_insertFilesystem
    (invariant : ResourceOwnership world)
    (recordFound : world.sandboxes fs.sandbox = some record)
    (owner : record.owner = fs.owner)
    (workspace : record.workspace = fsId) :
    ResourceOwnership (setFilesystem world fsId fs) := by
  rcases invariant with ⟨vms, filesystems, snapshots, slots⟩
  refine ⟨by simpa [VmOwnership, setFilesystem] using vms, ?_,
    by simpa [SnapshotOwnership, setFilesystem] using snapshots,
    by simpa [NetSlotOwnership, setFilesystem] using slots⟩
  intro candidate current present
  by_cases same : candidate = fsId
  · subst candidate
    simp [setFilesystem, put] at present
    subst current
    exact ⟨record, recordFound, owner, workspace⟩
  · exact filesystems candidate current
      (by simpa [setFilesystem, put, same] using present)

theorem resourceOwnership_insertSnapshot
    (invariant : ResourceOwnership world)
    (recordFound : world.sandboxes snapshot.sandbox = some record)
    (owner : record.owner = snapshot.owner)
    (workspace : record.workspace = snapshot.workspace) :
    ResourceOwnership (setSnapshot world snapshotId snapshot) := by
  rcases invariant with ⟨vms, filesystems, snapshots, slots⟩
  refine ⟨by simpa [VmOwnership, setSnapshot] using vms,
    by simpa [FilesystemOwnership, setSnapshot] using filesystems, ?_,
    by simpa [NetSlotOwnership, setSnapshot] using slots⟩
  intro candidate current present
  by_cases same : candidate = snapshotId
  · subst candidate
    simp [setSnapshot, put] at present
    subst current
    exact ⟨record, recordFound, owner, workspace⟩
  · exact snapshots candidate current
      (by simpa [setSnapshot, put, same] using present)

theorem resourceOwnership_insertNetSlot
    (invariant : ResourceOwnership world)
    (recordFound : world.sandboxes slot.sandbox = some record)
    (owner : record.owner = slot.owner) :
    ResourceOwnership (setNetSlot world slotId slot) := by
  rcases invariant with ⟨vms, filesystems, snapshots, slots⟩
  refine ⟨by simpa [VmOwnership, setNetSlot] using vms,
    by simpa [FilesystemOwnership, setNetSlot] using filesystems,
    by simpa [SnapshotOwnership, setNetSlot] using snapshots, ?_⟩
  intro candidate current present
  by_cases same : candidate = slotId
  · subst candidate
    simp [setNetSlot, put] at present
    subst current
    exact ⟨record, recordFound, owner⟩
  · exact slots candidate current (by simpa [setNetSlot, put, same] using present)

theorem resourceOwnership_beginCreate
    (invariant : ResourceOwnership world)
    (freshSandbox : world.sandboxes sid = none) :
    ResourceOwnership (beginCreate world sid owner workspace policy operationId) := by
  let record : SandboxRecord := {
    owner := owner
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
    currentOperation := some ⟨operationId, 1, .create, .pending⟩
  }
  let filesystem : Filesystem := ⟨owner, sid, 1⟩
  have withSandbox := resourceOwnership_insertSandbox (record := record)
    invariant freshSandbox
  have recordFound : (setSandbox world sid record).sandboxes sid = some record := by
    simp [setSandbox, put]
  have withFilesystem := resourceOwnership_insertFilesystem
    (fs := filesystem) withSandbox recordFound rfl rfl
  simpa [beginCreate, record, filesystem, enqueue, setSandbox, setFilesystem] using
    withFilesystem

theorem resourceOwnership_applyRevocationFence
    (invariant : ResourceOwnership world)
    (found : world.sandboxes sid = some record) :
    ResourceOwnership (applyRevocationFence world sid record) := by
  let updated := { record with effective := .closed }
  have closed := resourceOwnership_setSandbox (after := updated)
    invariant found rfl rfl
  have fenced : ResourceOwnership (setFence (setSandbox world sid updated)
      sid ⟨record.epoch, record.terminal⟩) := by
    simpa [ResourceOwnership, VmOwnership, FilesystemOwnership,
      SnapshotOwnership, NetSlotOwnership, setFence] using closed
  unfold applyRevocationFence
  cases slotChoice : record.netSlot with
  | none => simpa [slotChoice, updated] using fenced
  | some slotId =>
      cases slotFound : world.netSlots slotId with
      | none => simpa [slotChoice, slotFound, updated] using fenced
      | some slot =>
          have foundAfterFence :
              (setFence (setSandbox world sid updated)
                sid ⟨record.epoch, record.terminal⟩).netSlots slotId = some slot := by
            simpa [setFence, setSandbox] using slotFound
          simpa [slotChoice, slotFound, updated] using
            (resourceOwnership_setNetSlot
              (after := { slot with active := false }) fenced foundAfterFence rfl rfl)

theorem resourceOwnership_prepareResources
    (invariant : ResourceOwnership world)
    (found : world.sandboxes sid = some record) :
    ResourceOwnership (prepareResources world sid record vmId slotId) := by
  let vm : Vm := ⟨record.owner, sid, record.generation, .prepared⟩
  let slot : NetSlot := {
    owner := record.owner
    sandbox := sid
    generation := record.generation
    vm := vmId
    policy := record.policy
    configured := false
    active := false
  }
  let withVm := setVm world vmId vm
  have vmOwned := resourceOwnership_insertVm (vmId := vmId) (vm := vm)
    invariant found rfl
  have recordAfterVm : withVm.sandboxes sid = some record := by
    simpa [withVm, setVm] using found
  have slotOwned := resourceOwnership_insertNetSlot (slotId := slotId) (slot := slot) vmOwned
    recordAfterVm rfl
  let updated := { record with currentVm := some vmId, netSlot := some slotId }
  have recordBeforeUpdate :
      (setNetSlot withVm slotId slot).sandboxes sid = some record := by
    simpa [withVm, setNetSlot] using recordAfterVm
  have updatedOwned := resourceOwnership_setSandbox (after := updated) slotOwned
    recordBeforeUpdate rfl rfl
  simpa [prepareResources, vm, slot, withVm, updated, enqueue] using updatedOwned

theorem resourceOwnership_publishRunning
    (invariant : ResourceOwnership world)
    (foundSandbox : world.sandboxes sid = some record)
    (foundSlot : world.netSlots slotId = some slot) :
    ResourceOwnership (publishRunning world sid record slotId slot) := by
  let activated := { slot with active := true }
  have slotOwned := resourceOwnership_setNetSlot (after := activated)
    invariant foundSlot rfl rfl
  let base := { record with
    phase := .running
    terminal := false
    restoreFrom := none
    currentOperation := record.currentOperation.map
      (fun (op : Operation) => { op with status := .succeeded })
  }
  let authority : Authority :=
    .running sid base.generation slot.vm base.workspace slotId base.policy
  let running := { base with desired := authority, effective := authority }
  have foundAfterSlot :
      (setNetSlot world slotId activated).sandboxes sid = some record := by
    simpa [setNetSlot] using foundSandbox
  have sandboxOwned := resourceOwnership_setSandbox (after := running)
    slotOwned foundAfterSlot rfl rfl
  simpa [publishRunning, activated, base, authority, running] using sandboxOwned

theorem resourceOwnership_sealSnapshot
    (invariant : ResourceOwnership world)
    (found : world.sandboxes sid = some record) :
    ResourceOwnership (sealSnapshot world snapshotId record sid) := by
  let snapshot : Snapshot := {
    owner := record.owner
    sandbox := sid
    sourceGeneration := record.generation
    workspace := record.workspace
    stage := .localSealed
    immutable := false
  }
  simpa [sealSnapshot, snapshot] using
    (resourceOwnership_insertSnapshot (snapshot := snapshot)
      (snapshotId := snapshotId)
      invariant found rfl rfl)

theorem step_preserves_resourceOwnership {world next : World} {event : Event}
    (invariant : ResourceOwnership world) (step : Step world event next) :
    ResourceOwnership next := by
  cases step
  case acceptCreate =>
    exact resourceOwnership_beginCreate invariant (by assumption)
  case installStartFence =>
    simpa [applyStartFence, setFence] using invariant
  case installRevocationFence =>
    exact resourceOwnership_applyRevocationFence invariant (by assumption)
  case dispatchPrepare =>
    simpa [enqueue] using invariant
  case prepareVm =>
    exact resourceOwnership_prepareResources invariant (by assumption)
  case configureNetwork =>
    simpa [configureSlot] using
      (resourceOwnership_setNetSlot (after := { ‹NetSlot› with configured := true })
        (slotId := ‹NetSlotId›) (before := ‹NetSlot›)
        invariant (by assumption) rfl rfl)
  case activateVm =>
    simpa [activateResource] using
      (resourceOwnership_setVm (after := { ‹Vm› with status := .executing })
        (vmId := ‹VmId›) (before := ‹Vm›)
        invariant (by assumption) rfl rfl)
  case publishRunning =>
    exact resourceOwnership_publishRunning invariant (by assumption) (by assumption)
  case acceptPause =>
    let record := ‹SandboxRecord›
    let operationId := ‹OperationId›
    let updated := { record with
      phase := .pausing
      epoch := record.epoch + 1
      desired := .closed
      currentOperation := some ⟨operationId, record.epoch + 1, .pause, .pending⟩
    }
    simpa [beginPause, enqueue] using
      (resourceOwnership_setSandbox (after := updated)
        (sid := ‹SandboxId›) (before := record)
        invariant (by assumption) rfl rfl)
  case freezeVm =>
    simpa [setVm] using
      (resourceOwnership_setVm (after := { ‹Vm› with status := .frozen })
        (vmId := ‹VmId›) (before := ‹Vm›)
        invariant (by assumption) rfl rfl)
  case sealSnapshot =>
    exact resourceOwnership_sealSnapshot invariant (by assumption)
  case commitSnapshot =>
    simpa [makeSnapshotDurable] using
      (resourceOwnership_setSnapshot
        (after := { ‹Snapshot› with stage := .durable, immutable := true })
        (snapshotId := ‹SnapshotId›) (before := ‹Snapshot›)
        invariant (by assumption) rfl rfl rfl)
  case stopVm =>
    simpa [stopResource] using
      (resourceOwnership_setVm (after := { ‹Vm› with status := .stopped })
        (vmId := ‹VmId›) (before := ‹Vm›)
        invariant (by assumption) rfl rfl)
  case publishPaused =>
    let record := ‹SandboxRecord›
    let sid := ‹SandboxId›
    let snapshotId := ‹SnapshotId›
    let base := { record with
      phase := .paused
      currentVm := none
      durableSnapshot := some snapshotId
      restoreFrom := none
      netSlot := none
      currentOperation := record.currentOperation.map
        (fun (op : Operation) => { op with status := .succeeded })
    }
    let authority : Authority := .paused sid snapshotId
    let updated := { base with desired := authority, effective := authority }
    simpa [publishPaused] using
      (resourceOwnership_setSandbox (after := updated)
        (sid := sid) (before := record)
        invariant (by assumption) rfl rfl)
  case acceptResume =>
    let record := ‹SandboxRecord›
    let operationId := ‹OperationId›
    let snapshotId := ‹SnapshotId›
    let updated := { record with
      phase := .starting
      epoch := record.epoch + 1
      generation := record.generation + 1
      currentVm := none
      restoreFrom := some snapshotId
      netSlot := none
      desired := .closed
      effective := .closed
      currentOperation := some ⟨operationId, record.epoch + 1, .resume, .pending⟩
    }
    simpa [beginResume, enqueue] using
      (resourceOwnership_setSandbox (after := updated)
        (sid := ‹SandboxId›) (before := record)
        invariant (by assumption) rfl rfl)
  case acceptKill =>
    let record := ‹SandboxRecord›
    let operationId := ‹OperationId›
    let updated := { record with
      phase := .revoking
      terminal := true
      epoch := record.epoch + 1
      desired := .closed
      effective := record.effective.withoutResume
      currentOperation := some ⟨operationId, record.epoch + 1, .kill, .pending⟩
    }
    simpa [beginKill, enqueue] using
      (resourceOwnership_setSandbox (after := updated)
        (sid := ‹SandboxId›) (before := record)
        invariant (by assumption) rfl rfl)
  case publishDestroyed =>
    let record := ‹SandboxRecord›
    let destroyed := { record with
      phase := .destroyed
      terminal := true
      currentVm := none
      netSlot := none
      desired := .closed
      effective := .closed
      currentOperation := record.currentOperation.map
        (fun (op : Operation) => { op with status := .succeeded })
    }
    simpa [publishDestroyed] using
      (resourceOwnership_setSandbox (after := destroyed)
        (sid := ‹SandboxId›) (before := record)
        invariant (by assumption) rfl rfl)
  case rpcTimeout =>
    let record := ‹SandboxRecord›
    let operation := ‹Operation›
    let updated := { record with
      currentOperation := some { operation with status := .outcomeUnknown }
    }
    simpa [markOutcomeUnknown] using
      (resourceOwnership_setSandbox (after := updated)
        (sid := ‹SandboxId›) (before := record)
        invariant (by assumption) rfl rfl)
  all_goals simpa using invariant

theorem emptyWorld_workspaceBindingValid : WorkspaceBindingValid emptyWorld := by
  simp [WorkspaceBindingValid, emptyWorld]

theorem workspaceBindingValid_beginCreate
    (invariant : WorkspaceBindingValid world)
    (freshWorkspace : world.filesystems workspace = none) :
    WorkspaceBindingValid (beginCreate world sid owner workspace policy operationId) := by
  intro target record found
  by_cases same : target = sid
  · subst target
    simp [beginCreate, enqueue, setSandbox, setFilesystem, put] at found
    subst record
    exact ⟨⟨owner, sid, 1⟩, by simp [beginCreate, enqueue, setSandbox,
      setFilesystem, put], rfl, rfl⟩
  · have oldFound : world.sandboxes target = some record := by
      simpa [beginCreate, enqueue, setSandbox, setFilesystem, put, same] using found
    rcases invariant target record oldFound with
      ⟨filesystem, filesystemFound, fsOwner, fsSandbox⟩
    have different : record.workspace ≠ workspace := by
      intro equal
      rw [equal, freshWorkspace] at filesystemFound
      contradiction
    exact ⟨filesystem, by simpa [beginCreate, enqueue, setSandbox,
      setFilesystem, put, different] using filesystemFound, fsOwner, fsSandbox⟩

theorem workspaceBindingValid_applyRevocationFence
    (invariant : WorkspaceBindingValid world)
    (found : world.sandboxes sid = some record) :
    WorkspaceBindingValid (applyRevocationFence world sid record) := by
  intro target current currentFound
  by_cases same : target = sid
  · subst target
    have currentClosed : current = { record with effective := .closed } := by
      simp [applyRevocationFence_sandboxes, put] at currentFound
      exact currentFound.symm
    subst current
    simpa only [applyRevocationFence_filesystems] using invariant sid record found
  · have oldFound : world.sandboxes target = some current := by
      simpa [applyRevocationFence_sandboxes, put, same] using currentFound
    simpa only [applyRevocationFence_filesystems] using invariant target current oldFound

set_option maxHeartbeats 1000000 in
theorem step_preserves_workspaceBindingValid {world next : World} {event : Event}
    (invariant : WorkspaceBindingValid world) (step : Step world event next) :
    WorkspaceBindingValid next := by
  cases step
  case acceptCreate =>
    exact workspaceBindingValid_beginCreate invariant
      (by assumption)
  case installRevocationFence =>
    exact workspaceBindingValid_applyRevocationFence invariant (by assumption)
  all_goals
    simp_all [WorkspaceBindingValid, applyStartFence,
      prepareResources, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, setSandbox, setFence, setVm,
      setSnapshot, setNetSlot, enqueue, put] <;>
    grind

theorem emptyWorld_uniqueActiveSlot : UniqueActiveSlot emptyWorld := by
  simp [UniqueActiveSlot, emptyWorld]

theorem uniqueActiveSlot_set_inactive
    {world : World} {slotId : NetSlotId} {slot : NetSlot}
    (invariant : UniqueActiveSlot world) :
    UniqueActiveSlot (setNetSlot world slotId { slot with active := false }) := by
  intro id₁ slot₁ id₂ slot₂ found₁ active₁ found₂ active₂ sameSandbox
  by_cases first : id₁ = slotId
  · subst id₁
    simp [setNetSlot, put] at found₁
    subst slot₁
    simp at active₁
  · have oldFound₁ : world.netSlots id₁ = some slot₁ := by
      simpa [setNetSlot, put, first] using found₁
    by_cases second : id₂ = slotId
    · subst id₂
      simp [setNetSlot, put] at found₂
      subst slot₂
      simp at active₂
    · exact invariant id₁ slot₁ id₂ slot₂ oldFound₁ active₁
        (by simpa [setNetSlot, put, second] using found₂) active₂ sameSandbox

theorem uniqueActiveSlot_configureSlot
    (invariant : UniqueActiveSlot world)
    (foundChanged : world.netSlots changedId = some changed) :
    UniqueActiveSlot (configureSlot world changedId changed) := by
  intro id₁ slot₁ id₂ slot₂ found₁ active₁ found₂ active₂ sameSandbox
  by_cases first : id₁ = changedId
  · subst id₁
    simp [configureSlot, setNetSlot, put] at found₁
    subst slot₁
    by_cases second : id₂ = changedId
    · exact second.symm
    · have oldFound₂ : world.netSlots id₂ = some slot₂ := by
        simpa [configureSlot, setNetSlot, put, second] using found₂
      exact invariant changedId changed id₂ slot₂ foundChanged active₁
        oldFound₂ active₂ sameSandbox
  · have oldFound₁ : world.netSlots id₁ = some slot₁ := by
      simpa [configureSlot, setNetSlot, put, first] using found₁
    by_cases second : id₂ = changedId
    · subst id₂
      simp [configureSlot, setNetSlot, put] at found₂
      subst slot₂
      exact invariant id₁ slot₁ changedId changed oldFound₁ active₁
        foundChanged active₂ sameSandbox
    · exact invariant id₁ slot₁ id₂ slot₂ oldFound₁ active₁
        (by simpa [configureSlot, setNetSlot, put, second] using found₂)
        active₂ sameSandbox

theorem uniqueActiveSlot_applyRevocationFence
    (invariant : UniqueActiveSlot world) :
    UniqueActiveSlot (applyRevocationFence world sid record) := by
  unfold applyRevocationFence
  cases slotChoice : record.netSlot with
  | none => simpa [slotChoice, setFence, setSandbox] using invariant
  | some slotId =>
      cases slotFound : world.netSlots slotId with
      | none => simpa [slotChoice, slotFound, setFence, setSandbox] using invariant
      | some slot =>
          simpa [slotChoice, slotFound, setFence, setSandbox] using
            (uniqueActiveSlot_set_inactive (slotId := slotId) (slot := slot) invariant)

theorem uniqueActiveSlot_publishRunning
    (invariant : UniqueActiveSlot world)
    (slotSandbox : slot.sandbox = sid)
    (exclusive : noActiveSlot world sid) :
    UniqueActiveSlot (publishRunning world sid record slotId slot) := by
  intro id₁ slot₁ id₂ slot₂ found₁ active₁ found₂ active₂ sameSandbox
  by_cases first : id₁ = slotId
  · subst id₁
    simp [publishRunning, setSandbox, setNetSlot, put] at found₁
    subst slot₁
    change slot.sandbox = slot₂.sandbox at sameSandbox
    by_cases second : id₂ = slotId
    · exact second.symm
    · have oldFound₂ : world.netSlots id₂ = some slot₂ := by
        simpa [publishRunning, setSandbox, setNetSlot, put, second] using found₂
      have closed := exclusive id₂ slot₂ oldFound₂ (sameSandbox.symm.trans slotSandbox)
      exact False.elim (by grind)
  · have oldFound₁ : world.netSlots id₁ = some slot₁ := by
      simpa [publishRunning, setSandbox, setNetSlot, put, first] using found₁
    by_cases second : id₂ = slotId
    · subst id₂
      simp [publishRunning, setSandbox, setNetSlot, put] at found₂
      subst slot₂
      change slot₁.sandbox = slot.sandbox at sameSandbox
      have closed := exclusive id₁ slot₁ oldFound₁ (sameSandbox.trans slotSandbox)
      exact False.elim (by grind)
    · exact invariant id₁ slot₁ id₂ slot₂ oldFound₁ active₁
        (by simpa [publishRunning, setSandbox, setNetSlot, put, second] using found₂)
        active₂ sameSandbox

set_option maxHeartbeats 800000 in
theorem step_preserves_uniqueActiveSlot {world next : World} {event : Event}
    (invariant : UniqueActiveSlot world) (step : Step world event next) :
    UniqueActiveSlot next := by
  cases step
  case installRevocationFence =>
    exact uniqueActiveSlot_applyRevocationFence invariant
  case prepareVm =>
    let record := ‹SandboxRecord›
    let sid := ‹SandboxId›
    let vmId := ‹VmId›
    let slot : NetSlot := {
      owner := record.owner
      sandbox := sid
      generation := record.generation
      vm := vmId
      policy := record.policy
      configured := false
      active := false
    }
    simpa [prepareResources, enqueue, setSandbox, setVm] using
      (uniqueActiveSlot_set_inactive
        (slotId := ‹NetSlotId›)
        (slot := slot) invariant)
  case configureNetwork =>
    exact uniqueActiveSlot_configureSlot invariant (by assumption)
  case publishRunning =>
    exact uniqueActiveSlot_publishRunning invariant (by grind) (by assumption)
  all_goals
    simpa [beginCreate, applyStartFence, prepareResources, activateResource,
      beginPause, sealSnapshot, makeSnapshotDurable, stopResource,
      publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, setSandbox, setFilesystem, setFence, setVm,
      setSnapshot, enqueue] using invariant

theorem emptyWorld_authorityShape : AuthorityShape emptyWorld := by
  simp [AuthorityShape, emptyWorld]

set_option maxHeartbeats 1000000 in
theorem step_preserves_authorityShape {world next : World} {event : Event}
    (invariant : AuthorityShape world) (step : Step world event next) :
    AuthorityShape next := by
  cases step <;>
    simp_all [AuthorityShape, beginCreate, prepareResources, configureSlot,
      activateResource, publishRunning, beginPause, sealSnapshot,
      makeSnapshotDurable, stopResource, publishPaused, beginResume, beginKill,
      publishDestroyed, markOutcomeUnknown, setSandbox, setFilesystem, setVm,
      setSnapshot, setNetSlot, enqueue, put, Authority.withoutResume] <;>
    grind

theorem reachable_authority_shape {world : World} (reachable : Reachable world) :
    AuthorityShape world := by
  induction reachable with
  | initial => exact emptyWorld_authorityShape
  | step _ transition inductionHypothesis =>
      exact step_preserves_authorityShape inductionHypothesis transition

theorem emptyWorld_fenceClosesAuthority : FenceClosesAuthority emptyWorld := by
  simp [FenceClosesAuthority, emptyWorld]

theorem emptyWorld_fenceWellFormed : FenceWellFormed emptyWorld := by
  simp [FenceWellFormed, emptyWorld]

set_option maxHeartbeats 1000000 in
theorem step_preserves_fenceWellFormed {world next : World} {event : Event}
    (invariant : FenceWellFormed world) (step : Step world event next) :
    FenceWellFormed next := by
  cases step <;>
    simp_all [FenceWellFormed,
      beginCreate, prepareResources, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, setSandbox, setFilesystem, setVm,
      setSnapshot, setNetSlot, enqueue, put] <;>
    grind

set_option maxHeartbeats 1000000 in
theorem step_preserves_fenceClosesAuthority {world next : World} {event : Event}
    (ordering : FenceWellFormed world)
    (invariant : FenceClosesAuthority world) (step : Step world event next) :
    FenceClosesAuthority next := by
  cases step <;>
    simp_all [FenceWellFormed, FenceClosesAuthority,
      applyStartFence,
      beginCreate, prepareResources, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, setSandbox, setFilesystem, setFence, setVm,
      setSnapshot, setNetSlot, enqueue, put] <;>
    grind

theorem emptyWorld_pausedSnapshotValid : PausedSnapshotValid emptyWorld := by
  simp [PausedSnapshotValid, emptyWorld]

set_option maxHeartbeats 1000000 in
theorem step_preserves_pausedSnapshotValid {world next : World} {event : Event}
    (invariant : PausedSnapshotValid world) (step : Step world event next) :
    PausedSnapshotValid next := by
  cases step <;>
    simp_all [PausedSnapshotValid, beginCreate, prepareResources,
      configureSlot, activateResource, publishRunning, beginPause,
      sealSnapshot, makeSnapshotDurable, stopResource, publishPaused,
      beginResume, beginKill, publishDestroyed, markOutcomeUnknown,
      setSandbox, setFilesystem, setVm, setSnapshot, setNetSlot,
      enqueue, put] <;>
    grind

theorem emptyWorld_stableStatesQuiescent : StableStatesQuiescent emptyWorld := by
  simp [StableStatesQuiescent, emptyWorld]

theorem noActiveSlot_set_inactive
    {world : World} {target : SandboxId} {slotId : NetSlotId} {slot : NetSlot}
    (quiet : noActiveSlot world target) :
    noActiveSlot (setNetSlot world slotId { slot with active := false }) target := by
  intro candidate current present owned
  by_cases same : candidate = slotId
  · subst candidate
    simp [setNetSlot, put] at present
    subst current
    rfl
  · exact quiet candidate current (by simpa [setNetSlot, put, same] using present) owned

@[simp] theorem noActiveSlot_applyRevocationFence
    {world : World} {target sid : SandboxId} {record : SandboxRecord}
    (quiet : noActiveSlot world target) :
    noActiveSlot (applyRevocationFence world sid record) target := by
  unfold applyRevocationFence
  cases slotChoice : record.netSlot with
  | none => simpa [slotChoice, setFence, setSandbox] using quiet
  | some slotId =>
      cases slotFound : world.netSlots slotId with
      | none => simpa [slotChoice, slotFound, setFence, setSandbox] using quiet
      | some slot =>
          simpa [slotChoice, slotFound, setFence, setSandbox] using
            (noActiveSlot_set_inactive (slotId := slotId) (slot := slot) quiet)

theorem stableStatesQuiescent_applyRevocationFence
    (invariant : StableStatesQuiescent world)
    (revoking : record.phase = .pausing ∨ record.phase = .revoking) :
    StableStatesQuiescent (applyRevocationFence world sid record) := by
  intro target current currentFound stable
  by_cases same : target = sid
  · subst target
    have currentIsClosed : current = { record with effective := .closed } := by
      simp [applyRevocationFence_sandboxes, put] at currentFound
      exact currentFound.symm
    subst current
    simp only at stable
    grind
  · have oldFound : world.sandboxes target = some current := by
      simpa [applyRevocationFence_sandboxes, put, same] using currentFound
    rcases invariant target current oldFound stable with ⟨vmQuiet, slotQuiet⟩
    exact ⟨by simpa only [noExecutingVm, applyRevocationFence_vms] using vmQuiet,
      noActiveSlot_applyRevocationFence slotQuiet⟩

set_option maxHeartbeats 1200000 in
theorem step_preserves_stableStatesQuiescent {world next : World} {event : Event}
    (invariant : StableStatesQuiescent world) (step : Step world event next) :
    StableStatesQuiescent next := by
  cases step
  case installRevocationFence =>
    exact stableStatesQuiescent_applyRevocationFence invariant (by assumption)
  all_goals
    simp_all [StableStatesQuiescent, noExecutingVm, noActiveSlot,
      beginCreate, applyStartFence, prepareResources,
      configureSlot, activateResource, publishRunning, beginPause,
      sealSnapshot, makeSnapshotDurable, stopResource, publishPaused,
      beginResume, beginKill, publishDestroyed, markOutcomeUnknown,
      setSandbox, setFilesystem, setFence, setVm, setSnapshot, setNetSlot,
      enqueue, put] <;>
    grind

theorem emptyWorld_effectiveRuntimeValid : EffectiveRuntimeValid emptyWorld := by
  simp [EffectiveRuntimeValid, emptyWorld]

theorem effectiveRuntimeValid_applyRevocationFence
    (invariant : EffectiveRuntimeValid world)
    (slotIdentity : ∀ slotId, record.netSlot = some slotId →
      ∃ slot, world.netSlots slotId = some slot ∧ slot.sandbox = sid) :
    EffectiveRuntimeValid (applyRevocationFence world sid record) := by
  intro target current currentFound effective phase
  by_cases same : target = sid
  · subst target
    have currentClosed : current = { record with effective := .closed } := by
      simp [applyRevocationFence_sandboxes, put] at currentFound
      exact currentFound.symm
    subst current
    contradiction
  · have oldFound : world.sandboxes target = some current := by
      simpa [applyRevocationFence_sandboxes, put, same] using currentFound
    rcases invariant target current oldFound effective phase with
      ⟨vmId, vm, slotId, slot, currentVm, foundVm, vmOwner, vmSandbox,
        vmGeneration, executing, currentSlot, foundSlot, slotOwner,
        slotSandbox, slotGeneration, slotVm, slotPolicy, configured, active⟩
    refine ⟨vmId, vm, slotId, slot, currentVm, ?_, vmOwner, vmSandbox,
      vmGeneration, executing, currentSlot, ?_, slotOwner, slotSandbox,
      slotGeneration, slotVm, slotPolicy, configured, active⟩
    · simpa only [applyRevocationFence_vms] using foundVm
    · rw [applyRevocationFence_netSlots]
      cases slotChoice : record.netSlot with
      | none => simpa [slotChoice] using foundSlot
      | some closedSlotId =>
          cases closedSlotFound : world.netSlots closedSlotId with
          | none => simpa [slotChoice, closedSlotFound] using foundSlot
          | some closedSlot =>
              have different : slotId ≠ closedSlotId := by
                intro equal
                subst closedSlotId
                have sameSlot : closedSlot = slot := by
                  exact Option.some.inj (closedSlotFound.symm.trans foundSlot)
                subst closedSlot
                rcases slotIdentity slotId slotChoice with ⟨witness, witnessFound, owned⟩
                have witnessIsSlot : witness = slot := by
                  exact Option.some.inj (witnessFound.symm.trans foundSlot)
                subst witness
                exact same (slotSandbox.symm.trans owned)
              simpa [slotChoice, closedSlotFound, put, different] using foundSlot

theorem effectiveRuntimeValid_prepareResources
    (shape : AuthorityShape world)
    (invariant : EffectiveRuntimeValid world)
    (found : world.sandboxes sid = some record)
    (starting : record.phase = .starting)
    (freshVm : world.vms newVmId = none)
    (freshSlot : world.netSlots newSlotId = none) :
    EffectiveRuntimeValid (prepareResources world sid record newVmId newSlotId) := by
  have closed : record.effective = .closed := by
    have shaped := shape sid record found
    have pair : record.desired = .closed ∧ record.effective = .closed := by
      simpa [starting] using shaped
    exact pair.2
  intro target current currentFound effective phase
  by_cases same : target = sid
  · subst target
    have currentUpdated : current = { record with
        currentVm := some newVmId, netSlot := some newSlotId } := by
      simp [prepareResources, enqueue, setSandbox, setNetSlot, setVm, put] at currentFound
      exact currentFound.symm
    subst current
    exact False.elim (effective closed)
  · have oldFound : world.sandboxes target = some current := by
      simpa [prepareResources, enqueue, setSandbox, setNetSlot, setVm, put, same]
        using currentFound
    rcases invariant target current oldFound effective phase with
      ⟨vmId, vm, slotId, slot, currentVm, foundVm, vmOwner, vmSandbox,
        vmGeneration, executing, currentSlot, foundSlot, slotOwner,
        slotSandbox, slotGeneration, slotVm, slotPolicy, configured, active⟩
    have vmDifferent : vmId ≠ newVmId := by
      intro equal
      subst newVmId
      rw [freshVm] at foundVm
      contradiction
    have slotDifferent : slotId ≠ newSlotId := by
      intro equal
      subst newSlotId
      rw [freshSlot] at foundSlot
      contradiction
    refine ⟨vmId, vm, slotId, slot, currentVm, ?_, vmOwner, vmSandbox,
      vmGeneration, executing, currentSlot, ?_, slotOwner, slotSandbox,
      slotGeneration, slotVm, slotPolicy, configured, active⟩
    · simpa [prepareResources, enqueue, setSandbox, setNetSlot, setVm, put,
        vmDifferent] using foundVm
    · simpa [prepareResources, enqueue, setSandbox, setNetSlot, setVm, put,
        slotDifferent] using foundSlot

theorem effectiveRuntimeValid_configureSlot
    (invariant : EffectiveRuntimeValid world)
    (foundChanged : world.netSlots changedId = some changed) :
    EffectiveRuntimeValid (configureSlot world changedId changed) := by
  intro sid record recordFound effective phase
  rcases invariant sid record recordFound effective phase with
    ⟨vmId, vm, slotId, slot, currentVm, foundVm, vmOwner, vmSandbox,
      vmGeneration, executing, currentSlot, foundSlot, slotOwner,
      slotSandbox, slotGeneration, slotVm, slotPolicy, configured, active⟩
  refine ⟨vmId, vm, slotId, slot, currentVm,
    by simpa [configureSlot, setNetSlot] using foundVm,
    vmOwner, vmSandbox, vmGeneration, executing, currentSlot, ?_, slotOwner,
    slotSandbox, slotGeneration, slotVm, slotPolicy, configured, active⟩
  by_cases same : slotId = changedId
  · subst changedId
    have sameSlot : changed = slot := by
      exact Option.some.inj (foundChanged.symm.trans foundSlot)
    subst changed
    cases slot
    simp_all [configureSlot, setNetSlot, put]
  · have unchanged :
        (configureSlot world changedId changed).netSlots slotId = some slot := by
      simpa [configureSlot, setNetSlot, put, same] using foundSlot
    simpa only using unchanged

theorem effectiveRuntimeValid_publishRunning
    (invariant : EffectiveRuntimeValid world)
    (currentVm : record.currentVm = some vmId)
    (foundVm : world.vms vmId = some vm)
    (vmIdentity : vm.owner = record.owner ∧ vm.sandbox = sid ∧
      vm.generation = record.generation)
    (executing : vm.status = .executing)
    (foundSlot : world.netSlots slotId = some slot)
    (currentSlot : record.netSlot = some slotId)
    (slotReady : slot.owner = record.owner ∧ slot.sandbox = sid ∧
      slot.generation = record.generation ∧ slot.vm = vmId ∧
      slot.policy = record.policy ∧ slot.configured = true ∧ slot.active = false) :
    EffectiveRuntimeValid (publishRunning world sid record slotId slot) := by
  intro target current currentFound effective phase
  by_cases same : target = sid
  · subst target
    simp [publishRunning, setSandbox, setNetSlot, put] at currentFound
    subst current
    rcases vmIdentity with ⟨vmOwner, vmSandbox, vmGeneration⟩
    rcases slotReady with
      ⟨slotOwner, slotSandbox, slotGeneration, slotVm, slotPolicy,
        configured, inactive⟩
    refine ⟨vmId, vm, slotId, { slot with active := true }, currentVm,
      by simpa [publishRunning, setSandbox, setNetSlot] using foundVm,
      vmOwner, vmSandbox, vmGeneration, executing, currentSlot, ?_,
      slotOwner, slotSandbox, slotGeneration, slotVm, slotPolicy, configured, rfl⟩
    simp [publishRunning, setSandbox, setNetSlot, put]
  · have oldFound : world.sandboxes target = some current := by
      simpa [publishRunning, setSandbox, setNetSlot, put, same] using currentFound
    rcases invariant target current oldFound effective phase with
      ⟨oldVmId, oldVm, oldSlotId, oldSlot, oldCurrentVm, oldFoundVm,
        oldVmOwner, oldVmSandbox, oldVmGeneration, oldExecuting,
        oldCurrentSlot, oldFoundSlot, oldSlotOwner, oldSlotSandbox,
        oldSlotGeneration, oldSlotVm, oldSlotPolicy, oldConfigured, oldActive⟩
    have different : oldSlotId ≠ slotId := by
      intro equal
      subst oldSlotId
      have sameSlot : oldSlot = slot := by
        exact Option.some.inj (oldFoundSlot.symm.trans foundSlot)
      subst oldSlot
      exact same (oldSlotSandbox.symm.trans slotReady.2.1)
    refine ⟨oldVmId, oldVm, oldSlotId, oldSlot, oldCurrentVm,
      by simpa [publishRunning, setSandbox, setNetSlot] using oldFoundVm,
      oldVmOwner, oldVmSandbox, oldVmGeneration, oldExecuting,
      oldCurrentSlot, ?_, oldSlotOwner, oldSlotSandbox, oldSlotGeneration,
      oldSlotVm, oldSlotPolicy, oldConfigured, oldActive⟩
    simpa [publishRunning, setSandbox, setNetSlot, put, different] using oldFoundSlot

theorem effectiveRuntimeValid_freezeVm
    (terminal : TerminalConsistent world)
    (fenceClosed : FenceClosesAuthority world)
    (invariant : EffectiveRuntimeValid world)
    (foundSandbox : world.sandboxes sid = some record)
    (pausing : record.phase = .pausing)
    (foundVm : world.vms vmId = some vm)
    (vmSandbox : vm.sandbox = sid)
    (fenced : world.workerFences sid = some ⟨record.epoch, false⟩) :
    EffectiveRuntimeValid (setVm world vmId { vm with status := .frozen }) := by
  have live : record.terminal = false := by
    cases value : record.terminal with
    | false => rfl
    | true =>
        have invalid := (terminal sid record foundSandbox).mp value
        grind
  have closed : record.effective = .closed :=
    fenceClosed sid record foundSandbox (by simpa [live] using fenced) (Or.inl pausing)
  intro target current currentFound effective phase
  by_cases same : target = sid
  · subst target
    have sameRecord : current = record := by
      exact Option.some.inj (currentFound.symm.trans foundSandbox)
    subst current
    contradiction
  · rcases invariant target current currentFound effective phase with
      ⟨oldVmId, oldVm, slotId, slot, currentVm, oldFoundVm, oldVmOwner,
        oldVmSandbox, oldVmGeneration, oldExecuting, currentSlot, foundSlot,
        slotOwner, slotSandbox, slotGeneration, slotVm, slotPolicy,
        configured, active⟩
    have different : oldVmId ≠ vmId := by
      intro equal
      have sameLookup := oldFoundVm
      rw [equal] at sameLookup
      have sameVm : oldVm = vm := by
        exact Option.some.inj (sameLookup.symm.trans foundVm)
      subst oldVm
      exact same (oldVmSandbox.symm.trans vmSandbox)
    refine ⟨oldVmId, oldVm, slotId, slot, currentVm, ?_, oldVmOwner,
      oldVmSandbox, oldVmGeneration, oldExecuting, currentSlot,
      by simpa [setVm] using foundSlot, slotOwner, slotSandbox,
      slotGeneration, slotVm, slotPolicy, configured, active⟩
    simpa [setVm, put, different] using oldFoundVm

theorem effectiveRuntimeValid_activateVm
    (shape : AuthorityShape world)
    (invariant : EffectiveRuntimeValid world)
    (foundSandbox : world.sandboxes sid = some record)
    (starting : record.phase = .starting)
    (foundVm : world.vms vmId = some vm)
    (vmSandbox : vm.sandbox = sid) :
    EffectiveRuntimeValid (activateResource world vmId vm) := by
  have shaped := shape sid record foundSandbox
  have pair : record.desired = .closed ∧ record.effective = .closed := by
    simpa [starting] using shaped
  have closed : record.effective = .closed := pair.2
  intro target current currentFound effective phase
  by_cases same : target = sid
  · subst target
    have sameRecord : current = record := by
      exact Option.some.inj (currentFound.symm.trans foundSandbox)
    subst current
    contradiction
  · rcases invariant target current currentFound effective phase with
      ⟨oldVmId, oldVm, slotId, slot, currentVm, oldFoundVm, oldVmOwner,
        oldVmSandbox, oldVmGeneration, oldExecuting, currentSlot, foundSlot,
        slotOwner, slotSandbox, slotGeneration, slotVm, slotPolicy,
        configured, active⟩
    have different : oldVmId ≠ vmId := by
      intro equal
      have sameLookup := oldFoundVm
      rw [equal] at sameLookup
      have sameVm : oldVm = vm := by
        exact Option.some.inj (sameLookup.symm.trans foundVm)
      subst oldVm
      exact same (oldVmSandbox.symm.trans vmSandbox)
    refine ⟨oldVmId, oldVm, slotId, slot, currentVm, ?_, oldVmOwner,
      oldVmSandbox, oldVmGeneration, oldExecuting, currentSlot,
      by simpa [activateResource, setVm] using foundSlot, slotOwner, slotSandbox,
      slotGeneration, slotVm, slotPolicy, configured, active⟩
    simpa [activateResource, setVm, put, different] using oldFoundVm

set_option maxHeartbeats 2000000 in
theorem step_preserves_effectiveRuntimeValid {world next : World} {event : Event}
    (shape : AuthorityShape world)
    (terminal : TerminalConsistent world)
    (fenceClosed : FenceClosesAuthority world)
    (invariant : EffectiveRuntimeValid world) (step : Step world event next) :
    EffectiveRuntimeValid next := by
  cases step
  case installRevocationFence =>
    exact effectiveRuntimeValid_applyRevocationFence invariant (by assumption)
  case prepareVm =>
    exact effectiveRuntimeValid_prepareResources shape invariant
      (by assumption) (by assumption) (by assumption) (by assumption)
  case configureNetwork =>
    exact effectiveRuntimeValid_configureSlot invariant (by assumption)
  case activateVm =>
    exact effectiveRuntimeValid_activateVm shape invariant
      (by assumption) (by assumption) (by assumption) (by grind)
  case publishRunning =>
    exact effectiveRuntimeValid_publishRunning invariant
      (by assumption) (by assumption) (by assumption) (by assumption)
      (by assumption) (by assumption) (by assumption)
  case freezeVm =>
    exact effectiveRuntimeValid_freezeVm terminal fenceClosed invariant
      (by assumption) (by assumption) (by assumption) (by grind) (by assumption)
  all_goals
    simp_all [AuthorityShape, FenceClosesAuthority, EffectiveRuntimeValid,
      beginCreate, applyStartFence, beginPause,
      sealSnapshot, makeSnapshotDurable, stopResource, publishPaused,
      beginResume, beginKill, publishDestroyed, markOutcomeUnknown,
      Authority.withoutResume, setSandbox, setFilesystem, setFence, setVm,
      setSnapshot, enqueue, put] <;>
    grind

theorem step_event_authorized {world next : World} {event : Event}
    (step : Step world event next) : EventAuthorized world event := by
  cases step <;> simp_all [EventAuthorized] <;> grind

theorem step_egress_policy_enforced {world next : World} {event : Event}
    (step : Step world event next) : EgressPolicyEnforced world event := by
  cases step <;> simp_all [EgressPolicyEnforced] <;> grind

theorem step_file_event_workspace_bound {world next : World} {event : Event}
    (workspace : WorkspaceBindingValid world)
    (step : Step world event next) : FileEventWorkspaceBound world event := by
  cases step <;> simp_all [FileEventWorkspaceBound, WorkspaceBindingValid] <;> grind

theorem rpcTimeout_marks_outcome_unknown
    (step : Step world (.rpcTimeout sid) next) :
    ∃ record operation,
      world.sandboxes sid = some record ∧
      record.currentOperation = some operation ∧
      operation.status = .pending ∧
      next = markOutcomeUnknown world sid record operation := by
  cases step
  case rpcTimeout =>
    exact ⟨_, _, by assumption, by assumption, by assumption, rfl⟩

theorem step_keeps_destroyed {world next : World} {event : Event}
    (step : Step world event next) :
    ∀ sid before, world.sandboxes sid = some before → before.phase = .destroyed →
      ∃ after, next.sandboxes sid = some after ∧ after.phase = .destroyed := by
  cases step <;>
    simp_all [beginCreate, applyStartFence, prepareResources, configureSlot,
      activateResource, publishRunning, beginPause, sealSnapshot,
      makeSnapshotDurable, stopResource, publishPaused, beginResume, beginKill,
      publishDestroyed, markOutcomeUnknown, setSandbox, setFilesystem,
      setFence, setVm, setSnapshot, setNetSlot, enqueue, put] <;>
    grind

theorem emptyWorld_safe : Safe emptyWorld := by
  exact ⟨emptyWorld_resourceOwnership,
    emptyWorld_workspaceBindingValid,
    emptyWorld_uniqueExecution,
    emptyWorld_uniqueActiveSlot,
    emptyWorld_durableSnapshotsImmutable,
    emptyWorld_authorityShape,
    emptyWorld_fenceWellFormed,
    emptyWorld_fenceClosesAuthority,
    emptyWorld_effectiveRuntimeValid,
    emptyWorld_stableStatesQuiescent,
    emptyWorld_terminalConsistent,
    emptyWorld_pausedSnapshotValid⟩

theorem step_preserves_safe {world next : World} {event : Event}
    (safe : Safe world) (step : Step world event next) : Safe next := by
  rcases safe with
    ⟨resources, workspace, unique, activeSlot, durable, shape, fenceWellFormed, fenceClosed,
      runtime, stable, terminal, pausedSnapshot⟩
  exact ⟨step_preserves_resourceOwnership resources step,
    step_preserves_workspaceBindingValid workspace step,
    step_preserves_uniqueExecution unique step,
    step_preserves_uniqueActiveSlot activeSlot step,
    step_preserves_durableSnapshotsImmutable durable step,
    step_preserves_authorityShape shape step,
    step_preserves_fenceWellFormed fenceWellFormed step,
    step_preserves_fenceClosesAuthority fenceWellFormed fenceClosed step,
    step_preserves_effectiveRuntimeValid shape terminal fenceClosed runtime step,
    step_preserves_stableStatesQuiescent stable step,
    step_preserves_terminalConsistent terminal step,
    step_preserves_pausedSnapshotValid pausedSnapshot step⟩

/-- Every finite protocol history generated by `Step` satisfies every predicate in `Safe`. -/
theorem reachable_safe {world : World} (reachable : Reachable world) : Safe world := by
  induction reachable with
  | initial => exact emptyWorld_safe
  | step _ transition inductionHypothesis =>
      exact step_preserves_safe inductionHypothesis transition

end SandboxModel
