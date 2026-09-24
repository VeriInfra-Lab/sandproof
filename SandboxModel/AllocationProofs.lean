import SandboxModel.Proofs

/-!
Monotone allocation-cursor invariant.

The controller reserves identities from four durable cursors.  This file
connects those executable reservations to a reachable-state invariant: every
resource identity already stored, indexed, or retained by a sandbox/operation
plan is strictly below the corresponding next cursor.  Consequently the
identity at a current cursor is genuinely fresh; freshness is no longer an
allocator-side assumption.
-/

namespace SandboxModel

def RestoreSourceBelowCursors (world : World) (source : RestoreSource) : Prop :=
  source.snapshot.value < world.nextSnapshotId ∧
    source.manifest.digest.value < world.nextManifestDigest

def SandboxReferencesBelowCursors (world : World)
    (record : SandboxRecord) : Prop :=
  (∀ vmId, record.currentVm = some vmId →
    vmId.value < world.nextVmId) ∧
  (∀ slotId, record.netSlot = some slotId →
    slotId.value < world.nextSlotId) ∧
  (∀ source, record.durableSnapshot = some source →
    RestoreSourceBelowCursors world source) ∧
  (∀ source, record.restoreFrom = some source →
    RestoreSourceBelowCursors world source)

def OperationPlanBelowCursors (world : World)
    (plan : OperationPlan) : Prop :=
  (∀ vmId, plan.vm = some vmId → vmId.value < world.nextVmId) ∧
  (∀ vmId, vmId ∈ plan.stopVms → vmId.value < world.nextVmId) ∧
  (∀ slotId, plan.slot = some slotId →
    slotId.value < world.nextSlotId) ∧
  (∀ slotId, slotId ∈ plan.closeSlots →
    slotId.value < world.nextSlotId) ∧
  (∀ source, plan.restoreFrom = some source →
    RestoreSourceBelowCursors world source) ∧
  (∀ source, plan.targetSnapshot = some source →
    RestoreSourceBelowCursors world source)

structure AllocationCursorValid (world : World) : Prop where
  vmStore : ∀ vmId vm, world.vms vmId = some vm →
    vmId.value < world.nextVmId
  slotStore : ∀ slotId slot, world.netSlots slotId = some slot →
    slotId.value < world.nextSlotId ∧ slot.vm.value < world.nextVmId
  snapshotStore : ∀ snapshotId snapshot,
    world.snapshots snapshotId = some snapshot →
      snapshotId.value < world.nextSnapshotId ∧
      snapshot.manifest.digest.value < world.nextManifestDigest
  vmIndex : ∀ sid vmId, vmId ∈ (world.resourceIndexes sid).vms →
    vmId.value < world.nextVmId
  slotIndex : ∀ sid slotId, slotId ∈ (world.resourceIndexes sid).slots →
    slotId.value < world.nextSlotId
  snapshotIndex : ∀ sid snapshotId,
    snapshotId ∈ (world.resourceIndexes sid).snapshots →
      snapshotId.value < world.nextSnapshotId
  sandbox : ∀ sid record, world.sandboxes sid = some record →
    SandboxReferencesBelowCursors world record
  plan : ∀ key operationPlan,
    world.operationPlans key = some operationPlan →
      OperationPlanBelowCursors world operationPlan

@[simp] theorem mem_insertUnique_iff [DecidableEq α]
    (candidate value : α) (values : List α) :
    candidate ∈ insertUnique value values ↔
      candidate = value ∨ candidate ∈ values := by
  by_cases present : value ∈ values
  · simp [insertUnique, present]
    intro same
    simpa [same] using present
  · simp [insertUnique, present]

theorem RestoreSourceBelowCursors.mono
    (below : RestoreSourceBelowCursors before source)
    (snapshotLe : before.nextSnapshotId ≤ after.nextSnapshotId)
    (manifestLe : before.nextManifestDigest ≤ after.nextManifestDigest) :
    RestoreSourceBelowCursors after source := by
  rcases below with ⟨snapshot, manifest⟩
  exact ⟨Nat.lt_of_lt_of_le snapshot snapshotLe,
    Nat.lt_of_lt_of_le manifest manifestLe⟩

theorem SandboxReferencesBelowCursors.mono
    (below : SandboxReferencesBelowCursors before record)
    (vmLe : before.nextVmId ≤ after.nextVmId)
    (slotLe : before.nextSlotId ≤ after.nextSlotId)
    (snapshotLe : before.nextSnapshotId ≤ after.nextSnapshotId)
    (manifestLe : before.nextManifestDigest ≤ after.nextManifestDigest) :
    SandboxReferencesBelowCursors after record := by
  rcases below with ⟨vm, slot, durable, restore⟩
  exact ⟨
    fun vmId found => Nat.lt_of_lt_of_le (vm vmId found) vmLe,
    fun slotId found => Nat.lt_of_lt_of_le (slot slotId found) slotLe,
    fun source found => (durable source found).mono snapshotLe manifestLe,
    fun source found => (restore source found).mono snapshotLe manifestLe⟩

theorem OperationPlanBelowCursors.mono
    (below : OperationPlanBelowCursors before plan)
    (vmLe : before.nextVmId ≤ after.nextVmId)
    (slotLe : before.nextSlotId ≤ after.nextSlotId)
    (snapshotLe : before.nextSnapshotId ≤ after.nextSnapshotId)
    (manifestLe : before.nextManifestDigest ≤ after.nextManifestDigest) :
    OperationPlanBelowCursors after plan := by
  rcases below with ⟨vm, stopVms, slot, closeSlots, restore, target⟩
  exact ⟨
    fun vmId found => Nat.lt_of_lt_of_le (vm vmId found) vmLe,
    fun vmId found => Nat.lt_of_lt_of_le (stopVms vmId found) vmLe,
    fun slotId found => Nat.lt_of_lt_of_le (slot slotId found) slotLe,
    fun slotId found => Nat.lt_of_lt_of_le (closeSlots slotId found) slotLe,
    fun source found => (restore source found).mono snapshotLe manifestLe,
    fun source found => (target source found).mono snapshotLe manifestLe⟩

theorem emptyWorld_allocationCursorValid :
    AllocationCursorValid emptyWorld := by
  constructor <;> simp [emptyWorld, SandboxReferencesBelowCursors,
    OperationPlanBelowCursors, ResourceIndex.empty]

theorem AllocationCursorValid.nextVmFresh
    (valid : AllocationCursorValid world) :
    world.vms ⟨world.nextVmId⟩ = none := by
  cases found : world.vms ⟨world.nextVmId⟩ with
  | none => rfl
  | some vm =>
      have below := valid.vmStore _ _ found
      change world.nextVmId < world.nextVmId at below
      omega

theorem AllocationCursorValid.nextSlotFresh
    (valid : AllocationCursorValid world) :
    world.netSlots ⟨world.nextSlotId⟩ = none := by
  cases found : world.netSlots ⟨world.nextSlotId⟩ with
  | none => rfl
  | some slot =>
      have below := (valid.slotStore _ _ found).1
      change world.nextSlotId < world.nextSlotId at below
      omega

