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
  effects : EffectId → Option EffectStatus
  /-- Non-authoritative transport history. Dispatch and execution authority come
      only from `effects`; this set merely records that delivery may recur. -/
  inflight : Command → Prop
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
  effects := fun _ => none
  inflight := fun _ => False
  controllerUp := true

def put [DecidableEq α] (store : α → Option β) (key : α) (value : β) : α → Option β :=
  fun candidate => if candidate = key then some value else store candidate

def addCommand (inflight : Command → Prop) (command : Command) : Command → Prop :=
  fun candidate => candidate = command ∨ inflight candidate

def setSandbox (world : World) (sid : SandboxId) (record : SandboxRecord) : World :=
  { world with sandboxes := put world.sandboxes sid record }

def setVm (world : World) (vmId : VmId) (vm : Vm) : World :=
  { world with vms := put world.vms vmId vm }

def setFilesystem (world : World) (fsId : FsId) (fs : Filesystem) : World :=
  { world with filesystems := put world.filesystems fsId fs }

def setSnapshot (world : World) (snapshotId : SnapshotId) (snapshot : Snapshot) : World :=
  { world with snapshots := put world.snapshots snapshotId snapshot }

def setNetSlot (world : World) (slotId : NetSlotId) (slot : NetSlot) : World :=
  { world with netSlots := put world.netSlots slotId slot }

def setAppliedFilePolicy (world : World) (sid : SandboxId)
    (policy : AppliedFilePolicy) : World :=
  { world with appliedFilePolicies := put world.appliedFilePolicies sid policy }

def setFence (world : World) (sid : SandboxId) (fence : WorkerFence) : World :=
  { world with workerFences := put world.workerFences sid fence }

def setOperation (world : World) (key : OperationKey)
    (operation : OperationLedgerRecord) : World :=
  { world with operations := put world.operations key operation }

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

def setEffect (world : World) (effectId : EffectId) (status : EffectStatus) : World :=
  { world with effects := put world.effects effectId status }

def effectId (command : Command) : EffectId := ⟨command⟩

def commitEffect (world : World) (command : Command) : World :=
  let id := effectId command
  match world.effects id with
  | none => setEffect world id .committed
  | some _ => world

@[simp] theorem commitEffect_sandboxes :
    (commitEffect world command).sandboxes = world.sandboxes := by
  cases h : world.effects (effectId command) <;> simp [commitEffect, h, setEffect]

@[simp] theorem commitEffect_vms :
    (commitEffect world command).vms = world.vms := by
  cases h : world.effects (effectId command) <;> simp [commitEffect, h, setEffect]

@[simp] theorem commitEffect_filesystems :
    (commitEffect world command).filesystems = world.filesystems := by
  cases h : world.effects (effectId command) <;> simp [commitEffect, h, setEffect]

@[simp] theorem commitEffect_snapshots :
    (commitEffect world command).snapshots = world.snapshots := by
  cases h : world.effects (effectId command) <;> simp [commitEffect, h, setEffect]

@[simp] theorem commitEffect_netSlots :
    (commitEffect world command).netSlots = world.netSlots := by
  cases h : world.effects (effectId command) <;> simp [commitEffect, h, setEffect]

@[simp] theorem commitEffect_appliedFilePolicies :
    (commitEffect world command).appliedFilePolicies = world.appliedFilePolicies := by
  cases h : world.effects (effectId command) <;> simp [commitEffect, h, setEffect]

@[simp] theorem commitEffect_workerFences :
    (commitEffect world command).workerFences = world.workerFences := by
  cases h : world.effects (effectId command) <;> simp [commitEffect, h, setEffect]

@[simp] theorem commitEffect_operations :
    (commitEffect world command).operations = world.operations := by
  cases h : world.effects (effectId command) <;> simp [commitEffect, h, setEffect]

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

def dispatchEffect (world : World) (id : EffectId) : World :=
  setEffect world id .dispatched

def acknowledgeEffect (world : World) (id : EffectId) (result : ResultDigest) : World :=
  setEffect world id (.acknowledged result)

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
    (acknowledgeEffect world effectKey result).sandboxes = world.sandboxes := rfl

@[simp] theorem acknowledgeEffect_vms :
    (acknowledgeEffect world effectKey result).vms = world.vms := rfl

@[simp] theorem acknowledgeEffect_filesystems :
    (acknowledgeEffect world effectKey result).filesystems = world.filesystems := rfl

@[simp] theorem acknowledgeEffect_snapshots :
    (acknowledgeEffect world effectKey result).snapshots = world.snapshots := rfl

@[simp] theorem acknowledgeEffect_netSlots :
    (acknowledgeEffect world effectKey result).netSlots = world.netSlots := rfl

@[simp] theorem acknowledgeEffect_appliedFilePolicies :
    (acknowledgeEffect world effectKey result).appliedFilePolicies =
      world.appliedFilePolicies := rfl

@[simp] theorem acknowledgeEffect_workerFences :
    (acknowledgeEffect world effectKey result).workerFences = world.workerFences := rfl

@[simp] theorem acknowledgeEffect_operations :
    (acknowledgeEffect world effectKey result).operations = world.operations := rfl

@[simp] theorem commitEffect_controllerUp :
    (commitEffect world command).controllerUp = world.controllerUp := by
  simp only [commitEffect]
  split <;> simp [setEffect]

end SandboxModel
