import SandboxModel.Step

/-!
Finite, executable publication checks.

The authoritative resource tables remain lookup functions, while
`World.resourceIndexes` supplies the finite domain that a deployed controller
can actually scan.  The validity predicate below connects both views.  This
keeps the executable checks small without weakening the relational safety
conditions used by the protocol.
-/

namespace SandboxModel

def VmIndexSound (world : World) : Prop :=
  ∀ sid vmId, vmId ∈ (world.resourceIndexes sid).vms →
    ∃ vm, world.vms vmId = some vm ∧ vm.sandbox = sid

def VmIndexComplete (world : World) : Prop :=
  ∀ sid vmId vm, world.vms vmId = some vm → vm.sandbox = sid →
    vmId ∈ (world.resourceIndexes sid).vms

def SlotIndexSound (world : World) : Prop :=
  ∀ sid slotId, slotId ∈ (world.resourceIndexes sid).slots →
    ∃ slot, world.netSlots slotId = some slot ∧ slot.sandbox = sid

def SlotIndexComplete (world : World) : Prop :=
  ∀ sid slotId slot, world.netSlots slotId = some slot → slot.sandbox = sid →
    slotId ∈ (world.resourceIndexes sid).slots

def SnapshotIndexSound (world : World) : Prop :=
  ∀ sid snapshotId, snapshotId ∈ (world.resourceIndexes sid).snapshots →
    ∃ snapshot, world.snapshots snapshotId = some snapshot ∧
      snapshot.sandbox = sid

def SnapshotIndexComplete (world : World) : Prop :=
  ∀ sid snapshotId snapshot,
    world.snapshots snapshotId = some snapshot → snapshot.sandbox = sid →
      snapshotId ∈ (world.resourceIndexes sid).snapshots

def ResourceIndexValid (world : World) : Prop :=
  VmIndexSound world ∧ VmIndexComplete world ∧
    SlotIndexSound world ∧ SlotIndexComplete world ∧
    SnapshotIndexSound world ∧ SnapshotIndexComplete world

def vmStoppedOrReclaimedCheck (world : World) (vmId : VmId) : Bool :=
  match world.vms vmId with
  | none => false
  | some vm => decide (vm.status = .stopped ∨ vm.status = .reclaimed)

def indexedVmsClosedCheck (world : World) (sid : SandboxId) : Bool :=
  (world.resourceIndexes sid).vms.all (vmStoppedOrReclaimedCheck world)

def IndexedVmsClosed (world : World) (sid : SandboxId) : Prop :=
  ∀ vmId, vmId ∈ (world.resourceIndexes sid).vms →
    ∃ vm, world.vms vmId = some vm ∧
      (vm.status = .stopped ∨ vm.status = .reclaimed)

theorem vmStoppedOrReclaimedCheck_spec :
    vmStoppedOrReclaimedCheck world vmId = true ↔
      ∃ vm, world.vms vmId = some vm ∧
        (vm.status = .stopped ∨ vm.status = .reclaimed) := by
  unfold vmStoppedOrReclaimedCheck
  cases found : world.vms vmId <;> simp

theorem indexedVmsClosedCheck_spec :
    indexedVmsClosedCheck world sid = true ↔ IndexedVmsClosed world sid := by
  unfold indexedVmsClosedCheck IndexedVmsClosed
  induction (world.resourceIndexes sid).vms with
  | nil => simp
  | cons vmId rest ih =>
      simp [List.all_cons, vmStoppedOrReclaimedCheck_spec, ih]

def slotInactiveCheck (world : World) (slotId : NetSlotId) : Bool :=
  match world.netSlots slotId with
  | none => false
  | some slot => decide (slot.active = false)

def indexedSlotsClosedCheck (world : World) (sid : SandboxId) : Bool :=
  (world.resourceIndexes sid).slots.all (slotInactiveCheck world)

def IndexedSlotsClosed (world : World) (sid : SandboxId) : Prop :=
  ∀ slotId, slotId ∈ (world.resourceIndexes sid).slots →
    ∃ slot, world.netSlots slotId = some slot ∧ slot.active = false

theorem slotInactiveCheck_spec :
    slotInactiveCheck world slotId = true ↔
      ∃ slot, world.netSlots slotId = some slot ∧ slot.active = false := by
  unfold slotInactiveCheck
  cases found : world.netSlots slotId <;> simp

