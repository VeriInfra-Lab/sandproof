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

@[simp] theorem operationKey_sandbox :
    (operationKey sid operationId).sandbox = sid := rfl

@[simp] theorem operationKey_operation :
    (operationKey sid operationId).operation = operationId := rfl

def initialOperationPlan (world : World) (sid : SandboxId)
    (operation : Operation) (input : OperationInput) : OperationPlan :=
  let index := world.resourceIndexes sid
  {
    key := operationKey sid operation.id
    epoch := operation.epoch
    generation := match input with
      | .create _ _ _ => 1
      | .pause generation | .kill generation => generation
      | .resume _ =>
          match world.sandboxes sid with
          | some record => record.generation
          | none => 0
    vm := none
    slot := none
    restoreFrom := match input with
      | .resume source => some source
      | _ => none
    targetSnapshot := none
    stopVms := index.vms
    closeSlots := index.slots
    retryLimit := defaultRetryLimit
    sealed := operation.kind = .kill
  }

def storeNewOperation (world : World) (sid : SandboxId) (operation : Operation)
    (input : OperationInput) : World :=
  let key := operationKey sid operation.id
  let stored := setOperation world key ⟨sid, operation, input, none⟩
  setOperationPlan stored key (initialOperationPlan world sid operation input)

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

/-- Finalize the operation that a kill request replaces.  Only genuinely
    unfinished operations with no stored result are changed; already-final
    ledger entries remain immutable. -/
def supersedeCurrentOperation (world : World) (sid : SandboxId) :
    Option Operation → World
  | none => world
  | some operation =>
      match operation.status with
      | .pending | .outcomeUnknown =>
          let key := operationKey sid operation.id
          match world.operations key with
          | some ledger =>
              match ledger.result with
              | none =>
                  if ledger.operation = operation then
                    updateOperation world sid
                      { operation with status := .failed .cancelled }
                      (some (.failed .cancelled))
                  else world
              | some _ => world
          | none => world
      | .compensating _ | .succeeded | .failed _ => world

/-- The old operation must be present and exactly match the sandbox record
    before kill can replace an unfinished lifecycle decision. -/
def SupersedeReady (world : World) (sid : SandboxId) :
    Option Operation → Prop
  | none => True
  | some operation =>
      match operation.status with
      | .pending | .outcomeUnknown =>
          ∃ ledger,
            world.operations (operationKey sid operation.id) = some ledger ∧
            ledger.operation = operation ∧ ledger.result = none
      | .compensating _ | .succeeded | .failed _ => True

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

@[simp] theorem storeNewOperation_operationPlans :
    (storeNewOperation world sid operation input).operationPlans =
      put world.operationPlans (operationKey sid operation.id)
        (initialOperationPlan world sid operation input) := by
  simp [storeNewOperation]

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

@[simp] theorem updateOperation_operationPlans :
    (updateOperation world sid operation result).operationPlans =
      world.operationPlans := rfl

private theorem supersedeCurrentOperation_preservesField
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

@[simp] theorem supersedeCurrentOperation_sandboxes :
    (supersedeCurrentOperation world sid current).sandboxes = world.sandboxes :=
  supersedeCurrentOperation_preservesField World.sandboxes (by intros; rfl)

@[simp] theorem supersedeCurrentOperation_vms :
    (supersedeCurrentOperation world sid current).vms = world.vms :=
  supersedeCurrentOperation_preservesField World.vms (by intros; rfl)

@[simp] theorem supersedeCurrentOperation_filesystems :
    (supersedeCurrentOperation world sid current).filesystems = world.filesystems :=
  supersedeCurrentOperation_preservesField World.filesystems (by intros; rfl)

@[simp] theorem supersedeCurrentOperation_snapshots :
    (supersedeCurrentOperation world sid current).snapshots = world.snapshots :=
  supersedeCurrentOperation_preservesField World.snapshots (by intros; rfl)

@[simp] theorem supersedeCurrentOperation_netSlots :
    (supersedeCurrentOperation world sid current).netSlots = world.netSlots :=
  supersedeCurrentOperation_preservesField World.netSlots (by intros; rfl)

@[simp] theorem supersedeCurrentOperation_appliedFilePolicies :
    (supersedeCurrentOperation world sid current).appliedFilePolicies =
      world.appliedFilePolicies :=
  supersedeCurrentOperation_preservesField World.appliedFilePolicies
    (by intros; rfl)

@[simp] theorem supersedeCurrentOperation_workerFences :
    (supersedeCurrentOperation world sid current).workerFences =
      world.workerFences :=
  supersedeCurrentOperation_preservesField World.workerFences (by intros; rfl)

@[simp] theorem supersedeCurrentOperation_effects :
    (supersedeCurrentOperation world sid current).effects = world.effects :=
  supersedeCurrentOperation_preservesField World.effects (by intros; rfl)

@[simp] theorem supersedeCurrentOperation_inflight :
    (supersedeCurrentOperation world sid current).inflight = world.inflight :=
  supersedeCurrentOperation_preservesField World.inflight (by intros; rfl)

@[simp] theorem supersedeCurrentOperation_controllerUp :
    (supersedeCurrentOperation world sid current).controllerUp =
      world.controllerUp :=
  supersedeCurrentOperation_preservesField World.controllerUp (by intros; rfl)

