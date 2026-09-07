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
    (beginCreate world sid owner workspace filePolicy policy operationId).vms = world.vms := by
  simp [beginCreate, setSandbox, setFilesystem]

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
  simp [prepareResources, setSandbox, setNetSlot, setVm]

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
  simp [beginPause, setSandbox]

@[simp] theorem sealSnapshot_vms :
    (sealSnapshot world snapshotId manifest record sid).vms = world.vms := by
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
  simp [beginResume, setSandbox]

@[simp] theorem beginKill_vms :
    (beginKill world sid record operationId).vms = world.vms := by
  simp [beginKill, setSandbox]

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
    (beginCreate world sid owner workspace filePolicy policy operationId).snapshots = world.snapshots := by
  simp [beginCreate, setSandbox, setFilesystem]

@[simp] theorem prepareResources_snapshots :
    (prepareResources world sid record vmId slotId).snapshots = world.snapshots := by
  simp [prepareResources, setSandbox, setNetSlot, setVm]

@[simp] theorem publishRunning_snapshots :
    (publishRunning world sid record slotId slot).snapshots = world.snapshots := by
  simp [publishRunning, setSandbox, setNetSlot]

@[simp] theorem beginPause_snapshots :
    (beginPause world sid record operationId vmId).snapshots = world.snapshots := by
  simp [beginPause, setSandbox]

@[simp] theorem publishPaused_snapshots :
    (publishPaused world sid record snapshotId).snapshots = world.snapshots := by
  simp [publishPaused, setSandbox]

@[simp] theorem beginResume_snapshots :
    (beginResume world sid record operationId snapshotId).snapshots = world.snapshots := by
  simp [beginResume, setSandbox]

@[simp] theorem beginKill_snapshots :
    (beginKill world sid record operationId).snapshots = world.snapshots := by
  simp [beginKill, setSandbox]

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
    simp_all [UniqueExecution, setVm, put, noExecutingVm] <;>
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
      setNetSlot, put] <;>
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
      put] <;>
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
      setNetSlot, put] <;>
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
    ResourceOwnership (beginCreate world sid owner workspace filePolicy policy operationId) := by
  let record : SandboxRecord := {
    owner := owner
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
    reclaimed := false
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
    .running sid base.generation slot.vm base.workspace base.workspaceAccess slotId base.policy
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
    ResourceOwnership (sealSnapshot world snapshotId manifest record sid) := by
  let snapshot : Snapshot := {
    owner := record.owner
    sandbox := sid
    sourceGeneration := record.generation
    workspace := record.workspace
    manifest := manifest
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
    let source := ‹RestoreSource›
    let base := { record with
      phase := .paused
      currentVm := none
      durableSnapshot := some source
      restoreFrom := none
      netSlot := none
      currentOperation := record.currentOperation.map
        (fun (op : Operation) => { op with status := .succeeded })
    }
    let authority : Authority := .paused sid source
    let updated := { base with desired := authority, effective := authority }
    simpa [publishPaused] using
      (resourceOwnership_setSandbox (after := updated)
        (sid := sid) (before := record)
        invariant (by assumption) rfl rfl)
  case acceptResume =>
    let record := ‹SandboxRecord›
    let operationId := ‹OperationId›
    let source := ‹RestoreSource›
    let updated := { record with
      phase := .starting
      epoch := record.epoch + 1
      generation := record.generation + 1
      currentVm := none
      restoreFrom := some source
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
  case cleanupVm =>
    simpa [reclaimVm] using
      (resourceOwnership_setVm
        (after := { ‹Vm› with status := .reclaimed })
        (vmId := ‹VmId›) (before := ‹Vm›)
        invariant (by assumption) rfl rfl)
  case cleanupSlot =>
    simpa [reclaimSlot] using
      (resourceOwnership_setNetSlot
        (after := { ‹NetSlot› with active := false, reclaimed := true })
        (slotId := ‹NetSlotId›) (before := ‹NetSlot›)
        invariant (by assumption) rfl rfl)
  all_goals
    simpa [ResourceOwnership, VmOwnership, FilesystemOwnership,
      SnapshotOwnership, NetSlotOwnership] using invariant

theorem emptyWorld_workspaceBindingValid : WorkspaceBindingValid emptyWorld := by
  simp [WorkspaceBindingValid, emptyWorld]

theorem workspaceBindingValid_beginCreate
    (invariant : WorkspaceBindingValid world)
    (freshWorkspace : world.filesystems workspace = none) :
    WorkspaceBindingValid (beginCreate world sid owner workspace filePolicy policy operationId) := by
  intro target record found
  by_cases same : target = sid
  · subst target
    simp [beginCreate, setSandbox, setFilesystem, put] at found
    subst record
    exact ⟨⟨owner, sid, 1⟩, by simp [beginCreate, setSandbox,
      setFilesystem, put], rfl, rfl⟩
  · have oldFound : world.sandboxes target = some record := by
      simpa [beginCreate, setSandbox, setFilesystem, put, same] using found
    rcases invariant target record oldFound with
      ⟨filesystem, filesystemFound, fsOwner, fsSandbox⟩
    have different : record.workspace ≠ workspace := by
      intro equal
      rw [equal, freshWorkspace] at filesystemFound
      contradiction
    exact ⟨filesystem, by simpa [beginCreate, setSandbox,
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
      setSnapshot, setNetSlot, put] <;>
    grind

theorem emptyWorld_workspaceApiAccessFixed : WorkspaceApiAccessFixed emptyWorld := by
  simp [WorkspaceApiAccessFixed, emptyWorld]

set_option maxHeartbeats 1000000 in
theorem step_preserves_workspaceApiAccessFixed {world next : World} {event : Event}
    (_invariant : WorkspaceApiAccessFixed world) (step : Step world event next) :
    WorkspaceApiAccessFixed next := by
  cases step <;>
    simp_all [WorkspaceApiAccessFixed, beginCreate, applyStartFence,
      applyRevocationFence, prepareResources, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, Authority.withoutResume, setSandbox, setFilesystem,
      setFence, setVm, setSnapshot, setNetSlot, put] <;>
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

theorem uniqueActiveSlot_reclaimSlot
    {world : World} {slotId : NetSlotId} {slot : NetSlot}
    (invariant : UniqueActiveSlot world) :
    UniqueActiveSlot (reclaimSlot world slotId slot) := by
  simpa [reclaimSlot] using
    (uniqueActiveSlot_set_inactive (world := world) (slotId := slotId)
      (slot := { slot with reclaimed := true }) invariant)

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
      reclaimed := false
    }
    simpa [prepareResources, enqueue, setSandbox, setVm] using
      (uniqueActiveSlot_set_inactive
        (slotId := ‹NetSlotId›)
        (slot := slot) invariant)
  case configureNetwork =>
    exact uniqueActiveSlot_configureSlot invariant (by assumption)
  case publishRunning =>
    exact uniqueActiveSlot_publishRunning invariant (by grind) (by assumption)
  case cleanupSlot =>
    exact uniqueActiveSlot_reclaimSlot invariant
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
      setSnapshot, setNetSlot, put, Authority.withoutResume] <;>
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
      setSnapshot, setNetSlot, put] <;>
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
      setSnapshot, setNetSlot, put] <;>
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
      put] <;>
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
      put] <;>
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
      simp [prepareResources, setSandbox, setNetSlot, setVm, put] at currentFound
      exact currentFound.symm
    subst current
    exact False.elim (effective closed)
  · have oldFound : world.sandboxes target = some current := by
      simpa [prepareResources, setSandbox, setNetSlot, setVm, put, same]
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
    · simpa [prepareResources, setSandbox, setNetSlot, setVm, put,
        vmDifferent] using foundVm
    · simpa [prepareResources, setSandbox, setNetSlot, setVm, put,
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
      setSnapshot, put] <;>
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
      setFence, setVm, setSnapshot, setNetSlot, put] <;>
    grind

theorem emptyWorld_currentResourcesValid : CurrentResourcesValid emptyWorld := by
  simp [CurrentResourcesValid, emptyWorld]

theorem currentResourcesValid_applyRevocationFence
    (invariant : CurrentResourcesValid world)
    (found : world.sandboxes sid = some record)
    (slotIdentity : ∀ slotId, record.netSlot = some slotId →
      ∃ slot, world.netSlots slotId = some slot ∧ slot.sandbox = sid) :
    CurrentResourcesValid (applyRevocationFence world sid record) := by
  intro target current currentFound
  by_cases same : target = sid
  · subst target
    have currentEq : current = { record with effective := .closed } := by
      simp [applyRevocationFence_sandboxes, put] at currentFound
      exact currentFound.symm
    subst current
    rcases invariant sid record found with ⟨vmValid, slotValid⟩
    constructor
    · simpa only [applyRevocationFence_vms] using vmValid
    · intro slotId currentSlot
      rcases slotValid slotId currentSlot with
        ⟨slot, slotFound, owner, sandbox, generation, currentVm, policy⟩
      refine ⟨{ slot with active := false }, ?_, owner, sandbox, generation,
        currentVm, policy⟩
      rw [applyRevocationFence_netSlots]
      change record.netSlot = some slotId at currentSlot
      simp [currentSlot, slotFound, put]
  · have oldFound : world.sandboxes target = some current := by
      simpa [applyRevocationFence_sandboxes, put, same] using currentFound
    rcases invariant target current oldFound with ⟨vmValid, slotValid⟩
    constructor
    · simpa only [applyRevocationFence_vms] using vmValid
    · intro slotId currentSlot
      rcases slotValid slotId currentSlot with
        ⟨slot, slotFound, owner, sandbox, generation, currentVm, policy⟩
      refine ⟨slot, ?_, owner, sandbox, generation, currentVm, policy⟩
      rw [applyRevocationFence_netSlots]
      cases choice : record.netSlot with
      | none => simp [slotFound]
      | some closedId =>
          cases closedFound : world.netSlots closedId with
          | none =>
              simpa [closedFound] using slotFound
          | some closedSlot =>
              have different : slotId ≠ closedId := by
                intro equal
                subst closedId
                have slotEq : closedSlot = slot :=
                  Option.some.inj (closedFound.symm.trans slotFound)
                subst closedSlot
                rcases slotIdentity slotId choice with ⟨witness, witnessFound, owned⟩
                have witnessEq : witness = slot :=
                  Option.some.inj (witnessFound.symm.trans slotFound)
                subst witness
                exact same (sandbox.symm.trans owned)
              simpa [closedFound, put, different] using slotFound

