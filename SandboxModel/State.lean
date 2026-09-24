import SandboxModel.Types

/-! Persistent protocol state and resource registries. -/

namespace SandboxModel

structure SandboxRecord where
  owner : TenantId
  phase : Phase
  terminal : Bool
  epoch : Epoch
  generation : Generation
  currentVm : Option VmId
  workspace : FsId
  workspaceAccess : WorkspaceApiAccess
  filePolicy : FilePolicyRef
  durableSnapshot : Option RestoreSource
  /-- Snapshot selected atomically by AcceptResume; later commands cannot swap it. -/
  restoreFrom : Option RestoreSource
  netSlot : Option NetSlotId
  policy : NetworkPolicy
  desired : Authority
  effective : Authority
  currentOperation : Option Operation

structure Vm where
  owner : TenantId
  sandbox : SandboxId
  generation : Generation
  status : VmStatus
  deriving Repr

structure Filesystem where
  owner : TenantId
  sandbox : SandboxId
  createdGeneration : Generation
  deriving Repr

structure Snapshot where
  owner : TenantId
  sandbox : SandboxId
  sourceGeneration : Generation
  workspace : FsId
  manifest : SnapshotManifestRef
  stage : SnapshotStage
  immutable : Bool
  deriving Repr

structure NetSlot where
  owner : TenantId
  sandbox : SandboxId
  generation : Generation
  vm : VmId
  policy : NetworkPolicy
  configured : Bool
  active : Bool
  reclaimed : Bool
  deriving Repr

structure AppliedFilePolicy where
  sandbox : SandboxId
  generation : Generation
  workspace : FsId
  policy : FilePolicyRef
  deriving Repr

structure WorkerFence where
  epoch : Epoch
  terminal : Bool
  deriving Repr

structure World where
  sandboxes : SandboxId → Option SandboxRecord
  vms : VmId → Option Vm
  filesystems : FsId → Option Filesystem
  snapshots : SnapshotId → Option Snapshot
  netSlots : NetSlotId → Option NetSlot
  appliedFilePolicies : SandboxId → Option AppliedFilePolicy
  workerFences : SandboxId → Option WorkerFence
  operations : OperationKey → Option OperationLedgerRecord
  operationPlans : OperationKey → Option OperationPlan
  resourceIndexes : SandboxId → ResourceIndex
  logicalEffects : LogicalEffectId → Option LogicalEffectRecord
  effectKeys : List EffectId
  effects : EffectId → Option EffectStatus
  /-- Monotone controller-owned allocation cursors.  Reservation handlers
      copy the selected identities into an operation plan before any worker
      command can refer to them. -/
  nextVmId : Nat
  nextSlotId : Nat
  nextSnapshotId : Nat
  nextManifestDigest : Nat
  /-- Durable outbox membership, keyed by the exact attempt number.  A broker
      delivery may be lost, but reconciliation can deliver an item again from
      this journaled set.  Membership alone grants no execution authority:
      the exact current attempt and its status in `effects` are rechecked. -/
  inflight : EffectId → Prop
  controllerUp : Bool

def emptyWorld : World where
  sandboxes := fun _ => none
  vms := fun _ => none
  filesystems := fun _ => none
  snapshots := fun _ => none
  netSlots := fun _ => none
  appliedFilePolicies := fun _ => none
  workerFences := fun _ => none
  operations := fun _ => none
  operationPlans := fun _ => none
  resourceIndexes := fun _ => ResourceIndex.empty
  logicalEffects := fun _ => none
  effectKeys := []
  effects := fun _ => none
  nextVmId := 0
  nextSlotId := 0
  nextSnapshotId := 0
  nextManifestDigest := 0
  inflight := fun _ => False
  controllerUp := true

def put [DecidableEq α] (store : α → Option β) (key : α) (value : β) : α → Option β :=
  fun candidate => if candidate = key then some value else store candidate

def addEffect (inflight : EffectId → Prop) (id : EffectId) : EffectId → Prop :=
  fun candidate => candidate = id ∨ inflight candidate

def insertUnique [DecidableEq α] (value : α) (values : List α) : List α :=
  if value ∈ values then values else value :: values

@[simp] def setResourceIndex (world : World) (sid : SandboxId)
    (index : ResourceIndex) : World :=
  { world with resourceIndexes := fun candidate =>
      if candidate = sid then index else world.resourceIndexes candidate }

@[simp] def registerVm (world : World) (sid : SandboxId) (vmId : VmId) : World :=
  let index := world.resourceIndexes sid
  setResourceIndex world sid { index with vms := insertUnique vmId index.vms }

@[simp] def registerSlot (world : World) (sid : SandboxId)
    (slotId : NetSlotId) : World :=
  let index := world.resourceIndexes sid
  setResourceIndex world sid { index with slots := insertUnique slotId index.slots }

@[simp] def registerSnapshot (world : World) (sid : SandboxId)
    (snapshotId : SnapshotId) : World :=
  let index := world.resourceIndexes sid
  setResourceIndex world sid {
    index with snapshots := insertUnique snapshotId index.snapshots }

