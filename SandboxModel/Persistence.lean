import SandboxModel.Step

/-!
The protocol state is split explicitly at the persistence boundary.

`JournalState` is the value which a real controller must replace atomically in
one journal transaction.  It includes the durable outbox: transport delivery
may be repeated or lost after the transaction, but the intent remains in the
journal.  `RuntimeState` contains only process availability and may disappear
on a crash.

Keeping `World` as the convenient state-machine record avoids hiding fields
behind nested projections.  The projections below nevertheless make the
crash boundary a theorem rather than documentation.
-/

namespace SandboxModel

structure JournalState where
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
  nextVmId : Nat
  nextSlotId : Nat
  nextSnapshotId : Nat
  nextManifestDigest : Nat
  outbox : EffectId → Prop

structure RuntimeState where
  controllerUp : Bool
  deriving DecidableEq, Repr

def World.journal (world : World) : JournalState where
  sandboxes := world.sandboxes
  vms := world.vms
  filesystems := world.filesystems
  snapshots := world.snapshots
  netSlots := world.netSlots
  appliedFilePolicies := world.appliedFilePolicies
  workerFences := world.workerFences
  operations := world.operations
  operationPlans := world.operationPlans
  resourceIndexes := world.resourceIndexes
  logicalEffects := world.logicalEffects
  effectKeys := world.effectKeys
  effects := world.effects
  nextVmId := world.nextVmId
  nextSlotId := world.nextSlotId
  nextSnapshotId := world.nextSnapshotId
  nextManifestDigest := world.nextManifestDigest
  outbox := world.inflight

def World.runtime (world : World) : RuntimeState :=
  ⟨world.controllerUp⟩

def World.fromParts (journal : JournalState) (runtime : RuntimeState) : World where
  sandboxes := journal.sandboxes
  vms := journal.vms
  filesystems := journal.filesystems
  snapshots := journal.snapshots
  netSlots := journal.netSlots
  appliedFilePolicies := journal.appliedFilePolicies
  workerFences := journal.workerFences
  operations := journal.operations
  operationPlans := journal.operationPlans
  resourceIndexes := journal.resourceIndexes
  logicalEffects := journal.logicalEffects
  effectKeys := journal.effectKeys
  effects := journal.effects
  nextVmId := journal.nextVmId
  nextSlotId := journal.nextSlotId
  nextSnapshotId := journal.nextSnapshotId
  nextManifestDigest := journal.nextManifestDigest
  inflight := journal.outbox
  controllerUp := runtime.controllerUp

@[simp] theorem World.fromParts_journal
    (persisted : JournalState) (running : RuntimeState) :
    (World.fromParts persisted running).journal = persisted := by
  cases persisted
  rfl

@[simp] theorem World.fromParts_runtime
    (persisted : JournalState) (running : RuntimeState) :
    (World.fromParts persisted running).runtime = running := by
  cases running
  rfl

@[simp] theorem World.fromParts_projection :
    World.fromParts world.journal world.runtime = world := by
  cases world
  rfl

/-- A process crash cannot roll back an accepted request, operation plan,
    effect record, allocation cursor, or durable outbox item. -/
theorem controllerCrash_preserves_journal
    (transition : Step before .controllerCrash after) :
    after.journal = before.journal := by
  cases transition
  cases before
  rfl

/-- Recovery changes only process availability; reconciliation observes the
    exact journal left before the crash. -/
theorem controllerRecover_preserves_journal
    (transition : Step before .controllerRecover after) :
    after.journal = before.journal := by
  cases transition
  cases before
  rfl

/-- The initial fence intent and the accepted create operation are part of the
    same journal value returned by the reducer.  A runtime shell has one
    all-or-nothing value to commit; there is no intermediate accepted state
    without the outbox entry. -/
theorem acceptedCreate_journals_initial_fence
    (freshEffect : before.effects
      (effectId (.installFence sid 1 false)) = none)
    (transition : Step before
      (.acceptCreate actor owner sid workspace filePolicy policy operationId)
      after) :
    after.operations (operationKey sid operationId) = some
        ⟨sid, ⟨operationId, 1, .create, .pending⟩,
          .create workspace policy filePolicy, none⟩ ∧
      after.effects (effectId (.installFence sid 1 false)) =
        some .committed ∧
      after.inflight (effectId (.installFence sid 1 false)) := by
  cases transition with
  | acceptCreate up freshSandbox freshWorkspace freshOperation freshPlan auth =>
      have freshInitial : before.effects
          (effectIdAt (.installFence sid 1 false) 0) = none := by
        simpa [effectId] using freshEffect
      cases logicalFound : before.logicalEffects
          ({ command := .installFence sid 1 false } : LogicalEffectId) <;>
      simp [beginCreate, storeNewOperation, initialOperationPlan, enqueue,
        commitEffect, setOperation, setOperationPlan, setSandbox,
        setFilesystem, setEffect, setLogicalEffect, addCommand, addEffect,
        effectId, logicalEffectId, operationKey, put, freshInitial,
        logicalFound]

end SandboxModel
