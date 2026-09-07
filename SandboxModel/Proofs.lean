import SandboxModel.Safety

/-! Machine-checked safety theorems for arbitrary reachable worlds. -/

namespace SandboxModel

theorem emptyWorld_uniqueExecution : UniqueExecution emptyWorld := by
  simp [UniqueExecution, emptyWorld]

@[simp] theorem beginCreate_vms :
    (beginCreate world sid owner workspace policy operationId).vms = world.vms := by
  simp [beginCreate, enqueue, setSandbox, setFilesystem]

@[simp] theorem applyFence_vms :
    (applyFence world sid record).vms = world.vms := by
  unfold applyFence
  cases slotChoice : record.netSlot with
  | none => simp [setFence, setSandbox]
  | some slotId =>
      cases slotFound : world.netSlots slotId <;>
        simp [slotFound, setFence, setSandbox, setNetSlot]

@[simp] theorem applyFence_sandboxes :
    (applyFence world sid record).sandboxes =
      put world.sandboxes sid { record with effective := noCapabilities } := by
  unfold applyFence
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

@[simp] theorem applyFence_snapshots :
    (applyFence world sid record).snapshots = world.snapshots := by
  unfold applyFence
  split <;> simp [setFence, setSandbox, setNetSlot]
  split <;> simp

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

theorem reachable_core_safe {world : World} (reachable : Reachable world) :
    CoreSafety world := by
  exact ⟨reachable_unique_execution reachable,
    reachable_durable_snapshots_are_immutable reachable,
    reachable_terminal_consistent reachable⟩

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

end SandboxModel