@[simp] theorem supersedeCurrentOperation_operationPlans :
    (supersedeCurrentOperation world sid current).operationPlans =
      world.operationPlans :=
  supersedeCurrentOperation_preservesField World.operationPlans (by intros; rfl)

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

@[simp] theorem updateCurrentOperation_operationPlans :
    (updateCurrentOperation world sid operation result).operationPlans =
      world.operationPlans := by
  cases operation <;> simp [updateCurrentOperation]

def effectHasStatus (world : World) (command : Command)
    (status : EffectStatus) : Prop :=
  world.effects (currentEffectId world command) = some status

def effectDispatched (world : World) (command : Command) : Prop :=
  effectHasStatus world command .dispatched

def effectAwaiting (world : World) (command : Command) : Prop :=
  effectHasStatus world command .dispatched ∨
    effectHasStatus world command .outcomeUnknown

def effectAcknowledged (world : World) (command : Command) : Prop :=
  ∃ outcome, effectHasStatus world command (.acknowledged outcome)

def effectSucceeded (world : World) (command : Command) : Prop :=
  ∃ result, effectHasStatus world command (.acknowledged (.succeeded result))

/-- Transport membership names the exact current attempt. -/
def deliveryQueued (world : World) (command : Command) : Prop :=
  world.inflight (currentEffectId world command)

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

@[simp] theorem applyStartFence_operationPlans :
    (applyStartFence world sid record).operationPlans = world.operationPlans := rfl

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

@[simp] theorem applyRevocationFence_operationPlans :
    (applyRevocationFence world sid record).operationPlans =
      world.operationPlans := by
  cases slotChoice : record.netSlot with
  | none => simp [applyRevocationFence, slotChoice]
  | some slotId =>
      cases slotFound : world.netSlots slotId <;>
        simp [applyRevocationFence, slotChoice, slotFound]

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
  let configure := .configureNetwork sid record.epoch record.generation vmId
    slotId record.policy
  enqueue (enqueue (enqueue prepared configure) activate) applyPolicy

@[simp] theorem prepareResources_operations :
    (prepareResources world sid record vmId slotId).operations = world.operations := by
  simp [prepareResources, setSandbox, setNetSlot, setVm]

@[simp] theorem prepareResources_operationPlans :
    (prepareResources world sid record vmId slotId).operationPlans =
      world.operationPlans := by
  simp [prepareResources]

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

@[simp] theorem applyFilePolicyResult_operationPlans :
    (applyFilePolicyResult world sid record).operationPlans =
      world.operationPlans := rfl

def configureSlot (world : World) (slotId : NetSlotId) (slot : NetSlot) : World :=
  setNetSlot world slotId { slot with configured := true }

@[simp] theorem configureSlot_operations :
    (configureSlot world slotId slot).operations = world.operations := rfl

@[simp] theorem configureSlot_operationPlans :
    (configureSlot world slotId slot).operationPlans = world.operationPlans := rfl

def activateResource (world : World) (vmId : VmId) (vm : Vm) : World :=
  setVm world vmId { vm with status := .executing }

@[simp] theorem activateResource_operations :
    (activateResource world vmId vm).operations = world.operations := rfl

@[simp] theorem activateResource_operationPlans :
    (activateResource world vmId vm).operationPlans = world.operationPlans := rfl

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

@[simp] theorem publishRunning_operationPlans :
    (publishRunning world sid record slotId slot).operationPlans =
      world.operationPlans := by
  simp [publishRunning]

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

@[simp] theorem sealSnapshot_operationPlans :
    (sealSnapshot world snapshotId manifest record sid).operationPlans =
      world.operationPlans := rfl

/-- Durability and immutability are committed together. -/
def makeSnapshotDurable (world : World) (snapshotId : SnapshotId)
    (snapshot : Snapshot) : World :=
  setSnapshot world snapshotId { snapshot with stage := .durable, immutable := true }

@[simp] theorem makeSnapshotDurable_operations :
    (makeSnapshotDurable world snapshotId snapshot).operations = world.operations := rfl

@[simp] theorem makeSnapshotDurable_operationPlans :
    (makeSnapshotDurable world snapshotId snapshot).operationPlans =
      world.operationPlans := rfl

def stopResource (world : World) (vmId : VmId) (vm : Vm) : World :=
  setVm world vmId { vm with status := .stopped }

@[simp] theorem stopResource_operations :
    (stopResource world vmId vm).operations = world.operations := rfl

@[simp] theorem stopResource_operationPlans :
    (stopResource world vmId vm).operationPlans = world.operationPlans := rfl

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

@[simp] theorem publishPaused_operationPlans :
    (publishPaused world sid record source).operationPlans =
      world.operationPlans := by
  simp [publishPaused]

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
  let superseded := supersedeCurrentOperation world sid record.currentOperation
  let persisted := storeNewOperation (setSandbox superseded sid updated) sid operation
    (.kill record.generation)
  enqueue persisted (.installFence sid epoch true)

/-- Convert a failed create/resume into an explicit terminal compensation
    protocol.  The original operation identity is retained; its status records
    why compensation began.  Installing the terminal fence before publication
    makes every previously issued start command stale through the record's
    terminal guard. -/
