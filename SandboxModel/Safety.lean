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
          record.workspace slotId record.policy ∧
        record.effective = .running sid record.generation vmId
          record.workspace slotId record.policy
    | .pausing => record.desired = .closed ∧
        (record.effective = .closed ∨ ∃ vmId slotId,
          record.currentVm = some vmId ∧ record.netSlot = some slotId ∧
          record.effective = .running sid record.generation vmId
            record.workspace slotId record.policy)
    | .paused => ∃ snapshotId,
        record.durableSnapshot = some snapshotId ∧
        record.desired = .paused sid snapshotId ∧
        record.effective = .paused sid snapshotId
    | .revoking => record.desired = .closed ∧
        (record.effective = .closed ∨ ∃ vmId slotId,
          record.currentVm = some vmId ∧ record.netSlot = some slotId ∧
          record.effective = .running sid record.generation vmId
            record.workspace slotId record.policy)
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

/-! Policy properties belong to transitions because they speak about the event being admitted. -/

def EventAuthorized (world : World) : Event → Prop
  | .acceptCreate actor owner _ _ _ _ => authorized actor owner
  | .acceptPause actor sid _
  | .acceptResume actor sid _
  | .acceptKill actor sid _
  | .exec actor sid
  | .fileRead actor sid
  | .fileWrite actor sid
  | .ingress actor sid =>
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
  | .fileRead _ sid
  | .fileWrite _ sid =>
      ∃ record filesystem,
        world.sandboxes sid = some record ∧
        world.filesystems record.workspace = some filesystem ∧
        filesystem.owner = record.owner ∧ filesystem.sandbox = sid
  | _ => True

def Safe (world : World) : Prop :=
  ResourceOwnership world ∧
  WorkspaceBindingValid world ∧
  UniqueExecution world ∧
  UniqueActiveSlot world ∧
  DurableSnapshotsImmutable world ∧
  AuthorityShape world ∧
  FenceWellFormed world ∧
  FenceClosesAuthority world ∧
  EffectiveRuntimeValid world ∧
  StableStatesQuiescent world ∧
  TerminalConsistent world ∧
  PausedSnapshotValid world

end SandboxModel