def setSandbox (world : World) (sid : SandboxId) (record : SandboxRecord) : World :=
  { world with sandboxes := put world.sandboxes sid record }

def setVm (world : World) (vmId : VmId) (vm : Vm) : World :=
  let registered := registerVm world vm.sandbox vmId
  { registered with vms := put registered.vms vmId vm }

def setFilesystem (world : World) (fsId : FsId) (fs : Filesystem) : World :=
  { world with filesystems := put world.filesystems fsId fs }

def setSnapshot (world : World) (snapshotId : SnapshotId) (snapshot : Snapshot) : World :=
  let registered := registerSnapshot world snapshot.sandbox snapshotId
  { registered with snapshots := put registered.snapshots snapshotId snapshot }

def setNetSlot (world : World) (slotId : NetSlotId) (slot : NetSlot) : World :=
  let registered := registerSlot world slot.sandbox slotId
  { registered with netSlots := put registered.netSlots slotId slot }

def setAppliedFilePolicy (world : World) (sid : SandboxId)
    (policy : AppliedFilePolicy) : World :=
  { world with appliedFilePolicies := put world.appliedFilePolicies sid policy }

def setFence (world : World) (sid : SandboxId) (fence : WorkerFence) : World :=
  { world with workerFences := put world.workerFences sid fence }

def setOperation (world : World) (key : OperationKey)
    (operation : OperationLedgerRecord) : World :=
  { world with operations := put world.operations key operation }

@[simp] def setOperationPlan (world : World) (key : OperationKey)
    (plan : OperationPlan) : World :=
  { world with operationPlans := put world.operationPlans key plan }

/-! Explicit projection lemmas keep proofs stable across Lean versions.  In
    particular, callers should not need to unfold the resource-index
    registration hidden inside `setVm`, `setNetSlot`, and `setSnapshot` merely
    to learn that an unrelated store or allocation cursor is unchanged. -/

@[simp] theorem setSandbox_vms :
    (setSandbox world sid record).vms = world.vms := rfl
@[simp] theorem setSandbox_sandboxes :
    (setSandbox world sid record).sandboxes = put world.sandboxes sid record := rfl
@[simp] theorem setSandbox_filesystems :
    (setSandbox world sid record).filesystems = world.filesystems := rfl
@[simp] theorem setSandbox_snapshots :
    (setSandbox world sid record).snapshots = world.snapshots := rfl
@[simp] theorem setSandbox_netSlots :
    (setSandbox world sid record).netSlots = world.netSlots := rfl
@[simp] theorem setSandbox_resourceIndexes :
    (setSandbox world sid record).resourceIndexes = world.resourceIndexes := rfl
@[simp] theorem setSandbox_nextVmId :
    (setSandbox world sid record).nextVmId = world.nextVmId := rfl
@[simp] theorem setSandbox_nextSlotId :
    (setSandbox world sid record).nextSlotId = world.nextSlotId := rfl
@[simp] theorem setSandbox_nextSnapshotId :
    (setSandbox world sid record).nextSnapshotId = world.nextSnapshotId := rfl
@[simp] theorem setSandbox_nextManifestDigest :
    (setSandbox world sid record).nextManifestDigest =
      world.nextManifestDigest := rfl

@[simp] theorem setVm_sandboxes :
    (setVm world vmId vm).sandboxes = world.sandboxes := rfl
@[simp] theorem setVm_vms :
    (setVm world vmId vm).vms = put world.vms vmId vm := rfl
@[simp] theorem setVm_filesystems :
    (setVm world vmId vm).filesystems = world.filesystems := rfl
@[simp] theorem setVm_snapshots :
    (setVm world vmId vm).snapshots = world.snapshots := rfl
@[simp] theorem setVm_netSlots :
    (setVm world vmId vm).netSlots = world.netSlots := rfl
@[simp] theorem setVm_nextVmId :
    (setVm world vmId vm).nextVmId = world.nextVmId := rfl
@[simp] theorem setVm_nextSlotId :
    (setVm world vmId vm).nextSlotId = world.nextSlotId := rfl
@[simp] theorem setVm_nextSnapshotId :
    (setVm world vmId vm).nextSnapshotId = world.nextSnapshotId := rfl
@[simp] theorem setVm_nextManifestDigest :
    (setVm world vmId vm).nextManifestDigest = world.nextManifestDigest := rfl

@[simp] theorem setFilesystem_sandboxes :
    (setFilesystem world fsId filesystem).sandboxes = world.sandboxes := rfl
@[simp] theorem setFilesystem_filesystems :
    (setFilesystem world fsId filesystem).filesystems =
      put world.filesystems fsId filesystem := rfl
@[simp] theorem setFilesystem_vms :
    (setFilesystem world fsId filesystem).vms = world.vms := rfl
@[simp] theorem setFilesystem_snapshots :
    (setFilesystem world fsId filesystem).snapshots = world.snapshots := rfl
@[simp] theorem setFilesystem_netSlots :
    (setFilesystem world fsId filesystem).netSlots = world.netSlots := rfl
