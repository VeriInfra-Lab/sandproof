import SandboxModel.State

/-!
The asynchronous protocol relation.

Controller intent, worker effects, and publication are deliberately separate
constructors. This exposes the windows in which an RPC may time out or an old
command may arrive. Commands remain in a delivery set, while the durable effect
ledger records commit, dispatch, and acknowledgement. Worker-side identity
checks make stale or repeated delivery harmless.
-/

namespace SandboxModel

def operationKey (sid : SandboxId) (operationId : OperationId) : OperationKey :=
  ⟨sid, operationId⟩

def storeNewOperation (world : World) (sid : SandboxId) (operation : Operation)
    (input : OperationInput) : World :=
  setOperation world (operationKey sid operation.id)
    ⟨sid, operation, input, none⟩

def updateOperation (world : World) (sid : SandboxId) (operation : Operation)
    (result : Option OperationResult) : World :=
  let key := operationKey sid operation.id
  { world with operations := fun candidate =>
      if candidate = key then
        match world.operations key with
        | none => none
        | some persisted =>
            some { persisted with operation := operation, result := result }
      else world.operations candidate }

@[simp] theorem storeNewOperation_sandboxes :
    (storeNewOperation world sid operation input).sandboxes = world.sandboxes := by
  simp [storeNewOperation, setOperation]

@[simp] theorem storeNewOperation_vms :
    (storeNewOperation world sid operation input).vms = world.vms := by
  simp [storeNewOperation, setOperation]

@[simp] theorem storeNewOperation_filesystems :
    (storeNewOperation world sid operation input).filesystems = world.filesystems := by
  simp [storeNewOperation, setOperation]

@[simp] theorem storeNewOperation_snapshots :
    (storeNewOperation world sid operation input).snapshots = world.snapshots := by
  simp [storeNewOperation, setOperation]

@[simp] theorem storeNewOperation_netSlots :
    (storeNewOperation world sid operation input).netSlots = world.netSlots := by
  simp [storeNewOperation, setOperation]

@[simp] theorem storeNewOperation_appliedFilePolicies :
    (storeNewOperation world sid operation input).appliedFilePolicies =
      world.appliedFilePolicies := by
  simp [storeNewOperation, setOperation]

@[simp] theorem storeNewOperation_workerFences :
    (storeNewOperation world sid operation input).workerFences = world.workerFences := by
  simp [storeNewOperation, setOperation]

@[simp] theorem storeNewOperation_effects :
    (storeNewOperation world sid operation input).effects = world.effects := by
  simp [storeNewOperation, setOperation]

@[simp] theorem storeNewOperation_controllerUp :
    (storeNewOperation world sid operation input).controllerUp = world.controllerUp := by
  simp [storeNewOperation, setOperation]

@[simp] theorem updateOperation_sandboxes :
    (updateOperation world sid operation result).sandboxes = world.sandboxes := rfl

@[simp] theorem updateOperation_vms :
    (updateOperation world sid operation result).vms = world.vms := rfl

@[simp] theorem updateOperation_filesystems :
    (updateOperation world sid operation result).filesystems = world.filesystems := rfl

@[simp] theorem updateOperation_snapshots :
    (updateOperation world sid operation result).snapshots = world.snapshots := rfl

@[simp] theorem updateOperation_netSlots :
    (updateOperation world sid operation result).netSlots = world.netSlots := rfl

@[simp] theorem updateOperation_appliedFilePolicies :
    (updateOperation world sid operation result).appliedFilePolicies =
      world.appliedFilePolicies := rfl

@[simp] theorem updateOperation_workerFences :
    (updateOperation world sid operation result).workerFences = world.workerFences := rfl

@[simp] theorem updateOperation_effects :
    (updateOperation world sid operation result).effects = world.effects := rfl

@[simp] theorem updateOperation_controllerUp :
    (updateOperation world sid operation result).controllerUp = world.controllerUp := rfl