theorem currentResourcesValid_prepareResources
    (invariant : CurrentResourcesValid world)
    (freshVm : world.vms vmId = none)
    (freshSlot : world.netSlots slotId = none) :
    CurrentResourcesValid (prepareResources world sid record vmId slotId) := by
  intro target current currentFound
  by_cases same : target = sid
  · subst target
    simp [prepareResources, setSandbox, setVm, setNetSlot, put] at currentFound
    subst current
    constructor
    · intro candidate currentVm
      simp at currentVm
      subst candidate
      let newVm : Vm := ⟨record.owner, sid, record.generation, .prepared⟩
      exact ⟨newVm, by simp [newVm, prepareResources, setSandbox,
        setVm, setNetSlot, put]⟩
    · intro candidate currentSlot
      simp at currentSlot
      subst candidate
      let newSlot : NetSlot := {
        owner := record.owner
        sandbox := sid
        generation := record.generation
        vm := vmId
        policy := record.policy
        configured := false
        active := false
        reclaimed := false
      }
      exact ⟨newSlot, by simp [newSlot, prepareResources, setSandbox,
        setVm, setNetSlot, put]⟩
  · have oldFound : world.sandboxes target = some current := by
      simpa [prepareResources, setSandbox, setVm, setNetSlot, put, same]
        using currentFound
    rcases invariant target current oldFound with ⟨vmValid, slotValid⟩
    constructor
    · intro candidate currentVm
      rcases vmValid candidate currentVm with ⟨vm, vmFound, owner, sandbox, generation⟩
      have different : candidate ≠ vmId := by
        intro equal
        subst candidate
        rw [freshVm] at vmFound
        contradiction
      exact ⟨vm, by simpa [prepareResources, setSandbox, setVm,
        setNetSlot, put, different] using vmFound, owner, sandbox, generation⟩
    · intro candidate currentSlot
      rcases slotValid candidate currentSlot with
        ⟨slot, slotFound, owner, sandbox, generation, currentVm, policy⟩
      have different : candidate ≠ slotId := by
        intro equal
        subst candidate
        rw [freshSlot] at slotFound
        contradiction
      exact ⟨slot, by simpa [prepareResources, setSandbox, setVm,
        setNetSlot, put, different] using slotFound,
        owner, sandbox, generation, currentVm, policy⟩

set_option maxHeartbeats 1200000 in
theorem step_preserves_currentResourcesValid {world next : World} {event : Event}
    (invariant : CurrentResourcesValid world) (step : Step world event next) :
    CurrentResourcesValid next := by
  cases step
  case installRevocationFence =>
    exact currentResourcesValid_applyRevocationFence invariant (by assumption) (by assumption)
  case prepareVm =>
    exact currentResourcesValid_prepareResources invariant (by assumption) (by assumption)
  all_goals
    simp_all [CurrentResourcesValid, beginCreate, applyStartFence,
      configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, setSandbox, setFilesystem, setFence, setVm,
      setSnapshot, setNetSlot, put] <;>
    grind

theorem emptyWorld_activeSlotValid : ActiveSlotValid emptyWorld := by
  simp [ActiveSlotValid, emptyWorld]

set_option maxHeartbeats 1200000 in
theorem activeSlotValid_applyRevocationFence
    (invariant : ActiveSlotValid world)
    (found : world.sandboxes sid = some record) :
    ActiveSlotValid (applyRevocationFence world sid record) := by
  unfold applyRevocationFence
  cases choice : record.netSlot with
  | none =>
      simp_all [ActiveSlotValid, setFence, setSandbox, put] <;> grind
  | some closedId =>
      cases closedFound : world.netSlots closedId with
      | none =>
          simp_all [ActiveSlotValid, setFence, setSandbox, put] <;> grind
      | some closedSlot =>
          simp_all [ActiveSlotValid, setFence, setSandbox, setNetSlot, put] <;>
            grind

set_option maxHeartbeats 1600000 in
theorem step_preserves_activeSlotValid {world next : World} {event : Event}
    (stable : StableStatesQuiescent world)
    (invariant : ActiveSlotValid world) (step : Step world event next) :
    ActiveSlotValid next := by
  cases step
  case installRevocationFence =>
    exact activeSlotValid_applyRevocationFence invariant (by assumption)
  all_goals
    simp_all [ActiveSlotValid, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, Authority.withoutResume, StableStatesQuiescent,
      noActiveSlot, setSandbox, setFilesystem,
      setFence, setVm, setSnapshot, setNetSlot, put] <;>
    grind

theorem emptyWorld_currentOperationValid : CurrentOperationValid emptyWorld := by
  simp [CurrentOperationValid, emptyWorld]

theorem emptyWorld_restoreSourceValid : RestoreSourceValid emptyWorld := by
  simp [RestoreSourceValid, emptyWorld]

theorem restoreSourceValid_applyRevocationFence
    (invariant : RestoreSourceValid world)
    (found : world.sandboxes sid = some record) :
    RestoreSourceValid (applyRevocationFence world sid record) := by
  intro target current snapshotId currentFound starting restore
  by_cases same : target = sid
  · subst target
    have currentEq : current = { record with effective := .closed } := by
      simp [applyRevocationFence_sandboxes, put] at currentFound
      exact currentFound.symm
    subst current
    rcases invariant sid record snapshotId found starting restore with
      ⟨snapshot, snapshotFound, owner, sandbox, workspace, stage, immutable, generation⟩
    exact ⟨snapshot, by simpa only [applyRevocationFence_snapshots] using snapshotFound,
      owner, sandbox, workspace, stage, immutable, generation⟩
  · have oldFound : world.sandboxes target = some current := by
      simpa [applyRevocationFence_sandboxes, put, same] using currentFound
    rcases invariant target current snapshotId oldFound starting restore with
      ⟨snapshot, snapshotFound, owner, sandbox, workspace, stage, immutable, generation⟩
    exact ⟨snapshot, by simpa only [applyRevocationFence_snapshots] using snapshotFound,
      owner, sandbox, workspace, stage, immutable, generation⟩

set_option maxHeartbeats 1400000 in
theorem step_preserves_restoreSourceValid {world next : World} {event : Event}
    (pausedSnapshot : PausedSnapshotValid world)
    (invariant : RestoreSourceValid world) (step : Step world event next) :
    RestoreSourceValid next := by
  cases step
  case installRevocationFence =>
    exact restoreSourceValid_applyRevocationFence invariant (by assumption)
  all_goals
    simp_all [RestoreSourceValid, PausedSnapshotValid,
      beginCreate, applyStartFence, prepareResources,
      configureSlot, activateResource, publishRunning, beginPause,
      sealSnapshot, makeSnapshotDurable, stopResource, publishPaused,
      beginResume, beginKill, publishDestroyed, markOutcomeUnknown,
      Authority.withoutResume, setSandbox, setFilesystem, setFence, setVm,
      setSnapshot, setNetSlot, put] <;>
    grind

theorem currentOperationValid_applyRevocationFence
    (invariant : CurrentOperationValid world)
    (found : world.sandboxes sid = some record) :
    CurrentOperationValid (applyRevocationFence world sid record) := by
  intro target current currentFound
  by_cases same : target = sid
  · subst target
    have currentEq : current = { record with effective := .closed } := by
      simp [applyRevocationFence_sandboxes, put] at currentFound
      exact currentFound.symm
    subst current
    rcases invariant sid record found with ⟨operation, currentOperation, epoch, phase⟩
    exact ⟨operation, currentOperation, epoch, phase⟩
  · have oldFound : world.sandboxes target = some current := by
      simpa [applyRevocationFence_sandboxes, put, same] using currentFound
    exact invariant target current oldFound

set_option maxHeartbeats 1200000 in
theorem step_preserves_currentOperationValid {world next : World} {event : Event}
    (invariant : CurrentOperationValid world) (step : Step world event next) :
    CurrentOperationValid next := by
  cases step
  case installRevocationFence =>
    exact currentOperationValid_applyRevocationFence invariant (by assumption)
  all_goals
    simp_all [CurrentOperationValid, OperationMatchesPhase,
      beginCreate, applyStartFence, prepareResources,
      configureSlot, activateResource, publishRunning, beginPause,
      sealSnapshot, makeSnapshotDurable, stopResource, publishPaused,
      beginResume, beginKill, publishDestroyed, markOutcomeUnknown,
      Authority.withoutResume, setSandbox, setFilesystem, setFence, setVm,
      setSnapshot, setNetSlot, put] <;>
    grind

theorem emptyWorld_safe : Safe emptyWorld := by
  exact ⟨emptyWorld_resourceOwnership,
    emptyWorld_workspaceBindingValid,
    emptyWorld_workspaceApiAccessFixed,
    emptyWorld_currentResourcesValid,
    emptyWorld_uniqueExecution,
    emptyWorld_uniqueActiveSlot,
    emptyWorld_activeSlotValid,
    emptyWorld_durableSnapshotsImmutable,
    emptyWorld_authorityShape,
    emptyWorld_fenceWellFormed,
    emptyWorld_fenceClosesAuthority,
    emptyWorld_effectiveRuntimeValid,
    emptyWorld_stableStatesQuiescent,
    emptyWorld_terminalConsistent,
    emptyWorld_pausedSnapshotValid,
    emptyWorld_restoreSourceValid,
    emptyWorld_currentOperationValid⟩