theorem indexedSlotsClosedCheck_spec :
    indexedSlotsClosedCheck world sid = true ↔ IndexedSlotsClosed world sid := by
  unfold indexedSlotsClosedCheck IndexedSlotsClosed
  induction (world.resourceIndexes sid).slots with
  | nil => simp
  | cons slotId rest ih =>
      simp [List.all_cons, slotInactiveCheck_spec, ih]

theorem indexedVmsClosed_iff_allVmsStoppedOrReclaimed
    (sound : VmIndexSound world) (complete : VmIndexComplete world) :
    IndexedVmsClosed world sid ↔ allVmsStoppedOrReclaimed world sid := by
  constructor
  · intro indexed vmId vm found owned
    have member := complete sid vmId vm found owned
    rcases indexed vmId member with ⟨indexedVm, indexedFound, closed⟩
    have same : indexedVm = vm := Option.some.inj (indexedFound.symm.trans found)
    simpa [same] using closed
  · intro global vmId member
    rcases sound sid vmId member with ⟨vm, found, owned⟩
    exact ⟨vm, found, global vmId vm found owned⟩

theorem indexedSlotsClosed_iff_noActiveSlot
    (sound : SlotIndexSound world) (complete : SlotIndexComplete world) :
    IndexedSlotsClosed world sid ↔ noActiveSlot world sid := by
  constructor
  · intro indexed slotId slot found owned
    have member := complete sid slotId slot found owned
    rcases indexed slotId member with ⟨indexedSlot, indexedFound, inactive⟩
    have same : indexedSlot = slot := Option.some.inj (indexedFound.symm.trans found)
    simpa [same] using inactive
  · intro global slotId member
    rcases sound sid slotId member with ⟨slot, found, owned⟩
    exact ⟨slot, found, global slotId slot found owned⟩

theorem indexedVmsClosedCheck_global_spec
    (valid : ResourceIndexValid world) :
    indexedVmsClosedCheck world sid = true ↔
      allVmsStoppedOrReclaimed world sid := by
  rw [indexedVmsClosedCheck_spec]
  exact indexedVmsClosed_iff_allVmsStoppedOrReclaimed valid.1 valid.2.1

theorem indexedSlotsClosedCheck_global_spec
    (valid : ResourceIndexValid world) :
    indexedSlotsClosedCheck world sid = true ↔ noActiveSlot world sid := by
  rw [indexedSlotsClosedCheck_spec]
  exact indexedSlotsClosed_iff_noActiveSlot valid.2.2.1 valid.2.2.2.1

def effectSucceededCheck (world : World) (command : Command) : Bool :=
  match world.effects (currentEffectId world command) with
  | some (.acknowledged (.succeeded _)) => true
  | _ => false

theorem effectSucceededCheck_spec :
    effectSucceededCheck world command = true ↔ effectSucceeded world command := by
  unfold effectSucceededCheck effectSucceeded effectHasStatus
  cases found : world.effects (currentEffectId world command) with
  | none => simp
  | some status =>
      cases status with
      | committed => simp
      | dispatched => simp
      | outcomeUnknown => simp
      | acknowledged outcome => cases outcome <;> simp

def plannedStopsSucceededCheck (world : World) (sid : SandboxId)
    (plan : OperationPlan) : Bool :=
  plan.stopVms.all fun vmId =>
    match world.vms vmId with
    | none => false
    | some vm => effectSucceededCheck world
        (.stopVm sid plan.epoch vm.generation vmId)

def PlannedStopsSucceeded (world : World) (sid : SandboxId)
    (plan : OperationPlan) : Prop :=
  ∀ vmId, vmId ∈ plan.stopVms →
    ∃ vm, world.vms vmId = some vm ∧
      effectSucceeded world (.stopVm sid plan.epoch vm.generation vmId)

theorem plannedStopsSucceededCheck_spec :
    plannedStopsSucceededCheck world sid plan = true ↔
      PlannedStopsSucceeded world sid plan := by
  unfold plannedStopsSucceededCheck PlannedStopsSucceeded
  induction plan.stopVms with
  | nil => simp
  | cons vmId rest ih =>
      cases found : world.vms vmId <;>
        simp [List.all_cons, found, effectSucceededCheck_spec, ih]