def updateCurrentOperation (world : World) (sid : SandboxId)
    (operation : Option Operation) (result : Option OperationResult) : World :=
  { world with operations := fun candidate =>
      match operation with
      | none => world.operations candidate
      | some current =>
          let key := operationKey sid current.id
          if candidate = key then
            match world.operations key with
            | none => none
            | some persisted =>
                some { persisted with operation := current, result := result }
          else world.operations candidate }

@[simp] theorem updateCurrentOperation_sandboxes :
    (updateCurrentOperation world sid operation result).sandboxes = world.sandboxes := by
  cases operation <;> simp [updateCurrentOperation]

@[simp] theorem updateCurrentOperation_vms :
    (updateCurrentOperation world sid operation result).vms = world.vms := by
  cases operation <;> simp [updateCurrentOperation]

@[simp] theorem updateCurrentOperation_filesystems :
    (updateCurrentOperation world sid operation result).filesystems = world.filesystems := by
  cases operation <;> simp [updateCurrentOperation]

@[simp] theorem updateCurrentOperation_snapshots :
    (updateCurrentOperation world sid operation result).snapshots = world.snapshots := by
  cases operation <;> simp [updateCurrentOperation]

@[simp] theorem updateCurrentOperation_netSlots :
    (updateCurrentOperation world sid operation result).netSlots = world.netSlots := by
  cases operation <;> simp [updateCurrentOperation]

@[simp] theorem updateCurrentOperation_appliedFilePolicies :
    (updateCurrentOperation world sid operation result).appliedFilePolicies =
      world.appliedFilePolicies := by
  cases operation <;> simp [updateCurrentOperation]

@[simp] theorem updateCurrentOperation_workerFences :
    (updateCurrentOperation world sid operation result).workerFences = world.workerFences := by
  cases operation <;> simp [updateCurrentOperation]

@[simp] theorem updateCurrentOperation_effects :
    (updateCurrentOperation world sid operation result).effects = world.effects := by
  cases operation <;> simp [updateCurrentOperation]

@[simp] theorem updateCurrentOperation_controllerUp :
    (updateCurrentOperation world sid operation result).controllerUp = world.controllerUp := by
  cases operation <;> simp [updateCurrentOperation]

def effectHasStatus (world : World) (command : Command)
    (status : EffectStatus) : Prop :=
  world.effects (effectId command) = some status

def effectDispatched (world : World) (command : Command) : Prop :=
  effectHasStatus world command .dispatched

def effectAcknowledged (world : World) (command : Command) : Prop :=
  ∃ result, effectHasStatus world command (.acknowledged result)

def fenceCanAdvance (oldFence : Option WorkerFence) (epoch : Epoch)
    (terminal : Bool) : Prop :=
  match oldFence with
  | none => True
  | some old => old.epoch ≤ epoch ∧ (old.terminal = true → terminal = true)

/-- Persist initial intent before any VM or network resource can be prepared. -/
def beginCreate (world : World) (sid : SandboxId) (owner : TenantId)
    (workspace : FsId) (filePolicy : FilePolicyRef) (policy : NetworkPolicy)
    (operationId : OperationId) : World :=
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
    workspaceAccess := .managedReadWrite
    filePolicy := filePolicy
    durableSnapshot := none
    restoreFrom := none
    netSlot := none
    policy := policy
    desired := .closed
    effective := .closed
    currentOperation := some operation
  }
  let filesystem : Filesystem := ⟨owner, sid, generation⟩
  let persisted := storeNewOperation
    (setSandbox (setFilesystem world workspace filesystem) sid record) sid operation
    (.create workspace policy filePolicy)
  enqueue persisted (.installFence sid epoch false)

/-- A start fence installs the new epoch before resources are prepared. -/
def applyStartFence (world : World) (sid : SandboxId)
    (record : SandboxRecord) : World :=
  setFence world sid ⟨record.epoch, false⟩

@[simp] theorem applyStartFence_operations :
    (applyStartFence world sid record).operations = world.operations := rfl