def beginCompensation (world : World) (sid : SandboxId)
    (record : SandboxRecord) (operation : Operation)
    (failure : FailureKind) : World :=
  let compensating := { operation with status := .compensating failure }
  let updated := { record with
    phase := .revoking
    terminal := true
    desired := .closed
    effective := .closed
    currentOperation := some compensating
  }
  let persisted := updateOperation (setSandbox world sid updated) sid
    compensating none
  enqueue persisted (.installFence sid record.epoch true)

@[simp] theorem beginCompensation_operationPlans
    (world : World) (sid : SandboxId) (record : SandboxRecord)
    (operation : Operation) (failure : FailureKind) :
    (beginCompensation world sid record operation failure).operationPlans =
      world.operationPlans := by
  simp [beginCompensation]

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

@[simp] theorem publishDestroyed_operationPlans :
    (publishDestroyed world sid record).operationPlans = world.operationPlans := by
  simp [publishDestroyed]

/-- Finish terminal compensation.  Unlike a successful kill, the sandbox is
    destroyed with the original create/resume operation finalized as failed. -/
def publishFailure (world : World) (sid : SandboxId) (record : SandboxRecord)
    (failure : FailureKind) : World :=
  let destroyed := { record with
    phase := .destroyed
    terminal := true
    currentVm := none
    netSlot := none
    desired := .closed
    effective := .closed
    currentOperation := record.currentOperation.map
      (fun (op : Operation) => { op with status := .failed failure })
  }
  let published := setSandbox world sid destroyed
  updateCurrentOperation published sid destroyed.currentOperation
    (some (.failed failure))

@[simp] theorem publishFailure_operationPlans
    (world : World) (sid : SandboxId) (record : SandboxRecord)
    (failure : FailureKind) :
    (publishFailure world sid record failure).operationPlans =
      world.operationPlans := by
  simp [publishFailure]

def markOutcomeUnknown (world : World) (sid : SandboxId) (record : SandboxRecord)
    (operation : Operation) : World :=
  let operation := { operation with status := .outcomeUnknown }
  updateOperation (setSandbox world sid { record with
    currentOperation := some operation
  }) sid operation none

@[simp] theorem markOutcomeUnknown_operationPlans :
    (markOutcomeUnknown world sid record operation).operationPlans =
      world.operationPlans := by
  simp [markOutcomeUnknown]

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

@[simp] theorem reclaimVm_operationPlans :
    (reclaimVm world vmId vm).operationPlans = world.operationPlans := rfl

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

@[simp] theorem reclaimSlot_operationPlans :
    (reclaimSlot world slotId slot).operationPlans = world.operationPlans := rfl

@[simp] theorem reclaimSlot_appliedFilePolicies :
    (reclaimSlot world slotId slot).appliedFilePolicies = world.appliedFilePolicies := rfl

def noExecutingVm (world : World) (sid : SandboxId) : Prop :=
  ∀ vmId vm, world.vms vmId = some vm → vm.sandbox = sid → vm.status ≠ .executing

/-- A stable lifecycle state may no longer hide a VM that still needs a stop
    operation.  `noExecutingVm` is sufficient for authority safety, but it is
    too weak for resource-accounting safety: prepared and frozen VMs are also
    live resources. -/
def allVmsStoppedOrReclaimed (world : World) (sid : SandboxId) : Prop :=
  ∀ vmId vm, world.vms vmId = some vm → vm.sandbox = sid →
    vm.status = .stopped ∨ vm.status = .reclaimed

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
      ∃ fence, world.workerFences sid = some fence ∧ epoch ≤ fence.epoch ∧
        (terminal = true → fence.terminal = true)
  | .prepareVm sid _ generation vmId slotId _ =>
      ∃ vm slot, world.vms vmId = some vm ∧ world.netSlots slotId = some slot ∧
        vm.sandbox = sid ∧ vm.generation = generation ∧
        slot.sandbox = sid ∧ slot.generation = generation ∧ slot.vm = vmId
  | .configureNetwork sid _ generation vmId slotId policy =>
      ∃ slot, world.netSlots slotId = some slot ∧ slot.sandbox = sid ∧
        slot.generation = generation ∧ slot.vm = vmId ∧
        slot.policy = policy ∧ slot.configured = true
  | .activateVm sid _ generation vmId =>
      ∃ vm, world.vms vmId = some vm ∧ vm.sandbox = sid ∧
        vm.generation = generation ∧
        (vm.status = .executing ∨ vm.status = .frozen ∨
          vm.status = .stopped ∨ vm.status = .reclaimed)
  | .freezeVm sid _ generation vmId =>
      ∃ vm, world.vms vmId = some vm ∧ vm.sandbox = sid ∧
        vm.generation = generation ∧
        (vm.status = .frozen ∨ vm.status = .stopped ∨
          vm.status = .reclaimed)
  | .applyFilePolicy sid _ generation workspace policy =>
      ∃ applied, world.appliedFilePolicies sid = some applied ∧
        applied.sandbox = sid ∧ generation ≤ applied.generation ∧
        applied.workspace = workspace ∧ applied.policy = policy
  | .sealSnapshot sid _ generation _ workspace snapshotId manifest =>
      ∃ snapshot, world.snapshots snapshotId = some snapshot ∧
        snapshot.sandbox = sid ∧ snapshot.sourceGeneration = generation ∧
        snapshot.workspace = workspace ∧ snapshot.manifest = manifest
  | .commitSnapshot sid _ generation snapshotId manifest =>
      ∃ snapshot, world.snapshots snapshotId = some snapshot ∧
        snapshot.sandbox = sid ∧ snapshot.sourceGeneration = generation ∧
        snapshot.manifest = manifest ∧ snapshot.stage = .durable ∧
        snapshot.immutable = true
  | .stopVm sid _ generation vmId =>
      ∃ vm, world.vms vmId = some vm ∧ vm.sandbox = sid ∧
        vm.generation = generation ∧
        (vm.status = .stopped ∨ vm.status = .reclaimed)
  | .cleanupVm sid generation vmId =>
      ∃ vm, world.vms vmId = some vm ∧ vm.sandbox = sid ∧
        vm.generation = generation ∧ vm.status = .reclaimed
  | .cleanupSlot sid generation slotId =>
      ∃ slot, world.netSlots slotId = some slot ∧ slot.sandbox = sid ∧
        slot.generation = generation ∧ slot.reclaimed = true

