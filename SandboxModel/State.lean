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
  durableSnapshot : Option SnapshotId
  /-- Snapshot selected atomically by AcceptResume; later commands cannot swap it. -/
  restoreFrom : Option SnapshotId
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
  workerFences : SandboxId → Option WorkerFence
  inflight : Command → Prop
  controllerUp : Bool

def emptyWorld : World where
  sandboxes := fun _ => none
  vms := fun _ => none
  filesystems := fun _ => none
  snapshots := fun _ => none
  netSlots := fun _ => none
  workerFences := fun _ => none
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

def setFence (world : World) (sid : SandboxId) (fence : WorkerFence) : World :=
  { world with workerFences := put world.workerFences sid fence }

def enqueue (world : World) (command : Command) : World :=
  { world with inflight := addCommand world.inflight command }

end SandboxModel