/-- A pause/kill fence closes authority and the current network slot. -/
def applyRevocationFence (world : World) (sid : SandboxId)
    (record : SandboxRecord) : World :=
  let closed := setFence (setSandbox world sid { record with effective := .closed })
    sid ⟨record.epoch, record.terminal⟩
  match record.netSlot with
  | none => closed
  | some slotId =>
      match world.netSlots slotId with
      | none => closed
      | some slot => setNetSlot closed slotId { slot with active := false }

@[simp] theorem applyRevocationFence_appliedFilePolicies :
    (applyRevocationFence world sid record).appliedFilePolicies =
      world.appliedFilePolicies := by
  cases slotChoice : record.netSlot with
  | none => simp [applyRevocationFence, slotChoice, setFence, setSandbox]
  | some slotId =>
      cases slotFound : world.netSlots slotId <;>
        simp [applyRevocationFence, slotChoice, slotFound, setFence,
          setSandbox, setNetSlot]

@[simp] theorem applyRevocationFence_operations :
    (applyRevocationFence world sid record).operations = world.operations := by
  cases slotChoice : record.netSlot with
  | none => simp [applyRevocationFence, slotChoice, setFence, setSandbox]
  | some slotId =>
      cases slotFound : world.netSlots slotId <;>
        simp [applyRevocationFence, slotChoice, slotFound, setFence,
          setSandbox, setNetSlot]

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
    reclaimed := false
  }
  let updated := { record with currentVm := some vmId, netSlot := some slotId }
  let prepared := setSandbox (setNetSlot (setVm world vmId vm) slotId slot) sid updated
  let activate := .activateVm sid record.epoch record.generation vmId
  let applyPolicy := .applyFilePolicy sid record.epoch record.generation
    record.workspace record.filePolicy
  enqueue (enqueue prepared activate) applyPolicy

@[simp] theorem prepareResources_operations :
    (prepareResources world sid record vmId slotId).operations = world.operations := by
  simp [prepareResources, setSandbox, setNetSlot, setVm]

def applyFilePolicyResult (world : World) (sid : SandboxId)
    (record : SandboxRecord) : World :=
  setAppliedFilePolicy world sid
    ⟨sid, record.generation, record.workspace, record.filePolicy⟩

@[simp] theorem applyFilePolicyResult_sandboxes :
    (applyFilePolicyResult world sid record).sandboxes = world.sandboxes := rfl

@[simp] theorem applyFilePolicyResult_vms :
    (applyFilePolicyResult world sid record).vms = world.vms := rfl

@[simp] theorem applyFilePolicyResult_filesystems :
    (applyFilePolicyResult world sid record).filesystems = world.filesystems := rfl

@[simp] theorem applyFilePolicyResult_snapshots :
    (applyFilePolicyResult world sid record).snapshots = world.snapshots := rfl

@[simp] theorem applyFilePolicyResult_netSlots :
    (applyFilePolicyResult world sid record).netSlots = world.netSlots := rfl

@[simp] theorem applyFilePolicyResult_workerFences :
    (applyFilePolicyResult world sid record).workerFences = world.workerFences := rfl

@[simp] theorem applyFilePolicyResult_operations :
    (applyFilePolicyResult world sid record).operations = world.operations := rfl

def configureSlot (world : World) (slotId : NetSlotId) (slot : NetSlot) : World :=
  setNetSlot world slotId { slot with configured := true }

@[simp] theorem configureSlot_operations :
    (configureSlot world slotId slot).operations = world.operations := rfl

def activateResource (world : World) (vmId : VmId) (vm : Vm) : World :=
  setVm world vmId { vm with status := .executing }

@[simp] theorem activateResource_operations :
    (activateResource world vmId vm).operations = world.operations := rfl

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
  let authority := .running sid base.generation slot.vm base.workspace
    base.workspaceAccess slotId base.policy
  let running := { base with desired := authority, effective := authority }
  let published := setSandbox
    (setNetSlot world slotId { slot with active := true }) sid running
  updateCurrentOperation published sid running.currentOperation
    (some (.running base.generation))