theorem AllocationCursorValid.nextSnapshotFresh
    (valid : AllocationCursorValid world) :
    world.snapshots ⟨world.nextSnapshotId⟩ = none := by
  cases found : world.snapshots ⟨world.nextSnapshotId⟩ with
  | none => rfl
  | some snapshot =>
      have below := (valid.snapshotStore _ _ found).1
      change world.nextSnapshotId < world.nextSnapshotId at below
      omega

/-- The manifest cursor is not a map key, so freshness means that neither an
    existing snapshot nor any durable plan reference can name it. -/
theorem AllocationCursorValid.nextManifestFreshInSnapshots
    (valid : AllocationCursorValid world)
    (found : world.snapshots snapshotId = some snapshot) :
    snapshot.manifest.digest.value ≠ world.nextManifestDigest := by
  have below := (valid.snapshotStore snapshotId snapshot found).2
  omega

theorem allocationCursorValid_setSandbox
    (valid : AllocationCursorValid world)
    (references : SandboxReferencesBelowCursors world record) :
    AllocationCursorValid (setSandbox world sid record) := by
  refine {
    vmStore := by simpa using valid.vmStore
    slotStore := by simpa using valid.slotStore
    snapshotStore := by simpa using valid.snapshotStore
    vmIndex := by simpa using valid.vmIndex
    slotIndex := by simpa using valid.slotIndex
    snapshotIndex := by simpa using valid.snapshotIndex
    sandbox := ?_
    plan := by
      simpa [OperationPlanBelowCursors, RestoreSourceBelowCursors] using
        valid.plan }
  intro candidate candidateRecord found
  by_cases same : candidate = sid
  · subst candidate
    simp [setSandbox, put] at found
    subst candidateRecord
    simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors] using
      references
  · apply valid.sandbox candidate candidateRecord
    simpa [setSandbox, put, same] using found

theorem allocationCursorValid_setOperationPlan
    (valid : AllocationCursorValid world)
    (below : OperationPlanBelowCursors world operationPlan) :
    AllocationCursorValid (setOperationPlan world key operationPlan) := by
  refine {
    vmStore := by simpa using valid.vmStore
    slotStore := by simpa using valid.slotStore
    snapshotStore := by simpa using valid.snapshotStore
    vmIndex := by simpa using valid.vmIndex
    slotIndex := by simpa using valid.slotIndex
    snapshotIndex := by simpa using valid.snapshotIndex
    sandbox := by
      simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors] using
        valid.sandbox
    plan := ?_ }
  intro candidate candidatePlan found
  by_cases same : candidate = key
  · subst candidate
    simp [setOperationPlan, put] at found
    subst candidatePlan
    simpa [OperationPlanBelowCursors, RestoreSourceBelowCursors] using below
  · apply valid.plan candidate candidatePlan
    simpa [setOperationPlan, put, same] using found

theorem allocationCursorValid_setVm
    (valid : AllocationCursorValid world)
    (below : vmId.value < world.nextVmId) :
    AllocationCursorValid (setVm world vmId vm) := by
  refine {
    vmStore := ?_
    slotStore := by simpa using valid.slotStore
    snapshotStore := by simpa using valid.snapshotStore
    vmIndex := ?_
    slotIndex := ?_
    snapshotIndex := ?_
    sandbox := by
      simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors] using
        valid.sandbox
    plan := by
      simpa [OperationPlanBelowCursors, RestoreSourceBelowCursors] using
        valid.plan }
  · intro candidate candidateVm found
    by_cases same : candidate = vmId
    · simpa [same] using below
    · apply valid.vmStore candidate candidateVm
      simpa [setVm, put, same] using found
  · intro candidateSid candidate membership
    by_cases sameSid : candidateSid = vm.sandbox
    · subst candidateSid
      rw [show (setVm world vmId vm).resourceIndexes vm.sandbox =
          { world.resourceIndexes vm.sandbox with
            vms := insertUnique vmId (world.resourceIndexes vm.sandbox).vms } by
        simp [setVm, registerVm, setResourceIndex]] at membership
      rcases (mem_insertUnique_iff candidate vmId
        (world.resourceIndexes vm.sandbox).vms).mp membership with same | old
      · simpa [same] using below
      · exact valid.vmIndex vm.sandbox candidate old
    · have unchanged : (setVm world vmId vm).resourceIndexes candidateSid =
          world.resourceIndexes candidateSid := by
        simp [setVm, registerVm, setResourceIndex, sameSid]
      rw [unchanged] at membership
      exact valid.vmIndex candidateSid candidate membership
  · intro candidateSid candidate membership
    by_cases sameSid : candidateSid = vm.sandbox <;>
      simp [setVm, registerVm, setResourceIndex, sameSid] at membership
    · exact valid.slotIndex vm.sandbox candidate membership
    · exact valid.slotIndex candidateSid candidate membership
  · intro candidateSid candidate membership
    by_cases sameSid : candidateSid = vm.sandbox <;>
      simp [setVm, registerVm, setResourceIndex, sameSid] at membership
    · exact valid.snapshotIndex vm.sandbox candidate membership
    · exact valid.snapshotIndex candidateSid candidate membership

theorem allocationCursorValid_setNetSlot
    (valid : AllocationCursorValid world)
    (slotBelow : slotId.value < world.nextSlotId)
    (vmBelow : slot.vm.value < world.nextVmId) :
    AllocationCursorValid (setNetSlot world slotId slot) := by
  refine {
    vmStore := by simpa using valid.vmStore
    slotStore := ?_
    snapshotStore := by simpa using valid.snapshotStore
    vmIndex := ?_
    slotIndex := ?_
    snapshotIndex := ?_
    sandbox := by
      simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors] using
        valid.sandbox
    plan := by
      simpa [OperationPlanBelowCursors, RestoreSourceBelowCursors] using
        valid.plan }
  · intro candidate candidateSlot found
    by_cases same : candidate = slotId
    · simp [setNetSlot, put, same] at found
      subst candidateSlot
      exact ⟨by simpa [same] using slotBelow, vmBelow⟩
    · exact valid.slotStore candidate candidateSlot
        (by simpa [setNetSlot, put, same] using found)
  · intro candidateSid candidate membership
    by_cases sameSid : candidateSid = slot.sandbox <;>
      simp [setNetSlot, registerSlot, setResourceIndex, sameSid] at membership
    · exact valid.vmIndex slot.sandbox candidate membership
    · exact valid.vmIndex candidateSid candidate membership
  · intro candidateSid candidate membership
    by_cases sameSid : candidateSid = slot.sandbox
    · subst candidateSid
      rw [show (setNetSlot world slotId slot).resourceIndexes slot.sandbox =
          { world.resourceIndexes slot.sandbox with
            slots := insertUnique slotId (world.resourceIndexes slot.sandbox).slots } by
        simp [setNetSlot, registerSlot, setResourceIndex]] at membership
      rcases (mem_insertUnique_iff candidate slotId
        (world.resourceIndexes slot.sandbox).slots).mp membership with same | old
      · simpa [same] using slotBelow
      · exact valid.slotIndex slot.sandbox candidate old
    · have unchanged : (setNetSlot world slotId slot).resourceIndexes candidateSid =
          world.resourceIndexes candidateSid := by
        simp [setNetSlot, registerSlot, setResourceIndex, sameSid]
      rw [unchanged] at membership
      exact valid.slotIndex candidateSid candidate membership
  · intro candidateSid candidate membership
    by_cases sameSid : candidateSid = slot.sandbox <;>
      simp [setNetSlot, registerSlot, setResourceIndex, sameSid] at membership
    · exact valid.snapshotIndex slot.sandbox candidate membership
    · exact valid.snapshotIndex candidateSid candidate membership

