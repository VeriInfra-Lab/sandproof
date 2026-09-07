import SandboxModel.State

/-!
The asynchronous protocol relation.

Controller intent, worker effects, and publication are deliberately separate
constructors. This exposes the windows in which an RPC may time out or an old
command may arrive. Commands remain in `inflight`, so the environment may
deliver them repeatedly; the worker-side epoch and resource checks must make
such delivery harmless.
-/

namespace SandboxModel

/-- Persist initial intent before any VM or network resource can be prepared. -/
def beginCreate (world : World) (sid : SandboxId) (owner : TenantId)
    (workspace : FsId) (policy : NetworkPolicy) (operationId : OperationId) : World :=
  let epoch := 1
  let generation := 1
  let operation : Operation := ⟨operationId, epoch, .create, .pending⟩
  let record : SandboxRecord := {
    owner := owner
    phase := .starting
    terminal := false
    epoch := epoch
    generation := generation
    currentVm := none
    workspace := workspace
    durableSnapshot := none
    restoreFrom := none
    netSlot := none
    policy := policy
    desired := noCapabilities
    effective := noCapabilities
    currentOperation := some operation
  }
  let filesystem : Filesystem := ⟨owner, sid, generation⟩
  enqueue (setSandbox (setFilesystem world workspace filesystem) sid record)
    (.installFence sid epoch false)

/-- Installing a fence closes effective authority and the current network slot. -/
def applyFence (world : World) (sid : SandboxId) (record : SandboxRecord) : World :=
  let closed := { record with effective := noCapabilities }
  let withRecord := setFence (setSandbox world sid closed) sid
    ⟨record.epoch, record.terminal⟩
  match record.netSlot with
  | none => withRecord
  | some slotId =>
      match world.netSlots slotId with
      | none => withRecord
      | some slot => setNetSlot withRecord slotId { slot with active := false }

def prepareResources (world : World) (sid : SandboxId) (record : SandboxRecord)
    (vmId : VmId) (slotId : NetSlotId) : World :=
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
  let updated := { record with currentVm := some vmId, netSlot := some slotId }
  let command := .activateVm sid record.epoch record.generation vmId
  enqueue (setSandbox (setNetSlot (setVm world vmId vm) slotId slot) sid updated) command

def configureSlot (world : World) (slotId : NetSlotId) (slot : NetSlot) : World :=
  setNetSlot world slotId { slot with configured := true }

def activateResource (world : World) (vmId : VmId) (vm : Vm) : World :=
  setVm world vmId { vm with status := .executing }

/-- Publish authority only after VM activation and network preparation were observed. -/
def publishRunning (world : World) (sid : SandboxId) (record : SandboxRecord)
    (slotId : NetSlotId) (slot : NetSlot) : World :=
  let base := { record with
    phase := .running
    terminal := false
    restoreFrom := none
    currentOperation := record.currentOperation.map
      (fun (op : Operation) => { op with status := .succeeded })
  }
  let running := { base with
    desired := runningCapabilities sid base
    effective := runningCapabilities sid base
  }
  setSandbox (setNetSlot world slotId { slot with active := true }) sid running

/-- Pause first closes desired authority; effective authority closes at the fence. -/
def beginPause (world : World) (sid : SandboxId) (record : SandboxRecord)
    (operationId : OperationId) (vmId : VmId) : World :=
  let epoch := record.epoch + 1
  let operation : Operation := ⟨operationId, epoch, .pause, .pending⟩
  let updated := { record with
    phase := .pausing
    epoch := epoch
    desired := noCapabilities
    currentOperation := some operation
  }
  enqueue (enqueue (setSandbox world sid updated) (.installFence sid epoch false))
    (.freezeVm sid epoch record.generation vmId)

def sealSnapshot (world : World) (snapshotId : SnapshotId) (record : SandboxRecord)
    (sid : SandboxId) : World :=
  let snapshot : Snapshot := {
    owner := record.owner
    sandbox := sid
    sourceGeneration := record.generation
    workspace := record.workspace
    stage := .localSealed
    immutable := false
  }
  setSnapshot world snapshotId snapshot

/-- Durability and immutability are committed together. -/
def makeSnapshotDurable (world : World) (snapshotId : SnapshotId)
    (snapshot : Snapshot) : World :=
  setSnapshot world snapshotId { snapshot with stage := .durable, immutable := true }