/-- Pause first closes desired authority; effective authority closes at the fence. -/
def beginPause (world : World) (sid : SandboxId) (record : SandboxRecord)
    (operationId : OperationId) (vmId : VmId) : World :=
  let epoch := record.epoch + 1
  let operation : Operation := ⟨operationId, epoch, .pause, .pending⟩
  let updated := { record with
    phase := .pausing
    epoch := epoch
    desired := .closed
    currentOperation := some operation
  }
  let persisted := storeNewOperation (setSandbox world sid updated) sid operation
    (.pause record.generation)
  enqueue (enqueue persisted (.installFence sid epoch false))
    (.freezeVm sid epoch record.generation vmId)

def sealSnapshot (world : World) (snapshotId : SnapshotId)
    (manifest : SnapshotManifestRef) (record : SandboxRecord) (sid : SandboxId) : World :=
  let snapshot : Snapshot := {
    owner := record.owner
    sandbox := sid
    sourceGeneration := record.generation
    workspace := record.workspace
    manifest := manifest
    stage := .localSealed
    immutable := false
  }
  setSnapshot world snapshotId snapshot

@[simp] theorem sealSnapshot_operations :
    (sealSnapshot world snapshotId manifest record sid).operations = world.operations := rfl

/-- Durability and immutability are committed together. -/
def makeSnapshotDurable (world : World) (snapshotId : SnapshotId)
    (snapshot : Snapshot) : World :=
  setSnapshot world snapshotId { snapshot with stage := .durable, immutable := true }

@[simp] theorem makeSnapshotDurable_operations :
    (makeSnapshotDurable world snapshotId snapshot).operations = world.operations := rfl

def stopResource (world : World) (vmId : VmId) (vm : Vm) : World :=
  setVm world vmId { vm with status := .stopped }

@[simp] theorem stopResource_operations :
    (stopResource world vmId vm).operations = world.operations := rfl

def publishPaused (world : World) (sid : SandboxId) (record : SandboxRecord)
    (source : RestoreSource) : World :=
  let base := { record with
    phase := .paused
    currentVm := none
    durableSnapshot := some source
    restoreFrom := none
    netSlot := none
    currentOperation := record.currentOperation.map
      (fun (op : Operation) => { op with status := .succeeded })
  }
  let authority := .paused sid source
  let paused := { base with desired := authority, effective := authority }
  let published := setSandbox world sid paused
  updateCurrentOperation published sid paused.currentOperation (some (.paused source))

/-- Resume atomically selects its source and allocates a new epoch and generation. -/
def beginResume (world : World) (sid : SandboxId) (record : SandboxRecord)
    (operationId : OperationId) (source : RestoreSource) : World :=
  let epoch := record.epoch + 1
  let generation := record.generation + 1
  let operation : Operation := ⟨operationId, epoch, .resume, .pending⟩
  let updated := { record with
    phase := .starting
    epoch := epoch
    generation := generation
    currentVm := none
    restoreFrom := some source
    netSlot := none
    desired := .closed
    effective := .closed
    currentOperation := some operation
  }
  let persisted := storeNewOperation (setSandbox world sid updated) sid operation
    (.resume source)
  enqueue persisted (.installFence sid epoch false)

/-- Kill writes the irreversible tombstone before asynchronous resource cleanup. -/
def beginKill (world : World) (sid : SandboxId) (record : SandboxRecord)
    (operationId : OperationId) : World :=
  let epoch := record.epoch + 1
  let operation : Operation := ⟨operationId, epoch, .kill, .pending⟩
  let updated := { record with
    phase := .revoking
    terminal := true
    epoch := epoch
    desired := .closed
    effective := record.effective.withoutResume
    currentOperation := some operation
  }
  let persisted := storeNewOperation (setSandbox world sid updated) sid operation
    (.kill record.generation)
  enqueue persisted (.installFence sid epoch true)

def publishDestroyed (world : World) (sid : SandboxId) (record : SandboxRecord) : World :=
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
  let published := setSandbox world sid destroyed
  updateCurrentOperation published sid destroyed.currentOperation (some .destroyed)

def markOutcomeUnknown (world : World) (sid : SandboxId) (record : SandboxRecord)
    (operation : Operation) : World :=
  let operation := { operation with status := .outcomeUnknown }
  updateOperation (setSandbox world sid { record with
    currentOperation := some operation
  }) sid operation none