@[simp] theorem setFilesystem_resourceIndexes :
    (setFilesystem world fsId filesystem).resourceIndexes =
      world.resourceIndexes := rfl
@[simp] theorem setFilesystem_nextVmId :
    (setFilesystem world fsId filesystem).nextVmId = world.nextVmId := rfl
@[simp] theorem setFilesystem_nextSlotId :
    (setFilesystem world fsId filesystem).nextSlotId = world.nextSlotId := rfl
@[simp] theorem setFilesystem_nextSnapshotId :
    (setFilesystem world fsId filesystem).nextSnapshotId =
      world.nextSnapshotId := rfl
@[simp] theorem setFilesystem_nextManifestDigest :
    (setFilesystem world fsId filesystem).nextManifestDigest =
      world.nextManifestDigest := rfl

@[simp] theorem setSnapshot_sandboxes :
    (setSnapshot world snapshotId snapshot).sandboxes = world.sandboxes := rfl
@[simp] theorem setSnapshot_snapshots :
    (setSnapshot world snapshotId snapshot).snapshots =
      put world.snapshots snapshotId snapshot := rfl
@[simp] theorem setSnapshot_vms :
    (setSnapshot world snapshotId snapshot).vms = world.vms := rfl
@[simp] theorem setSnapshot_filesystems :
    (setSnapshot world snapshotId snapshot).filesystems = world.filesystems := rfl
@[simp] theorem setSnapshot_netSlots :
    (setSnapshot world snapshotId snapshot).netSlots = world.netSlots := rfl
@[simp] theorem setSnapshot_nextVmId :
    (setSnapshot world snapshotId snapshot).nextVmId = world.nextVmId := rfl
@[simp] theorem setSnapshot_nextSlotId :
    (setSnapshot world snapshotId snapshot).nextSlotId = world.nextSlotId := rfl
@[simp] theorem setSnapshot_nextSnapshotId :
    (setSnapshot world snapshotId snapshot).nextSnapshotId =
      world.nextSnapshotId := rfl
@[simp] theorem setSnapshot_nextManifestDigest :
    (setSnapshot world snapshotId snapshot).nextManifestDigest =
      world.nextManifestDigest := rfl

@[simp] theorem setNetSlot_sandboxes :
    (setNetSlot world slotId slot).sandboxes = world.sandboxes := rfl
@[simp] theorem setNetSlot_netSlots :
    (setNetSlot world slotId slot).netSlots = put world.netSlots slotId slot := rfl
@[simp] theorem setNetSlot_vms :
    (setNetSlot world slotId slot).vms = world.vms := rfl
@[simp] theorem setNetSlot_filesystems :
    (setNetSlot world slotId slot).filesystems = world.filesystems := rfl
@[simp] theorem setNetSlot_snapshots :
    (setNetSlot world slotId slot).snapshots = world.snapshots := rfl
@[simp] theorem setNetSlot_nextVmId :
    (setNetSlot world slotId slot).nextVmId = world.nextVmId := rfl
@[simp] theorem setNetSlot_nextSlotId :
    (setNetSlot world slotId slot).nextSlotId = world.nextSlotId := rfl
@[simp] theorem setNetSlot_nextSnapshotId :
    (setNetSlot world slotId slot).nextSnapshotId = world.nextSnapshotId := rfl
@[simp] theorem setNetSlot_nextManifestDigest :
    (setNetSlot world slotId slot).nextManifestDigest =
      world.nextManifestDigest := rfl

/-- Atomically reserve startup identities in the operation plan and advance
    both allocation cursors. -/
def reserveStartupResources (world : World) (key : OperationKey)
    (plan : OperationPlan) : World :=
  let vm : VmId := ⟨world.nextVmId⟩
  let slot : NetSlotId := ⟨world.nextSlotId⟩
  setOperationPlan
    { world with
      nextVmId := world.nextVmId + 1
      nextSlotId := world.nextSlotId + 1 }
    key { plan with
      vm := some vm
      slot := some slot
      stopVms := insertUnique vm plan.stopVms
      closeSlots := insertUnique slot plan.closeSlots
      sealed := true }

/-- Atomically reserve a snapshot/manifest pair in the pause plan and record
    the VM whose frozen state will be sealed. -/
def reservePauseSnapshot (world : World) (key : OperationKey)
    (plan : OperationPlan) (vm : VmId) : World :=
  let snapshot : SnapshotId := ⟨world.nextSnapshotId⟩
  let manifest : SnapshotManifestRef := ⟨⟨world.nextManifestDigest⟩⟩
  setOperationPlan
    { world with
      nextSnapshotId := world.nextSnapshotId + 1
      nextManifestDigest := world.nextManifestDigest + 1 }
    key { plan with
      vm := some vm
      targetSnapshot := some ⟨snapshot, manifest⟩
      sealed := true }

@[simp] theorem reserveStartupResources_sandboxes :
    (reserveStartupResources world key plan).sandboxes = world.sandboxes := rfl
@[simp] theorem reserveStartupResources_vms :
    (reserveStartupResources world key plan).vms = world.vms := rfl
