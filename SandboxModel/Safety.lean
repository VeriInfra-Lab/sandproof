import SandboxModel.Step

/-!
Safety predicates are kept separate from transitions. This makes the trust
boundary visible: a transition is executable protocol behavior; `Safe` states
what every reachable world must satisfy.
-/

namespace SandboxModel

def VmOwnership (world : World) : Prop :=
  ∀ vmId vm, world.vms vmId = some vm →
    ∃ record, world.sandboxes vm.sandbox = some record ∧ record.owner = vm.owner

def FilesystemOwnership (world : World) : Prop :=
  ∀ fsId fs, world.filesystems fsId = some fs →
    ∃ record, world.sandboxes fs.sandbox = some record ∧
      record.owner = fs.owner ∧ record.workspace = fsId

def SnapshotOwnership (world : World) : Prop :=
  ∀ snapshotId snapshot, world.snapshots snapshotId = some snapshot →
    ∃ record, world.sandboxes snapshot.sandbox = some record ∧
      record.owner = snapshot.owner ∧ record.workspace = snapshot.workspace

def NetSlotOwnership (world : World) : Prop :=
  ∀ slotId slot, world.netSlots slotId = some slot →
    ∃ record, world.sandboxes slot.sandbox = some record ∧
      record.owner = slot.owner

def ResourceOwnership (world : World) : Prop :=
  VmOwnership world ∧ FilesystemOwnership world ∧
    SnapshotOwnership world ∧ NetSlotOwnership world