theorem allocationCursorValid_setSnapshot
    (valid : AllocationCursorValid world)
    (snapshotBelow : snapshotId.value < world.nextSnapshotId)
    (manifestBelow : snapshot.manifest.digest.value <
      world.nextManifestDigest) :
    AllocationCursorValid (setSnapshot world snapshotId snapshot) := by
  refine {
    vmStore := by simpa using valid.vmStore
    slotStore := by simpa using valid.slotStore
    snapshotStore := ?_
    vmIndex := ?_
    slotIndex := ?_
    snapshotIndex := ?_
    sandbox := by
      simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors] using
        valid.sandbox
    plan := by
      simpa [OperationPlanBelowCursors, RestoreSourceBelowCursors] using
        valid.plan }
  · intro candidate candidateSnapshot found
    by_cases same : candidate = snapshotId
    · simp [setSnapshot, put, same] at found
      subst candidateSnapshot
      exact ⟨by simpa [same] using snapshotBelow, manifestBelow⟩
    · exact valid.snapshotStore candidate candidateSnapshot
        (by simpa [setSnapshot, put, same] using found)
  · intro candidateSid candidate membership
    by_cases sameSid : candidateSid = snapshot.sandbox <;>
      simp [setSnapshot, registerSnapshot, setResourceIndex, sameSid] at membership
    · exact valid.vmIndex snapshot.sandbox candidate membership
    · exact valid.vmIndex candidateSid candidate membership
  · intro candidateSid candidate membership
    by_cases sameSid : candidateSid = snapshot.sandbox <;>
      simp [setSnapshot, registerSnapshot, setResourceIndex, sameSid] at membership
    · exact valid.slotIndex snapshot.sandbox candidate membership
    · exact valid.slotIndex candidateSid candidate membership
  · intro candidateSid candidate membership
    by_cases sameSid : candidateSid = snapshot.sandbox
    · subst candidateSid
      rw [show (setSnapshot world snapshotId snapshot).resourceIndexes snapshot.sandbox =
          { world.resourceIndexes snapshot.sandbox with
            snapshots := insertUnique snapshotId
              (world.resourceIndexes snapshot.sandbox).snapshots } by
        simp [setSnapshot, registerSnapshot, setResourceIndex]] at membership
      rcases (mem_insertUnique_iff candidate snapshotId
        (world.resourceIndexes snapshot.sandbox).snapshots).mp membership with same | old
      · simpa [same] using snapshotBelow
      · exact valid.snapshotIndex snapshot.sandbox candidate old
    · have unchanged :
          (setSnapshot world snapshotId snapshot).resourceIndexes candidateSid =
            world.resourceIndexes candidateSid := by
        simp [setSnapshot, registerSnapshot, setResourceIndex, sameSid]
      rw [unchanged] at membership
      exact valid.snapshotIndex candidateSid candidate membership

theorem allocationCursorValid_of_sameAllocationFields
    (valid : AllocationCursorValid before)
    (sandboxes : after.sandboxes = before.sandboxes)
    (vms : after.vms = before.vms)
    (snapshots : after.snapshots = before.snapshots)
    (slots : after.netSlots = before.netSlots)
    (indexes : after.resourceIndexes = before.resourceIndexes)
    (plans : after.operationPlans = before.operationPlans)
    (nextVm : after.nextVmId = before.nextVmId)
    (nextSlot : after.nextSlotId = before.nextSlotId)
    (nextSnapshot : after.nextSnapshotId = before.nextSnapshotId)
    (nextManifest : after.nextManifestDigest = before.nextManifestDigest) :
    AllocationCursorValid after := by
  refine {
    vmStore := ?_
    slotStore := ?_
    snapshotStore := ?_
    vmIndex := ?_
    slotIndex := ?_
    snapshotIndex := ?_
    sandbox := ?_
    plan := ?_ }
  · intro vmId vm found
    have oldFound : before.vms vmId = some vm := by
      simpa [vms] using found
    simpa [nextVm] using valid.vmStore vmId vm oldFound
  · intro slotId slot found
    have oldFound : before.netSlots slotId = some slot := by
      simpa [slots] using found
    rcases valid.slotStore slotId slot oldFound with ⟨slotBelow, vmBelow⟩
    simpa [nextSlot, nextVm] using And.intro slotBelow vmBelow
  · intro snapshotId snapshot found
    have oldFound : before.snapshots snapshotId = some snapshot := by
      simpa [snapshots] using found
    rcases valid.snapshotStore snapshotId snapshot oldFound with
      ⟨snapshotBelow, manifestBelow⟩
    simpa [nextSnapshot, nextManifest] using
      And.intro snapshotBelow manifestBelow
  · intro sid vmId found
    have oldFound : vmId ∈ (before.resourceIndexes sid).vms := by
      simpa [indexes] using found
    simpa [nextVm] using valid.vmIndex sid vmId oldFound
  · intro sid slotId found
    have oldFound : slotId ∈ (before.resourceIndexes sid).slots := by
      simpa [indexes] using found
    simpa [nextSlot] using valid.slotIndex sid slotId oldFound
  · intro sid snapshotId found
    have oldFound : snapshotId ∈ (before.resourceIndexes sid).snapshots := by
      simpa [indexes] using found
    simpa [nextSnapshot] using valid.snapshotIndex sid snapshotId oldFound
  · intro sid record found
    have oldFound : before.sandboxes sid = some record := by
      simpa [sandboxes] using found
    simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors,
      nextVm, nextSlot, nextSnapshot, nextManifest] using
      valid.sandbox sid record oldFound
  · intro key plan found
    have oldFound : before.operationPlans key = some plan := by
      simpa [plans] using found
    simpa [OperationPlanBelowCursors, RestoreSourceBelowCursors,
      nextVm, nextSlot, nextSnapshot, nextManifest] using
      valid.plan key plan oldFound