@[simp] theorem reserveStartupResources_filesystems :
    (reserveStartupResources world key plan).filesystems = world.filesystems := rfl
@[simp] theorem reserveStartupResources_snapshots :
    (reserveStartupResources world key plan).snapshots = world.snapshots := rfl
@[simp] theorem reserveStartupResources_netSlots :
    (reserveStartupResources world key plan).netSlots = world.netSlots := rfl
@[simp] theorem reserveStartupResources_appliedFilePolicies :
    (reserveStartupResources world key plan).appliedFilePolicies =
      world.appliedFilePolicies := rfl
@[simp] theorem reserveStartupResources_workerFences :
    (reserveStartupResources world key plan).workerFences = world.workerFences := rfl
@[simp] theorem reserveStartupResources_operations :
    (reserveStartupResources world key plan).operations = world.operations := rfl
@[simp] theorem reserveStartupResources_resourceIndexes :
    (reserveStartupResources world key plan).resourceIndexes =
      world.resourceIndexes := rfl
@[simp] theorem reserveStartupResources_logicalEffects :
    (reserveStartupResources world key plan).logicalEffects = world.logicalEffects := rfl
@[simp] theorem reserveStartupResources_effectKeys :
    (reserveStartupResources world key plan).effectKeys = world.effectKeys := rfl
@[simp] theorem reserveStartupResources_effects :
    (reserveStartupResources world key plan).effects = world.effects := rfl
@[simp] theorem reserveStartupResources_inflight :
    (reserveStartupResources world key plan).inflight = world.inflight := rfl
@[simp] theorem reserveStartupResources_controllerUp :
    (reserveStartupResources world key plan).controllerUp = world.controllerUp := rfl

@[simp] theorem reservePauseSnapshot_sandboxes :
    (reservePauseSnapshot world key plan vm).sandboxes = world.sandboxes := rfl
@[simp] theorem reservePauseSnapshot_vms :
    (reservePauseSnapshot world key plan vm).vms = world.vms := rfl
@[simp] theorem reservePauseSnapshot_filesystems :
    (reservePauseSnapshot world key plan vm).filesystems = world.filesystems := rfl
@[simp] theorem reservePauseSnapshot_snapshots :
    (reservePauseSnapshot world key plan vm).snapshots = world.snapshots := rfl
@[simp] theorem reservePauseSnapshot_netSlots :
    (reservePauseSnapshot world key plan vm).netSlots = world.netSlots := rfl
@[simp] theorem reservePauseSnapshot_appliedFilePolicies :
    (reservePauseSnapshot world key plan vm).appliedFilePolicies =
      world.appliedFilePolicies := rfl
@[simp] theorem reservePauseSnapshot_workerFences :
    (reservePauseSnapshot world key plan vm).workerFences = world.workerFences := rfl
@[simp] theorem reservePauseSnapshot_operations :
    (reservePauseSnapshot world key plan vm).operations = world.operations := rfl
@[simp] theorem reservePauseSnapshot_resourceIndexes :
    (reservePauseSnapshot world key plan vm).resourceIndexes =
      world.resourceIndexes := rfl
@[simp] theorem reservePauseSnapshot_logicalEffects :
    (reservePauseSnapshot world key plan vm).logicalEffects = world.logicalEffects := rfl
@[simp] theorem reservePauseSnapshot_effectKeys :
    (reservePauseSnapshot world key plan vm).effectKeys = world.effectKeys := rfl
@[simp] theorem reservePauseSnapshot_effects :
    (reservePauseSnapshot world key plan vm).effects = world.effects := rfl
@[simp] theorem reservePauseSnapshot_inflight :
    (reservePauseSnapshot world key plan vm).inflight = world.inflight := rfl
@[simp] theorem reservePauseSnapshot_controllerUp :
    (reservePauseSnapshot world key plan vm).controllerUp = world.controllerUp := rfl

@[simp] theorem setOperationPlan_operations :
    (setOperationPlan world key plan).operations = world.operations := rfl

@[simp] theorem setOperationPlan_effects :
    (setOperationPlan world key plan).effects = world.effects := rfl

@[simp] def setLogicalEffect (world : World) (id : LogicalEffectId)
    (record : LogicalEffectRecord) : World :=
  { world with logicalEffects := put world.logicalEffects id record }

@[simp] theorem setSandbox_operations :
    (setSandbox world sid record).operations = world.operations := rfl

@[simp] theorem setVm_operations :
    (setVm world vmId vm).operations = world.operations := rfl

@[simp] theorem setFilesystem_operations :
    (setFilesystem world fsId filesystem).operations = world.operations := rfl

@[simp] theorem setSnapshot_operations :
    (setSnapshot world snapshotId snapshot).operations = world.operations := rfl

@[simp] theorem setNetSlot_operations :
    (setNetSlot world slotId slot).operations = world.operations := rfl

@[simp] theorem setSandbox_effects :
    (setSandbox world sid record).effects = world.effects := rfl

@[simp] theorem setVm_effects :
    (setVm world vmId vm).effects = world.effects := rfl

