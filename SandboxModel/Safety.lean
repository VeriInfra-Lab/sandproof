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
def WorkspaceModeFixed (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    record.workspaceMode = .readWrite

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
          record.workspace record.workspaceMode slotId record.policy ∧
        record.effective = .running sid record.generation vmId
          record.workspace record.workspaceMode slotId record.policy
    | .pausing => record.desired = .closed ∧
        (record.effective = .closed ∨ ∃ vmId slotId,
          record.currentVm = some vmId ∧ record.netSlot = some slotId ∧
          record.effective = .running sid record.generation vmId
            record.workspace record.workspaceMode slotId record.policy)
    | .paused => ∃ snapshotId,
        record.durableSnapshot = some snapshotId ∧
        record.desired = .paused sid snapshotId ∧
        record.effective = .paused sid snapshotId
    | .revoking => record.desired = .closed ∧
        (record.effective = .closed ∨ ∃ vmId slotId,
          record.currentVm = some vmId ∧ record.netSlot = some slotId ∧
          record.effective = .running sid record.generation vmId
            record.workspace record.workspaceMode slotId record.policy)
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
    noExecutingVm world sid ∧ noActiveSlot world sid

def TerminalConsistent (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    (record.terminal = true ↔ record.phase = .revoking ∨ record.phase = .destroyed)

def PausedSnapshotValid (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record → record.phase = .paused →
    ∃ snapshotId snapshot,
      record.durableSnapshot = some snapshotId ∧
      world.snapshots snapshotId = some snapshot ∧
      snapshot.owner = record.owner ∧ snapshot.sandbox = sid ∧
      snapshot.sourceGeneration = record.generation ∧
      snapshot.workspace = record.workspace ∧
      snapshot.stage = .durable ∧ snapshot.immutable = true

/-- A resumed Starting state remembers exactly one sealed source from the
    preceding generation. Fresh creates have no restore source. -/
def RestoreSourceValid (world : World) : Prop :=
  ∀ sid record snapshotId, world.sandboxes sid = some record →
    record.phase = .starting → record.restoreFrom = some snapshotId →
    ∃ snapshot, world.snapshots snapshotId = some snapshot ∧
      snapshot.owner = record.owner ∧ snapshot.sandbox = sid ∧
      snapshot.workspace = record.workspace ∧ snapshot.stage = .durable ∧
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
  | .revoking => operation.kind = .kill ∧
      (operation.status = .pending ∨ operation.status = .outcomeUnknown)
  | .destroyed => operation.kind = .kill ∧ operation.status = .succeeded

/-- Every live sandbox record carries one operation whose epoch and result
    agree with the lifecycle phase. This makes timeout semantics part of Safe. -/
def CurrentOperationValid (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    ∃ operation, record.currentOperation = some operation ∧
      operation.epoch = record.epoch ∧ OperationMatchesPhase operation record.phase

/-! Workspace authority specifications. These names mirror the research claims
    so each claim can be inspected independently of the bundled `Safe` theorem. -/

def WorkspaceIdentityStable (before after : World) : Prop :=
  ∀ sid oldRecord newRecord,
    before.sandboxes sid = some oldRecord →
    after.sandboxes sid = some newRecord →
    oldRecord.workspace = newRecord.workspace

def WorkspaceModeStable (before after : World) : Prop :=
  ∀ sid oldRecord newRecord,
    before.sandboxes sid = some oldRecord →
    after.sandboxes sid = some newRecord →
    oldRecord.workspaceMode = newRecord.workspaceMode

def RunningWorkspaceAuthorityExact (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record → record.phase = .running →
    ∃ vmId slotId,
      record.currentVm = some vmId ∧ record.netSlot = some slotId ∧
      record.effective = .running sid record.generation vmId
        record.workspace record.workspaceMode slotId record.policy ∧
      record.effective.allows
        (.fileRead sid record.generation record.workspace record.workspaceMode) ∧
      record.effective.allows
        (.fileWrite sid record.generation record.workspace record.workspaceMode)

/-! Policy properties belong to transitions because they speak about the event being admitted. -/

def EventAuthorized (world : World) : Event → Prop
  | .acceptCreate actor owner _ _ _ _ => authorized actor owner
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

def Safe (world : World) : Prop :=
  ResourceOwnership world ∧
  WorkspaceBindingValid world ∧
  WorkspaceModeFixed world ∧
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

end SandboxModel