theorem AllocationCursorValid.advanceStartup
    (valid : AllocationCursorValid world) :
    AllocationCursorValid
      { world with
        nextVmId := world.nextVmId + 1
        nextSlotId := world.nextSlotId + 1 } := by
  let advanced : World :=
    { world with
      nextVmId := world.nextVmId + 1
      nextSlotId := world.nextSlotId + 1 }
  change AllocationCursorValid advanced
  refine {
    vmStore := ?_
    slotStore := ?_
    snapshotStore := ?_
    vmIndex := ?_
    slotIndex := ?_
    snapshotIndex := ?_
    sandbox := ?_
    plan := ?_ }
  · intro vmId vm found
    have below := valid.vmStore vmId vm (by simpa [advanced] using found)
    simpa [advanced] using Nat.lt_succ_of_lt below
  · intro slotId slot found
    have oldFound : world.netSlots slotId = some slot := by
      simpa [advanced] using found
    rcases valid.slotStore slotId slot oldFound with ⟨slotBelow, vmBelow⟩
    exact ⟨by simpa [advanced] using Nat.lt_succ_of_lt slotBelow,
      by simpa [advanced] using Nat.lt_succ_of_lt vmBelow⟩
  · intro snapshotId snapshot found
    simpa [advanced] using valid.snapshotStore snapshotId snapshot
      (by simpa [advanced] using found)
  · intro sid vmId found
    have below := valid.vmIndex sid vmId (by simpa [advanced] using found)
    simpa [advanced] using Nat.lt_succ_of_lt below
  · intro sid slotId found
    have below := valid.slotIndex sid slotId (by simpa [advanced] using found)
    simpa [advanced] using Nat.lt_succ_of_lt below
  · intro sid snapshotId found
    exact valid.snapshotIndex sid snapshotId (by simpa [advanced] using found)
  · intro sid record found
    apply (valid.sandbox sid record (by simpa [advanced] using found)).mono
    all_goals simp [advanced]
  · intro key plan found
    apply (valid.plan key plan (by simpa [advanced] using found)).mono
    all_goals simp [advanced]

theorem AllocationCursorValid.advanceSnapshot
    (valid : AllocationCursorValid world) :
    AllocationCursorValid
      { world with
        nextSnapshotId := world.nextSnapshotId + 1
        nextManifestDigest := world.nextManifestDigest + 1 } := by
  let advanced : World :=
    { world with
      nextSnapshotId := world.nextSnapshotId + 1
      nextManifestDigest := world.nextManifestDigest + 1 }
  change AllocationCursorValid advanced
  refine {
    vmStore := by simpa [advanced] using valid.vmStore
    slotStore := by simpa [advanced] using valid.slotStore
    snapshotStore := ?_
    vmIndex := by simpa [advanced] using valid.vmIndex
    slotIndex := by simpa [advanced] using valid.slotIndex
    snapshotIndex := ?_
    sandbox := ?_
    plan := ?_ }
  · intro snapshotId snapshot found
    have oldFound : world.snapshots snapshotId = some snapshot := by
      simpa [advanced] using found
    rcases valid.snapshotStore snapshotId snapshot oldFound with
      ⟨snapshotBelow, manifestBelow⟩
    exact ⟨by simpa [advanced] using Nat.lt_succ_of_lt snapshotBelow,
      by simpa [advanced] using Nat.lt_succ_of_lt manifestBelow⟩
  · intro sid snapshotId found
    have below := valid.snapshotIndex sid snapshotId
      (by simpa [advanced] using found)
    simpa [advanced] using Nat.lt_succ_of_lt below
  · intro sid record found
    apply (valid.sandbox sid record (by simpa [advanced] using found)).mono
    all_goals simp [advanced]
  · intro key plan found
    apply (valid.plan key plan (by simpa [advanced] using found)).mono
    all_goals simp [advanced]

theorem allocationCursorValid_reserveStartupResources
    (valid : AllocationCursorValid world)
    (found : world.operationPlans key = some plan) :
    AllocationCursorValid (reserveStartupResources world key plan) := by
  let advanced : World :=
    { world with
      nextVmId := world.nextVmId + 1
      nextSlotId := world.nextSlotId + 1 }
  have advancedValid : AllocationCursorValid advanced := by
    simpa [advanced] using valid.advanceStartup
  apply allocationCursorValid_setOperationPlan advancedValid
  rcases valid.plan key plan found with
    ⟨oldVm, oldStopVms, oldSlot, oldCloseSlots, oldRestore, oldTarget⟩
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_⟩
  · intro vmId selected
    simp at selected
    subst vmId
    simp [advanced]
  · intro vmId member
    rw [mem_insertUnique_iff] at member
    rcases member with same | old
    · subst vmId
      simp [advanced]
    · have below := oldStopVms vmId old
      simpa [advanced] using Nat.lt_succ_of_lt below
  · intro slotId selected
    simp at selected
    subst slotId
    simp [advanced]
  · intro slotId member
    rw [mem_insertUnique_iff] at member
    rcases member with same | old
    · subst slotId
      simp [advanced]
    · have below := oldCloseSlots slotId old
      simpa [advanced] using Nat.lt_succ_of_lt below
  · intro source selected
    apply (oldRestore source (by simpa using selected)).mono
    all_goals simp [advanced]
  · intro source selected
    apply (oldTarget source (by simpa using selected)).mono
    all_goals simp [advanced]

theorem allocationCursorValid_reservePauseSnapshot
    (valid : AllocationCursorValid world)
    (foundSandbox : world.sandboxes sid = some record)
    (currentVm : record.currentVm = some vmId)
    (foundPlan : world.operationPlans key = some plan) :
    AllocationCursorValid (reservePauseSnapshot world key plan vmId) := by
  let advanced : World :=
    { world with
      nextSnapshotId := world.nextSnapshotId + 1
      nextManifestDigest := world.nextManifestDigest + 1 }
  have advancedValid : AllocationCursorValid advanced := by
    simpa [advanced] using valid.advanceSnapshot
  have vmBelow : vmId.value < world.nextVmId :=
    (valid.sandbox sid record foundSandbox).1 vmId currentVm
  apply allocationCursorValid_setOperationPlan advancedValid
  rcases valid.plan key plan foundPlan with
    ⟨oldVm, oldStopVms, oldSlot, oldCloseSlots, oldRestore, oldTarget⟩
  refine ⟨?_, ?_, ?_, ?_, ?_, ?_⟩
  · intro selectedVm selected
    simp at selected
    subst selectedVm
    simpa [advanced] using vmBelow
  · intro stopped member
    simpa [advanced] using oldStopVms stopped member
  · intro selectedSlot selected
    simpa [advanced] using oldSlot selectedSlot selected
  · intro closed member
    simpa [advanced] using oldCloseSlots closed member
  · intro source selected
    apply (oldRestore source (by simpa using selected)).mono
    all_goals simp [advanced]
  · intro source selected
    simp at selected
    subst source
    constructor <;> simp [advanced]

def OperationInputBelowCursors (world : World) : OperationInput → Prop
  | .resume source => RestoreSourceBelowCursors world source
  | _ => True