@[simp] theorem setFilesystem_effects :
    (setFilesystem world fsId filesystem).effects = world.effects := rfl

@[simp] theorem setSnapshot_effects :
    (setSnapshot world snapshotId snapshot).effects = world.effects := rfl

@[simp] theorem setNetSlot_effects :
    (setNetSlot world slotId slot).effects = world.effects := rfl

@[simp] theorem setAppliedFilePolicy_effects :
    (setAppliedFilePolicy world sid policy).effects = world.effects := rfl

@[simp] theorem setFence_effects :
    (setFence world sid fence).effects = world.effects := rfl

@[simp] theorem setOperation_effects :
    (setOperation world key operation).effects = world.effects := rfl

@[simp] theorem setSandbox_operationPlans :
    (setSandbox world sid record).operationPlans = world.operationPlans := rfl

@[simp] theorem setVm_operationPlans :
    (setVm world vmId vm).operationPlans = world.operationPlans := rfl

@[simp] theorem setFilesystem_operationPlans :
    (setFilesystem world fsId filesystem).operationPlans =
      world.operationPlans := rfl

@[simp] theorem setSnapshot_operationPlans :
    (setSnapshot world snapshotId snapshot).operationPlans =
      world.operationPlans := rfl

@[simp] theorem setNetSlot_operationPlans :
    (setNetSlot world slotId slot).operationPlans = world.operationPlans := rfl

@[simp] theorem setAppliedFilePolicy_operationPlans :
    (setAppliedFilePolicy world sid policy).operationPlans =
      world.operationPlans := rfl

@[simp] theorem setFence_operationPlans :
    (setFence world sid fence).operationPlans = world.operationPlans := rfl

@[simp] theorem setOperation_operationPlans :
    (setOperation world key operation).operationPlans = world.operationPlans := rfl

@[simp] theorem setLogicalEffect_operationPlans :
    (setLogicalEffect world logical record).operationPlans =
      world.operationPlans := rfl

def setEffect (world : World) (effectId : EffectId) (status : EffectStatus) : World :=
  { world with
      effects := put world.effects effectId status
      effectKeys := insertUnique effectId world.effectKeys }

@[simp] theorem setEffect_operationPlans :
    (setEffect world effectKey status).operationPlans = world.operationPlans := rfl

def logicalEffectId (command : Command) : LogicalEffectId := ⟨command⟩

def effectIdAt (command : Command) (attempt : AttemptNo) : EffectId :=
  ⟨logicalEffectId command, attempt⟩

/-- Compatibility constructor for the initial attempt. -/
def effectId (command : Command) : EffectId := effectIdAt command 0

/-- Compatibility helper used by initial scheduling sites.  Retries use
    `addEffect` directly with their nonzero exact attempt identity. -/
def addCommand (inflight : EffectId → Prop) (command : Command) : EffectId → Prop :=
  addEffect inflight (effectId command)

@[simp] theorem effectIdAt_inj (left right : Command)
    (leftAttempt rightAttempt : AttemptNo) :
    effectIdAt left leftAttempt = effectIdAt right rightAttempt ↔
      left = right ∧ leftAttempt = rightAttempt := by
  simp [effectIdAt, logicalEffectId]

@[simp] theorem effectId_inj (left right : Command) :
    effectId left = effectId right ↔ left = right := by
  simp [effectId]

@[simp] theorem EffectId.command_effectIdAt (command : Command)
    (attempt : AttemptNo) :
    (effectIdAt command attempt).command = command := rfl

def inferredEffectOrigin (world : World) (command : Command) : EffectOrigin :=
  let sid := command.sandbox
  match command.epoch?, world.sandboxes sid with
  | some epoch, some record =>
      match record.currentOperation with
      | some operation =>
          if operation.epoch = epoch then
            .operation ⟨sid, operation.id⟩
          else .maintenance sid
      | none => .maintenance sid
  | _, _ => .maintenance sid

def currentEffectId (world : World) (command : Command) : EffectId :=
  let logical := logicalEffectId command
  match world.logicalEffects logical with
  | some record => effectIdAt command record.currentAttempt
  | none => effectId command

def commitEffect (world : World) (command : Command) : World :=
  let id := effectId command
  match world.effects id with
  | none =>
      let logical := logicalEffectId command
      let committed := setEffect world id .committed
      match world.logicalEffects logical with
      | some _ => committed
      | none => setLogicalEffect committed logical {
          origin := inferredEffectOrigin world command
          currentAttempt := 0
          remainingRetries := defaultRetryLimit
          result := none
  }

  | some _ => world

@[simp] theorem reserveStartupResources_operationPlans :
    (reserveStartupResources world key plan).operationPlans =
      put world.operationPlans key
        { plan with
          vm := some ⟨world.nextVmId⟩
          slot := some ⟨world.nextSlotId⟩
          stopVms := insertUnique ⟨world.nextVmId⟩ plan.stopVms
          closeSlots := insertUnique ⟨world.nextSlotId⟩ plan.closeSlots
          sealed := true } := rfl