theorem step_preserves_safe {world next : World} {event : Event}
    (safe : Safe world) (step : Step world event next) : Safe next := by
  rcases safe with
    ⟨resources, workspace, workspaceAccess, currentResources, unique, activeSlot, activeSlotValid,
      durable, shape, fenceWellFormed, fenceClosed, runtime, stable, terminal,
      pausedSnapshot, restoreSource, currentOperation⟩
  exact ⟨step_preserves_resourceOwnership resources step,
    step_preserves_workspaceBindingValid workspace step,
    step_preserves_workspaceApiAccessFixed workspaceAccess step,
    step_preserves_currentResourcesValid currentResources step,
    step_preserves_uniqueExecution unique step,
    step_preserves_uniqueActiveSlot activeSlot step,
    step_preserves_activeSlotValid stable activeSlotValid step,
    step_preserves_durableSnapshotsImmutable durable step,
    step_preserves_authorityShape shape step,
    step_preserves_fenceWellFormed fenceWellFormed step,
    step_preserves_fenceClosesAuthority fenceWellFormed fenceClosed step,
    step_preserves_effectiveRuntimeValid shape terminal fenceClosed runtime step,
    step_preserves_stableStatesQuiescent stable step,
    step_preserves_terminalConsistent terminal step,
    step_preserves_pausedSnapshotValid pausedSnapshot step,
    step_preserves_restoreSourceValid pausedSnapshot restoreSource step,
    step_preserves_currentOperationValid currentOperation step⟩

/-- Every finite protocol history generated by `Step` satisfies every predicate in `Safe`. -/
theorem reachable_safe {world : World} (reachable : Reachable world) : Safe world := by
  induction reachable with
  | initial => exact emptyWorld_safe
  | step _ transition inductionHypothesis =>
      exact step_preserves_safe inductionHypothesis transition

/-! ## Workspace authority claims

These theorems expose the research claims directly. They are consequences of
the same `Step` relation used by `reachable_safe`, rather than tests over a
finite collection of traces. -/

set_option maxHeartbeats 1200000 in
theorem step_workspaceIdentityStable {world next : World} {event : Event}
    (transition : Step world event next) : WorkspaceIdentityStable world next := by
  cases transition
  case installRevocationFence =>
    have sourceFound := ‹world.sandboxes _ = some _›
    intro target oldRecord newRecord oldFound newFound
    simp only [applyRevocationFence_sandboxes] at newFound
    rw [put_eq_some_iff] at newFound
    rcases newFound with ⟨same, closed⟩ | ⟨different, unchanged⟩
    · subst target
      have oldEq := Option.some.inj (oldFound.symm.trans sourceFound)
      subst oldRecord
      subst newRecord
      rfl
    · rw [oldFound] at unchanged
      cases unchanged
      rfl
  all_goals
    simp_all [WorkspaceIdentityStable, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, Authority.withoutResume, setSandbox, setFilesystem,
      setFence, setVm, setSnapshot, setNetSlot, put] <;>
    grind

set_option maxHeartbeats 1200000 in
theorem step_workspaceApiAccessStable {world next : World} {event : Event}
    (_transition : Step world event next) : WorkspaceApiAccessStable world next := by
  intro _ oldRecord newRecord _ _
  cases oldRecord.workspaceAccess
  cases newRecord.workspaceAccess
  rfl

theorem authorityShape_runningWorkspaceAuthorityExact
    (shape : AuthorityShape world) : RunningWorkspaceAuthorityExact world := by
  intro sid record found running
  have current := shape sid record found
  rw [running] at current
  rcases current with ⟨vmId, slotId, currentVm, currentSlot, _, effective⟩
  refine ⟨vmId, slotId, currentVm, currentSlot, effective, ?_, ?_⟩ <;>
    simp [effective, Authority.allows]

theorem reachable_runningWorkspaceAuthorityExact {world : World}
    (reachable : Reachable world) : RunningWorkspaceAuthorityExact world := by
  rcases reachable_safe reachable with
    ⟨_, _, _, _, _, _, _, _, shape, _⟩
  exact authorityShape_runningWorkspaceAuthorityExact shape

theorem fileRead_requires_running_current_generation
    (transition : Step world (.fileRead actor sid generation) next) :
    ∃ record, world.sandboxes sid = some record ∧
      record.phase = .running ∧ generation = record.generation := by
  cases transition
  case fileRead => grind

theorem fileWrite_requires_running_current_generation
    (transition : Step world (.fileWrite actor sid generation) next) :
    ∃ record, world.sandboxes sid = some record ∧
      record.phase = .running ∧ generation = record.generation := by
  cases transition
  case fileWrite => grind

theorem noFileAuthorityOutsideRunning
    (found : world.sandboxes sid = some record)
    (outside : record.phase ≠ .running) :
    (¬ Step world (.fileRead actor sid generation) next) ∧
      ¬ Step world (.fileWrite actor sid generation) next := by
  constructor
  · intro transition
    rcases fileRead_requires_running_current_generation transition with
      ⟨current, currentFound, running, _⟩
    rw [found] at currentFound
    cases currentFound
    exact outside running
  · intro transition
    rcases fileWrite_requires_running_current_generation transition with
      ⟨current, currentFound, running, _⟩
    rw [found] at currentFound
    cases currentFound
    exact outside running

theorem staleGenerationCannotAccessWorkspace
    (found : world.sandboxes sid = some record)
    (stale : generation ≠ record.generation) :
    (¬ Step world (.fileRead actor sid generation) next) ∧
      ¬ Step world (.fileWrite actor sid generation) next := by
  constructor
  · intro transition
    rcases fileRead_requires_running_current_generation transition with
      ⟨current, currentFound, _, currentGeneration⟩
    rw [found] at currentFound
    cases currentFound
    exact stale currentGeneration
  · intro transition
    rcases fileWrite_requires_running_current_generation transition with
      ⟨current, currentFound, _, currentGeneration⟩
    rw [found] at currentFound
    cases currentFound
    exact stale currentGeneration

/-- Accepting resume selects data provenance but starts the new generation with
    both desired and effective authority closed. -/
theorem snapshotCarriesNoAuthority
    (transition : Step world (.acceptResume actor sid operationId) next) :
    ∃ oldRecord newRecord snapshotId,
      world.sandboxes sid = some oldRecord ∧
      oldRecord.durableSnapshot = some snapshotId ∧
      next.sandboxes sid = some newRecord ∧
      newRecord.workspace = oldRecord.workspace ∧
      newRecord.workspaceAccess = oldRecord.workspaceAccess ∧
      newRecord.generation = oldRecord.generation + 1 ∧
      newRecord.restoreFrom = some snapshotId ∧
      newRecord.desired = .closed ∧ newRecord.effective = .closed := by
  cases transition
  case acceptResume =>
    simp_all [beginResume, setSandbox, put]

/-- Publication creates fresh file authority from the current controller
    record. This covers resume publication; the snapshot is absent from the
    authority constructor and cannot override workspace, mode, or generation. -/
theorem resumeReissuesWorkspaceAuthority
    (transition : Step world (.publishRunning sid) next) :
    ∃ record vmId slotId,
      next.sandboxes sid = some record ∧ record.phase = .running ∧
      record.currentVm = some vmId ∧ record.netSlot = some slotId ∧
      record.desired = .running sid record.generation vmId
        record.workspace record.workspaceAccess slotId record.policy ∧
      record.effective = record.desired ∧
      record.effective.allows
        (.fileRead sid record.generation record.workspace record.workspaceAccess) ∧
      record.effective.allows
        (.fileWrite sid record.generation record.workspace record.workspaceAccess) := by
  cases transition
  case publishRunning =>
    simp_all [publishRunning, setSandbox, setNetSlot, put, Authority.allows]

/-- Once the worker acknowledges the current pause/kill fence, even lingering
    effective authority is closed. -/
theorem reachable_revocationFenceClosesFileAuthority
    (reachable : Reachable world)
    (found : world.sandboxes sid = some record)
    (fenced : world.workerFences sid = some ⟨record.epoch, record.terminal⟩)
    (revoking : record.phase = .pausing ∨ record.phase = .revoking) :
    record.effective = .closed := by
  rcases reachable_safe reachable with
    ⟨_, _, _, _, _, _, _, _, _, _, fenceClosed, _⟩
  exact fenceClosed sid record found fenced revoking

/-! ## Policy publication, manifests, operations, effects, and cleanup -/

theorem runningPolicy_applyRevocationFence
    (invariant : RunningRequiresAppliedFilePolicy world)
    (notRunning : record.phase = .pausing ∨ record.phase = .revoking) :
    RunningRequiresAppliedFilePolicy (applyRevocationFence world sid record) := by
  intro target current currentFound running
  simp only [applyRevocationFence_sandboxes] at currentFound
  rw [put_eq_some_iff] at currentFound
  rcases currentFound with ⟨same, exactRecord⟩ | ⟨different, oldFound⟩
  · subst target
    subst current
    rcases notRunning with pausing | revoking <;> simp_all
  · have oldPolicy := invariant target current oldFound running
    rw [applyRevocationFence_appliedFilePolicies]
    exact oldPolicy

theorem emptyWorld_runningRequiresAppliedFilePolicy :
    RunningRequiresAppliedFilePolicy emptyWorld := by
  simp [RunningRequiresAppliedFilePolicy, emptyWorld]

set_option maxHeartbeats 1500000 in
theorem step_preserves_runningRequiresAppliedFilePolicy
    {world next : World} {event : Event}
    (invariant : RunningRequiresAppliedFilePolicy world)
    (step : Step world event next) :
    RunningRequiresAppliedFilePolicy next := by
  cases step
  case installRevocationFence =>
    exact runningPolicy_applyRevocationFence invariant (by assumption)
  all_goals
    simp_all [RunningRequiresAppliedFilePolicy, beginCreate, applyStartFence,
      prepareResources, applyFilePolicyResult, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, reclaimVm, reclaimSlot, Authority.withoutResume,
      setSandbox, setFilesystem, setFence, setVm, setSnapshot, setNetSlot,
      setAppliedFilePolicy, enqueue, put] <;>
    grind