theorem resourceIndexValid_emptyWorld : ResourceIndexValid emptyWorld := by
  simp [ResourceIndexValid, VmIndexSound, VmIndexComplete,
    SlotIndexSound, SlotIndexComplete, SnapshotIndexSound,
    SnapshotIndexComplete, emptyWorld, ResourceIndex.empty]

@[simp] theorem mem_insertUnique [DecidableEq α] (candidate value : α)
    (values : List α) :
    candidate ∈ insertUnique value values ↔
      candidate = value ∨ candidate ∈ values := by
  by_cases present : value ∈ values
  · rw [show insertUnique value values = values by simp [insertUnique, present]]
    constructor
    · exact Or.inr
    · intro member
      rcases member with rfl | member
      · exact present
      · exact member
  · simp [insertUnique, present]

@[simp] theorem setVm_index_slots :
    ((setVm world vmId vm).resourceIndexes sid).slots =
      (world.resourceIndexes sid).slots := by
  by_cases owner : sid = vm.sandbox <;>
    simp [setVm, registerVm, setResourceIndex, owner]

@[simp] theorem setVm_index_snapshots :
    ((setVm world vmId vm).resourceIndexes sid).snapshots =
      (world.resourceIndexes sid).snapshots := by
  by_cases owner : sid = vm.sandbox <;>
    simp [setVm, registerVm, setResourceIndex, owner]

/-- Updating a VM is index-safe when the identifier is fresh or already belongs
    to the same sandbox.  Every VM-writing protocol step establishes exactly
    this local condition. -/
theorem resourceIndexValid_setVm
    (valid : ResourceIndexValid world)
    (compatible : ∀ old, world.vms vmId = some old →
      old.sandbox = vm.sandbox) :
    ResourceIndexValid (setVm world vmId vm) := by
  rcases valid with ⟨vmSound, vmComplete, slotSound, slotComplete,
    snapshotSound, snapshotComplete⟩
  constructor
  · intro sid candidate member
    by_cases owner : sid = vm.sandbox
    · subst sid
      simp only [setVm, registerVm, setResourceIndex] at member
      simp only [if_pos, mem_insertUnique] at member
      rcases member with rfl | oldMember
      · exact ⟨vm, by simp [setVm, registerVm, setResourceIndex, put], rfl⟩
      · by_cases same : candidate = vmId
        · subst candidate
          exact ⟨vm, by simp [setVm, registerVm, setResourceIndex, put], rfl⟩
        · rcases vmSound vm.sandbox candidate oldMember with
            ⟨old, oldFound, oldOwner⟩
          exact ⟨old, by simp [setVm, registerVm, setResourceIndex, put,
            same, oldFound], oldOwner⟩
    · have unchanged :
          (setVm world vmId vm).resourceIndexes sid =
            world.resourceIndexes sid := by
          simp [setVm, registerVm, setResourceIndex, owner]
      rw [unchanged] at member
      rcases vmSound sid candidate member with ⟨old, oldFound, oldOwner⟩
      by_cases same : candidate = vmId
      · subst candidate
        have rebound := compatible old oldFound
        exact False.elim (owner (oldOwner.symm.trans rebound))
      · exact ⟨old, by simp [setVm, registerVm, setResourceIndex, put,
          same, oldFound], oldOwner⟩
  · constructor
    · intro sid candidate candidateVm found owned
      by_cases same : candidate = vmId
      · subst candidate
        have candidateEq : vm = candidateVm := by
          simpa [setVm, registerVm, setResourceIndex, put] using found
        subst candidateVm
        have owner : sid = vm.sandbox := owned.symm
        subst sid
        simp [setVm, registerVm, setResourceIndex]
      · have oldFound : world.vms candidate = some candidateVm := by
          simpa [setVm, registerVm, setResourceIndex, put, same] using found
        have oldMember := vmComplete sid candidate candidateVm oldFound owned
        by_cases owner : sid = vm.sandbox
        · have moved : candidate ∈
              (world.resourceIndexes vm.sandbox).vms := by
            simpa [owner] using oldMember
          simp [setVm, registerVm, setResourceIndex, owner,
            mem_insertUnique, moved]
        · simpa [setVm, registerVm, setResourceIndex, owner] using oldMember
    · constructor
      · intro sid slotId member
        have oldMember : slotId ∈ (world.resourceIndexes sid).slots := by
          simpa only [setVm_index_slots] using member
        rcases slotSound sid slotId oldMember with ⟨slot, found, owned⟩
        exact ⟨slot, found, owned⟩
      · constructor
        · intro sid slotId slot found owned
          have oldFound : world.netSlots slotId = some slot := by
            exact found
          have oldMember := slotComplete sid slotId slot oldFound owned
          simpa only [setVm_index_slots] using oldMember
        · constructor
          · intro sid snapshotId member
            have oldMember : snapshotId ∈
                (world.resourceIndexes sid).snapshots := by
              simpa only [setVm_index_snapshots] using member
            rcases snapshotSound sid snapshotId oldMember with
              ⟨snapshot, found, owned⟩
            exact ⟨snapshot,
              found,
              owned⟩
          · intro sid snapshotId snapshot found owned
            have oldFound : world.snapshots snapshotId = some snapshot := by
              exact found
            have oldMember := snapshotComplete sid snapshotId snapshot oldFound owned
            simpa only [setVm_index_snapshots] using oldMember