theorem initialOperationPlan_belowCursors
    (valid : AllocationCursorValid world)
    (inputBelow : OperationInputBelowCursors world input) :
    OperationPlanBelowCursors world
      (initialOperationPlan world sid operation input) := by
  cases input with
  | create workspace policy filePolicy =>
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_⟩
      · simp [initialOperationPlan]
      · intro vmId member
        exact valid.vmIndex sid vmId (by simpa [initialOperationPlan] using member)
      · simp [initialOperationPlan]
      · intro slotId member
        exact valid.slotIndex sid slotId
          (by simpa [initialOperationPlan] using member)
      · simp [initialOperationPlan]
      · simp [initialOperationPlan]
  | pause generation =>
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_⟩
      · simp [initialOperationPlan]
      · intro vmId member
        exact valid.vmIndex sid vmId (by simpa [initialOperationPlan] using member)
      · simp [initialOperationPlan]
      · intro slotId member
        exact valid.slotIndex sid slotId
          (by simpa [initialOperationPlan] using member)
      · simp [initialOperationPlan]
      · simp [initialOperationPlan]
  | resume source =>
      have sourceBelow : RestoreSourceBelowCursors world source := by
        simpa [OperationInputBelowCursors] using inputBelow
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_⟩
      · simp [initialOperationPlan]
      · intro vmId member
        exact valid.vmIndex sid vmId (by simpa [initialOperationPlan] using member)
      · simp [initialOperationPlan]
      · intro slotId member
        exact valid.slotIndex sid slotId
          (by simpa [initialOperationPlan] using member)
      · intro selected found
        have same : selected = source := by
          simpa [initialOperationPlan] using Option.some.inj found.symm
        simpa [same] using sourceBelow
      · simp [initialOperationPlan]
  | kill generation =>
      refine ⟨?_, ?_, ?_, ?_, ?_, ?_⟩
      · simp [initialOperationPlan]
      · intro vmId member
        exact valid.vmIndex sid vmId (by simpa [initialOperationPlan] using member)
      · simp [initialOperationPlan]
      · intro slotId member
        exact valid.slotIndex sid slotId
          (by simpa [initialOperationPlan] using member)
      · simp [initialOperationPlan]
      · simp [initialOperationPlan]

theorem allocationCursorValid_storeNewOperation
    (valid : AllocationCursorValid world)
    (inputBelow : OperationInputBelowCursors world input) :
    AllocationCursorValid (storeNewOperation world sid operation input) := by
  unfold storeNewOperation
  have storedValid : AllocationCursorValid
      (setOperation world (operationKey sid operation.id) {
        sandbox := sid
        operation := operation
        input := input
        result := none }) := by
    apply allocationCursorValid_of_sameAllocationFields valid <;> rfl
  apply allocationCursorValid_setOperationPlan storedValid
  apply (initialOperationPlan_belowCursors valid inputBelow).mono <;>
    exact Nat.le_refl _

private theorem commitEffect_preservesAllocationField
    (field : World → α)
    (setEffectStable : ∀ (base : World) (id : EffectId)
      (status : EffectStatus), field (setEffect base id status) = field base)
    (setLogicalStable : ∀ (base : World) (id : LogicalEffectId)
      (record : LogicalEffectRecord),
      field (setLogicalEffect base id record) = field base) :
    field (commitEffect world command) = field world := by
  cases found : world.effects (effectId command) with
  | some status => simp [commitEffect, found]
  | none =>
      cases logical : world.logicalEffects (logicalEffectId command) with
      | some record =>
          simpa [commitEffect, found, logical] using
            setEffectStable world (effectId command) .committed
      | none =>
          rw [show commitEffect world command =
              setLogicalEffect
                (setEffect world (effectId command) .committed)
                (logicalEffectId command) {
                  origin := inferredEffectOrigin world command
                  currentAttempt := 0
                  remainingRetries := defaultRetryLimit
                  result := none } by
            simp [commitEffect, found, logical]]
          rw [setLogicalStable, setEffectStable]

theorem allocationCursorValid_enqueue
    (valid : AllocationCursorValid world) :
    AllocationCursorValid (enqueue world command) := by
  apply allocationCursorValid_of_sameAllocationFields valid
  all_goals
    simp only [enqueue]
    apply commitEffect_preservesAllocationField <;> intros <;> rfl

theorem allocationCursorValid_updateOperation
    (valid : AllocationCursorValid world) :
    AllocationCursorValid (updateOperation world sid operation result) := by
  apply allocationCursorValid_of_sameAllocationFields valid <;>
    rfl

theorem allocationCursorValid_updateCurrentOperation
    (valid : AllocationCursorValid world) :
    AllocationCursorValid (updateCurrentOperation world sid operation result) := by
  apply allocationCursorValid_of_sameAllocationFields valid <;>
    rfl

private theorem supersedeCurrentOperation_preservesAllocationField
    (field : World → α)
    (stable : ∀ (base : World) (target : SandboxId) (operation : Operation)
      (result : Option OperationResult),
      field (updateOperation base target operation result) = field base) :
    field (supersedeCurrentOperation world sid current) = field world := by
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
                · simpa [final, sameOperation] using
                    stable world sid
                      { operation with status := .failed .cancelled }
                      (some (.failed .cancelled))
                · simp [final, sameOperation]
            | some result => simp [final]

theorem allocationCursorValid_supersedeCurrentOperation
    (valid : AllocationCursorValid world) :
    AllocationCursorValid (supersedeCurrentOperation world sid current) := by
  apply allocationCursorValid_of_sameAllocationFields valid
  · exact supersedeCurrentOperation_sandboxes
  · exact supersedeCurrentOperation_vms
  · exact supersedeCurrentOperation_snapshots
  · exact supersedeCurrentOperation_netSlots
  · apply supersedeCurrentOperation_preservesAllocationField
    intro base target operation result
    rfl
  · exact supersedeCurrentOperation_operationPlans
  all_goals
    apply supersedeCurrentOperation_preservesAllocationField
    intro base target operation result
    rfl

theorem allocationCursorValid_setFilesystem
    (valid : AllocationCursorValid world) :
    AllocationCursorValid (setFilesystem world fsId filesystem) := by
  apply allocationCursorValid_of_sameAllocationFields valid <;> rfl

theorem allocationCursorValid_setFence
    (valid : AllocationCursorValid world) :
    AllocationCursorValid (setFence world sid fence) := by
  apply allocationCursorValid_of_sameAllocationFields valid <;> rfl

theorem allocationCursorValid_setAppliedFilePolicy
    (valid : AllocationCursorValid world) :
    AllocationCursorValid (setAppliedFilePolicy world sid policy) := by
  apply allocationCursorValid_of_sameAllocationFields valid <;> rfl

theorem allocationCursorValid_setEffect
    (valid : AllocationCursorValid world) (id : EffectId)
    (status : EffectStatus) :
    AllocationCursorValid (setEffect world id status) := by
  apply allocationCursorValid_of_sameAllocationFields valid <;> rfl

theorem allocationCursorValid_dispatchEffect
    (valid : AllocationCursorValid world) (id : EffectId) :
    AllocationCursorValid (dispatchEffect world id) := by
  exact allocationCursorValid_setEffect valid id .dispatched