theorem reachable_runningRequiresAppliedFilePolicy {world : World}
    (reachable : Reachable world) : RunningRequiresAppliedFilePolicy world := by
  induction reachable with
  | initial => exact emptyWorld_runningRequiresAppliedFilePolicy
  | step _ transition inductionHypothesis =>
      exact step_preserves_runningRequiresAppliedFilePolicy inductionHypothesis transition

set_option maxHeartbeats 1500000 in
theorem step_epochMonotone {world next : World} {event : Event}
    (transition : Step world event next) : EpochMonotone world next := by
  cases transition
  case installRevocationFence =>
    have sourceFound := ‹world.sandboxes _ = some _›
    intro target oldRecord newRecord oldFound newFound
    simp only [applyRevocationFence_sandboxes] at newFound
    rw [put_eq_some_iff] at newFound
    rcases newFound with ⟨same, closed⟩ | ⟨different, unchanged⟩
    · subst target
      have oldEq := Option.some.inj (oldFound.symm.trans sourceFound)
      subst oldRecord
      subst newRecord
      exact Nat.le_refl _
    · rw [oldFound] at unchanged
      cases unchanged
      exact Nat.le_refl _
  all_goals
    simp_all [EpochMonotone, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource, publishRunning,
      beginPause, sealSnapshot, makeSnapshotDurable, stopResource,
      publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, Authority.withoutResume, setSandbox, setFilesystem,
      setFence, setVm, setSnapshot, setNetSlot, enqueue, put] <;>
    grind

set_option maxHeartbeats 1500000 in
theorem step_generationMonotone {world next : World} {event : Event}
    (transition : Step world event next) : GenerationMonotone world next := by
  cases transition
  case installRevocationFence =>
    have sourceFound := ‹world.sandboxes _ = some _›
    intro target oldRecord newRecord oldFound newFound
    simp only [applyRevocationFence_sandboxes] at newFound
    rw [put_eq_some_iff] at newFound
    rcases newFound with ⟨same, closed⟩ | ⟨different, unchanged⟩
    · subst target
      have oldEq := Option.some.inj (oldFound.symm.trans sourceFound)
      subst oldRecord
      subst newRecord
      exact Nat.le_refl _
    · rw [oldFound] at unchanged
      cases unchanged
      exact Nat.le_refl _
  all_goals
    simp_all [GenerationMonotone, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource, publishRunning,
      beginPause, sealSnapshot, makeSnapshotDurable, stopResource,
      publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, Authority.withoutResume, setSandbox, setFilesystem,
      setFence, setVm, setSnapshot, setNetSlot, enqueue, put] <;>
    grind

set_option maxHeartbeats 1500000 in
theorem step_filePolicyStable {world next : World} {event : Event}
    (transition : Step world event next) : FilePolicyStable world next := by
  cases transition
  case installRevocationFence =>
    have sourceFound := ‹world.sandboxes _ = some _›
    intro target oldRecord newRecord oldFound newFound
    simp only [applyRevocationFence_sandboxes] at newFound
    rw [put_eq_some_iff] at newFound
    rcases newFound with ⟨same, closed⟩ | ⟨different, unchanged⟩
    · subst target
      have oldEq := Option.some.inj (oldFound.symm.trans sourceFound)
      subst oldRecord
      subst newRecord
      rfl
    · rw [oldFound] at unchanged
      cases unchanged
      rfl
  all_goals
    simp_all [FilePolicyStable, beginCreate, applyStartFence,
      prepareResources, configureSlot, activateResource, publishRunning,
      beginPause, sealSnapshot, makeSnapshotDurable, stopResource,
      publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, Authority.withoutResume, setSandbox, setFilesystem,
      setFence, setVm, setSnapshot, setNetSlot, enqueue, put] <;>
    grind

theorem workerFenceMonotone_setFence
    (advance : fenceCanAdvance (world.workerFences sid) epoch terminal) :
    WorkerFenceMonotone world (setFence world sid ⟨epoch, terminal⟩) := by
  intro target oldFence newFence oldFound newFound
  by_cases same : target = sid
  · subst target
    simp [setFence, put] at newFound
    subst newFence
    simp [oldFound] at advance
    exact advance
  · have unchanged : world.workerFences target = some newFence := by
      simpa [setFence, put, same] using newFound
    rw [oldFound] at unchanged
    cases unchanged
    exact ⟨Nat.le_refl _, fun h => h⟩

set_option maxHeartbeats 1500000 in
theorem step_workerFenceMonotone {world next : World} {event : Event}
    (transition : Step world event next) : WorkerFenceMonotone world next := by
  cases transition
  case installStartFence =>
    simpa [applyStartFence] using
      (workerFenceMonotone_setFence (world := world) (sid := ‹SandboxId›)
        (epoch := ‹SandboxRecord›.epoch) (terminal := false) (by assumption))
  case installRevocationFence =>
    have preserved : WorkerFenceMonotone world
        (setFence world _ ⟨‹SandboxRecord›.epoch, ‹SandboxRecord›.terminal⟩) :=
      workerFenceMonotone_setFence (by assumption)
    intro target oldFence newFence oldFound newFound
    apply preserved target oldFence newFence oldFound
    simpa only [applyRevocationFence_workerFences, setFence] using newFound
  all_goals
    simp_all [WorkerFenceMonotone, beginCreate, prepareResources,
      configureSlot, activateResource, publishRunning, beginPause,
      sealSnapshot, makeSnapshotDurable, stopResource, publishPaused,
      beginResume, beginKill, publishDestroyed, markOutcomeUnknown,
      reclaimVm, reclaimSlot, setSandbox, setFilesystem, setVm,
      setSnapshot, setNetSlot, enqueue] <;>
    grind

set_option maxHeartbeats 1500000 in
theorem step_snapshotManifestStable {world next : World} {event : Event}
    (transition : Step world event next) : SnapshotManifestStable world next := by
  cases transition <;>
    simp_all [SnapshotManifestStable, beginCreate, applyStartFence,
      applyRevocationFence, prepareResources, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, reclaimVm, reclaimSlot, setSandbox, setFilesystem,
      setFence, setVm, setSnapshot, setNetSlot, enqueue, put] <;>
    grind

theorem effectIdentityByConstruction (world : World) : EffectIdentityValid world := by
  intro effectKey status _
  cases effectKey
  rfl

theorem effectCommittedBeforeDispatch
    (transition : Step world (.dispatchEffect requestedId) next) :
    world.effects requestedId = some .committed := by
  cases transition
  assumption

theorem uncommittedEffectCannotDispatch
    (uncommitted : world.effects requestedId ≠ some .committed) :
    ¬ Step world (.dispatchEffect requestedId) next := by
  intro transition
  exact uncommitted (effectCommittedBeforeDispatch transition)

theorem ackMatchesEffect
    (transition : Step world (.acknowledgeEffect requestedId result) next) :
    world.effects requestedId = some .dispatched ∧
      commandResultPresent world requestedId.command := by
  cases transition
  exact ⟨by assumption, by assumption⟩

theorem step_ackCannotChangeAnotherEffect
    (transition : Step world (.acknowledgeEffect requestedId result) next)
    (different : otherId ≠ requestedId) :
    next.effects otherId = world.effects otherId := by
  cases transition
  simp [acknowledgeEffect, setEffect, put, different]

private theorem effectResultStable_of_effects_eq
    (same : after.effects = before.effects) : EffectResultStable before after := by
  intro id result old
  rw [same]
  exact old

private theorem effectResultStable_commit :
    EffectResultStable world (commitEffect world command) := by
  intro id result old
  cases found : world.effects (effectId command) with
  | none =>
      by_cases same : id = effectId command
      · subst id
        rw [found] at old
        contradiction
      · simp [commitEffect, found, setEffect, put, same]
        exact old
  | some existing =>
      simp [commitEffect, found]
      exact old

private theorem effectResultStable_enqueue :
    EffectResultStable world (enqueue world command) := by
  simpa [enqueue] using
    (effectResultStable_commit (world := world) (command := command))

private theorem effectResultStable_dispatch
    (committed : world.effects requestedId = some .committed) :
    EffectResultStable world (dispatchEffect world requestedId) := by
  intro id result old
  by_cases same : id = requestedId
  · subst id
    rw [committed] at old
    cases old
  · simp [dispatchEffect, setEffect, put, same]
    exact old

private theorem effectResultStable_ack
    (dispatched : world.effects requestedId = some .dispatched) :
    EffectResultStable world
      (acknowledgeEffect world requestedId ackResult) := by
  intro id result old
  by_cases same : id = requestedId
  · subst id
    rw [dispatched] at old
    cases old
  · simp [acknowledgeEffect, setEffect, put, same]
    exact old

private theorem effectResultStable_trans
    (first : EffectResultStable a b) (second : EffectResultStable b c) :
    EffectResultStable a c := by
  intro id result found
  exact second id result (first id result found)

private theorem effectResultStable_enqueue_after
    (same : base.effects = world.effects) :
    EffectResultStable world (enqueue base command) := by
  apply effectResultStable_trans (effectResultStable_of_effects_eq same)
  exact effectResultStable_enqueue

private theorem effectResultStable_two_enqueue_after
    (same : base.effects = world.effects) :
    EffectResultStable world (enqueue (enqueue base first) second) := by
  apply effectResultStable_trans (effectResultStable_enqueue_after same)
  exact effectResultStable_enqueue