@[simp] theorem setNetSlot_index_vms :
    ((setNetSlot world slotId slot).resourceIndexes sid).vms =
      (world.resourceIndexes sid).vms := by
  by_cases owner : sid = slot.sandbox <;>
    simp [setNetSlot, registerSlot, setResourceIndex, owner]

@[simp] theorem setNetSlot_index_snapshots :
    ((setNetSlot world slotId slot).resourceIndexes sid).snapshots =
      (world.resourceIndexes sid).snapshots := by
  by_cases owner : sid = slot.sandbox <;>
    simp [setNetSlot, registerSlot, setResourceIndex, owner]

/-- Updating a network slot preserves the finite index when its identifier is
    fresh or continues to name a slot owned by the same sandbox. -/
theorem resourceIndexValid_setNetSlot
    (valid : ResourceIndexValid world)
    (compatible : ∀ old, world.netSlots slotId = some old →
      old.sandbox = slot.sandbox) :
    ResourceIndexValid (setNetSlot world slotId slot) := by
  rcases valid with ⟨vmSound, vmComplete, slotSound, slotComplete,
    snapshotSound, snapshotComplete⟩
  constructor
  · intro sid candidate member
    have oldMember : candidate ∈ (world.resourceIndexes sid).vms := by
      simpa only [setNetSlot_index_vms] using member
    rcases vmSound sid candidate oldMember with ⟨candidateVm, found, owned⟩
    exact ⟨candidateVm, found, owned⟩
  · constructor
    · intro sid candidate candidateVm found owned
      have oldFound : world.vms candidate = some candidateVm := found
      have oldMember := vmComplete sid candidate candidateVm oldFound owned
      simpa only [setNetSlot_index_vms] using oldMember
    · constructor
      · intro sid candidate member
        by_cases owner : sid = slot.sandbox
        · subst sid
          simp only [setNetSlot, registerSlot, setResourceIndex] at member
          simp only [if_pos, mem_insertUnique] at member
          rcases member with rfl | oldMember
          · exact ⟨slot,
              by simp [setNetSlot, registerSlot, setResourceIndex, put], rfl⟩
          · by_cases same : candidate = slotId
            · subst candidate
              exact ⟨slot,
                by simp [setNetSlot, registerSlot, setResourceIndex, put], rfl⟩
            · rcases slotSound slot.sandbox candidate oldMember with
                ⟨old, oldFound, oldOwner⟩
              exact ⟨old,
                by simp [setNetSlot, registerSlot, setResourceIndex, put,
                  same, oldFound],
                oldOwner⟩
        · have unchanged :
              (setNetSlot world slotId slot).resourceIndexes sid =
                world.resourceIndexes sid := by
              simp [setNetSlot, registerSlot, setResourceIndex, owner]
          rw [unchanged] at member
          rcases slotSound sid candidate member with ⟨old, oldFound, oldOwner⟩
          by_cases same : candidate = slotId
          · subst candidate
            have rebound := compatible old oldFound
            exact False.elim (owner (oldOwner.symm.trans rebound))
          · exact ⟨old,
              by simp [setNetSlot, registerSlot, setResourceIndex, put,
                same, oldFound],
              oldOwner⟩
      · constructor
        · intro sid candidate candidateSlot found owned
          by_cases same : candidate = slotId
          · subst candidate
            have candidateEq : slot = candidateSlot := by
              simpa [setNetSlot, registerSlot, setResourceIndex, put] using found
            subst candidateSlot
            have owner : sid = slot.sandbox := owned.symm
            subst sid
            simp [setNetSlot, registerSlot, setResourceIndex]
          · have oldFound : world.netSlots candidate = some candidateSlot := by
              simpa [setNetSlot, registerSlot, setResourceIndex, put, same]
                using found
            have oldMember :=
              slotComplete sid candidate candidateSlot oldFound owned
            by_cases owner : sid = slot.sandbox
            · have moved : candidate ∈
                  (world.resourceIndexes slot.sandbox).slots := by
                simpa [owner] using oldMember
              simp [setNetSlot, registerSlot, setResourceIndex, owner,
                mem_insertUnique, moved]
            · simpa [setNetSlot, registerSlot, setResourceIndex, owner]
                using oldMember
        · constructor
          · intro sid snapshotId member
            have oldMember : snapshotId ∈
                (world.resourceIndexes sid).snapshots := by
              simpa only [setNetSlot_index_snapshots] using member
            rcases snapshotSound sid snapshotId oldMember with
              ⟨snapshot, found, owned⟩
            exact ⟨snapshot, found, owned⟩
          · intro sid snapshotId snapshot found owned
            have oldFound : world.snapshots snapshotId = some snapshot := found
            have oldMember :=
              snapshotComplete sid snapshotId snapshot oldFound owned
            simpa only [setNetSlot_index_snapshots] using oldMember