def stopResource (world : World) (vmId : VmId) (vm : Vm) : World :=
  setVm world vmId { vm with status := .stopped }

def publishPaused (world : World) (sid : SandboxId) (record : SandboxRecord)
    (snapshotId : SnapshotId) : World :=
  let base := { record with
    phase := .paused
    currentVm := none
    durableSnapshot := some snapshotId
    restoreFrom := none
    netSlot := none
    currentOperation := record.currentOperation.map
      (fun (op : Operation) => { op with status := .succeeded })
  }
  let paused := { base with
    desired := pausedCapabilities sid snapshotId
    effective := pausedCapabilities sid snapshotId
  }
  setSandbox world sid paused

/-- Resume atomically selects its source and allocates a new epoch and generation. -/
def beginResume (world : World) (sid : SandboxId) (record : SandboxRecord)
    (operationId : OperationId) (snapshotId : SnapshotId) : World :=
  let epoch := record.epoch + 1
  let generation := record.generation + 1
  let operation : Operation := ⟨operationId, epoch, .resume, .pending⟩
  let updated := { record with
    phase := .starting
    epoch := epoch
    generation := generation
    currentVm := none
    restoreFrom := some snapshotId
    netSlot := none
    desired := noCapabilities
    effective := noCapabilities
    currentOperation := some operation
  }
  enqueue (setSandbox world sid updated) (.installFence sid epoch false)

/-- Kill writes the irreversible tombstone before asynchronous resource cleanup. -/
def beginKill (world : World) (sid : SandboxId) (record : SandboxRecord)
    (operationId : OperationId) : World :=
  let epoch := record.epoch + 1
  let operation : Operation := ⟨operationId, epoch, .kill, .pending⟩
  let updated := { record with
    phase := .revoking
    terminal := true
    epoch := epoch
    desired := noCapabilities
    effective := withoutResume record.effective
    currentOperation := some operation
  }
  enqueue (setSandbox world sid updated) (.installFence sid epoch true)

def publishDestroyed (world : World) (sid : SandboxId) (record : SandboxRecord) : World :=
  let destroyed := { record with
    phase := .destroyed
    terminal := true
    currentVm := none
    netSlot := none
    desired := noCapabilities
    effective := noCapabilities
    currentOperation := record.currentOperation.map
      (fun (op : Operation) => { op with status := .succeeded })
  }
  setSandbox world sid destroyed

def markOutcomeUnknown (world : World) (sid : SandboxId) (record : SandboxRecord)
    (operation : Operation) : World :=
  setSandbox world sid { record with
    currentOperation := some { operation with status := .outcomeUnknown }
  }

def noExecutingVm (world : World) (sid : SandboxId) : Prop :=
  ∀ vmId vm, world.vms vmId = some vm → vm.sandbox = sid → vm.status ≠ .executing

def noActiveSlot (world : World) (sid : SandboxId) : Prop :=
  ∀ slotId slot, world.netSlots slotId = some slot → slot.sandbox = sid → slot.active = false

def usableSnapshot (world : World) (sid : SandboxId) (record : SandboxRecord)
    (snapshotId : SnapshotId) : Prop :=
  ∃ snapshot, world.snapshots snapshotId = some snapshot ∧
    snapshot.owner = record.owner ∧ snapshot.sandbox = sid ∧
    snapshot.stage = .durable ∧ snapshot.immutable = true

