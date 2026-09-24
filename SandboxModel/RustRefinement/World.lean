import SandboxModel.RustRefinement.Store

/-!
World-level representation skeleton.

The concrete value translations are deliberately parameters of
`WorldValueRelations`: this keeps the store/capacity proof independent from
the command and effect translations while still making every persistent field
part of one machine-checkable boundary.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std


/-- Store relation for maps whose abstract side has a total default value.
    Rust omits empty resource indexes; the state-machine model represents the
    same fact by returning `ResourceIndex.empty`. -/
structure DefaultStoreRep [DecidableEq RustKey]
    (keyMap : RustKey → ModelKey)
    (valueRel : RustValue → ModelValue → Prop)
    (rust : SandboxControllerRust.store.Store RustKey RustValue)
    (model : ModelKey → ModelValue) (default : ModelValue) : Prop where
  keyUnique : (rust.entries.val.map Prod.fst).Nodup
  lookupAgrees : ∀ key,
    match storeLookup rust key with
    | none => model (keyMap key) = default
    | some value => valueRel value (model (keyMap key))
  modelSupport : ∀ key, model key ≠ default →
    ∃ rustKey, keyMap rustKey = key
  capacity : rust.entries.val.length ≤ Std.Usize.max

/-- All non-key value relations needed to compare complete worlds. -/
structure WorldValueRelations where
  sandboxRecord : SandboxControllerRust.model.SandboxRecord → SandboxRecord → Prop
  vm : SandboxControllerRust.model.Vm → Vm → Prop
  filesystem : SandboxControllerRust.model.Filesystem → Filesystem → Prop
  snapshot : SandboxControllerRust.model.Snapshot → Snapshot → Prop
  netSlot : SandboxControllerRust.model.NetSlot → NetSlot → Prop
  appliedFilePolicy : SandboxControllerRust.model.AppliedFilePolicy → AppliedFilePolicy → Prop
  workerFence : SandboxControllerRust.model.WorkerFence → WorkerFence → Prop
  operationKey : SandboxControllerRust.model.OperationKey → OperationKey
  operationKeyInjective : Function.Injective operationKey
  operationLedger : SandboxControllerRust.model.OperationLedgerRecord → OperationLedgerRecord → Prop
  operationPlan : SandboxControllerRust.model.OperationPlan → OperationPlan → Prop
  resourceIndex : SandboxControllerRust.model.ResourceIndex → ResourceIndex → Prop
  logicalEffectId : SandboxControllerRust.model.LogicalEffectId → LogicalEffectId
  logicalEffectIdInjective : Function.Injective logicalEffectId
  logicalEffectRecord : SandboxControllerRust.model.LogicalEffectRecord → LogicalEffectRecord → Prop
  effectId : SandboxControllerRust.model.EffectId → EffectId
  effectIdInjective : Function.Injective effectId
  effectStatus : SandboxControllerRust.model.EffectStatus → EffectStatus → Prop

/-- Complete persistent-world representation relation.