@[simp] theorem setSnapshot_index_vms :
    ((setSnapshot world snapshotId snapshot).resourceIndexes sid).vms =
      (world.resourceIndexes sid).vms := by
  by_cases owner : sid = snapshot.sandbox <;>
    simp [setSnapshot, registerSnapshot, setResourceIndex, owner]

@[simp] theorem setSnapshot_index_slots :
    ((setSnapshot world snapshotId snapshot).resourceIndexes sid).slots =
      (world.resourceIndexes sid).slots := by
  by_cases owner : sid = snapshot.sandbox <;>
    simp [setSnapshot, registerSnapshot, setResourceIndex, owner]

/-- Updating a snapshot preserves the finite index when its identifier is
    fresh or continues to name a snapshot owned by the same sandbox. -/
theorem resourceIndexValid_setSnapshot
    (valid : ResourceIndexValid world)
    (compatible : ∀ old, world.snapshots snapshotId = some old →
      old.sandbox = snapshot.sandbox) :
    ResourceIndexValid (setSnapshot world snapshotId snapshot) := by
  rcases valid with ⟨vmSound, vmComplete, slotSound, slotComplete,
    snapshotSound, snapshotComplete⟩
  constructor
  · intro sid vmId member
    have oldMember : vmId ∈ (world.resourceIndexes sid).vms := by
      simpa only [setSnapshot_index_vms] using member
    rcases vmSound sid vmId oldMember with ⟨vm, found, owned⟩
    exact ⟨vm, found, owned⟩
  · constructor
    · intro sid vmId vm found owned
      have oldFound : world.vms vmId = some vm := found
      have oldMember := vmComplete sid vmId vm oldFound owned
      simpa only [setSnapshot_index_vms] using oldMember
    · constructor
      · intro sid slotId member
        have oldMember : slotId ∈ (world.resourceIndexes sid).slots := by
          simpa only [setSnapshot_index_slots] using member
        rcases slotSound sid slotId oldMember with ⟨slot, found, owned⟩
        exact ⟨slot, found, owned⟩
      · constructor
        · intro sid slotId slot found owned
          have oldFound : world.netSlots slotId = some slot := found
          have oldMember := slotComplete sid slotId slot oldFound owned
          simpa only [setSnapshot_index_slots] using oldMember
        · constructor
          · intro sid candidate member
            by_cases owner : sid = snapshot.sandbox
            · subst sid
              simp only [setSnapshot, registerSnapshot, setResourceIndex] at member
              simp only [if_pos, mem_insertUnique] at member
              rcases member with rfl | oldMember
              · exact ⟨snapshot,
                  by simp [setSnapshot, registerSnapshot, setResourceIndex, put],
                  rfl⟩
              · by_cases same : candidate = snapshotId
                · subst candidate
                  exact ⟨snapshot,
                    by simp [setSnapshot, registerSnapshot, setResourceIndex,
                      put],
                    rfl⟩
                · rcases snapshotSound snapshot.sandbox candidate oldMember with
                    ⟨old, oldFound, oldOwner⟩
                  exact ⟨old,
                    by simp [setSnapshot, registerSnapshot, setResourceIndex,
                      put, same, oldFound],
                    oldOwner⟩
            · have unchanged :
                  (setSnapshot world snapshotId snapshot).resourceIndexes sid =
                    world.resourceIndexes sid := by
                  simp [setSnapshot, registerSnapshot, setResourceIndex, owner]
              rw [unchanged] at member
              rcases snapshotSound sid candidate member with
                ⟨old, oldFound, oldOwner⟩
              by_cases same : candidate = snapshotId
              · subst candidate
                have rebound := compatible old oldFound
                exact False.elim (owner (oldOwner.symm.trans rebound))
              · exact ⟨old,
                  by simp [setSnapshot, registerSnapshot, setResourceIndex,
                    put, same, oldFound],
                  oldOwner⟩
          · intro sid candidate candidateSnapshot found owned
            by_cases same : candidate = snapshotId
            · subst candidate
              have candidateEq : snapshot = candidateSnapshot := by
                simpa [setSnapshot, registerSnapshot, setResourceIndex, put]
                  using found
              subst candidateSnapshot
              have owner : sid = snapshot.sandbox := owned.symm
              subst sid
              simp [setSnapshot, registerSnapshot, setResourceIndex]
            · have oldFound :
                  world.snapshots candidate = some candidateSnapshot := by
                simpa [setSnapshot, registerSnapshot, setResourceIndex, put,
                  same] using found
              have oldMember := snapshotComplete sid candidate
                candidateSnapshot oldFound owned
              by_cases owner : sid = snapshot.sandbox
              · have moved : candidate ∈
                    (world.resourceIndexes snapshot.sandbox).snapshots := by
                  simpa [owner] using oldMember
                simp [setSnapshot, registerSnapshot, setResourceIndex, owner,
                  mem_insertUnique, moved]
              · simpa [setSnapshot, registerSnapshot, setResourceIndex, owner]
                  using oldMember