theorem step_effectResultStable
    (transition : Step world event next) : EffectResultStable world next := by
  cases transition
  case acceptCreate =>
    apply effectResultStable_enqueue_after
    simp
  case dispatchEffect =>
    exact effectResultStable_dispatch (by assumption)
  case acknowledgeEffect =>
    exact effectResultStable_ack (by assumption)
  case installRevocationFence =>
    apply effectResultStable_of_effects_eq
    rename_i sid record revoking queued dispatched found monotone slotIdentity
    cases slotChoice : record.netSlot with
    | none => simp [applyRevocationFence, slotChoice]
    | some slotId =>
        cases slotFound : world.netSlots slotId <;>
          simp [applyRevocationFence, slotChoice, slotFound]
  case dispatchPrepare =>
    exact effectResultStable_enqueue
  case prepareVm =>
    unfold prepareResources
    apply effectResultStable_two_enqueue_after
    simp
  case acceptPause =>
    unfold beginPause
    apply effectResultStable_two_enqueue_after
    simp
  case scheduleCleanupVm =>
    exact effectResultStable_enqueue
  case scheduleCleanupSlot =>
    exact effectResultStable_enqueue
  case acceptResume =>
    unfold beginResume
    apply effectResultStable_enqueue_after
    simp
  case acceptKill =>
    unfold beginKill
    apply effectResultStable_enqueue_after
    simp
  all_goals
    apply effectResultStable_of_effects_eq
    first
    | rfl
    | simp [applyStartFence, applyRevocationFence, configureSlot,
        activateResource, publishRunning, sealSnapshot, makeSnapshotDurable,
        stopResource, publishPaused, publishDestroyed, markOutcomeUnknown,
        reclaimVm, reclaimSlot]

theorem duplicateDeliveryIsIdempotent
    (transition : Step world (.duplicateOrLateDelivery command) next) :
    next = world := by
  cases transition
  rfl

theorem operationRetryIdempotent
    (transition : Step world (.retryOperation actor sid operationId kind input) next) :
    next = world ∧ ∃ ledger,
      world.operations (operationKey sid operationId) = some ledger ∧
      ledger.operation.kind = kind ∧ ledger.input = input := by
  cases transition
  exact ⟨rfl, ⟨_, by assumption, by assumption, by assumption⟩⟩

theorem queryReturnsCommittedResult
    (transition : Step world (.queryOperation actor sid operationId status result) next) :
    next = world ∧ ∃ ledger,
      world.operations (operationKey sid operationId) = some ledger ∧
      status = ledger.operation.status ∧ result = ledger.result := by
  cases transition
  exact ⟨rfl, ⟨_, by assumption, by assumption, by assumption⟩⟩

private theorem operationResultStable_of_operations_eq
    (same : after.operations = before.operations) :
    OperationResultStable before after := by
  intro key oldRecord newRecord foundOld _ foundNew
  rw [same, foundOld] at foundNew
  cases foundNew
  rfl

private theorem operationResultStable_beginCreate
    (fresh : world.operations (operationKey sid operationId) = none) :
    OperationResultStable world
      (beginCreate world sid owner workspace filePolicy policy operationId) := by
  intro key oldRecord newRecord foundOld finalOld foundNew
  by_cases same : key = operationKey sid operationId
  · subst key
    rw [fresh] at foundOld
    contradiction
  · simp [beginCreate, storeNewOperation, setOperation, put, same] at foundNew
    rw [foundOld] at foundNew
    cases foundNew
    rfl

private theorem operationResultStable_beginPause
    (fresh : world.operations (operationKey sid operationId) = none) :
    OperationResultStable world (beginPause world sid record operationId vmId) := by
  intro key oldRecord newRecord foundOld finalOld foundNew
  by_cases same : key = operationKey sid operationId
  · subst key
    rw [fresh] at foundOld
    contradiction
  · simp [beginPause, storeNewOperation, setOperation, put, same] at foundNew
    rw [foundOld] at foundNew
    cases foundNew
    rfl

private theorem operationResultStable_beginResume
    (fresh : world.operations (operationKey sid operationId) = none) :
    OperationResultStable world
      (beginResume world sid record operationId source) := by
  intro key oldRecord newRecord foundOld finalOld foundNew
  by_cases same : key = operationKey sid operationId
  · subst key
    rw [fresh] at foundOld
    contradiction
  · simp [beginResume, storeNewOperation, setOperation, put, same] at foundNew
    rw [foundOld] at foundNew
    cases foundNew
    rfl

private theorem operationResultStable_beginKill
    (fresh : world.operations (operationKey sid operationId) = none) :
    OperationResultStable world (beginKill world sid record operationId) := by
  intro key oldRecord newRecord foundOld finalOld foundNew
  by_cases same : key = operationKey sid operationId
  · subst key
    rw [fresh] at foundOld
    contradiction
  · simp [beginKill, storeNewOperation, setOperation, put, same] at foundNew
    rw [foundOld] at foundNew
    cases foundNew
    rfl

private theorem operationResultStable_updateCurrent
    (pending : ∀ current, operation = some current →
      ∃ ledger, world.operations (operationKey sid current.id) = some ledger ∧
        ledger.result = none) :
    OperationResultStable world
      (updateCurrentOperation world sid operation result) := by
  intro key oldRecord newRecord foundOld finalOld foundNew
  cases choice : operation with
  | none =>
      simp [updateCurrentOperation, choice] at foundNew
      rw [foundOld] at foundNew
      cases foundNew
      rfl
  | some current =>
      obtain ⟨ledger, foundLedger, empty⟩ := pending current choice
      by_cases same : key = operationKey sid current.id
      · subst key
        have oldIsLedger : oldRecord = ledger :=
          Option.some.inj (foundOld.symm.trans foundLedger)
        subst oldRecord
        exact (finalOld empty).elim
      · simp [updateCurrentOperation, choice, same] at foundNew
        rw [foundOld] at foundNew
        cases foundNew
        rfl

private theorem operationResultStable_publishRunning
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (empty : ledger.result = none) :
    OperationResultStable world (publishRunning world sid record slotId slot) := by
  unfold publishRunning
  apply operationResultStable_updateCurrent
  intro published mapped
  simp [current] at mapped
  subst published
  exact ⟨ledger, found, empty⟩

private theorem operationResultStable_publishPaused
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (empty : ledger.result = none) :
    OperationResultStable world (publishPaused world sid record source) := by
  unfold publishPaused
  apply operationResultStable_updateCurrent
  intro published mapped
  simp [current] at mapped
  subst published
  exact ⟨ledger, found, empty⟩

private theorem operationResultStable_publishDestroyed
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (empty : ledger.result = none) :
    OperationResultStable world (publishDestroyed world sid record) := by
  unfold publishDestroyed
  apply operationResultStable_updateCurrent
  intro published mapped
  simp [current] at mapped
  subst published
  exact ⟨ledger, found, empty⟩

private theorem operationResultStable_markOutcomeUnknown
    (found : world.operations (operationKey sid operation.id) = some ledger)
    (empty : ledger.result = none) :
    OperationResultStable world (markOutcomeUnknown world sid record operation) := by
  unfold markOutcomeUnknown
  intro key oldRecord newRecord foundOld finalOld foundNew
  by_cases same : key = operationKey sid operation.id
  · subst key
    have oldIsLedger : oldRecord = ledger :=
      Option.some.inj (foundOld.symm.trans found)
    subst oldRecord
    exact (finalOld empty).elim
  · simp [updateOperation, same] at foundNew
    rw [foundOld] at foundNew
    cases foundNew
    rfl

theorem step_operationResultStable
    (transition : Step world event next) :
    OperationResultStable world next := by
  cases transition
  case acceptCreate =>
    rename_i actor owner sid workspace filePolicy policy operationId auth up
      freshSandbox freshWorkspace freshOperation
    exact operationResultStable_beginCreate freshOperation
  case acceptPause =>
    rename_i record vm actor sid operationId running live current auth up found
      freshOperation
    exact operationResultStable_beginPause freshOperation
  case acceptResume =>
    rename_i record source sid operationId actor paused live currentSnapshot auth up
      found sourceValid freshOperation
    exact operationResultStable_beginResume freshOperation
  case acceptKill =>
    rename_i record actor sid operationId notDestroyed auth up found freshOperation
    exact operationResultStable_beginKill freshOperation
  case publishRunning =>
    rename_i record vmId vm sid slot slotId operation ledger starting live currentVm
      vmIdentity executing currentSlot slotReady currentOperation operationMatches up
      foundSandbox foundVm foundSlot exclusiveNetwork appliedPolicy policyAcknowledged
      persistedOperation fenced
    exact operationResultStable_publishRunning currentOperation persistedOperation
      operationMatches.2.2
  case publishPaused =>
    rename_i record snapshot sid operation ledger source pausing snapshotReady
      currentOperation operationMatches up foundSandbox foundSnapshot fenced quiet
      networkClosed persistedOperation
    exact operationResultStable_publishPaused currentOperation persistedOperation
      operationMatches.2.2
  case publishDestroyed =>
    rename_i record sid operation ledger revoking terminal currentOperation
      operationMatches up found fenced quiet networkClosed persistedOperation
    exact operationResultStable_publishDestroyed currentOperation persistedOperation
      operationMatches.2.2
  case rpcTimeout =>
    rename_i record operation ledger sid current pending operationMatches foundSandbox
      persistedOperation
    exact operationResultStable_markOutcomeUnknown persistedOperation
      operationMatches.2.2
  all_goals
    apply operationResultStable_of_operations_eq
    simp

set_option maxHeartbeats 2000000 in
theorem step_operationIdentityStable
    (transition : Step world event next) :
    OperationIdentityStable world next := by
  cases transition <;>
    simp_all [OperationIdentityStable, beginCreate, applyStartFence,
      prepareResources, applyFilePolicyResult, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, reclaimVm, reclaimSlot, storeNewOperation,
      updateOperation, updateCurrentOperation, setSandbox, setFilesystem,
      setFence, setVm, setSnapshot, setNetSlot, setOperation, setEffect,
      setAppliedFilePolicy, enqueue, dispatchEffect, acknowledgeEffect, put] <;>
    grind

theorem acknowledgeEffectRequiresDispatched
    (transition : Step world (.acknowledgeEffect requestedId result) next) :
    world.effects requestedId = some .dispatched := by
  cases transition
  case acknowledgeEffect => assumption