def reclaimVm (world : World) (vmId : VmId) (vm : Vm) : World :=
  setVm world vmId { vm with status := .reclaimed }

def reclaimSlot (world : World) (slotId : NetSlotId) (slot : NetSlot) : World :=
  setNetSlot world slotId { slot with active := false, reclaimed := true }

@[simp] theorem reclaimVm_vms :
    (reclaimVm world vmId vm).vms =
      put world.vms vmId { vm with status := .reclaimed } := rfl

@[simp] theorem reclaimSlot_netSlots :
    (reclaimSlot world slotId slot).netSlots =
      put world.netSlots slotId { slot with active := false, reclaimed := true } := rfl

@[simp] theorem reclaimVm_sandboxes :
    (reclaimVm world vmId vm).sandboxes = world.sandboxes := rfl

@[simp] theorem reclaimVm_snapshots :
    (reclaimVm world vmId vm).snapshots = world.snapshots := rfl

@[simp] theorem reclaimVm_netSlots :
    (reclaimVm world vmId vm).netSlots = world.netSlots := rfl

@[simp] theorem reclaimVm_filesystems :
    (reclaimVm world vmId vm).filesystems = world.filesystems := rfl

@[simp] theorem reclaimVm_workerFences :
    (reclaimVm world vmId vm).workerFences = world.workerFences := rfl

@[simp] theorem reclaimVm_operations :
    (reclaimVm world vmId vm).operations = world.operations := rfl

@[simp] theorem reclaimVm_appliedFilePolicies :
    (reclaimVm world vmId vm).appliedFilePolicies = world.appliedFilePolicies := rfl

@[simp] theorem reclaimSlot_sandboxes :
    (reclaimSlot world slotId slot).sandboxes = world.sandboxes := rfl

@[simp] theorem reclaimSlot_vms :
    (reclaimSlot world slotId slot).vms = world.vms := rfl

@[simp] theorem reclaimSlot_snapshots :
    (reclaimSlot world slotId slot).snapshots = world.snapshots := rfl

@[simp] theorem reclaimSlot_filesystems :
    (reclaimSlot world slotId slot).filesystems = world.filesystems := rfl

@[simp] theorem reclaimSlot_workerFences :
    (reclaimSlot world slotId slot).workerFences = world.workerFences := rfl

@[simp] theorem reclaimSlot_operations :
    (reclaimSlot world slotId slot).operations = world.operations := rfl

@[simp] theorem reclaimSlot_appliedFilePolicies :
    (reclaimSlot world slotId slot).appliedFilePolicies = world.appliedFilePolicies := rfl

def noExecutingVm (world : World) (sid : SandboxId) : Prop :=
  ∀ vmId vm, world.vms vmId = some vm → vm.sandbox = sid → vm.status ≠ .executing

def noActiveSlot (world : World) (sid : SandboxId) : Prop :=
  ∀ slotId slot, world.netSlots slotId = some slot → slot.sandbox = sid → slot.active = false

def usableSnapshot (world : World) (sid : SandboxId) (record : SandboxRecord)
    (source : RestoreSource) : Prop :=
  ∃ snapshot, world.snapshots source.snapshot = some snapshot ∧
    snapshot.owner = record.owner ∧ snapshot.sandbox = sid ∧
    snapshot.workspace = record.workspace ∧ snapshot.manifest = source.manifest ∧
    snapshot.stage = .durable ∧ snapshot.immutable = true