theorem resourceIndexValid_of_resource_fields_eq
    (valid : ResourceIndexValid world)
    (vms : next.vms = world.vms)
    (slots : next.netSlots = world.netSlots)
    (snapshots : next.snapshots = world.snapshots)
    (indexes : next.resourceIndexes = world.resourceIndexes) :
    ResourceIndexValid next := by
  unfold ResourceIndexValid VmIndexSound VmIndexComplete SlotIndexSound
    SlotIndexComplete SnapshotIndexSound SnapshotIndexComplete at valid ⊢
  simpa only [vms, slots, snapshots, indexes] using valid

@[simp] theorem commitEffect_resourceIndexes :
    (commitEffect world command).resourceIndexes = world.resourceIndexes := by
  cases found : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, found, logical, setEffect, setLogicalEffect]
  · simp [commitEffect, found]

@[simp] theorem enqueue_resourceIndexes :
    (enqueue world command).resourceIndexes = world.resourceIndexes := by
  simp [enqueue]

@[simp] theorem dispatchEffect_resourceIndexes {id : EffectId} :
    (dispatchEffect world id).resourceIndexes = world.resourceIndexes := rfl

@[simp] theorem acknowledgeEffect_resourceIndexes {id : EffectId}
    {outcome : EffectOutcome} :
    (acknowledgeEffect world id outcome).resourceIndexes =
      world.resourceIndexes := rfl

@[simp] theorem markEffectOutcomeUnknown_resourceIndexes {id : EffectId} :
    (markEffectOutcomeUnknown world id).resourceIndexes =
      world.resourceIndexes := rfl