theorem staleFilePolicyCommandRejected
    (found : world.sandboxes sid = some record)
    (stale : generation ≠ record.generation) :
    ¬ Step world (.applyFilePolicy sid generation) next := by
  intro transition
  cases transition
  case applyFilePolicy =>
    have sameRecord := Option.some.inj (found.symm.trans (by assumption))
    subst record
    exact stale rfl

theorem cleanupVmTargetsExactResource
    (transition : Step world (.cleanupVm sid generation vmId) next) :
    ∃ vm : Vm,
      world.vms vmId = some vm ∧ vm.sandbox = sid ∧
      vm.generation = generation ∧ vm.status = .stopped ∧
      (∀ record, world.sandboxes sid = some record →
        record.currentVm ≠ some vmId) ∧
      next.vms vmId = some { vm with status := .reclaimed } := by
  cases transition
  case cleanupVm =>
    refine ⟨‹Vm›, ?_⟩
    simp_all [reclaimVm, setVm, put]

theorem cleanupSlotTargetsExactResource
    (transition : Step world (.cleanupSlot sid generation slotId) next) :
    ∃ slot : NetSlot,
      world.netSlots slotId = some slot ∧ slot.sandbox = sid ∧
      slot.generation = generation ∧ slot.active = false ∧
      (∀ record, world.sandboxes sid = some record →
        record.netSlot ≠ some slotId) ∧
      next.netSlots slotId = some
        { slot with active := false, reclaimed := true } := by
  cases transition
  case cleanupSlot =>
    refine ⟨‹NetSlot›, ?_⟩
    simp_all [reclaimSlot, setNetSlot, put]

theorem cleanupVmLeavesReclaimedResourceNonCurrent
    (transition : Step world (.cleanupVm sid generation vmId) next) :
    ∃ vm : Vm,
      next.vms vmId = some { vm with status := .reclaimed } ∧
      ∀ record, next.sandboxes sid = some record →
        record.currentVm ≠ some vmId := by
  cases transition
  case cleanupVm =>
    refine ⟨‹Vm›, ?_, ?_⟩
    · simp_all [reclaimVm, setVm, put]
    · intro record found
      apply ‹∀ record, world.sandboxes sid = some record →
        record.currentVm ≠ some vmId› record
      simpa using found

theorem cleanupSlotLeavesReclaimedResourceNonCurrent
    (transition : Step world (.cleanupSlot sid generation slotId) next) :
    ∃ slot : NetSlot,
      next.netSlots slotId = some
        { slot with active := false, reclaimed := true } ∧
      ∀ record, next.sandboxes sid = some record →
        record.netSlot ≠ some slotId := by
  cases transition
  case cleanupSlot =>
    refine ⟨‹NetSlot›, ?_, ?_⟩
    · simp_all [reclaimSlot, setNetSlot, put]
    · intro record found
      apply ‹∀ record, world.sandboxes sid = some record →
        record.netSlot ≠ some slotId› record
      simpa using found

theorem emptyWorld_appliedFilePolicyValid :
    AppliedFilePolicyValid emptyWorld := by
  simp [AppliedFilePolicyValid, emptyWorld]

set_option maxHeartbeats 1500000 in
theorem step_preserves_appliedFilePolicyValid
    (invariant : AppliedFilePolicyValid world)
    (transition : Step world event next) : AppliedFilePolicyValid next := by
  cases transition <;>
    simp_all [AppliedFilePolicyValid, beginCreate, applyStartFence,
      prepareResources, applyFilePolicyResult, configureSlot, activateResource,
      publishRunning, beginPause, sealSnapshot, makeSnapshotDurable,
      stopResource, publishPaused, beginResume, beginKill, publishDestroyed,
      markOutcomeUnknown, reclaimVm, reclaimSlot, setSandbox, setFilesystem,
      setFence, setVm, setSnapshot, setNetSlot, setAppliedFilePolicy,
      enqueue, put] <;>
    grind

theorem reachable_appliedFilePolicyValid
    (reachable : Reachable world) : AppliedFilePolicyValid world := by
  induction reachable with
  | initial => exact emptyWorld_appliedFilePolicyValid
  | step reachable transition ih =>
      exact step_preserves_appliedFilePolicyValid ih transition

theorem emptyWorld_operationLedgerValid : OperationLedgerValid emptyWorld := by
  simp [OperationLedgerValid, emptyWorld]

private theorem operationLedgerValid_store
    (invariant : OperationLedgerValid world) :
    OperationLedgerValid (storeNewOperation world sid operation input) := by
  intro key ledger found
  rw [show (storeNewOperation world sid operation input).operations =
    put world.operations (operationKey sid operation.id)
      ⟨sid, operation, input, none⟩ by rfl] at found
  rw [put_eq_some_iff] at found
  rcases found with ⟨same, value⟩ | ⟨different, old⟩
  · subst key
    subst ledger
    simp [operationKey]
  · exact invariant key ledger old

private theorem operationLedgerValid_update
    (invariant : OperationLedgerValid world) :
    OperationLedgerValid (updateOperation world sid operation result) := by
  intro key ledger found
  by_cases same : key = operationKey sid operation.id
  · subst key
    simp [updateOperation] at found
    cases oldFound : world.operations (operationKey sid operation.id) with
    | none => simp [oldFound] at found
    | some old =>
        simp [oldFound] at found
        subst ledger
        have valid := invariant (operationKey sid operation.id) old oldFound
        exact ⟨valid.1, rfl⟩
  · simp [updateOperation, same] at found
    exact invariant key ledger found

private theorem operationLedgerValid_updateCurrent
    (invariant : OperationLedgerValid world) :
    OperationLedgerValid
      (updateCurrentOperation world sid operation result) := by
  cases choice : operation with
  | none => simpa [updateCurrentOperation, choice] using invariant
  | some current =>
      intro key ledger found
      by_cases same : key = operationKey sid current.id
      · subst key
        simp [updateCurrentOperation] at found
        cases oldFound : world.operations (operationKey sid current.id) with
        | none => simp [oldFound] at found
        | some old =>
            simp [oldFound] at found
            subst ledger
            have valid := invariant (operationKey sid current.id) old oldFound
            exact ⟨valid.1, rfl⟩
      · simp [updateCurrentOperation, same] at found
        exact invariant key ledger found

private theorem operationLedgerValid_of_operations_eq
    (same : after.operations = before.operations)
    (invariant : OperationLedgerValid before) :
    OperationLedgerValid after := by
  intro key ledger found
  apply invariant key ledger
  rw [← same]
  exact found

theorem step_preserves_operationLedgerValid
    (invariant : OperationLedgerValid world)
    (transition : Step world event next) : OperationLedgerValid next := by
  cases transition
  case acceptCreate =>
    simp only [beginCreate, OperationLedgerValid, enqueue_operations]
    apply operationLedgerValid_store
    exact operationLedgerValid_of_operations_eq (by simp) invariant
  case acceptPause =>
    simp only [beginPause, OperationLedgerValid, enqueue_operations]
    apply operationLedgerValid_store
    exact operationLedgerValid_of_operations_eq (by simp) invariant
  case acceptResume =>
    simp only [beginResume, OperationLedgerValid, enqueue_operations]
    apply operationLedgerValid_store
    exact operationLedgerValid_of_operations_eq (by simp) invariant
  case acceptKill =>
    simp only [beginKill, OperationLedgerValid, enqueue_operations]
    apply operationLedgerValid_store
    exact operationLedgerValid_of_operations_eq (by simp) invariant
  case publishRunning =>
    unfold publishRunning
    apply operationLedgerValid_updateCurrent
    exact operationLedgerValid_of_operations_eq (by simp) invariant
  case publishPaused =>
    unfold publishPaused
    apply operationLedgerValid_updateCurrent
    exact operationLedgerValid_of_operations_eq (by simp) invariant
  case publishDestroyed =>
    unfold publishDestroyed
    apply operationLedgerValid_updateCurrent
    exact operationLedgerValid_of_operations_eq (by simp) invariant
  case rpcTimeout =>
    unfold markOutcomeUnknown
    apply operationLedgerValid_update
    exact operationLedgerValid_of_operations_eq (by simp) invariant
  all_goals
    exact operationLedgerValid_of_operations_eq (by simp) invariant

theorem reachable_operationLedgerValid
    (reachable : Reachable world) : OperationLedgerValid world := by
  induction reachable with
  | initial => exact emptyWorld_operationLedgerValid
  | step reachable transition ih =>
      exact step_preserves_operationLedgerValid ih transition

theorem emptyWorld_currentOperationInLedger :
    CurrentOperationInLedger emptyWorld := by
  simp [CurrentOperationInLedger, emptyWorld]

private theorem currentOperationInLedger_setStore
    (invariant : CurrentOperationInLedger world)
    (current : updated.currentOperation = some operation) :
    CurrentOperationInLedger
      (storeNewOperation (setSandbox world sid updated) sid operation input) := by
  intro otherSid otherRecord otherOperation foundRecord foundCurrent
  by_cases sameSid : otherSid = sid
  · subst otherSid
    simp [setSandbox, put] at foundRecord
    subst otherRecord
    have sameOperation : otherOperation = operation :=
      Option.some.inj (foundCurrent.symm.trans current)
    subst otherOperation
    refine ⟨⟨sid, operation, input, none⟩, ?_⟩
    simp [storeNewOperation, setOperation, put]
  · have foundOld : world.sandboxes otherSid = some otherRecord := by
      simpa [setSandbox, put, sameSid] using foundRecord
    obtain ⟨ledger, foundLedger⟩ :=
      invariant otherSid otherRecord otherOperation foundOld foundCurrent
    refine ⟨ledger, ?_⟩
    have differentKey : operationKey otherSid otherOperation.id ≠
        operationKey sid operation.id := by
      intro equal
      have equalSid := congrArg OperationKey.sandbox equal
      exact sameSid (by simpa [operationKey] using equalSid)
    simpa [storeNewOperation, setOperation, put, differentKey] using foundLedger