@[simp] theorem reservePauseSnapshot_operationPlans :
    (reservePauseSnapshot world key plan vm).operationPlans =
      put world.operationPlans key
        { plan with
          vm := some vm
          targetSnapshot := some
            ⟨⟨world.nextSnapshotId⟩,
              ⟨⟨world.nextManifestDigest⟩⟩⟩
          sealed := true } := rfl

@[simp] theorem commitEffect_sandboxes :
    (commitEffect world command).sandboxes = world.sandboxes := by
  cases h : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, h, logical, setEffect]
  · simp [commitEffect, h]

@[simp] theorem commitEffect_vms :
    (commitEffect world command).vms = world.vms := by
  cases h : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, h, logical, setEffect]
  · simp [commitEffect, h]

@[simp] theorem commitEffect_filesystems :
    (commitEffect world command).filesystems = world.filesystems := by
  cases h : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, h, logical, setEffect]
  · simp [commitEffect, h]

@[simp] theorem commitEffect_snapshots :
    (commitEffect world command).snapshots = world.snapshots := by
  cases h : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, h, logical, setEffect]
  · simp [commitEffect, h]

@[simp] theorem commitEffect_netSlots :
    (commitEffect world command).netSlots = world.netSlots := by
  cases h : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, h, logical, setEffect]
  · simp [commitEffect, h]

@[simp] theorem commitEffect_appliedFilePolicies :
    (commitEffect world command).appliedFilePolicies = world.appliedFilePolicies := by
  cases h : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, h, logical, setEffect]
  · simp [commitEffect, h]

@[simp] theorem commitEffect_workerFences :
    (commitEffect world command).workerFences = world.workerFences := by
  cases h : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, h, logical, setEffect]
  · simp [commitEffect, h]

@[simp] theorem commitEffect_operations :
    (commitEffect world command).operations = world.operations := by
  cases h : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, h, logical, setEffect]
  · simp [commitEffect, h]

@[simp] theorem commitEffect_operationPlans :
    (commitEffect world command).operationPlans = world.operationPlans := by
  cases h : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, h, logical, setEffect]
  · simp [commitEffect, h]

def enqueue (world : World) (command : Command) : World :=
  let committed := commitEffect world command
  { committed with inflight := addCommand committed.inflight command }

@[simp] theorem enqueue_sandboxes :
    (enqueue world command).sandboxes = world.sandboxes := by simp [enqueue]

@[simp] theorem enqueue_vms :
    (enqueue world command).vms = world.vms := by simp [enqueue]

@[simp] theorem enqueue_filesystems :
    (enqueue world command).filesystems = world.filesystems := by simp [enqueue]

@[simp] theorem enqueue_snapshots :
    (enqueue world command).snapshots = world.snapshots := by simp [enqueue]

@[simp] theorem enqueue_netSlots :
    (enqueue world command).netSlots = world.netSlots := by simp [enqueue]

@[simp] theorem enqueue_appliedFilePolicies :
    (enqueue world command).appliedFilePolicies = world.appliedFilePolicies := by
  simp [enqueue]

@[simp] theorem enqueue_workerFences :
    (enqueue world command).workerFences = world.workerFences := by simp [enqueue]

@[simp] theorem enqueue_operations :
    (enqueue world command).operations = world.operations := by simp [enqueue]

@[simp] theorem enqueue_operationPlans :
    (enqueue world command).operationPlans = world.operationPlans := by
  simp [enqueue]

def dispatchEffect (world : World) (id : EffectId) : World :=
  setEffect world id .dispatched

@[simp] theorem dispatchEffect_operationPlans :
    (dispatchEffect world effectKey).operationPlans = world.operationPlans := rfl

def acknowledgeEffect (world : World) (id : EffectId)
    (outcome : EffectOutcome) : World :=
  let logicalEffects :=
    match world.logicalEffects id.logical with
    | none => world.logicalEffects
    | some record =>
        if record.currentAttempt = id.attempt then
          match outcome with
          | .succeeded result => put world.logicalEffects id.logical {
              record with result := some (.succeeded id.attempt result) }
          | .failed _ => world.logicalEffects
        else world.logicalEffects
  { world with
      effects := put world.effects id (.acknowledged outcome)
      effectKeys := insertUnique id world.effectKeys
      logicalEffects := logicalEffects }

@[simp] theorem acknowledgeEffect_operationPlans :
    (acknowledgeEffect world effectKey result).operationPlans =
      world.operationPlans := by
  simp [acknowledgeEffect]

def nextAttemptData (world : World) (logical : LogicalEffectId)
    (failure : FailureKind) : Option (EffectId × LogicalEffectRecord) :=
  match world.logicalEffects logical with
  | none => none
  | some record =>
      let previous := ⟨logical, record.currentAttempt⟩
      match world.effects previous with
      | some (.acknowledged (.failed observed)) =>
          if observed = failure ∧ failure.retryable = true then
            match record.remainingRetries with
            | 0 => none
            | remaining + 1 =>
                let next := ⟨logical, record.currentAttempt + 1⟩
                match world.effects next with
                | some _ => none
                | none =>
                    some (next, {
                      record with
                        currentAttempt := record.currentAttempt + 1
                        remainingRetries := remaining
                    })
          else none
      | _ => none