theorem allocationCursorValid_acknowledgeEffect
    (valid : AllocationCursorValid world) (id : EffectId)
    (outcome : EffectOutcome) :
    AllocationCursorValid (acknowledgeEffect world id outcome) := by
  apply allocationCursorValid_of_sameAllocationFields valid <;> rfl

theorem allocationCursorValid_startNextAttempt
    (valid : AllocationCursorValid world) (logical : LogicalEffectId)
    (failure : FailureKind) :
    AllocationCursorValid (startNextAttempt world logical failure) := by
  apply allocationCursorValid_of_sameAllocationFields valid <;> rfl

theorem allocationCursorValid_finalizeLogicalFailure
    (valid : AllocationCursorValid world) (logical : LogicalEffectId)
    (failure : FailureKind) :
    AllocationCursorValid (finalizeLogicalFailure world logical failure) := by
  apply allocationCursorValid_of_sameAllocationFields valid <;> rfl

theorem allocationCursorValid_markEffectOutcomeUnknown
    (valid : AllocationCursorValid world) (id : EffectId) :
    AllocationCursorValid (markEffectOutcomeUnknown world id) := by
  exact allocationCursorValid_setEffect valid id .outcomeUnknown

theorem allocationCursorValid_beginCreate
    (valid : AllocationCursorValid world) :
    AllocationCursorValid
      (beginCreate world sid owner workspace filePolicy policy operationId) := by
  unfold beginCreate
  dsimp only
  apply allocationCursorValid_enqueue
  apply allocationCursorValid_storeNewOperation
  · apply allocationCursorValid_setSandbox
    · exact allocationCursorValid_setFilesystem valid
    · simp [SandboxReferencesBelowCursors]
  · simp [OperationInputBelowCursors]

theorem allocationCursorValid_beginPause
    (valid : AllocationCursorValid world)
    (found : world.sandboxes sid = some record) :
    AllocationCursorValid
      (beginPause world sid record operationId vmId) := by
  unfold beginPause
  dsimp only
  apply allocationCursorValid_enqueue
  apply allocationCursorValid_enqueue
  apply allocationCursorValid_storeNewOperation
  · apply allocationCursorValid_setSandbox valid
    simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors] using
      valid.sandbox sid record found
  · simp [OperationInputBelowCursors]

theorem allocationCursorValid_beginResume
    (valid : AllocationCursorValid world)
    (found : world.sandboxes sid = some record)
    (currentSnapshot : record.durableSnapshot = some source) :
    AllocationCursorValid
      (beginResume world sid record operationId source) := by
  have references := valid.sandbox sid record found
  have sourceBelow : RestoreSourceBelowCursors world source :=
    references.2.2.1 source currentSnapshot
  unfold beginResume
  dsimp only
  apply allocationCursorValid_enqueue
  apply allocationCursorValid_storeNewOperation
  · apply allocationCursorValid_setSandbox valid
    refine ⟨?_, ?_, ?_, ?_⟩
    · simp
    · simp
    · intro selected selectedSnapshot
      exact references.2.2.1 selected (by simpa using selectedSnapshot)
    · intro selected selectedSource
      have same : selected = source := by simpa using Option.some.inj selectedSource.symm
      simpa [same] using sourceBelow
  · change RestoreSourceBelowCursors
      (setSandbox world sid {
        owner := record.owner
        phase := .starting
        terminal := record.terminal
        epoch := record.epoch + 1
        generation := record.generation + 1
        currentVm := none
        workspace := record.workspace
        workspaceAccess := record.workspaceAccess
        filePolicy := record.filePolicy
        durableSnapshot := record.durableSnapshot
        restoreFrom := some source
        netSlot := none
        policy := record.policy
        desired := .closed
        effective := .closed
        currentOperation := some {
          id := operationId
          epoch := record.epoch + 1
          kind := .resume
          status := .pending } }) source
    apply sourceBelow.mono <;> exact Nat.le_refl _

theorem allocationCursorValid_beginKill
    (valid : AllocationCursorValid world)
    (found : world.sandboxes sid = some record) :
    AllocationCursorValid
      (beginKill world sid record operationId) := by
  let superseded := supersedeCurrentOperation world sid record.currentOperation
  have supersededValid : AllocationCursorValid superseded := by
    simpa [superseded] using
      allocationCursorValid_supersedeCurrentOperation
        (world := world) (sid := sid) (current := record.currentOperation) valid
  have supersededFound : superseded.sandboxes sid = some record := by
    simpa [superseded] using found
  have references := supersededValid.sandbox sid record supersededFound
  unfold beginKill
  dsimp only
  apply allocationCursorValid_enqueue
  apply allocationCursorValid_storeNewOperation
  · apply allocationCursorValid_setSandbox supersededValid
    simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors] using
      references
  · simp [OperationInputBelowCursors]

theorem allocationCursorValid_applyRevocationFence
    (valid : AllocationCursorValid world)
    (found : world.sandboxes sid = some record) :
    AllocationCursorValid (applyRevocationFence world sid record) := by
  have references := valid.sandbox sid record found
  have closedValid : AllocationCursorValid
      (setFence (setSandbox world sid { record with effective := .closed })
        sid ⟨record.epoch, record.terminal⟩) := by
    apply allocationCursorValid_setFence
    apply allocationCursorValid_setSandbox valid
    simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors] using
      references
  cases slotChoice : record.netSlot with
  | none => simpa [applyRevocationFence, slotChoice] using closedValid
  | some slotId =>
      cases slotFound : world.netSlots slotId with
      | none => simpa [applyRevocationFence, slotChoice, slotFound] using closedValid
      | some slot =>
          rcases valid.slotStore slotId slot slotFound with
            ⟨slotBelow, vmBelow⟩
          have updated := allocationCursorValid_setNetSlot
            (world := setFence
              (setSandbox world sid { record with effective := .closed }) sid
              ⟨record.epoch, record.terminal⟩)
            (slotId := slotId) (slot := { slot with active := false })
            closedValid (by
              change slotId.value < world.nextSlotId
              exact slotBelow) (by
              change slot.vm.value < world.nextVmId
              exact vmBelow)
          simpa [applyRevocationFence, slotChoice, slotFound] using updated