def commandResultPresent (world : World) : Command → Prop
  | .installFence sid epoch terminal =>
      world.workerFences sid = some ⟨epoch, terminal⟩
  | .prepareVm sid _ generation vmId slotId _ =>
      ∃ vm slot, world.vms vmId = some vm ∧ world.netSlots slotId = some slot ∧
        vm.sandbox = sid ∧ vm.generation = generation ∧
        slot.sandbox = sid ∧ slot.generation = generation ∧ slot.vm = vmId
  | .activateVm sid _ generation vmId =>
      ∃ vm, world.vms vmId = some vm ∧ vm.sandbox = sid ∧
        vm.generation = generation ∧ vm.status = .executing
  | .freezeVm sid _ generation vmId =>
      ∃ vm, world.vms vmId = some vm ∧ vm.sandbox = sid ∧
        vm.generation = generation ∧ vm.status = .frozen
  | .applyFilePolicy sid _ generation workspace policy =>
      world.appliedFilePolicies sid = some ⟨sid, generation, workspace, policy⟩
  | .cleanupVm sid _ generation vmId =>
      ∃ vm, world.vms vmId = some vm ∧ vm.sandbox = sid ∧
        vm.generation = generation ∧ vm.status = .reclaimed
  | .cleanupSlot sid _ generation slotId =>
      ∃ slot, world.netSlots slotId = some slot ∧ slot.sandbox = sid ∧
        slot.generation = generation ∧ slot.reclaimed = true