@[simp] theorem storeNewOperation_resourceIndexes :
    (storeNewOperation world sid operation input).resourceIndexes =
      world.resourceIndexes := by
  simp [storeNewOperation, setOperation]

@[simp] theorem updateOperation_resourceIndexes :
    (updateOperation world sid operation result).resourceIndexes =
      world.resourceIndexes := rfl

@[simp] theorem supersedeCurrentOperation_resourceIndexes :
    (supersedeCurrentOperation world sid current).resourceIndexes =
      world.resourceIndexes := by
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
                · simp [final, sameOperation]
                · simp [final, sameOperation]
            | some result => simp [final]

@[simp] theorem updateCurrentOperation_resourceIndexes :
    (updateCurrentOperation world sid operation result).resourceIndexes =
      world.resourceIndexes := rfl

@[simp] theorem applyStartFence_resourceIndexes :
    (applyStartFence world sid record).resourceIndexes =
      world.resourceIndexes := rfl

@[simp] theorem applyFilePolicyResult_resourceIndexes :
    (applyFilePolicyResult world sid record).resourceIndexes =
      world.resourceIndexes := rfl

theorem step_preserves_resourceIndexValid
    (valid : ResourceIndexValid world) (transition : Step world event next) :
    ResourceIndexValid next := by
  cases transition <;>
    try
      solve
        | apply resourceIndexValid_of_resource_fields_eq valid <;>
            simp [beginCreate, beginPause, beginResume, beginKill,
              SandboxModel.beginCompensation, publishPaused, publishDestroyed,
              SandboxModel.publishFailure, markOutcomeUnknown, publishRunning,
              applyStartFence, applyFilePolicyResult, setSandbox, setFilesystem,
              setAppliedFilePolicy, setFence, setOperationPlan,
              reserveStartupResources, reservePauseSnapshot]
  case installRevocationFence =>
    rename_i sid record revoking queued dispatched found monotone slotIdentity
    cases slotChoice : record.netSlot with
    | none =>
        apply resourceIndexValid_of_resource_fields_eq valid <;>
          simp [applyRevocationFence, slotChoice, setSandbox, setFence]
    | some slotId =>
        cases slotFound : world.netSlots slotId with
        | none =>
            apply resourceIndexValid_of_resource_fields_eq valid <;>
              simp [applyRevocationFence, slotChoice, slotFound, setSandbox,
                setFence]
        | some slot =>
            have slotOwner : slot.sandbox = sid := by
              rcases slotIdentity slotId slotChoice with
                ⟨expected, expectedFound, expectedOwner⟩
              have same : expected = slot :=
                Option.some.inj (expectedFound.symm.trans slotFound)
              simpa [same] using expectedOwner
            have closedValid : ResourceIndexValid
                (setFence
                  (setSandbox world sid { record with effective := .closed })
                  sid ⟨record.epoch, record.terminal⟩) := by
              apply resourceIndexValid_of_resource_fields_eq valid <;> rfl
            simp only [applyRevocationFence, slotChoice, slotFound]
            apply resourceIndexValid_setNetSlot closedValid
            intro old oldFound
            change world.netSlots slotId = some old at oldFound
            have same : old = slot :=
              Option.some.inj (oldFound.symm.trans slotFound)
            simpa [same] using slotOwner
  case prepareVm =>
    rename_i sid vmId slotId source record operation plan starting live
      currentOperation planKey planEpoch planGeneration planSealed plannedVm
      plannedSlot plannedSource selectedSource queued dispatched found foundPlan
      fenced freshVm freshSlot resumeSource
    have vmValid : ResourceIndexValid
        (setVm world vmId
          ⟨record.owner, sid, record.generation, .prepared⟩) := by
      apply resourceIndexValid_setVm valid
      intro old oldFound
      rw [freshVm] at oldFound
      contradiction
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
    have slotValid : ResourceIndexValid
        (setNetSlot
          (setVm world vmId
            ⟨record.owner, sid, record.generation, .prepared⟩)
          slotId slot) := by
      apply resourceIndexValid_setNetSlot vmValid
      intro old oldFound
      change world.netSlots slotId = some old at oldFound
      rw [freshSlot] at oldFound
      contradiction
    apply resourceIndexValid_of_resource_fields_eq slotValid <;>
      simp [prepareResources, slot, setSandbox]
  case configureNetwork =>
    rename_i sid vmId slotId record slot starting live current currentVm owned
      queued dispatched foundSandbox foundSlot fenced
    apply resourceIndexValid_setNetSlot valid
    intro old oldFound
    have same : old = slot := Option.some.inj (oldFound.symm.trans foundSlot)
    simpa [configureSlot, same]
  case activateVm =>
    rename_i sid vmId record vm starting live current ownedVm prepared queued
      dispatched foundSandbox foundVm foundSlot fenced exclusive
    apply resourceIndexValid_setVm valid
    intro old oldFound
    have same : old = vm := Option.some.inj (oldFound.symm.trans foundVm)
    simpa [activateResource, same]
  case publishRunning =>
    rename_i record vmId vm sid slot slotId operation ledger starting live
      currentVm vmIdentity executing currentSlot slotReady currentOperation
      operationMatches up foundSandbox foundVm foundSlot exclusiveNetwork
      appliedPolicy fenceSucceeded prepareSucceeded networkSucceeded
      activationSucceeded policyAcknowledged persistedOperation fenced
    have slotValid : ResourceIndexValid
        (setNetSlot world slotId { slot with active := true }) := by
      apply resourceIndexValid_setNetSlot valid
      intro old oldFound
      have same : old = slot := Option.some.inj (oldFound.symm.trans foundSlot)
      simpa [same]
    apply resourceIndexValid_of_resource_fields_eq slotValid <;>
      simp [publishRunning, setSandbox]
  case freezeVm =>
    rename_i sid vmId record vm pausing current vmIdentity executing queued
      dispatched foundSandbox foundVm fenced
    apply resourceIndexValid_setVm valid
    intro old oldFound
    have same : old = vm := Option.some.inj (oldFound.symm.trans foundVm)
    simpa [same]
  case sealSnapshot =>
    rename_i sid vmId snapshotId manifest record operation plan vm pausing
      currentOperation planKey planEpoch planGeneration planSealed plannedVm
      plannedSnapshot current vmIdentity frozen queued dispatched foundSandbox
      foundPlan foundVm fenced fresh
    apply resourceIndexValid_setSnapshot valid
    intro old oldFound
    rw [fresh] at oldFound
    contradiction
  case commitSnapshot =>
    rename_i sid snapshotId snapshot record vmId snapshotSandbox pausing current
      snapshotIdentity sealed queued dispatched found foundSandbox
    have snapshotValid : ResourceIndexValid
        (makeSnapshotDurable world snapshotId snapshot) := by
      apply resourceIndexValid_setSnapshot valid
      intro old oldFound
      have same : old = snapshot :=
        Option.some.inj (oldFound.symm.trans found)
      simpa [makeSnapshotDurable, same]
    apply resourceIndexValid_of_resource_fields_eq snapshotValid <;> simp
  case stopVm =>
    rename_i sid vmId record vm owned stoppable queued dispatched foundSandbox
      found authorizedStop
    apply resourceIndexValid_setVm valid
    intro old oldFound
    have same : old = vm := Option.some.inj (oldFound.symm.trans found)
    simpa [stopResource, same]
  case cleanupVm =>
    rename_i sid generation vmId vm identity stopped queued dispatched foundVm
      notCurrent
    apply resourceIndexValid_setVm valid
    intro old oldFound
    have same : old = vm := Option.some.inj (oldFound.symm.trans foundVm)
    simpa [reclaimVm, same]
  case cleanupSlot =>
    rename_i sid generation slotId slot identity inactive notReclaimed queued
      dispatched foundSlot notCurrent
    apply resourceIndexValid_setNetSlot valid
    intro old oldFound
    have same : old = slot := Option.some.inj (oldFound.symm.trans foundSlot)
    simpa [reclaimSlot, same]

theorem reachable_resourceIndexValid (reachable : Reachable world) :
    ResourceIndexValid world := by
  induction reachable with
  | initial => exact resourceIndexValid_emptyWorld
  | step _ transition inherited =>
      exact step_preserves_resourceIndexValid inherited transition

end SandboxModel