theorem allocationCursorValid_prepareResources
    (valid : AllocationCursorValid world)
    (found : world.sandboxes sid = some record)
    (vmBelow : vmId.value < world.nextVmId)
    (slotBelow : slotId.value < world.nextSlotId) :
    AllocationCursorValid (prepareResources world sid record vmId slotId) := by
  let vm : Vm := ⟨record.owner, sid, record.generation, .prepared⟩
  let slot : NetSlot := {
    owner := record.owner
    sandbox := sid
    generation := record.generation
    vm := vmId
    policy := record.policy
    configured := false
    active := false
    reclaimed := false }
  let updated := { record with currentVm := some vmId, netSlot := some slotId }
  have vmValid : AllocationCursorValid (setVm world vmId vm) :=
    allocationCursorValid_setVm valid vmBelow
  have slotValid : AllocationCursorValid
      (setNetSlot (setVm world vmId vm) slotId slot) := by
    apply allocationCursorValid_setNetSlot vmValid
    · simpa [vm] using slotBelow
    · simpa [vm, slot] using vmBelow
  have oldReferences := valid.sandbox sid record found
  have preparedValid : AllocationCursorValid
      (setSandbox (setNetSlot (setVm world vmId vm) slotId slot) sid updated) := by
    apply allocationCursorValid_setSandbox slotValid
    refine ⟨?_, ?_, ?_, ?_⟩
    · intro selected selectedVm
      have same : selected = vmId := by
        simpa [updated] using Option.some.inj selectedVm.symm
      simpa [vm, slot, updated, same] using vmBelow
    · intro selected selectedSlot
      have same : selected = slotId := by
        simpa [updated] using Option.some.inj selectedSlot.symm
      simpa [vm, slot, updated, same] using slotBelow
    · intro source selected
      exact oldReferences.2.2.1 source (by simpa [updated] using selected)
    · intro source selected
      exact oldReferences.2.2.2 source (by simpa [updated] using selected)
  unfold prepareResources
  dsimp only
  apply allocationCursorValid_enqueue
  apply allocationCursorValid_enqueue
  apply allocationCursorValid_enqueue
  simpa [vm, slot, updated] using preparedValid

theorem allocationCursorValid_configureSlot
    (valid : AllocationCursorValid world)
    (found : world.netSlots slotId = some slot) :
    AllocationCursorValid (configureSlot world slotId slot) := by
  rcases valid.slotStore slotId slot found with ⟨slotBelow, vmBelow⟩
  exact allocationCursorValid_setNetSlot valid slotBelow vmBelow

theorem allocationCursorValid_activateResource
    (valid : AllocationCursorValid world)
    (found : world.vms vmId = some vm) :
    AllocationCursorValid (activateResource world vmId vm) := by
  exact allocationCursorValid_setVm valid (valid.vmStore vmId vm found)

theorem allocationCursorValid_sealSnapshot
    (valid : AllocationCursorValid world)
    (sourceBelow : RestoreSourceBelowCursors world ⟨snapshotId, manifest⟩) :
    AllocationCursorValid (sealSnapshot world snapshotId manifest record sid) := by
  exact allocationCursorValid_setSnapshot valid sourceBelow.1 sourceBelow.2

theorem allocationCursorValid_makeSnapshotDurable
    (valid : AllocationCursorValid world)
    (found : world.snapshots snapshotId = some snapshot) :
    AllocationCursorValid (makeSnapshotDurable world snapshotId snapshot) := by
  rcases valid.snapshotStore snapshotId snapshot found with
    ⟨snapshotBelow, manifestBelow⟩
  exact allocationCursorValid_setSnapshot valid snapshotBelow manifestBelow

theorem allocationCursorValid_stopResource
    (valid : AllocationCursorValid world)
    (found : world.vms vmId = some vm) :
    AllocationCursorValid (stopResource world vmId vm) := by
  exact allocationCursorValid_setVm valid (valid.vmStore vmId vm found)

theorem allocationCursorValid_reclaimVm
    (valid : AllocationCursorValid world)
    (found : world.vms vmId = some vm) :
    AllocationCursorValid (reclaimVm world vmId vm) := by
  exact allocationCursorValid_setVm valid (valid.vmStore vmId vm found)

theorem allocationCursorValid_reclaimSlot
    (valid : AllocationCursorValid world)
    (found : world.netSlots slotId = some slot) :
    AllocationCursorValid (reclaimSlot world slotId slot) := by
  rcases valid.slotStore slotId slot found with ⟨slotBelow, vmBelow⟩
  exact allocationCursorValid_setNetSlot valid slotBelow vmBelow

theorem allocationCursorValid_publishRunning
    (valid : AllocationCursorValid world)
    (foundSandbox : world.sandboxes sid = some record)
    (foundSlot : world.netSlots slotId = some slot) :
    AllocationCursorValid (publishRunning world sid record slotId slot) := by
  rcases valid.slotStore slotId slot foundSlot with ⟨slotBelow, vmBelow⟩
  have slotValid : AllocationCursorValid
      (setNetSlot world slotId { slot with active := true }) :=
    allocationCursorValid_setNetSlot valid slotBelow vmBelow
  have references := valid.sandbox sid record foundSandbox
  let base := { record with
    phase := .running
    terminal := false
    restoreFrom := none
    currentOperation := record.currentOperation.map
      (fun (op : Operation) => { op with status := .succeeded }) }
  let authority : Authority := .running sid base.generation slot.vm base.workspace
    base.workspaceAccess slotId base.policy
  let running := { base with desired := authority, effective := authority }
  have publishedValid : AllocationCursorValid
      (setSandbox (setNetSlot world slotId { slot with active := true })
        sid running) := by
    apply allocationCursorValid_setSandbox slotValid
    refine ⟨?_, ?_, ?_, ?_⟩
    · intro selected selectedVm
      exact references.1 selected (by simpa [base, running] using selectedVm)
    · intro selected selectedSlot
      exact references.2.1 selected
        (by simpa [base, running] using selectedSlot)
    · intro source selected
      exact references.2.2.1 source
        (by simpa [base, running] using selected)
    · simp [base, running]
  unfold publishRunning
  dsimp only
  apply allocationCursorValid_updateCurrentOperation
  simpa [base, authority, running] using publishedValid

theorem allocationCursorValid_publishPaused
    (valid : AllocationCursorValid world)
    (foundSandbox : world.sandboxes sid = some record)
    (foundSnapshot : world.snapshots source.snapshot = some snapshot)
    (sameManifest : snapshot.manifest = source.manifest) :
    AllocationCursorValid (publishPaused world sid record source) := by
  rcases valid.snapshotStore source.snapshot snapshot foundSnapshot with
    ⟨snapshotBelow, manifestBelow⟩
  have sourceBelow : RestoreSourceBelowCursors world source := by
    refine ⟨snapshotBelow, ?_⟩
    simpa [sameManifest] using manifestBelow
  have references := valid.sandbox sid record foundSandbox
  let base := { record with
    phase := .paused
    currentVm := none
    durableSnapshot := some source
    restoreFrom := none
    netSlot := none
    currentOperation := record.currentOperation.map
      (fun (op : Operation) => { op with status := .succeeded }) }
  let authority : Authority := .paused sid source
  let paused := { base with desired := authority, effective := authority }
  have publishedValid : AllocationCursorValid (setSandbox world sid paused) := by
    apply allocationCursorValid_setSandbox valid
    refine ⟨?_, ?_, ?_, ?_⟩
    · simp [base, paused]
    · simp [base, paused]
    · intro selected selectedSource
      have same : selected = source := by
        simpa [base, paused] using Option.some.inj selectedSource.symm
      simpa [same] using sourceBelow
    · simp [base, paused]
  unfold publishPaused
  dsimp only
  apply allocationCursorValid_updateCurrentOperation
  simpa [base, authority, paused] using publishedValid

