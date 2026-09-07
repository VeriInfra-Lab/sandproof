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

def UniqueExecution (world : World) : Prop :=
  ∀ vmId₁ vm₁ vmId₂ vm₂,
    world.vms vmId₁ = some vm₁ → vm₁.status = .executing →
    world.vms vmId₂ = some vm₂ → vm₂.status = .executing →
    vm₁.sandbox = vm₂.sandbox → vmId₁ = vmId₂

def DurableSnapshotsImmutable (world : World) : Prop :=
  ∀ snapshotId snapshot, world.snapshots snapshotId = some snapshot →
    snapshot.stage = .durable → snapshot.immutable = true

def AuthorityShape (world : World) : Prop :=
  ∀ sid record capability, world.sandboxes sid = some record →
    record.effective capability →
      match record.phase with
      | .starting => False
      | .running => runningCapabilities sid record capability
      | .pausing => runningCapabilities sid record capability
      | .paused =>
          ∃ snapshotId, record.durableSnapshot = some snapshotId ∧
            pausedCapabilities sid snapshotId capability
      | .revoking => runningCapabilities sid record capability
      | .destroyed => False

def FenceClosesAuthority (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    world.workerFences sid = some ⟨record.epoch, record.terminal⟩ →
    (record.phase = .pausing ∨ record.phase = .revoking) →
    ∀ capability, ¬ record.effective capability

def EffectiveRuntimeValid (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record →
    (∃ capability, record.effective capability ∧ capability.isDataPlane = true) →
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
    record.terminal = true → record.phase = .revoking ∨ record.phase = .destroyed

def PausedSnapshotValid (world : World) : Prop :=
  ∀ sid record, world.sandboxes sid = some record → record.phase = .paused →
    ∃ snapshotId snapshot,
      record.durableSnapshot = some snapshotId ∧
      world.snapshots snapshotId = some snapshot ∧
      snapshot.owner = record.owner ∧ snapshot.sandbox = sid ∧
      snapshot.sourceGeneration = record.generation ∧
      snapshot.workspace = record.workspace ∧
      snapshot.stage = .durable ∧ snapshot.immutable = true

def Safe (world : World) : Prop :=
  ResourceOwnership world ∧
  UniqueExecution world ∧
  DurableSnapshotsImmutable world ∧
  AuthorityShape world ∧
  FenceClosesAuthority world ∧
  EffectiveRuntimeValid world ∧
  StableStatesQuiescent world ∧
  TerminalConsistent world ∧
  PausedSnapshotValid world

/-!
The first checked proof milestone. `Safe` remains the full research target;
the smaller name prevents unfinished obligations from being reported as done.
-/
def CoreSafety (world : World) : Prop :=
  UniqueExecution world ∧
  DurableSnapshotsImmutable world ∧
  TerminalConsistent world

end SandboxModel