inductive Step : World → Event → World → Prop where
  | acceptCreate
      (up : world.controllerUp = true)
      (freshSandbox : world.sandboxes sid = none)
      (freshWorkspace : world.filesystems workspace = none)
      (freshOperation : world.operations (operationKey sid operationId) = none)
      (auth : authorized actor owner) :
      Step world (.acceptCreate actor owner sid workspace filePolicy policy operationId)
        (beginCreate world sid owner workspace filePolicy policy operationId)

  | retryOperation
      (foundSandbox : world.sandboxes sid = some record)
      (auth : authorized actor record.owner)
      (foundOperation : world.operations (operationKey sid operationId) =
        some ledger)
      (sameKind : ledger.operation.kind = kind)
      (sameInput : ledger.input = input) :
      Step world (.retryOperation actor sid operationId kind input) world

  | queryOperation
      (foundSandbox : world.sandboxes sid = some record)
      (auth : authorized actor record.owner)
      (foundOperation : world.operations (operationKey sid operationId) =
        some ledger)
      (observedStatus : status = ledger.operation.status)
      (observedResult : result = ledger.result) :
      Step world (.queryOperation actor sid operationId status result) world

  | dispatchEffect
      (up : world.controllerUp = true)
      (committed : world.effects requestedId = some .committed) :
      Step world (.dispatchEffect requestedId)
        (dispatchEffect world requestedId)

  | acknowledgeEffect
      (dispatched : world.effects requestedId = some .dispatched)
      (resultPresent : commandResultPresent world requestedId.command) :
      Step world (.acknowledgeEffect requestedId result)
        (acknowledgeEffect world requestedId result)

  | installStartFence
      (queued : world.inflight (.installFence sid record.epoch record.terminal))
      (dispatched : effectDispatched world
        (.installFence sid record.epoch record.terminal))
      (found : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (monotone : fenceCanAdvance (world.workerFences sid) record.epoch false) :
      Step world (.installFence sid) (applyStartFence world sid record)

  | installRevocationFence
      (queued : world.inflight (.installFence sid record.epoch record.terminal))
      (dispatched : effectDispatched world
        (.installFence sid record.epoch record.terminal))
      (found : world.sandboxes sid = some record)
      (revoking : record.phase = .pausing ∨ record.phase = .revoking)
      (monotone : fenceCanAdvance (world.workerFences sid)
        record.epoch record.terminal)
      (slotIdentity : ∀ slotId, record.netSlot = some slotId →
        ∃ slot, world.netSlots slotId = some slot ∧ slot.sandbox = sid) :
      Step world (.installFence sid) (applyRevocationFence world sid record)

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
      (dispatched : effectDispatched world
        (.prepareVm sid record.epoch record.generation vmId slotId snapshotId))
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

  | applyFilePolicy
      (queued : world.inflight (.applyFilePolicy sid record.epoch
        record.generation record.workspace record.filePolicy))
      (dispatched : effectDispatched world (.applyFilePolicy sid record.epoch
        record.generation record.workspace record.filePolicy))
      (found : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩) :
      Step world (.applyFilePolicy sid record.generation)
        (applyFilePolicyResult world sid record)

  | activateVm
      (queued : world.inflight (.activateVm sid record.epoch record.generation vmId))
      (dispatched : effectDispatched world
        (.activateVm sid record.epoch record.generation vmId))
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
      (exclusiveNetwork : noActiveSlot world sid)
      (appliedPolicy : world.appliedFilePolicies sid = some
        ⟨sid, record.generation, record.workspace, record.filePolicy⟩)
      (policyAcknowledged : effectAcknowledged world
        (.applyFilePolicy sid record.epoch record.generation
          record.workspace record.filePolicy))
      (currentOperation : record.currentOperation = some operation)
      (persistedOperation : world.operations (operationKey sid operation.id) =
        some ledger)
      (operationMatches : ledger.sandbox = sid ∧ ledger.operation = operation ∧
        ledger.result = none)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩) :
      Step world (.publishRunning sid) (publishRunning world sid record slotId slot)

  | acceptPause
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (running : record.phase = .running)
      (live : record.terminal = false)
      (current : record.currentVm = some vmId)
      (freshOperation : world.operations (operationKey sid operationId) = none)
      (auth : authorized actor record.owner) :
      Step world (.acceptPause actor sid operationId)
        (beginPause world sid record operationId vmId)

  | freezeVm
      (queued : world.inflight (.freezeVm sid record.epoch record.generation vmId))
      (dispatched : effectDispatched world
        (.freezeVm sid record.epoch record.generation vmId))
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
      Step world (.sealSnapshot sid snapshotId manifest)
        (sealSnapshot world snapshotId manifest record sid)

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

  | scheduleCleanupVm
      (up : world.controllerUp = true)
      (foundVm : world.vms vmId = some vm)
      (identity : vm.sandbox = sid ∧ vm.generation = generation)
      (stopped : vm.status = .stopped)
      (notCurrent : ∀ record, world.sandboxes sid = some record →
        record.currentVm ≠ some vmId) :
      Step world (.scheduleCleanupVm sid generation vmId)
        (enqueue world (.cleanupVm sid epoch generation vmId))

  | scheduleCleanupSlot
      (up : world.controllerUp = true)
      (foundSlot : world.netSlots slotId = some slot)
      (identity : slot.sandbox = sid ∧ slot.generation = generation)
      (inactive : slot.active = false)
      (notReclaimed : slot.reclaimed = false)
      (notCurrent : ∀ record, world.sandboxes sid = some record →
        record.netSlot ≠ some slotId) :
      Step world (.scheduleCleanupSlot sid generation slotId)
        (enqueue world (.cleanupSlot sid epoch generation slotId))

  | cleanupVm
      (queued : world.inflight (.cleanupVm sid epoch generation vmId))
      (dispatched : effectDispatched world (.cleanupVm sid epoch generation vmId))
      (foundVm : world.vms vmId = some vm)
      (identity : vm.sandbox = sid ∧ vm.generation = generation)
      (stopped : vm.status = .stopped)
      (notCurrent : ∀ record, world.sandboxes sid = some record →
        record.currentVm ≠ some vmId) :
      Step world (.cleanupVm sid generation vmId) (reclaimVm world vmId vm)

  | cleanupSlot
      (queued : world.inflight (.cleanupSlot sid epoch generation slotId))
      (dispatched : effectDispatched world (.cleanupSlot sid epoch generation slotId))
      (foundSlot : world.netSlots slotId = some slot)
      (identity : slot.sandbox = sid ∧ slot.generation = generation)
      (inactive : slot.active = false)
      (notReclaimed : slot.reclaimed = false)
      (notCurrent : ∀ record, world.sandboxes sid = some record →
        record.netSlot ≠ some slotId) :
      Step world (.cleanupSlot sid generation slotId)
        (reclaimSlot world slotId slot)

  | publishPaused
      (up : world.controllerUp = true)
      (foundSandbox : world.sandboxes sid = some record)
      (pausing : record.phase = .pausing)
      (foundSnapshot : world.snapshots source.snapshot = some snapshot)
      (snapshotReady : snapshot.owner = record.owner ∧ snapshot.sandbox = sid ∧
        snapshot.sourceGeneration = record.generation ∧ snapshot.workspace = record.workspace ∧
        snapshot.manifest = source.manifest ∧
        snapshot.stage = .durable ∧ snapshot.immutable = true)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩)
      (quiet : noExecutingVm world sid)
      (networkClosed : noActiveSlot world sid)
      (currentOperation : record.currentOperation = some operation)
      (persistedOperation : world.operations (operationKey sid operation.id) =
        some ledger)
      (operationMatches : ledger.sandbox = sid ∧ ledger.operation = operation ∧
        ledger.result = none) :
      Step world (.publishPaused sid source.snapshot)
        (publishPaused world sid record source)

  | acceptResume
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (paused : record.phase = .paused)
      (live : record.terminal = false)
      (currentSnapshot : record.durableSnapshot = some source)
      (sourceValid : usableSnapshot world sid record source)
      (freshOperation : world.operations (operationKey sid operationId) = none)
      (auth : authorized actor record.owner) :
      Step world (.acceptResume actor sid operationId)
        (beginResume world sid record operationId source)

  | acceptKill
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (notDestroyed : record.phase ≠ .destroyed)
      (freshOperation : world.operations (operationKey sid operationId) = none)
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
      (networkClosed : noActiveSlot world sid)
      (currentOperation : record.currentOperation = some operation)
      (persistedOperation : world.operations (operationKey sid operation.id) =
        some ledger)
      (operationMatches : ledger.sandbox = sid ∧ ledger.operation = operation ∧
        ledger.result = none) :
      Step world (.publishDestroyed sid) (publishDestroyed world sid record)

  | rpcTimeout
      (foundSandbox : world.sandboxes sid = some record)
      (current : record.currentOperation = some operation)
      (pending : operation.status = .pending)
      (persistedOperation : world.operations (operationKey sid operation.id) =
        some ledger)
      (operationMatches : ledger.sandbox = sid ∧ ledger.operation = operation ∧
        ledger.result = none) :
      Step world (.rpcTimeout sid) (markOutcomeUnknown world sid record operation)

  | controllerCrash (up : world.controllerUp = true) :
      Step world .controllerCrash { world with controllerUp := false }

  | controllerRecover (down : world.controllerUp = false) :
      Step world .controllerRecover { world with controllerUp := true }

  | duplicateOrLateDelivery
      (persisted : world.effects (effectId command) = some effect) :
      Step world (.duplicateOrLateDelivery command) world

  | exec
      (found : world.sandboxes sid = some record)
      (auth : authorized actor record.owner)
      (capability : record.effective.allows (.exec sid record.generation vmId)) :
      Step world (.exec actor sid) world

  | fileRead
      (found : world.sandboxes sid = some record)
      (running : record.phase = .running)
      (currentGeneration : generation = record.generation)
      (auth : authorized actor record.owner)
      (capability : record.effective.allows
        (.fileRead sid generation record.workspace record.workspaceAccess)) :
      Step world (.fileRead actor sid generation) world

  | fileWrite
      (found : world.sandboxes sid = some record)
      (running : record.phase = .running)
      (currentGeneration : generation = record.generation)
      (auth : authorized actor record.owner)
      (capability : record.effective.allows
        (.fileWrite sid generation record.workspace record.workspaceAccess)) :
      Step world (.fileWrite actor sid generation) world

  | ingress
      (found : world.sandboxes sid = some record)
      (auth : authorized actor record.owner)
      (capability : record.effective.allows
        (.ingress sid record.generation vmId slotId)) :
      Step world (.ingress actor sid) world

  | egress
      (found : world.sandboxes sid = some record)
      (capability : record.effective.allows
        (.egress sid record.generation vmId slotId record.policy))
      (allowed : record.policy = .staticAllowlist endpoints ∧ destination ∈ endpoints) :
      Step world (.egress sid destination) world

inductive Reachable : World → Prop where
  | initial : Reachable emptyWorld
  | step : Reachable world → Step world event next → Reachable next

end SandboxModel