def startNextAttempt (world : World) (logical : LogicalEffectId)
    (failure : FailureKind) : World :=
  let nextAttempt := nextAttemptData world logical failure
  { world with
      effects := match nextAttempt with
        | none => world.effects
        | some (next, _) => put world.effects next .committed
      effectKeys := match nextAttempt with
        | none => world.effectKeys
        | some (next, _) => insertUnique next world.effectKeys
      logicalEffects := match nextAttempt with
        | none => world.logicalEffects
        | some (_, record) => put world.logicalEffects logical record
      inflight := match nextAttempt with
        | none => world.inflight
        | some (next, _) => addEffect world.inflight next }

variable {world : World} {logical : LogicalEffectId} {failure : FailureKind}

@[simp] theorem startNextAttempt_sandboxes :
    (startNextAttempt world logical failure).sandboxes = world.sandboxes := rfl

@[simp] theorem startNextAttempt_vms :
    (startNextAttempt world logical failure).vms = world.vms := rfl

@[simp] theorem startNextAttempt_filesystems :
    (startNextAttempt world logical failure).filesystems = world.filesystems := rfl

@[simp] theorem startNextAttempt_snapshots :
    (startNextAttempt world logical failure).snapshots = world.snapshots := rfl

@[simp] theorem startNextAttempt_netSlots :
    (startNextAttempt world logical failure).netSlots = world.netSlots := rfl

@[simp] theorem startNextAttempt_appliedFilePolicies :
    (startNextAttempt world logical failure).appliedFilePolicies =
      world.appliedFilePolicies := rfl

@[simp] theorem startNextAttempt_workerFences :
    (startNextAttempt world logical failure).workerFences = world.workerFences := rfl

@[simp] theorem startNextAttempt_operations :
    (startNextAttempt world logical failure).operations = world.operations := rfl

@[simp] theorem startNextAttempt_operationPlans :
    (startNextAttempt world logical failure).operationPlans = world.operationPlans := rfl

@[simp] theorem startNextAttempt_resourceIndexes :
    (startNextAttempt world logical failure).resourceIndexes = world.resourceIndexes := rfl

@[simp] theorem startNextAttempt_controllerUp :
    (startNextAttempt world logical failure).controllerUp = world.controllerUp := rfl

def finalizeLogicalFailure (world : World) (logical : LogicalEffectId)
    (failure : FailureKind) : World :=
  let completed : Option LogicalEffectRecord :=
    match world.logicalEffects logical with
    | none => none
    | some record =>
        let current := ⟨logical, record.currentAttempt⟩
        match world.effects current with
        | some (.acknowledged (.failed observed)) =>
            if observed = failure then
              some { record with
                result := some (.failed record.currentAttempt failure) }
            else none
        | _ => none
  { world with logicalEffects := match completed with
      | none => world.logicalEffects
      | some record => put world.logicalEffects logical record }

@[simp] theorem finalizeLogicalFailure_sandboxes :
    (finalizeLogicalFailure world logical failure).sandboxes = world.sandboxes := rfl

@[simp] theorem finalizeLogicalFailure_vms :
    (finalizeLogicalFailure world logical failure).vms = world.vms := rfl

@[simp] theorem finalizeLogicalFailure_filesystems :
    (finalizeLogicalFailure world logical failure).filesystems = world.filesystems := rfl

@[simp] theorem finalizeLogicalFailure_snapshots :
    (finalizeLogicalFailure world logical failure).snapshots = world.snapshots := rfl

@[simp] theorem finalizeLogicalFailure_netSlots :
    (finalizeLogicalFailure world logical failure).netSlots = world.netSlots := rfl

@[simp] theorem finalizeLogicalFailure_appliedFilePolicies :
    (finalizeLogicalFailure world logical failure).appliedFilePolicies =
      world.appliedFilePolicies := rfl

@[simp] theorem finalizeLogicalFailure_workerFences :
    (finalizeLogicalFailure world logical failure).workerFences = world.workerFences := rfl

@[simp] theorem finalizeLogicalFailure_operations :
    (finalizeLogicalFailure world logical failure).operations = world.operations := rfl

@[simp] theorem finalizeLogicalFailure_operationPlans :
    (finalizeLogicalFailure world logical failure).operationPlans = world.operationPlans := rfl

@[simp] theorem finalizeLogicalFailure_resourceIndexes :
    (finalizeLogicalFailure world logical failure).resourceIndexes =
      world.resourceIndexes := rfl

@[simp] theorem finalizeLogicalFailure_effects :
    (finalizeLogicalFailure world logical failure).effects = world.effects := rfl

@[simp] theorem finalizeLogicalFailure_effectKeys :
    (finalizeLogicalFailure world logical failure).effectKeys = world.effectKeys := rfl

@[simp] theorem finalizeLogicalFailure_inflight :
    (finalizeLogicalFailure world logical failure).inflight = world.inflight := rfl