def stoppableVmStatus : VmStatus → Prop
  | .prepared | .executing | .frozen => True
  | .stopped | .reclaimed => False

def vmStopSucceeded (world : World) (sid : SandboxId)
    (generation : Generation) (vmId : VmId) : Prop :=
  ∃ epoch result, effectHasStatus world (.stopVm sid epoch generation vmId)
    (.acknowledged (.succeeded result))

def allVmStopsSucceeded (world : World) (sid : SandboxId) : Prop :=
  ∀ vmId vm, world.vms vmId = some vm → vm.sandbox = sid →
    vmStopSucceeded world sid vm.generation vmId

/-- Successful receipts must be backed by the corresponding worker state.
    In this all-or-nothing command model, a failed receipt is valid only while
    the command's success postcondition is absent.  Partial success would need
    its own explicit outcome constructor rather than a contradictory failure. -/
def effectOutcomeValid (world : World) (command : Command) : EffectOutcome → Prop
  | .succeeded _ => commandResultPresent world command
  | .failed _ => ¬ commandResultPresent world command

/-- A worker failure may justify compensation only when the failed command
    belongs to the sandbox and epoch of the operation being compensated.
    Generation-free cleanup commands never justify lifecycle compensation. -/
def commandBelongsToOperation (command : Command) (sid : SandboxId)
    (epoch : Epoch) : Prop :=
  match command with
  | .installFence commandSid commandEpoch _
  | .prepareVm commandSid commandEpoch _ _ _ _
  | .configureNetwork commandSid commandEpoch _ _ _ _
  | .activateVm commandSid commandEpoch _ _
  | .freezeVm commandSid commandEpoch _ _
  | .applyFilePolicy commandSid commandEpoch _ _ _
  | .sealSnapshot commandSid commandEpoch _ _ _ _ _
  | .commitSnapshot commandSid commandEpoch _ _ _
  | .stopVm commandSid commandEpoch _ _ =>
      commandSid = sid ∧ commandEpoch = epoch
  | .cleanupVm _ _ _ | .cleanupSlot _ _ _ => False

/-- A worker-caused compensation may start only after the exact current retry
    head has been finalized as failed.  The logical origin binds the failure
    to the current operation identity; the command epoch and exact attempt
    receipt retain the worker-side causal evidence. -/
def FinalFailedAttemptEvidence (world : World) (key : OperationKey)
    (epoch : Epoch) (failure : FailureKind) (id : EffectId) : Prop :=
  match world.logicalEffects id.logical with
  | none => False
  | some logical =>
      logical.origin = .operation key ∧
        id.attempt = logical.currentAttempt ∧
        logical.result = some (.failed id.attempt failure) ∧
        world.effects id = some (.acknowledged (.failed failure)) ∧
        commandBelongsToOperation id.command key.sandbox epoch

/-- Cancellation and a controller-owned deadline are local causes.  Every
    other failure must be the finalized current attempt of a logical effect
    owned by this exact operation. -/
def CompensationEvidence (world : World) (key : OperationKey) (epoch : Epoch)
    (failure : FailureKind) : Prop :=
  failure = .cancelled ∨ failure = .deadlineExceeded ∨
    ∃ id, FinalFailedAttemptEvidence world key epoch failure id