The instance parameters make explicit which generated compound keys still need
derived equality instances.  `effectKeys` preserves order because scheduling
uses it as a deterministic scan list; `inflight` is related by membership
because the abstract model treats it as a set. -/
structure WorldRep
    [DecidableEq SandboxControllerRust.model.OperationKey]
    [DecidableEq SandboxControllerRust.model.LogicalEffectId]
    [DecidableEq SandboxControllerRust.model.EffectId]
    (relations : WorldValueRelations)
    (rust : SandboxControllerRust.model.World) (model : World) : Prop where
  sandboxes : StoreRep sandboxId relations.sandboxRecord
    rust.sandboxes model.sandboxes
  vms : StoreRep vmId relations.vm rust.vms model.vms
  filesystems : StoreRep fsId relations.filesystem
    rust.filesystems model.filesystems
  snapshots : StoreRep snapshotId relations.snapshot
    rust.snapshots model.snapshots
  netSlots : StoreRep netSlotId relations.netSlot
    rust.net_slots model.netSlots
  appliedFilePolicies : StoreRep sandboxId relations.appliedFilePolicy
    rust.applied_file_policies model.appliedFilePolicies
  workerFences : StoreRep sandboxId relations.workerFence
    rust.worker_fences model.workerFences
  operations : StoreRep relations.operationKey relations.operationLedger
    rust.operations model.operations
  operationPlans : StoreRep relations.operationKey relations.operationPlan
    rust.operation_plans model.operationPlans
  resourceIndexes : DefaultStoreRep sandboxId relations.resourceIndex
    rust.resource_indexes model.resourceIndexes ResourceIndex.empty
  logicalEffects : StoreRep relations.logicalEffectId relations.logicalEffectRecord
    rust.logical_effects model.logicalEffects
  effects : StoreRep relations.effectId relations.effectStatus
    rust.effects model.effects
  effectKeys : rust.effect_keys.val.map relations.effectId = model.effectKeys
  effectKeysUnique : rust.effect_keys.val.Nodup
  inflight : ∀ effect,
    effect ∈ rust.inflight.val ↔ model.inflight (relations.effectId effect)
  /-- Every abstract in-flight attempt has a bounded concrete identifier.
      Without this support direction, the pointwise membership equivalence
      above says nothing about model-only attempts outside the `u64` image. -/
  inflightSupport : ∀ effect, model.inflight effect →
    ∃ rustEffect, relations.effectId rustEffect = effect
  inflightUnique : rust.inflight.val.Nodup
  nextVmId : u64ToNat rust.next_vm_id = model.nextVmId
  nextSlotId : u64ToNat rust.next_slot_id = model.nextSlotId
  nextSnapshotId : u64ToNat rust.next_snapshot_id = model.nextSnapshotId
  nextManifestDigest :
    u64ToNat rust.next_manifest_digest = model.nextManifestDigest
  controllerUp : rust.controller_up = model.controllerUp

/-- Extra premises required by an allocation transition.  They are not baked
    into `WorldRep`: a world at the largest representable identifier is still a
    valid related world, but the checked Rust handler must reject allocation. -/
structure AllocationHeadroom (rust : SandboxControllerRust.model.World) : Prop where
  vm : U64Headroom rust.next_vm_id
  slot : U64Headroom rust.next_slot_id
  snapshot : U64Headroom rust.next_snapshot_id
  manifest : U64Headroom rust.next_manifest_digest

/-- A generated Rust vector can append one value without overflowing its
    machine-sized length. -/
def VecHeadroom (values : alloc.vec.Vec α) : Prop :=
  values.val.length < Std.Usize.max

/-- Uniform machine boundary sufficient for every currently generated
    controller handler.  It deliberately stays separate from `WorldRep`: a
    fully populated finite Rust world is still a valid representation, while
    an individual mutating handler may require room to append or increment.

    The entry-wise counter premises are stronger than any one handler needs,
    making this structure reusable at the unified request/effect/scheduler/
    worker/publication/auxiliary entry points. -/
structure HandlerMachineHeadroom
    (rust : SandboxControllerRust.model.World) : Prop where
  sandboxes : StoreHeadroom rust.sandboxes
  vms : StoreHeadroom rust.vms
  filesystems : StoreHeadroom rust.filesystems
  snapshots : StoreHeadroom rust.snapshots
  netSlots : StoreHeadroom rust.net_slots
  appliedFilePolicies : StoreHeadroom rust.applied_file_policies
  workerFences : StoreHeadroom rust.worker_fences
  operations : StoreHeadroom rust.operations
  operationPlans : StoreHeadroom rust.operation_plans
  resourceIndexes : StoreHeadroom rust.resource_indexes
  logicalEffects : StoreHeadroom rust.logical_effects
  effects : StoreHeadroom rust.effects
  effectKeys : VecHeadroom rust.effect_keys
  inflight : VecHeadroom rust.inflight
  resourceIndexVms : ∀ entry ∈ rust.resource_indexes.entries.val,
    VecHeadroom entry.2.vms
  resourceIndexSlots : ∀ entry ∈ rust.resource_indexes.entries.val,
    VecHeadroom entry.2.slots
  resourceIndexSnapshots : ∀ entry ∈ rust.resource_indexes.entries.val,
    VecHeadroom entry.2.snapshots
  sandboxEpoch : ∀ entry ∈ rust.sandboxes.entries.val,
    U64Headroom entry.2.epoch
  sandboxGeneration : ∀ entry ∈ rust.sandboxes.entries.val,
    U64Headroom entry.2.generation
  logicalAttempt : ∀ entry ∈ rust.logical_effects.entries.val,
    U64Headroom entry.2.current_attempt
  cursors : AllocationHeadroom rust

end SandboxModel.RustRefinement