@[simp] theorem finalizeLogicalFailure_controllerUp :
    (finalizeLogicalFailure world logical failure).controllerUp = world.controllerUp := rfl

/-- A transport timeout does not manufacture a command failure.  It records
    only that the controller no longer knows the outcome; a late receipt or a
    safe redispatch may still resolve the same effect. -/
def markEffectOutcomeUnknown (world : World) (id : EffectId) : World :=
  setEffect world id .outcomeUnknown

@[simp] theorem markEffectOutcomeUnknown_operationPlans :
    (markEffectOutcomeUnknown world effectKey).operationPlans =
      world.operationPlans := rfl

@[simp] theorem markEffectOutcomeUnknown_sandboxes :
    (markEffectOutcomeUnknown world effectKey).sandboxes = world.sandboxes := rfl

@[simp] theorem markEffectOutcomeUnknown_vms :
    (markEffectOutcomeUnknown world effectKey).vms = world.vms := rfl

@[simp] theorem markEffectOutcomeUnknown_filesystems :
    (markEffectOutcomeUnknown world effectKey).filesystems = world.filesystems := rfl

@[simp] theorem markEffectOutcomeUnknown_snapshots :
    (markEffectOutcomeUnknown world effectKey).snapshots = world.snapshots := rfl

@[simp] theorem markEffectOutcomeUnknown_netSlots :
    (markEffectOutcomeUnknown world effectKey).netSlots = world.netSlots := rfl

@[simp] theorem markEffectOutcomeUnknown_appliedFilePolicies :
    (markEffectOutcomeUnknown world effectKey).appliedFilePolicies =
      world.appliedFilePolicies := rfl

@[simp] theorem markEffectOutcomeUnknown_workerFences :
    (markEffectOutcomeUnknown world effectKey).workerFences = world.workerFences := rfl

@[simp] theorem markEffectOutcomeUnknown_operations :
    (markEffectOutcomeUnknown world effectKey).operations = world.operations := rfl

@[simp] theorem markEffectOutcomeUnknown_controllerUp :
    (markEffectOutcomeUnknown world effectKey).controllerUp = world.controllerUp := rfl

@[simp] theorem dispatchEffect_sandboxes :
    (dispatchEffect world effectKey).sandboxes = world.sandboxes := rfl

@[simp] theorem dispatchEffect_vms :
    (dispatchEffect world effectKey).vms = world.vms := rfl

@[simp] theorem dispatchEffect_filesystems :
    (dispatchEffect world effectKey).filesystems = world.filesystems := rfl

@[simp] theorem dispatchEffect_snapshots :
    (dispatchEffect world effectKey).snapshots = world.snapshots := rfl

@[simp] theorem dispatchEffect_netSlots :
    (dispatchEffect world effectKey).netSlots = world.netSlots := rfl

@[simp] theorem dispatchEffect_appliedFilePolicies :
    (dispatchEffect world effectKey).appliedFilePolicies = world.appliedFilePolicies := rfl

@[simp] theorem dispatchEffect_workerFences :
    (dispatchEffect world effectKey).workerFences = world.workerFences := rfl

@[simp] theorem dispatchEffect_operations :
    (dispatchEffect world effectKey).operations = world.operations := rfl

@[simp] theorem acknowledgeEffect_sandboxes :
    (acknowledgeEffect world effectKey result).sandboxes = world.sandboxes := by
  simp [acknowledgeEffect]

@[simp] theorem acknowledgeEffect_vms :
    (acknowledgeEffect world effectKey result).vms = world.vms := by
  simp [acknowledgeEffect]

@[simp] theorem acknowledgeEffect_filesystems :
    (acknowledgeEffect world effectKey result).filesystems = world.filesystems := by
  simp [acknowledgeEffect]

@[simp] theorem acknowledgeEffect_snapshots :
    (acknowledgeEffect world effectKey result).snapshots = world.snapshots := by
  simp [acknowledgeEffect]

@[simp] theorem acknowledgeEffect_netSlots :
    (acknowledgeEffect world effectKey result).netSlots = world.netSlots := by
  simp [acknowledgeEffect]

@[simp] theorem acknowledgeEffect_appliedFilePolicies :
    (acknowledgeEffect world effectKey result).appliedFilePolicies =
      world.appliedFilePolicies := by simp [acknowledgeEffect]

@[simp] theorem acknowledgeEffect_workerFences :
    (acknowledgeEffect world effectKey result).workerFences = world.workerFences := by
  simp [acknowledgeEffect]

@[simp] theorem acknowledgeEffect_operations :
    (acknowledgeEffect world effectKey result).operations = world.operations := by
  simp [acknowledgeEffect]

@[simp] theorem commitEffect_controllerUp :
    (commitEffect world command).controllerUp = world.controllerUp := by
  cases h : world.effects (effectId command)
  · cases logical : world.logicalEffects (logicalEffectId command) <;>
      simp [commitEffect, h, logical, setEffect]
  · simp [commitEffect, h]

end SandboxModel