/-- Every sandbox's workspace reference points back to that same sandbox and owner. -/
def WorkspaceBindingValid (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    ∃ filesystem, world.filesystems record.workspace = some filesystem ∧
      filesystem.owner = record.owner ∧ filesystem.sandbox = sid

/-- Version one deliberately has a single-constructor workspace policy. Keeping the
    mode in controller state makes later extension possible without admitting
    meaningless combinations of independent read/write flags. -/
def WorkspaceApiAccessFixed (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    record.workspaceAccess = .managedReadWrite

/-- References stored in a sandbox record cannot silently point at another
    sandbox's VM or network slot, including while authority is closed. -/
def CurrentResourcesValid (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    (∀ vmId, record.currentVm = some vmId →
      ∃ vm, world.vms vmId = some vm ∧
        vm.owner = record.owner ∧ vm.sandbox = sid ∧
        vm.generation = record.generation) ∧
    (∀ slotId, record.netSlot = some slotId →
      ∃ slot, world.netSlots slotId = some slot ∧
        slot.owner = record.owner ∧ slot.sandbox = sid ∧
        slot.generation = record.generation ∧
        record.currentVm = some slot.vm ∧ slot.policy = record.policy)

def UniqueExecution (world : World) : Prop :=
  ∀ vmId₁ vm₁ vmId₂ vm₂,
    world.vms vmId₁ = some vm₁ → vm₁.status = .executing →
    world.vms vmId₂ = some vm₂ → vm₂.status = .executing →
    vm₁.sandbox = vm₂.sandbox → vmId₁ = vmId₂

def UniqueActiveSlot (world : World) : Prop :=
  ∀ slotId₁ slot₁ slotId₂ slot₂,
    world.netSlots slotId₁ = some slot₁ → slot₁.active = true →
    world.netSlots slotId₂ = some slot₂ → slot₂.active = true →
    slot₁.sandbox = slot₂.sandbox → slotId₁ = slotId₂

/-- An active route is never orphaned: it is configured and is still the
    current generation/VM/slot named by its sandbox record. -/
def ActiveSlotValid (world : World) : Prop :=
  ∀ slotId slot, world.netSlots slotId = some slot → slot.active = true →
    ∃ record,
      world.sandboxes slot.sandbox = some record ∧
      record.netSlot = some slotId ∧ record.currentVm = some slot.vm ∧
      record.generation = slot.generation ∧ record.policy = slot.policy ∧
      (record.phase = .running ∨ record.phase = .pausing ∨
        record.phase = .revoking) ∧
      slot.configured = true

def DurableSnapshotsImmutable (world : World) : Prop :=
  ∀ snapshotId snapshot, world.snapshots snapshotId = some snapshot →
    snapshot.stage = .durable → snapshot.immutable = true

def AuthorityShape (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    match record.phase with
    | .starting => record.desired = .closed ∧ record.effective = .closed
    | .running => ∃ vmId slotId,
        record.currentVm = some vmId ∧ record.netSlot = some slotId ∧
        record.desired = .running sid record.generation vmId
          record.workspace record.workspaceAccess slotId record.policy ∧
        record.effective = .running sid record.generation vmId
          record.workspace record.workspaceAccess slotId record.policy
    | .pausing => record.desired = .closed ∧
        (record.effective = .closed ∨ ∃ vmId slotId,
          record.currentVm = some vmId ∧ record.netSlot = some slotId ∧
          record.effective = .running sid record.generation vmId
            record.workspace record.workspaceAccess slotId record.policy)
    | .paused => ∃ source,
        record.durableSnapshot = some source ∧
        record.desired = .paused sid source ∧
        record.effective = .paused sid source
    | .revoking => record.desired = .closed ∧
        (record.effective = .closed ∨ ∃ vmId slotId,
          record.currentVm = some vmId ∧ record.netSlot = some slotId ∧
          record.effective = .running sid record.generation vmId
            record.workspace record.workspaceAccess slotId record.policy)
    | .destroyed => record.desired = .closed ∧ record.effective = .closed

def FenceClosesAuthority (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    world.workerFences sid = some ⟨record.epoch, record.terminal⟩ →
    (record.phase = .pausing ∨ record.phase = .revoking) →
    record.effective = .closed

def FenceWellFormed (world : World) : Prop :=
  ∀ sid fence, world.workerFences sid = some fence →
    ∃ record, world.sandboxes sid = some record ∧
      fence.epoch ≤ record.epoch ∧
      (fence.terminal = true → record.terminal = true)

def EffectiveRuntimeValid (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    record.effective ≠ .closed → record.phase ≠ .paused →
    ∃ vmId vm slotId slot,
      record.currentVm = some vmId ∧ world.vms vmId = some vm ∧
      vm.owner = record.owner ∧ vm.sandbox = sid ∧
      vm.generation = record.generation ∧ vm.status = .executing ∧
      record.netSlot = some slotId ∧ world.netSlots slotId = some slot ∧
      slot.owner = record.owner ∧ slot.sandbox = sid ∧
      slot.generation = record.generation ∧ slot.vm = vmId ∧
      slot.policy = record.policy ∧ slot.configured = true ∧ slot.active = true

def StableStatesQuiescent (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    (record.phase = .paused ∨ record.phase = .destroyed) →
    allVmsStoppedOrReclaimed world sid ∧ noActiveSlot world sid

def TerminalConsistent (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    (record.terminal = true ↔ record.phase = .revoking ∨ record.phase = .destroyed)

def PausedSnapshotValid (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record → record.phase = .paused →
    ∃ source snapshot,
      record.durableSnapshot = some source ∧
      world.snapshots source.snapshot = some snapshot ∧
      snapshot.owner = record.owner ∧ snapshot.sandbox = sid ∧
      snapshot.sourceGeneration = record.generation ∧
      snapshot.workspace = record.workspace ∧
      snapshot.manifest = source.manifest ∧
      snapshot.stage = .durable ∧ snapshot.immutable = true

/-- A resumed Starting state remembers exactly one sealed source from the
    preceding generation. Fresh creates have no restore source. -/
def RestoreSourceValid (world : World) : Prop :=
  ∀ sid record source, world.sandboxes sid = some record →
    record.phase = .starting → record.restoreFrom = some source →
    ∃ snapshot, world.snapshots source.snapshot = some snapshot ∧
      snapshot.owner = record.owner ∧ snapshot.sandbox = sid ∧
      snapshot.workspace = record.workspace ∧ snapshot.manifest = source.manifest ∧
      snapshot.stage = .durable ∧
      snapshot.immutable = true ∧
      snapshot.sourceGeneration + 1 = record.generation

def OperationMatchesPhase (operation : Operation) : Phase → Prop
  | .starting =>
      (operation.kind = .create ∨ operation.kind = .resume) ∧
      (operation.status = .pending ∨ operation.status = .outcomeUnknown)
  | .running =>
      (operation.kind = .create ∨ operation.kind = .resume) ∧
      operation.status = .succeeded
  | .pausing => operation.kind = .pause ∧
      (operation.status = .pending ∨ operation.status = .outcomeUnknown)
  | .paused => operation.kind = .pause ∧ operation.status = .succeeded
  | .revoking =>
      (operation.kind = .kill ∧
        (operation.status = .pending ∨ operation.status = .outcomeUnknown)) ∨
      ((operation.kind = .create ∨ operation.kind = .resume ∨
          operation.kind = .pause) ∧
        ∃ failure, operation.status = .compensating failure)
  | .destroyed =>
      (operation.kind = .kill ∧ operation.status = .succeeded) ∨
      ((operation.kind = .create ∨ operation.kind = .resume ∨
          operation.kind = .pause) ∧
        ∃ failure, operation.status = .failed failure)

/-- Every live sandbox record carries one operation whose epoch and result
    agree with the lifecycle phase. This makes timeout semantics part of Safe. -/
def CurrentOperationValid (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    ∃ operation, record.currentOperation = some operation ∧
      operation.epoch = record.epoch ∧ OperationMatchesPhase operation record.phase

/-- A worker policy receipt may lag one generation during resume, but it can
    never name another sandbox/workspace/policy or a future generation. -/
def AppliedFilePolicyValid (world : World) : Prop :=
  ∀ sid applied, world.appliedFilePolicies sid = some applied →
    ∃ record, world.sandboxes sid = some record ∧
      applied.sandbox = sid ∧ applied.workspace = record.workspace ∧
      applied.policy = record.filePolicy ∧ applied.generation ≤ record.generation

def RunningRequiresAppliedFilePolicy (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record → record.phase = .running →
    world.appliedFilePolicies sid = some
      ⟨sid, record.generation, record.workspace, record.filePolicy⟩

def OperationLedgerValid (world : World) : Prop :=
  ∀ key ledger, world.operations key = some ledger →
    key.sandbox = ledger.sandbox ∧ key.operation = ledger.operation.id

def CurrentOperationInLedger (world : World) : Prop :=
  ∀ sid record operation, world.sandboxes sid = some record →
    record.currentOperation = some operation →
    ∃ ledger, world.operations (operationKey sid operation.id) = some ledger

def EffectIdentityValid (world : World) : Prop :=
  ∀ id status, world.effects id = some status →
    id = effectIdAt id.command id.attempt

def ReclaimedResourcesNotCurrent (world : World) : Prop :=
  (∀ vmId vm, world.vms vmId = some vm → vm.status = .reclaimed →
    ∀ record, world.sandboxes vm.sandbox = some record →
      record.currentVm ≠ some vmId) ∧
  (∀ slotId slot, world.netSlots slotId = some slot → slot.reclaimed = true →
    ∀ record, world.sandboxes slot.sandbox = some record →
      record.netSlot ≠ some slotId)

/-! Workspace authority specifications. These names mirror the research claims
    so each claim can be inspected independently of the bundled `Safe` theorem. -/

def WorkspaceIdentityStable (before after : World) : Prop :=
  ∀ sid oldRecord newRecord,
    before.sandboxes sid = some oldRecord →
    after.sandboxes sid = some newRecord →
    oldRecord.workspace = newRecord.workspace

def WorkspaceApiAccessStable (before after : World) : Prop :=
  ∀ sid oldRecord newRecord,
    before.sandboxes sid = some oldRecord →
    after.sandboxes sid = some newRecord →
    oldRecord.workspaceAccess = newRecord.workspaceAccess

def FilePolicyStable (before after : World) : Prop :=
  ∀ sid oldRecord newRecord,
    before.sandboxes sid = some oldRecord →
    after.sandboxes sid = some newRecord →
    oldRecord.filePolicy = newRecord.filePolicy

def EpochMonotone (before after : World) : Prop :=
  ∀ sid oldRecord newRecord,
    before.sandboxes sid = some oldRecord →
    after.sandboxes sid = some newRecord →
    oldRecord.epoch ≤ newRecord.epoch

def GenerationMonotone (before after : World) : Prop :=
  ∀ sid oldRecord newRecord,
    before.sandboxes sid = some oldRecord →
    after.sandboxes sid = some newRecord →
    oldRecord.generation ≤ newRecord.generation

def WorkerFenceMonotone (before after : World) : Prop :=
  ∀ sid oldFence newFence,
    before.workerFences sid = some oldFence →
    after.workerFences sid = some newFence →
    oldFence.epoch ≤ newFence.epoch ∧
      (oldFence.terminal = true → newFence.terminal = true)

def SnapshotManifestStable (before after : World) : Prop :=
  ∀ snapshotId oldSnapshot newSnapshot,
    before.snapshots snapshotId = some oldSnapshot →
    after.snapshots snapshotId = some newSnapshot →
    oldSnapshot.manifest = newSnapshot.manifest

/-- Once an operation has a final result, no later protocol step may replace it. -/
def OperationResultStable (before after : World) : Prop :=
  ∀ key oldRecord newRecord,
    before.operations key = some oldRecord →
    oldRecord.result ≠ none →
    after.operations key = some newRecord →
    newRecord.result = oldRecord.result

/-- Status and result may advance, but an existing operation's request identity
    and input can never be rewritten under the same ledger key. -/
def OperationIdentityStable (before after : World) : Prop :=
  ∀ key oldRecord newRecord,
    before.operations key = some oldRecord →
    after.operations key = some newRecord →
    oldRecord.sandbox = newRecord.sandbox ∧
    oldRecord.operation.id = newRecord.operation.id ∧
    oldRecord.operation.kind = newRecord.operation.kind ∧
    oldRecord.operation.epoch = newRecord.operation.epoch ∧
    oldRecord.input = newRecord.input

/-- An acknowledged effect keeps the same result under every later step. -/
def EffectResultStable (before after : World) : Prop :=
  ∀ id outcome,
    before.effects id = some (.acknowledged outcome) →
    after.effects id = some (.acknowledged outcome)

def RunningWorkspaceAuthorityExact (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record → record.phase = .running →
    ∃ vmId slotId,
      record.currentVm = some vmId ∧ record.netSlot = some slotId ∧
      record.effective = .running sid record.generation vmId
        record.workspace record.workspaceAccess slotId record.policy ∧
      record.effective.allows
        (.fileRead sid record.generation record.workspace record.workspaceAccess) ∧
      record.effective.allows
        (.fileWrite sid record.generation record.workspace record.workspaceAccess)

/-! Policy properties belong to transitions because they speak about the event being admitted. -/

def EventAuthorized (world : World) : Event → Prop
  | .acceptCreate actor owner _ _ _ _ _ => authorized actor owner
  | .retryOperation actor sid _ _ _
  | .queryOperation actor sid _ _ _
  | .acceptPause actor sid _
  | .acceptResume actor sid _
  | .acceptKill actor sid _
  | .exec actor sid
  | .ingress actor sid =>
      ∃ record, world.sandboxes sid = some record ∧ authorized actor record.owner
  | .fileRead actor sid _
  | .fileWrite actor sid _ =>
      ∃ record, world.sandboxes sid = some record ∧ authorized actor record.owner
  | _ => True

def EgressPolicyEnforced (world : World) : Event → Prop
  | .egress sid destination =>
      ∃ record vmId slotId endpoints,
        world.sandboxes sid = some record ∧
        record.effective.allows
          (.egress sid record.generation vmId slotId record.policy) ∧
        record.policy = .staticAllowlist endpoints ∧ destination ∈ endpoints
  | _ => True

def FileEventWorkspaceBound (world : World) : Event → Prop
  | .fileRead _ sid _
  | .fileWrite _ sid _ =>
      ∃ record filesystem,
        world.sandboxes sid = some record ∧
        world.filesystems record.workspace = some filesystem ∧
        filesystem.owner = record.owner ∧ filesystem.sandbox = sid
  | _ => True

/-- The durable outbox, rather than message presence alone, authorizes dispatch
    and acknowledgement. Duplicate delivery must also name a persisted effect. -/
def EffectAdmissionSafe (world : World) : Event → Prop
  | .dispatchEffect id =>
      world.effects id = some .committed ∨
        world.effects id = some .outcomeUnknown
  | .acknowledgeEffect id outcome =>
      ((world.effects id = some .dispatched ∨
          world.effects id = some .outcomeUnknown) ∧
        effectOutcomeValid world id.command outcome) ∨
      world.effects id = some (.acknowledged outcome)
  | .effectTimeout id =>
      world.effects id = some .dispatched ∨
        world.effects id = some .outcomeUnknown
  | .duplicateOrLateDelivery id =>
      ∃ status, world.effects id = some status
  | _ => True

/-- Cleanup is admitted only for the exact stopped/inactive old resource and
    only after the controller record no longer names it as current. -/
def CleanupAdmissionSafe (world : World) : Event → Prop
  | .scheduleCleanupVm sid generation vmId
  | .cleanupVm sid generation vmId =>
      ∃ vm, world.vms vmId = some vm ∧ vm.sandbox = sid ∧
        vm.generation = generation ∧ vm.status = .stopped ∧
        ∀ record, world.sandboxes sid = some record →
          record.currentVm ≠ some vmId
  | .scheduleCleanupSlot sid generation slotId
  | .cleanupSlot sid generation slotId =>
      ∃ slot, world.netSlots slotId = some slot ∧ slot.sandbox = sid ∧
        slot.generation = generation ∧ slot.active = false ∧
        slot.reclaimed = false ∧
        ∀ record, world.sandboxes sid = some record →
          record.netSlot ≠ some slotId
  | _ => True

def Safe (world : World) : Prop :=
  ResourceOwnership world ∧
  WorkspaceBindingValid world ∧
  WorkspaceApiAccessFixed world ∧
  CurrentResourcesValid world ∧
  UniqueExecution world ∧
  UniqueActiveSlot world ∧
  ActiveSlotValid world ∧
  DurableSnapshotsImmutable world ∧
  AuthorityShape world ∧
  FenceWellFormed world ∧
  FenceClosesAuthority world ∧
  EffectiveRuntimeValid world ∧
  StableStatesQuiescent world ∧
  TerminalConsistent world ∧
  PausedSnapshotValid world ∧
  RestoreSourceValid world ∧
  CurrentOperationValid world

/-- Single-state conclusions. Admission and monotonicity remain theorems over
    `Step`, because pretending that a relation is a field of one world would
    hide the history it depends on. -/
structure ProtocolSecurity (world : World) : Prop where
  state : Safe world
  appliedPolicy : AppliedFilePolicyValid world
  runningPolicy : RunningRequiresAppliedFilePolicy world
  operationLedger : OperationLedgerValid world
  currentOperation : CurrentOperationInLedger world
  effectIdentity : EffectIdentityValid world
  reclaimedResources : ReclaimedResourcesNotCurrent world

/-- Reviewable contract for one protocol step. Relational claims live here
    rather than being hidden inside a predicate about only the final world. -/
structure ProtocolStepSecurity (before : World) (event : Event)
    (after : World) : Prop where
  preservesState : Safe before → Safe after
  eventAuthorized : EventAuthorized before event
  egressEnforced : EgressPolicyEnforced before event
  fileWorkspaceBound : WorkspaceBindingValid before →
    FileEventWorkspaceBound before event
  effectAdmission : EffectAdmissionSafe before event
  cleanupAdmission : CleanupAdmissionSafe before event
  epochMonotone : EpochMonotone before after
  generationMonotone : GenerationMonotone before after
  fenceMonotone : WorkerFenceMonotone before after
  terminalMonotone : ∀ sid oldRecord newRecord,
    before.sandboxes sid = some oldRecord →
    after.sandboxes sid = some newRecord →
    oldRecord.terminal = true → newRecord.terminal = true
  workspaceStable : WorkspaceIdentityStable before after
  workspaceAccessStable : WorkspaceApiAccessStable before after
  filePolicyStable : FilePolicyStable before after
  snapshotManifestStable : SnapshotManifestStable before after
  operationIdentityStable : OperationIdentityStable before after
  operationResultStable : OperationResultStable before after
  effectResultStable : EffectResultStable before after

end SandboxModel