private theorem currentOperationInLedger_of_maps_eq
    (sameSandboxes : after.sandboxes = before.sandboxes)
    (sameOperations : after.operations = before.operations)
    (invariant : CurrentOperationInLedger before) :
    CurrentOperationInLedger after := by
  intro sid record operation found current
  have foundBefore : before.sandboxes sid = some record := by
    rw [← sameSandboxes]
    exact found
  obtain ⟨ledger, foundLedger⟩ :=
    invariant sid record operation foundBefore current
  refine ⟨ledger, ?_⟩
  rw [sameOperations]
  exact foundLedger

private theorem currentOperationInLedger_setUpdateCurrent
    (invariant : CurrentOperationInLedger world)
    (current : updated.currentOperation = some operation)
    (foundTarget : world.operations (operationKey sid operation.id) =
      some ledger) :
    CurrentOperationInLedger
      (updateCurrentOperation (setSandbox world sid updated) sid
        (some operation) result) := by
  intro otherSid otherRecord otherOperation foundRecord foundCurrent
  by_cases sameSid : otherSid = sid
  · subst otherSid
    simp [setSandbox, put] at foundRecord
    subst otherRecord
    have sameOperation : otherOperation = operation :=
      Option.some.inj (foundCurrent.symm.trans current)
    subst otherOperation
    refine ⟨{ ledger with operation := operation, result := result }, ?_⟩
    simp [updateCurrentOperation, foundTarget]
  · have foundOld : world.sandboxes otherSid = some otherRecord := by
      simpa [setSandbox, put, sameSid] using foundRecord
    obtain ⟨oldLedger, foundLedger⟩ :=
      invariant otherSid otherRecord otherOperation foundOld foundCurrent
    refine ⟨oldLedger, ?_⟩
    have differentKey : operationKey otherSid otherOperation.id ≠
        operationKey sid operation.id := by
      intro equal
      have equalSid := congrArg OperationKey.sandbox equal
      exact sameSid (by simpa [operationKey] using equalSid)
    simpa [updateCurrentOperation, differentKey] using foundLedger

private theorem currentOperationInLedger_setUpdateOperation
    (invariant : CurrentOperationInLedger world)
    (current : updated.currentOperation = some operation)
    (foundTarget : world.operations (operationKey sid operation.id) =
      some ledger) :
    CurrentOperationInLedger
      (updateOperation (setSandbox world sid updated) sid operation result) := by
  intro otherSid otherRecord otherOperation foundRecord foundCurrent
  by_cases sameSid : otherSid = sid
  · subst otherSid
    simp [setSandbox, put] at foundRecord
    subst otherRecord
    have sameOperation : otherOperation = operation :=
      Option.some.inj (foundCurrent.symm.trans current)
    subst otherOperation
    refine ⟨{ ledger with operation := operation, result := result }, ?_⟩
    simp [updateOperation, foundTarget]
  · have foundOld : world.sandboxes otherSid = some otherRecord := by
      simpa [setSandbox, put, sameSid] using foundRecord
    obtain ⟨oldLedger, foundLedger⟩ :=
      invariant otherSid otherRecord otherOperation foundOld foundCurrent
    refine ⟨oldLedger, ?_⟩
    have differentKey : operationKey otherSid otherOperation.id ≠
        operationKey sid operation.id := by
      intro equal
      have equalSid := congrArg OperationKey.sandbox equal
      exact sameSid (by simpa [operationKey] using equalSid)
    simpa [updateOperation, differentKey] using foundLedger

private theorem currentOperationInLedger_setSandbox_preserve
    (invariant : CurrentOperationInLedger world)
    (foundOld : world.sandboxes sid = some oldRecord)
    (preserved : updated.currentOperation = oldRecord.currentOperation) :
    CurrentOperationInLedger (setSandbox world sid updated) := by
  intro otherSid record operation foundRecord foundCurrent
  by_cases sameSid : otherSid = sid
  · subst otherSid
    simp [setSandbox, put] at foundRecord
    subst record
    have oldCurrent : oldRecord.currentOperation = some operation := by
      rw [← preserved]
      exact foundCurrent
    obtain ⟨ledger, foundLedger⟩ :=
      invariant sid oldRecord operation foundOld oldCurrent
    exact ⟨ledger, by simpa using foundLedger⟩
  · have foundBefore : world.sandboxes otherSid = some record := by
      simpa [setSandbox, put, sameSid] using foundRecord
    obtain ⟨ledger, foundLedger⟩ :=
      invariant otherSid record operation foundBefore foundCurrent
    exact ⟨ledger, by simpa using foundLedger⟩

private theorem currentOperationInLedger_enqueue
    (invariant : CurrentOperationInLedger world) :
    CurrentOperationInLedger (enqueue world command) :=
  currentOperationInLedger_of_maps_eq (by simp) (by simp) invariant

private theorem currentOperationInLedger_publishRunning
    (invariant : CurrentOperationInLedger world)
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger) :
    CurrentOperationInLedger
      (publishRunning world sid record slotId slot) := by
  simp only [publishRunning, current, Option.map]
  apply currentOperationInLedger_setUpdateCurrent
    (currentOperationInLedger_of_maps_eq (by rfl) (by rfl) invariant)
  · rfl
  · simpa using found

private theorem currentOperationInLedger_publishPaused
    (invariant : CurrentOperationInLedger world)
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger) :
    CurrentOperationInLedger (publishPaused world sid record source) := by
  simp only [publishPaused, current, Option.map]
  apply currentOperationInLedger_setUpdateCurrent invariant
  · rfl
  · simpa using found

private theorem currentOperationInLedger_publishDestroyed
    (invariant : CurrentOperationInLedger world)
    (current : record.currentOperation = some operation)
    (found : world.operations (operationKey sid operation.id) = some ledger) :
    CurrentOperationInLedger (publishDestroyed world sid record) := by
  simp only [publishDestroyed, current, Option.map]
  apply currentOperationInLedger_setUpdateCurrent invariant
  · rfl
  · simpa using found

private theorem currentOperationInLedger_timeout
    (invariant : CurrentOperationInLedger world)
    (found : world.operations (operationKey sid operation.id) = some ledger) :
    CurrentOperationInLedger
      (markOutcomeUnknown world sid record operation) := by
  simp only [markOutcomeUnknown]
  apply currentOperationInLedger_setUpdateOperation invariant
  · rfl
  · exact found

private theorem currentOperationInLedger_revocation
    (invariant : CurrentOperationInLedger world)
    (found : world.sandboxes sid = some record) :
    CurrentOperationInLedger (applyRevocationFence world sid record) := by
  have changed := currentOperationInLedger_setSandbox_preserve invariant found
    (updated := { record with effective := .closed }) rfl
  cases slotChoice : record.netSlot with
  | none => simpa [applyRevocationFence, slotChoice] using
      (currentOperationInLedger_of_maps_eq
        (by simp [setFence, slotChoice]) (by rfl) changed)
  | some slotId =>
      cases slotFound : world.netSlots slotId with
      | none => simpa [applyRevocationFence, slotChoice, slotFound] using
          (currentOperationInLedger_of_maps_eq
            (by simp [setFence, slotChoice]) (by rfl) changed)
      | some slot => simpa [applyRevocationFence, slotChoice, slotFound] using
          (currentOperationInLedger_of_maps_eq
            (by simp only [setNetSlot, setFence]; rw [slotChoice])
            (by rfl) changed)

private theorem currentOperationInLedger_prepare
    (invariant : CurrentOperationInLedger world)
    (found : world.sandboxes sid = some record) :
    CurrentOperationInLedger
      (prepareResources world sid record vmId slotId) := by
  unfold prepareResources
  apply currentOperationInLedger_enqueue
  apply currentOperationInLedger_enqueue
  apply currentOperationInLedger_setSandbox_preserve
    (currentOperationInLedger_of_maps_eq (by rfl) (by rfl) invariant)
  · simpa using found
  · rfl

theorem step_preserves_currentOperationInLedger
    (invariant : CurrentOperationInLedger world)
    (transition : Step world event next) : CurrentOperationInLedger next := by
  cases transition
  case acceptCreate =>
    simp only [beginCreate]
    apply currentOperationInLedger_enqueue
    apply currentOperationInLedger_setStore
      (currentOperationInLedger_of_maps_eq (by rfl) (by rfl) invariant)
    rfl
  case installRevocationFence =>
    exact currentOperationInLedger_revocation invariant (by assumption)
  case prepareVm =>
    exact currentOperationInLedger_prepare invariant (by assumption)
  case dispatchPrepare =>
    exact currentOperationInLedger_enqueue invariant
  case publishRunning =>
    rename_i record vmId vm sid slot slotId operation ledger starting live currentVm
      vmIdentity executing currentSlot slotReady currentOperation operationMatches up
      foundSandbox foundVm foundSlot exclusiveNetwork appliedPolicy policyAcknowledged
      persistedOperation fenced
    exact currentOperationInLedger_publishRunning invariant currentOperation
      persistedOperation
  case acceptPause =>
    simp only [beginPause]
    apply currentOperationInLedger_enqueue
    apply currentOperationInLedger_enqueue
    apply currentOperationInLedger_setStore invariant
    rfl
  case publishPaused =>
    rename_i record snapshot sid operation ledger source pausing snapshotReady
      currentOperation operationMatches up foundSandbox foundSnapshot fenced quiet
      networkClosed persistedOperation
    exact currentOperationInLedger_publishPaused invariant currentOperation
      persistedOperation
  case acceptResume =>
    simp only [beginResume]
    apply currentOperationInLedger_enqueue
    apply currentOperationInLedger_setStore invariant
    rfl
  case acceptKill =>
    simp only [beginKill]
    apply currentOperationInLedger_enqueue
    apply currentOperationInLedger_setStore invariant
    rfl
  case publishDestroyed =>
    rename_i record sid operation ledger revoking terminal currentOperation
      operationMatches up found fenced quiet networkClosed persistedOperation
    exact currentOperationInLedger_publishDestroyed invariant currentOperation
      persistedOperation
  case rpcTimeout =>
    rename_i record operation ledger sid current pending operationMatches foundSandbox
      persistedOperation
    exact currentOperationInLedger_timeout invariant persistedOperation
  case scheduleCleanupVm =>
    exact currentOperationInLedger_enqueue invariant
  case scheduleCleanupSlot =>
    exact currentOperationInLedger_enqueue invariant
  all_goals
    apply currentOperationInLedger_of_maps_eq
    · first | rfl | simp
    · first | rfl | simp
    · exact invariant