inductive Step : World → Event → World → Prop where
  | acceptCreate
      (up : world.controllerUp = true)
      (freshSandbox : world.sandboxes sid = none)
      (freshWorkspace : world.filesystems workspace = none)
      (auth : authorized actor owner) :
      Step world (.acceptCreate actor owner sid workspace policy operationId)
        (beginCreate world sid owner workspace policy operationId)

  | installFence
      (queued : world.inflight (.installFence sid record.epoch record.terminal))
      (found : world.sandboxes sid = some record) :
      Step world (.installFence sid) (applyFence world sid record)

  | dispatchPrepare
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩)
      (freshVm : world.vms vmId = none)
      (freshSlot : world.netSlots slotId = none)
      (selectedSource : snapshotId = record.restoreFrom)
      (resumeSource : ∀ source, snapshotId = some source →
        record.durableSnapshot = some source ∧ usableSnapshot world sid record source) :
      Step world (.dispatchPrepare sid vmId slotId snapshotId) (enqueue world
        (.prepareVm sid record.epoch record.generation vmId slotId snapshotId))

  | prepareVm
      (queued : world.inflight (.prepareVm sid record.epoch record.generation vmId slotId snapshotId))
      (found : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩)
      (freshVm : world.vms vmId = none)
      (freshSlot : world.netSlots slotId = none)
      (selectedSource : snapshotId = record.restoreFrom)
      (resumeSource : ∀ source, snapshotId = some source →
        record.durableSnapshot = some source ∧ usableSnapshot world sid record source) :
      Step world (.prepareVm sid vmId slotId snapshotId)
        (prepareResources world sid record vmId slotId)

  | configureNetwork
      (foundSandbox : world.sandboxes sid = some record)
      (foundSlot : world.netSlots slotId = some slot)
      (current : record.netSlot = some slotId)
      (owned : slot.owner = record.owner ∧ slot.sandbox = sid ∧
        slot.generation = record.generation ∧ slot.vm = vmId ∧ slot.policy = record.policy)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩) :
      Step world (.configureNetwork sid slotId) (configureSlot world slotId slot)

  | activateVm
      (queued : world.inflight (.activateVm sid record.epoch record.generation vmId))
      (foundSandbox : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (current : record.currentVm = some vmId)
      (foundVm : world.vms vmId = some vm)
      (ownedVm : vm.owner = record.owner ∧ vm.sandbox = sid ∧
        vm.generation = record.generation)
      (prepared : vm.status = .prepared)
      (foundSlot : ∃ slotId slot, record.netSlot = some slotId ∧
        world.netSlots slotId = some slot ∧ slot.vm = vmId ∧ slot.configured = true)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩)
      (exclusive : noExecutingVm world sid) :
      Step world (.activateVm sid vmId) (activateResource world vmId vm)

  | publishRunning
      (up : world.controllerUp = true)
      (foundSandbox : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (currentVm : record.currentVm = some vmId)
      (foundVm : world.vms vmId = some vm)
      (vmIdentity : vm.owner = record.owner ∧ vm.sandbox = sid ∧
        vm.generation = record.generation)
      (executing : vm.status = .executing)
      (foundSlot : world.netSlots slotId = some slot)
      (currentSlot : record.netSlot = some slotId)
      (slotReady : slot.owner = record.owner ∧ slot.sandbox = sid ∧
        slot.generation = record.generation ∧ slot.vm = vmId ∧
        slot.policy = record.policy ∧ slot.configured = true ∧ slot.active = false)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩) :
      Step world (.publishRunning sid) (publishRunning world sid record slotId slot)

  | acceptPause
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (running : record.phase = .running)
      (live : record.terminal = false)
      (current : record.currentVm = some vmId)
      (auth : authorized actor record.owner) :
      Step world (.acceptPause actor sid operationId)
        (beginPause world sid record operationId vmId)

  | freezeVm
      (queued : world.inflight (.freezeVm sid record.epoch record.generation vmId))
      (foundSandbox : world.sandboxes sid = some record)
      (pausing : record.phase = .pausing)
      (current : record.currentVm = some vmId)
      (foundVm : world.vms vmId = some vm)
      (vmIdentity : vm.owner = record.owner ∧ vm.sandbox = sid ∧
        vm.generation = record.generation)
      (executing : vm.status = .executing)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩) :
      Step world (.freezeVm sid vmId)
        (setVm world vmId { vm with status := .frozen })

  | sealSnapshot
      (foundSandbox : world.sandboxes sid = some record)
      (pausing : record.phase = .pausing)
      (foundVm : world.vms vmId = some vm)
      (current : record.currentVm = some vmId)
      (vmIdentity : vm.owner = record.owner ∧ vm.sandbox = sid ∧
        vm.generation = record.generation)
      (frozen : vm.status = .frozen)
      (fresh : world.snapshots snapshotId = none) :
      Step world (.sealSnapshot sid snapshotId)
        (sealSnapshot world snapshotId record sid)

  | commitSnapshot
      (found : world.snapshots snapshotId = some snapshot)
      (sealed : snapshot.stage = .localSealed) :
      Step world (.commitSnapshot snapshotId)
        (makeSnapshotDurable world snapshotId snapshot)

  | stopVm
      (found : world.vms vmId = some vm)
      (owned : vm.sandbox = sid)
      (authorizedStop :
        (∃ record, world.sandboxes sid = some record ∧ record.phase = .pausing ∧
          record.currentVm = some vmId ∧ vm.status = .frozen ∧
          world.workerFences sid = some ⟨record.epoch, false⟩) ∨
        (∃ record, world.sandboxes sid = some record ∧ record.phase = .revoking ∧
          record.terminal = true ∧
          world.workerFences sid = some ⟨record.epoch, true⟩)) :
      Step world (.stopVm sid vmId) (stopResource world vmId vm)

  | publishPaused
      (up : world.controllerUp = true)
      (foundSandbox : world.sandboxes sid = some record)
      (pausing : record.phase = .pausing)
      (foundSnapshot : world.snapshots snapshotId = some snapshot)
      (snapshotReady : snapshot.owner = record.owner ∧ snapshot.sandbox = sid ∧
        snapshot.sourceGeneration = record.generation ∧ snapshot.workspace = record.workspace ∧
        snapshot.stage = .durable ∧ snapshot.immutable = true)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩)
      (quiet : noExecutingVm world sid)
      (networkClosed : noActiveSlot world sid) :
      Step world (.publishPaused sid snapshotId)
        (publishPaused world sid record snapshotId)

  | acceptResume
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (paused : record.phase = .paused)
      (live : record.terminal = false)
      (currentSnapshot : record.durableSnapshot = some snapshotId)
      (source : usableSnapshot world sid record snapshotId)
      (auth : authorized actor record.owner) :
      Step world (.acceptResume actor sid operationId)
        (beginResume world sid record operationId snapshotId)

  | acceptKill
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (notDestroyed : record.phase ≠ .destroyed)
      (auth : authorized actor record.owner) :
      Step world (.acceptKill actor sid operationId)
        (beginKill world sid record operationId)

  | publishDestroyed
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (revoking : record.phase = .revoking)
      (terminal : record.terminal = true)
      (fenced : world.workerFences sid = some ⟨record.epoch, true⟩)
      (quiet : noExecutingVm world sid)
      (networkClosed : noActiveSlot world sid) :
      Step world (.publishDestroyed sid) (publishDestroyed world sid record)

  | rpcTimeout
      (foundSandbox : world.sandboxes sid = some record)
      (current : record.currentOperation = some operation)
      (pending : operation.status = .pending) :
      Step world (.rpcTimeout sid) (markOutcomeUnknown world sid record operation)

  | controllerCrash (up : world.controllerUp = true) :
      Step world .controllerCrash { world with controllerUp := false }

  | controllerRecover (down : world.controllerUp = false) :
      Step world .controllerRecover { world with controllerUp := true }

  | duplicateOrLateDelivery (queued : world.inflight command) :
      Step world (.duplicateOrLateDelivery command) world

  | exec
      (found : world.sandboxes sid = some record)
      (auth : authorized actor record.owner)
      (capability : record.effective (.exec sid record.generation vmId)) :
      Step world (.exec actor sid) world

  | fileRead
      (found : world.sandboxes sid = some record)
      (auth : authorized actor record.owner)
      (capability : record.effective
        (.fileRead sid record.generation record.workspace)) :
      Step world (.fileRead actor sid) world

  | fileWrite
      (found : world.sandboxes sid = some record)
      (auth : authorized actor record.owner)
      (capability : record.effective
        (.fileWrite sid record.generation record.workspace)) :
      Step world (.fileWrite actor sid) world

  | ingress
      (found : world.sandboxes sid = some record)
      (auth : authorized actor record.owner)
      (capability : record.effective (.ingress sid record.generation vmId slotId)) :
      Step world (.ingress actor sid) world

  | egress
      (found : world.sandboxes sid = some record)
      (capability : record.effective
        (.egress sid record.generation vmId slotId record.policy))
      (allowed : record.policy = .staticAllowlist endpoints ∧ destination ∈ endpoints) :
      Step world (.egress sid destination) world

inductive Reachable : World → Prop where
  | initial : Reachable emptyWorld
  | step : Reachable world → Step world event next → Reachable next

end SandboxModel