inductive Step : World → Event → World → Prop where
  | acceptCreate
      (up : world.controllerUp = true)
      (freshSandbox : world.sandboxes sid = none)
      (freshWorkspace : world.filesystems workspace = none)
      (freshOperation : world.operations (operationKey sid operationId) = none)
      (freshPlan : world.operationPlans (operationKey sid operationId) = none)
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
      (dispatchable : world.effects requestedId = some .committed ∨
        world.effects requestedId = some .outcomeUnknown) :
      Step world (.dispatchEffect requestedId)
        (dispatchEffect world requestedId)

  | acknowledgeEffect
      (awaiting : world.effects requestedId = some .dispatched ∨
        world.effects requestedId = some .outcomeUnknown)
      (outcomeValid : effectOutcomeValid world requestedId.command outcome) :
      Step world (.acknowledgeEffect requestedId outcome)
        (acknowledgeEffect world requestedId outcome)

  | replayEffectReceipt
      (recorded : world.effects requestedId = some (.acknowledged outcome)) :
      Step world (.acknowledgeEffect requestedId outcome) world

  | effectTimeout
      (dispatched : world.effects requestedId = some .dispatched) :
      Step world (.effectTimeout requestedId)
        (markEffectOutcomeUnknown world requestedId)

  | repeatEffectTimeout
      (unknown : world.effects requestedId = some .outcomeUnknown) :
      Step world (.effectTimeout requestedId) world

  | startNextAttempt
      (failure : FailureKind)
      (up : world.controllerUp = true)
      (found : world.logicalEffects logical = some record)
      (unfinished : record.result = none)
      (previousIdentity : previous = ⟨logical, record.currentAttempt⟩)
      (failed : world.effects previous = some (.acknowledged (.failed failure)))
      (retryable : failure.retryable = true)
      (budget : record.remainingRetries = remaining + 1)
      (nextIdentity : next = ⟨logical, record.currentAttempt + 1⟩)
      (fresh : world.effects next = none) :
      Step world (.startNextAttempt previous next)
        (startNextAttempt world logical failure)

  | finalizeEffectFailure
      (failure : FailureKind)
      (up : world.controllerUp = true)
      (found : world.logicalEffects logical = some record)
      (unfinished : record.result = none)
      (currentIdentity : current = ⟨logical, record.currentAttempt⟩)
      (failed : world.effects current = some (.acknowledged (.failed failure)))
      (terminal : failure.retryable = false ∨ record.remainingRetries = 0) :
      Step world (.finalizeEffectFailure current failure)
        (finalizeLogicalFailure world logical failure)

  | reserveStartupResources
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (currentOperation : record.currentOperation = some operation)
      (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
      (planKey : plan.key = operationKey sid operation.id)
      (planEpoch : plan.epoch = record.epoch)
      (planGeneration : plan.generation = record.generation)
      (planOpen : plan.sealed = false)
      (planVmOpen : plan.vm = none)
      (planSlotOpen : plan.slot = none)
      (planSource : plan.restoreFrom = record.restoreFrom)
      (freshVm : world.vms ⟨world.nextVmId⟩ = none)
      (freshSlot : world.netSlots ⟨world.nextSlotId⟩ = none) :
      Step world
        (.reserveStartupResources (operationKey sid operation.id)
          ⟨world.nextVmId⟩ ⟨world.nextSlotId⟩)
        (SandboxModel.reserveStartupResources world
          (operationKey sid operation.id) plan)

  | reservePauseSnapshot
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (pausing : record.phase = .pausing)
      (live : record.terminal = false)
      (currentOperation : record.currentOperation = some operation)
      (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
      (planKey : plan.key = operationKey sid operation.id)
      (planEpoch : plan.epoch = record.epoch)
      (planGeneration : plan.generation = record.generation)
      (planOpen : plan.sealed = false)
      (planVmOpen : plan.vm = none)
      (targetOpen : plan.targetSnapshot = none)
      (currentVm : record.currentVm = some vmId)
      (freshSnapshot : world.snapshots ⟨world.nextSnapshotId⟩ = none) :
      Step world
        (.reservePauseSnapshot (operationKey sid operation.id)
          ⟨world.nextSnapshotId⟩ ⟨⟨world.nextManifestDigest⟩⟩)
        (SandboxModel.reservePauseSnapshot world
          (operationKey sid operation.id) plan vmId)

  | installStartFence
      (queued : deliveryQueued world
        (.installFence sid record.epoch record.terminal))
      (dispatched : effectAwaiting world
        (.installFence sid record.epoch record.terminal))
      (found : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (monotone : fenceCanAdvance (world.workerFences sid) record.epoch false) :
      Step world (.installFence sid) (applyStartFence world sid record)

  | installRevocationFence
      (queued : deliveryQueued world
        (.installFence sid record.epoch record.terminal))
      (dispatched : effectAwaiting world
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
      (currentOperation : record.currentOperation = some operation)
      (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
      (planKey : plan.key = operationKey sid operation.id)
      (planEpoch : plan.epoch = record.epoch)
      (planGeneration : plan.generation = record.generation)
      (planSealed : plan.sealed = true)
      (plannedVm : plan.vm = some vmId)
      (plannedSlot : plan.slot = some slotId)
      (planSource : plan.restoreFrom = snapshotId)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩)
      (freshVm : world.vms vmId = none)
      (freshSlot : world.netSlots slotId = none)
      (selectedSource : snapshotId = record.restoreFrom)
      (resumeSource : ∀ source, snapshotId = some source →
        record.durableSnapshot = some source ∧ usableSnapshot world sid record source) :
      Step world (.dispatchPrepare sid vmId slotId snapshotId)
        (enqueue world
          (.prepareVm sid record.epoch record.generation vmId slotId snapshotId))

  | prepareVm
      (queued : deliveryQueued world
        (.prepareVm sid record.epoch record.generation vmId slotId snapshotId))
      (dispatched : effectAwaiting world
        (.prepareVm sid record.epoch record.generation vmId slotId snapshotId))
      (found : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (currentOperation : record.currentOperation = some operation)
      (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
      (planKey : plan.key = operationKey sid operation.id)
      (planEpoch : plan.epoch = record.epoch)
      (planGeneration : plan.generation = record.generation)
      (planSealed : plan.sealed = true)
      (plannedVm : plan.vm = some vmId)
      (plannedSlot : plan.slot = some slotId)
      (plannedSource : plan.restoreFrom = snapshotId)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩)
      (freshVm : world.vms vmId = none)
      (freshSlot : world.netSlots slotId = none)
      (selectedSource : snapshotId = record.restoreFrom)
      (resumeSource : ∀ source, snapshotId = some source →
        record.durableSnapshot = some source ∧ usableSnapshot world sid record source) :
      Step world (.prepareVm sid vmId slotId snapshotId)
        (prepareResources world sid record vmId slotId)

  | configureNetwork
      (queued : deliveryQueued world (.configureNetwork sid record.epoch
        record.generation vmId slotId record.policy))
      (dispatched : effectAwaiting world (.configureNetwork sid record.epoch
        record.generation vmId slotId record.policy))
      (foundSandbox : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (foundSlot : world.netSlots slotId = some slot)
      (current : record.netSlot = some slotId)
      (currentVm : record.currentVm = some vmId)
      (owned : slot.owner = record.owner ∧ slot.sandbox = sid ∧
        slot.generation = record.generation ∧ slot.vm = vmId ∧ slot.policy = record.policy)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩) :
      Step world (.configureNetwork sid slotId) (configureSlot world slotId slot)

  | applyFilePolicy
      (queued : deliveryQueued world (.applyFilePolicy sid record.epoch
        record.generation record.workspace record.filePolicy))
      (dispatched : effectAwaiting world (.applyFilePolicy sid record.epoch
        record.generation record.workspace record.filePolicy))
      (found : world.sandboxes sid = some record)
      (starting : record.phase = .starting)
      (live : record.terminal = false)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩) :
      Step world (.applyFilePolicy sid record.generation)
        (applyFilePolicyResult world sid record)

  | activateVm
      (queued : deliveryQueued world
        (.activateVm sid record.epoch record.generation vmId))
      (dispatched : effectAwaiting world
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
      (fenceSucceeded : effectSucceeded world
        (.installFence sid record.epoch false))
      (prepareSucceeded : effectSucceeded world
        (.prepareVm sid record.epoch record.generation vmId slotId
          record.restoreFrom))
      (networkSucceeded : effectSucceeded world
        (.configureNetwork sid record.epoch record.generation vmId slotId
          record.policy))
      (activationSucceeded : effectSucceeded world
        (.activateVm sid record.epoch record.generation vmId))
      (policyAcknowledged : effectSucceeded world
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
      (freshPlan : world.operationPlans (operationKey sid operationId) = none)
      (auth : authorized actor record.owner) :
      Step world (.acceptPause actor sid operationId)
        (beginPause world sid record operationId vmId)

  | freezeVm
      (queued : deliveryQueued world
        (.freezeVm sid record.epoch record.generation vmId))
      (dispatched : effectAwaiting world
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

  | scheduleSealSnapshot
      (up : world.controllerUp = true)
      (foundSandbox : world.sandboxes sid = some record)
      (pausing : record.phase = .pausing)
      (currentOperation : record.currentOperation = some operation)
      (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
      (planKey : plan.key = operationKey sid operation.id)
      (planEpoch : plan.epoch = record.epoch)
      (planGeneration : plan.generation = record.generation)
      (planSealed : plan.sealed = true)
      (plannedVm : plan.vm = some vmId)
      (plannedSnapshot : plan.targetSnapshot = some ⟨snapshotId, manifest⟩)
      (foundVm : world.vms vmId = some vm)
      (current : record.currentVm = some vmId)
      (vmIdentity : vm.owner = record.owner ∧ vm.sandbox = sid ∧
        vm.generation = record.generation)
      (frozen : vm.status = .frozen)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩)
      (freezeSucceeded : effectSucceeded world
        (.freezeVm sid record.epoch record.generation vmId))
      (fresh : world.snapshots snapshotId = none) :
      Step world (.scheduleSealSnapshot sid snapshotId manifest)
        (enqueue world
          (.sealSnapshot sid record.epoch record.generation vmId
            record.workspace snapshotId manifest))

  | sealSnapshot
      (queued : deliveryQueued world (.sealSnapshot sid record.epoch
        record.generation vmId record.workspace snapshotId manifest))
      (dispatched : effectAwaiting world (.sealSnapshot sid record.epoch
        record.generation vmId record.workspace snapshotId manifest))
      (foundSandbox : world.sandboxes sid = some record)
      (pausing : record.phase = .pausing)
      (currentOperation : record.currentOperation = some operation)
      (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
      (planKey : plan.key = operationKey sid operation.id)
      (planEpoch : plan.epoch = record.epoch)
      (planGeneration : plan.generation = record.generation)
      (planSealed : plan.sealed = true)
      (plannedVm : plan.vm = some vmId)
      (plannedSnapshot : plan.targetSnapshot = some ⟨snapshotId, manifest⟩)
      (foundVm : world.vms vmId = some vm)
      (current : record.currentVm = some vmId)
      (vmIdentity : vm.owner = record.owner ∧ vm.sandbox = sid ∧
        vm.generation = record.generation)
      (frozen : vm.status = .frozen)
      (fenced : world.workerFences sid = some ⟨record.epoch, false⟩)
      (fresh : world.snapshots snapshotId = none) :
      Step world (.sealSnapshot sid snapshotId manifest)
        (sealSnapshot world snapshotId manifest record sid)

  | scheduleCommitSnapshot
      (up : world.controllerUp = true)
      (foundSnapshot : world.snapshots snapshotId = some snapshot)
      (foundSandbox : world.sandboxes sid = some record)
      (snapshotSandbox : snapshot.sandbox = sid)
      (pausing : record.phase = .pausing)
      (current : record.currentVm = some vmId)
      (snapshotIdentity : snapshot.sourceGeneration = record.generation ∧
        snapshot.workspace = record.workspace)
      (sealed : snapshot.stage = .localSealed)
      (sealSucceeded : effectSucceeded world (.sealSnapshot sid record.epoch
        record.generation vmId record.workspace snapshotId snapshot.manifest)) :
      Step world (.scheduleCommitSnapshot sid snapshotId)
        (enqueue world (.commitSnapshot sid record.epoch record.generation
          snapshotId snapshot.manifest))

  | commitSnapshot
      (queued : deliveryQueued world (.commitSnapshot sid record.epoch
        record.generation snapshotId snapshot.manifest))
      (dispatched : effectAwaiting world (.commitSnapshot sid record.epoch
        record.generation snapshotId snapshot.manifest))
      (found : world.snapshots snapshotId = some snapshot)
      (foundSandbox : world.sandboxes sid = some record)
      (snapshotSandbox : snapshot.sandbox = sid)
      (pausing : record.phase = .pausing)
      (current : record.currentVm = some vmId)
      (snapshotIdentity : snapshot.sourceGeneration = record.generation ∧
        snapshot.workspace = record.workspace)
      (sealed : snapshot.stage = .localSealed) :
      Step world (.commitSnapshot snapshotId)
        (enqueue (makeSnapshotDurable world snapshotId snapshot)
          (.stopVm sid record.epoch record.generation vmId))

  | scheduleStopVm
      (up : world.controllerUp = true)
      (foundSandbox : world.sandboxes sid = some record)
      (revoking : record.phase = .revoking)
      (terminal : record.terminal = true)
      (currentOperation : record.currentOperation = some operation)
      (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
      (planKey : plan.key = operationKey sid operation.id)
      (planEpoch : plan.epoch = record.epoch)
      (planGeneration : plan.generation = record.generation)
      (planned : vmId ∈ plan.stopVms)
      (fenced : world.workerFences sid = some ⟨record.epoch, true⟩)
      (fenceSucceeded : effectSucceeded world
        (.installFence sid record.epoch true))
      (foundVm : world.vms vmId = some vm)
      (identity : vm.sandbox = sid ∧ vm.generation = generation)
      (stoppable : stoppableVmStatus vm.status) :
      Step world (.scheduleStopVm sid generation vmId)
        (enqueue world (.stopVm sid record.epoch generation vmId))

  | stopVm
      (queued : deliveryQueued world
        (.stopVm sid record.epoch vm.generation vmId))
      (dispatched : effectAwaiting world
        (.stopVm sid record.epoch vm.generation vmId))
      (foundSandbox : world.sandboxes sid = some record)
      (found : world.vms vmId = some vm)
      (owned : vm.sandbox = sid)
      (stoppable : stoppableVmStatus vm.status)
      (authorizedStop :
        (record.phase = .pausing ∧ record.currentVm = some vmId ∧
          record.terminal = false ∧
          vm.status = .frozen ∧
          world.workerFences sid = some ⟨record.epoch, false⟩ ∧
          ∃ operation plan source snapshot,
            record.currentOperation = some operation ∧
            world.operationPlans (operationKey sid operation.id) = some plan ∧
            plan.targetSnapshot = some source ∧
            world.snapshots source.snapshot = some snapshot ∧
            snapshot.sandbox = sid ∧
            snapshot.sourceGeneration = record.generation ∧
            snapshot.stage = .durable ∧ snapshot.immutable = true) ∨
        (record.phase = .revoking ∧ record.terminal = true ∧
          world.workerFences sid = some ⟨record.epoch, true⟩)) :
      Step world (.stopVm sid vmId) (stopResource world vmId vm)

  | scheduleCleanupVm
      (up : world.controllerUp = true)
      (foundSandbox : world.sandboxes sid = some record)
      (currentOperation : record.currentOperation = some operation)
      (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
      (planKey : plan.key = operationKey sid operation.id)
      (planEpoch : plan.epoch = record.epoch)
      (planGeneration : plan.generation = record.generation)
      (planned : vmId ∈ plan.stopVms)
      (foundVm : world.vms vmId = some vm)
      (identity : vm.sandbox = sid ∧ vm.generation = generation)
      (stopped : vm.status = .stopped)
      (notCurrent : ∀ record, world.sandboxes sid = some record →
        record.currentVm ≠ some vmId) :
      Step world (.scheduleCleanupVm sid generation vmId)
        (enqueue world (.cleanupVm sid generation vmId))

  | scheduleCleanupSlot
      (up : world.controllerUp = true)
      (foundSandbox : world.sandboxes sid = some record)
      (currentOperation : record.currentOperation = some operation)
      (foundPlan : world.operationPlans (operationKey sid operation.id) = some plan)
      (planKey : plan.key = operationKey sid operation.id)
      (planEpoch : plan.epoch = record.epoch)
      (planGeneration : plan.generation = record.generation)
      (planned : slotId ∈ plan.closeSlots)
      (foundSlot : world.netSlots slotId = some slot)
      (identity : slot.sandbox = sid ∧ slot.generation = generation)
      (inactive : slot.active = false)
      (notReclaimed : slot.reclaimed = false)
      (notCurrent : ∀ record, world.sandboxes sid = some record →
        record.netSlot ≠ some slotId) :
      Step world (.scheduleCleanupSlot sid generation slotId)
        (enqueue world (.cleanupSlot sid generation slotId))

  | cleanupVm
      (queued : deliveryQueued world (.cleanupVm sid generation vmId))
      (dispatched : effectAwaiting world (.cleanupVm sid generation vmId))
      (foundVm : world.vms vmId = some vm)
      (identity : vm.sandbox = sid ∧ vm.generation = generation)
      (stopped : vm.status = .stopped)
      (notCurrent : ∀ record, world.sandboxes sid = some record →
        record.currentVm ≠ some vmId) :
      Step world (.cleanupVm sid generation vmId) (reclaimVm world vmId vm)

  | cleanupSlot
      (queued : deliveryQueued world (.cleanupSlot sid generation slotId))
      (dispatched : effectAwaiting world (.cleanupSlot sid generation slotId))
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
      (currentVm : record.currentVm = some vmId)
      (fenceSucceeded : effectSucceeded world
        (.installFence sid record.epoch false))
      (freezeSucceeded : effectSucceeded world
        (.freezeVm sid record.epoch record.generation vmId))
      (sealSucceeded : effectSucceeded world
        (.sealSnapshot sid record.epoch record.generation vmId
          record.workspace source.snapshot source.manifest))
      (commitSucceeded : effectSucceeded world
        (.commitSnapshot sid record.epoch record.generation
          source.snapshot source.manifest))
      (stopSucceeded : effectSucceeded world
        (.stopVm sid record.epoch record.generation vmId))
      (vmsClosed : allVmsStoppedOrReclaimed world sid)
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
      (freshPlan : world.operationPlans (operationKey sid operationId) = none)
      (auth : authorized actor record.owner) :
      Step world (.acceptResume actor sid operationId)
        (beginResume world sid record operationId source)

  | acceptKill
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (notDestroyed : record.phase ≠ .destroyed)
      (live : record.terminal = false)
      (supersedeReady : SupersedeReady world sid record.currentOperation)
      (freshOperation : world.operations (operationKey sid operationId) = none)
      (freshPlan : world.operationPlans (operationKey sid operationId) = none)
      (auth : authorized actor record.owner) :
      Step world (.acceptKill actor sid operationId)
        (beginKill world sid record operationId)

  | beginCompensation
      (failure : FailureKind)
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (compensablePhase : record.phase = .starting ∨ record.phase = .pausing)
      (live : record.terminal = false)
      (currentOperation : record.currentOperation = some operation)
      (recoverableKind : operation.kind = .create ∨
        operation.kind = .resume ∨ operation.kind = .pause)
      (unfinished : operation.status = .pending ∨
        operation.status = .outcomeUnknown)
      (persistedOperation : world.operations (operationKey sid operation.id) =
        some ledger)
      (operationMatches : ledger.sandbox = sid ∧ ledger.operation = operation ∧
        ledger.result = none)
      (evidence : CompensationEvidence world (operationKey sid operation.id)
        operation.epoch failure) :
      Step world (.beginCompensation sid failure)
        (SandboxModel.beginCompensation world sid record operation failure)

  | publishDestroyed
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (revoking : record.phase = .revoking)
      (terminal : record.terminal = true)
      (fenced : world.workerFences sid = some ⟨record.epoch, true⟩)
      (fenceSucceeded : effectSucceeded world
        (.installFence sid record.epoch true))
      (vmsClosed : allVmsStoppedOrReclaimed world sid)
      (stopsSucceeded : allVmStopsSucceeded world sid)
      (networkClosed : noActiveSlot world sid)
      (currentOperation : record.currentOperation = some operation)
      (killOperation : operation.kind = .kill)
      (unfinished : operation.status = .pending ∨
        operation.status = .outcomeUnknown)
      (persistedOperation : world.operations (operationKey sid operation.id) =
        some ledger)
      (operationMatches : ledger.sandbox = sid ∧ ledger.operation = operation ∧
        ledger.result = none) :
      Step world (.publishDestroyed sid) (publishDestroyed world sid record)

  | publishFailure
      (failure : FailureKind)
      (up : world.controllerUp = true)
      (found : world.sandboxes sid = some record)
      (revoking : record.phase = .revoking)
      (terminal : record.terminal = true)
      (fenced : world.workerFences sid = some ⟨record.epoch, true⟩)
      (fenceSucceeded : effectSucceeded world
        (.installFence sid record.epoch true))
      (vmsClosed : allVmsStoppedOrReclaimed world sid)
      (stopsSucceeded : allVmStopsSucceeded world sid)
      (networkClosed : noActiveSlot world sid)
      (currentOperation : record.currentOperation = some operation)
      (compensating : operation.status = .compensating failure)
      (recoverableKind : operation.kind = .create ∨
        operation.kind = .resume ∨ operation.kind = .pause)
      (persistedOperation : world.operations (operationKey sid operation.id) =
        some ledger)
      (operationMatches : ledger.sandbox = sid ∧ ledger.operation = operation ∧
        ledger.result = none) :
      Step world (.publishFailure sid failure)
        (SandboxModel.publishFailure world sid record failure)

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
      (delivery : EffectId)
      (persisted : world.effects delivery = some effect) :
      Step world (.duplicateOrLateDelivery delivery) world

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