theorem allocationCursorValid_beginCompensation
    (valid : AllocationCursorValid world)
    (found : world.sandboxes sid = some record)
    (failure : FailureKind) :
    AllocationCursorValid
      (beginCompensation world sid record operation failure) := by
  have references := valid.sandbox sid record found
  unfold beginCompensation
  dsimp only
  apply allocationCursorValid_enqueue
  apply allocationCursorValid_updateOperation
  apply allocationCursorValid_setSandbox valid
  simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors] using references

theorem allocationCursorValid_publishDestroyed
    (valid : AllocationCursorValid world)
    (found : world.sandboxes sid = some record) :
    AllocationCursorValid (publishDestroyed world sid record) := by
  have references := valid.sandbox sid record found
  unfold publishDestroyed
  dsimp only
  apply allocationCursorValid_updateCurrentOperation
  apply allocationCursorValid_setSandbox valid
  refine ⟨?_, ?_, ?_, ?_⟩
  · simp
  · simp
  · intro source selected
    exact references.2.2.1 source (by simpa using selected)
  · intro source selected
    exact references.2.2.2 source (by simpa using selected)

theorem allocationCursorValid_publishFailure
    (valid : AllocationCursorValid world)
    (found : world.sandboxes sid = some record)
    (failure : FailureKind) :
    AllocationCursorValid (publishFailure world sid record failure) := by
  have references := valid.sandbox sid record found
  unfold publishFailure
  dsimp only
  apply allocationCursorValid_updateCurrentOperation
  apply allocationCursorValid_setSandbox valid
  refine ⟨?_, ?_, ?_, ?_⟩
  · simp
  · simp
  · intro source selected
    exact references.2.2.1 source (by simpa using selected)
  · intro source selected
    exact references.2.2.2 source (by simpa using selected)

theorem allocationCursorValid_markOutcomeUnknown
    (valid : AllocationCursorValid world)
    (found : world.sandboxes sid = some record) :
    AllocationCursorValid (markOutcomeUnknown world sid record operation) := by
  have references := valid.sandbox sid record found
  unfold markOutcomeUnknown
  dsimp only
  apply allocationCursorValid_updateOperation
  apply allocationCursorValid_setSandbox valid
  simpa [SandboxReferencesBelowCursors, RestoreSourceBelowCursors] using references

theorem Step.preservesAllocationCursorValid
    (transition : Step world event next)
    (valid : AllocationCursorValid world) :
    AllocationCursorValid next := by
  cases transition with
  | acceptCreate =>
      exact allocationCursorValid_beginCreate valid
  | retryOperation => exact valid
  | queryOperation => exact valid
  | dispatchEffect =>
      exact allocationCursorValid_dispatchEffect valid _
  | acknowledgeEffect =>
      exact allocationCursorValid_acknowledgeEffect valid _ _
  | replayEffectReceipt => exact valid
  | effectTimeout =>
      exact allocationCursorValid_markEffectOutcomeUnknown valid _
  | repeatEffectTimeout => exact valid
  | startNextAttempt failure =>
      exact allocationCursorValid_startNextAttempt valid _ failure
  | finalizeEffectFailure failure =>
      exact allocationCursorValid_finalizeLogicalFailure valid _ failure
  | reserveStartupResources =>
      apply allocationCursorValid_reserveStartupResources valid
      assumption
  | reservePauseSnapshot =>
      apply allocationCursorValid_reservePauseSnapshot valid <;> assumption
  | installStartFence =>
      exact allocationCursorValid_setFence valid
  | installRevocationFence =>
      apply allocationCursorValid_applyRevocationFence valid
      assumption
  | dispatchPrepare =>
      exact allocationCursorValid_enqueue valid
  | prepareVm =>
      have planBelow := valid.plan _ _ (by assumption)
      apply allocationCursorValid_prepareResources valid
      · assumption
      · apply planBelow.1
        assumption
      · apply planBelow.2.2.1
        assumption
  | configureNetwork =>
      apply allocationCursorValid_configureSlot valid
      assumption
  | applyFilePolicy =>
      exact allocationCursorValid_setAppliedFilePolicy valid
  | activateVm =>
      apply allocationCursorValid_activateResource valid
      assumption
  | publishRunning =>
      apply allocationCursorValid_publishRunning valid <;> assumption
  | acceptPause =>
      apply allocationCursorValid_beginPause valid
      assumption
  | freezeVm =>
      apply allocationCursorValid_setVm valid
      apply valid.vmStore <;> assumption
  | scheduleSealSnapshot =>
      exact allocationCursorValid_enqueue valid
  | sealSnapshot =>
      apply allocationCursorValid_sealSnapshot valid
      have planBelow := valid.plan _ _ (by assumption)
      apply planBelow.2.2.2.2.2
      assumption
  | scheduleCommitSnapshot =>
      exact allocationCursorValid_enqueue valid
  | commitSnapshot =>
      exact allocationCursorValid_enqueue
        (allocationCursorValid_makeSnapshotDurable valid (by assumption))
  | scheduleStopVm =>
      exact allocationCursorValid_enqueue valid
  | stopVm =>
      apply allocationCursorValid_stopResource valid
      assumption
  | scheduleCleanupVm =>
      exact allocationCursorValid_enqueue valid
  | scheduleCleanupSlot =>
      exact allocationCursorValid_enqueue valid
  | cleanupVm =>
      apply allocationCursorValid_reclaimVm valid
      assumption
  | cleanupSlot =>
      apply allocationCursorValid_reclaimSlot valid
      assumption
  | publishPaused =>
      apply allocationCursorValid_publishPaused valid
      · assumption
      · assumption
      · rcases ‹_ ∧ _ ∧ _ ∧ _ ∧ _ ∧ _ ∧ _› with
          ⟨_, _, _, _, sameManifest, _, _⟩
        exact sameManifest
  | acceptResume =>
      apply allocationCursorValid_beginResume valid <;> assumption
  | acceptKill =>
      apply allocationCursorValid_beginKill valid
      assumption
  | beginCompensation =>
      apply allocationCursorValid_beginCompensation valid <;> assumption
  | publishDestroyed =>
      apply allocationCursorValid_publishDestroyed valid
      assumption
  | publishFailure =>
      apply allocationCursorValid_publishFailure valid <;> assumption
  | rpcTimeout =>
      apply allocationCursorValid_markOutcomeUnknown valid
      assumption
  | controllerCrash =>
      apply allocationCursorValid_of_sameAllocationFields valid <;> rfl
  | controllerRecover =>
      apply allocationCursorValid_of_sameAllocationFields valid <;> rfl
  | duplicateOrLateDelivery => exact valid
  | exec => exact valid
  | fileRead => exact valid
  | fileWrite => exact valid
  | ingress => exact valid
  | egress => exact valid

theorem reachable_allocationCursorValid
    (reachable : Reachable world) : AllocationCursorValid world := by
  induction reachable with
  | initial => exact emptyWorld_allocationCursorValid
  | step reachable transition ih =>
      exact transition.preservesAllocationCursorValid ih

end SandboxModel