theorem reachable_currentOperationInLedger
    (reachable : Reachable world) : CurrentOperationInLedger world := by
  induction reachable with
  | initial => exact emptyWorld_currentOperationInLedger
  | step reachable transition ih =>
      exact step_preserves_currentOperationInLedger ih transition

theorem emptyWorld_reclaimedResourcesNotCurrent :
    ReclaimedResourcesNotCurrent emptyWorld := by
  simp [ReclaimedResourcesNotCurrent, emptyWorld]

private theorem reclaimedResources_of_maps_eq
    (sameVms : after.vms = before.vms)
    (sameSandboxes : after.sandboxes = before.sandboxes)
    (sameSlots : after.netSlots = before.netSlots)
    (invariant : ReclaimedResourcesNotCurrent before) :
    ReclaimedResourcesNotCurrent after := by
  rcases invariant with ⟨vms, slots⟩
  constructor
  · intro vmId vm found reclaimed record recordFound
    apply vms vmId vm
    · rw [← sameVms]
      exact found
    · exact reclaimed
    · rw [← sameSandboxes]
      exact recordFound
  · intro slotId slot found reclaimed record recordFound
    apply slots slotId slot
    · rw [← sameSlots]
      exact found
    · exact reclaimed
    · rw [← sameSandboxes]
      exact recordFound

private theorem reclaimedResources_setSandbox
    (invariant : ReclaimedResourcesNotCurrent world)
    (found : world.sandboxes sid = some before)
    (sameVm : after.currentVm = before.currentVm)
    (sameSlot : after.netSlot = before.netSlot) :
    ReclaimedResourcesNotCurrent (setSandbox world sid after) := by
  rcases invariant with ⟨vms, slots⟩
  constructor
  · intro vmId vm vmFound reclaimed record recordFound
    by_cases same : vm.sandbox = sid
    · rw [same] at recordFound
      simp [setSandbox, put] at recordFound
      subst record
      have oldNoncurrent := vms vmId vm vmFound reclaimed before
        (by simpa [same] using found)
      simpa [sameVm] using oldNoncurrent
    · apply vms vmId vm vmFound reclaimed record
      simpa [setSandbox, put, same] using recordFound
  · intro slotId slot slotFound reclaimed record recordFound
    by_cases same : slot.sandbox = sid
    · rw [same] at recordFound
      simp [setSandbox, put] at recordFound
      subst record
      have oldNoncurrent := slots slotId slot slotFound reclaimed before
        (by simpa [same] using found)
      simpa [sameSlot] using oldNoncurrent
    · apply slots slotId slot slotFound reclaimed record
      simpa [setSandbox, put, same] using recordFound

private theorem reclaimedResources_setVm_nonreclaimed
    (invariant : ReclaimedResourcesNotCurrent world)
    (notReclaimed : after.status ≠ .reclaimed) :
    ReclaimedResourcesNotCurrent (setVm world vmId after) := by
  rcases invariant with ⟨vms, slots⟩
  constructor
  · intro candidate vm vmFound reclaimed record recordFound
    by_cases same : candidate = vmId
    · subst candidate
      simp [setVm, put] at vmFound
      subst vm
      exact (notReclaimed reclaimed).elim
    · apply vms candidate vm
      · simpa [setVm, put, same] using vmFound
      · exact reclaimed
      · simpa [setVm] using recordFound
  · simpa [ReclaimedResourcesNotCurrent, setVm] using slots

private theorem reclaimedResources_setNetSlot_same
    (invariant : ReclaimedResourcesNotCurrent world)
    (found : world.netSlots slotId = some before)
    (sameSandbox : after.sandbox = before.sandbox)
    (sameReclaimed : after.reclaimed = before.reclaimed) :
    ReclaimedResourcesNotCurrent (setNetSlot world slotId after) := by
  rcases invariant with ⟨vms, slots⟩
  constructor
  · simpa [ReclaimedResourcesNotCurrent, setNetSlot] using vms
  · intro candidate slot slotFound reclaimed record recordFound
    by_cases same : candidate = slotId
    · subst candidate
      simp [setNetSlot, put] at slotFound
      subst slot
      apply slots slotId before found
      · simpa [sameReclaimed] using reclaimed
      · simpa [sameSandbox] using recordFound
    · apply slots candidate slot
      · simpa [setNetSlot, put, same] using slotFound
      · exact reclaimed
      · simpa [setNetSlot] using recordFound

set_option maxHeartbeats 2000000 in
theorem step_preserves_reclaimedResourcesNotCurrent
    (invariant : ReclaimedResourcesNotCurrent world)
    (transition : Step world event next) :
    ReclaimedResourcesNotCurrent next := by
  cases transition
  case installStartFence =>
    exact reclaimedResources_of_maps_eq rfl rfl rfl invariant
  case installRevocationFence =>
    rename_i sid record revoking queued dispatched found monotone slotIdentity
    unfold applyRevocationFence
    have sandboxInvariant : ReclaimedResourcesNotCurrent
        (setSandbox world sid { record with effective := .closed }) :=
      reclaimedResources_setSandbox invariant found rfl rfl
    have closedInvariant : ReclaimedResourcesNotCurrent
        (setFence (setSandbox world sid { record with effective := .closed })
          sid ⟨record.epoch, record.terminal⟩) :=
      reclaimedResources_of_maps_eq rfl rfl rfl sandboxInvariant
    cases choice : record.netSlot with
    | none => simpa [choice] using closedInvariant
    | some slotId =>
      cases slotFound : world.netSlots slotId with
      | none => simpa [choice, slotFound] using closedInvariant
      | some slot =>
        have transported := closedInvariant
        simp only [choice] at transported
        simp only [slotFound]
        apply reclaimedResources_setNetSlot_same (before := slot)
          (after := { slot with active := false }) transported
        · simpa [setFence, setSandbox] using slotFound
        · rfl
        · rfl
  case configureNetwork =>
    rename_i record slot slotId sid vmId current owned foundSandbox foundSlot fenced
    exact reclaimedResources_setNetSlot_same (before := slot)
      (after := { slot with configured := true }) invariant foundSlot rfl rfl
  case activateVm =>
    apply reclaimedResources_setVm_nonreclaimed invariant
    simp
  case sealSnapshot =>
    exact reclaimedResources_of_maps_eq rfl rfl rfl invariant
  case commitSnapshot =>
    exact reclaimedResources_of_maps_eq rfl rfl rfl invariant
  case stopVm =>
    apply reclaimedResources_setVm_nonreclaimed invariant
    simp
  all_goals
    simp_all [ReclaimedResourcesNotCurrent, beginCreate,
      prepareResources, publishRunning, beginPause, publishPaused, beginResume,
      beginKill, publishDestroyed, markOutcomeUnknown, reclaimVm, reclaimSlot,
      setSandbox, setFilesystem, setVm, setNetSlot, enqueue, put] <;>
    grind

theorem reachable_reclaimedResourcesNotCurrent
    (reachable : Reachable world) : ReclaimedResourcesNotCurrent world := by
  induction reachable with
  | initial => exact emptyWorld_reclaimedResourcesNotCurrent
  | step reachable transition ih =>
      exact step_preserves_reclaimedResourcesNotCurrent ih transition

theorem step_effectAdmissionSafe
    (transition : Step world event next) : EffectAdmissionSafe world event := by
  cases transition <;>
    simp_all [EffectAdmissionSafe, effectDispatched, effectHasStatus]

theorem step_cleanupAdmissionSafe
    (transition : Step world event next) : CleanupAdmissionSafe world event := by
  cases transition <;> simp_all [CleanupAdmissionSafe] <;> grind

theorem step_protocol_secure
    (transition : Step world event next) :
    ProtocolStepSecurity world event next := {
  preservesState := fun safe => step_preserves_safe safe transition
  eventAuthorized := step_event_authorized transition
  egressEnforced := step_egress_policy_enforced transition
  fileWorkspaceBound := fun workspace =>
    step_file_event_workspace_bound workspace transition
  effectAdmission := step_effectAdmissionSafe transition
  cleanupAdmission := step_cleanupAdmissionSafe transition
  epochMonotone := step_epochMonotone transition
  generationMonotone := step_generationMonotone transition
  fenceMonotone := step_workerFenceMonotone transition
  terminalMonotone := by
    intro sid oldRecord newRecord foundOld foundNew terminal
    exact step_never_clears_terminal transition sid oldRecord newRecord
      foundOld terminal foundNew
  workspaceStable := step_workspaceIdentityStable transition
  workspaceAccessStable := step_workspaceApiAccessStable transition
  filePolicyStable := step_filePolicyStable transition
  snapshotManifestStable := step_snapshotManifestStable transition
  operationIdentityStable := step_operationIdentityStable transition
  operationResultStable := step_operationResultStable transition
  effectResultStable := step_effectResultStable transition
}

theorem reachable_protocol_secure {world : World}
    (reachable : Reachable world) : ProtocolSecurity world := by
  exact ⟨reachable_safe reachable,
    reachable_appliedFilePolicyValid reachable,
    reachable_runningRequiresAppliedFilePolicy reachable,
    reachable_operationLedgerValid reachable,
    reachable_currentOperationInLedger reachable,
    effectIdentityByConstruction world,
    reachable_reclaimedResourcesNotCurrent reachable⟩

end SandboxModel
