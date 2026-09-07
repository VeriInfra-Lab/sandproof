/-!
Domain vocabulary for the sandbox protocol.

Identifiers are distinct structures so an epoch cannot accidentally be used as
a VM or filesystem identifier. They remain unbounded natural numbers; the model
does not fix the number of tenants, sandboxes, resources, or protocol steps.
-/

namespace SandboxModel

structure TenantId where
  value : Nat
  deriving DecidableEq, Repr

structure SandboxId where
  value : Nat
  deriving DecidableEq, Repr

structure VmId where
  value : Nat
  deriving DecidableEq, Repr

structure FsId where
  value : Nat
  deriving DecidableEq, Repr

structure SnapshotId where
  value : Nat
  deriving DecidableEq, Repr

structure NetSlotId where
  value : Nat
  deriving DecidableEq, Repr

structure OperationId where
  value : Nat
  deriving DecidableEq, Repr

structure PolicyDigest where
  value : Nat
  deriving DecidableEq, Repr

structure ManifestDigest where
  value : Nat
  deriving DecidableEq, Repr

structure ResultDigest where
  value : Nat
  deriving DecidableEq, Repr

abbrev Epoch := Nat
abbrev Generation := Nat

inductive Actor where
  | tenant (id : TenantId)
  | systemController
  deriving DecidableEq, Repr

inductive Phase where
  | starting
  | running
  | pausing
  | paused
  | revoking
  | destroyed
  deriving DecidableEq, Repr

inductive VmStatus where
  | prepared
  | executing
  | frozen
  | stopped
  | reclaimed
  deriving DecidableEq, Repr

inductive SnapshotStage where
  | localSealed
  | durable
  deriving DecidableEq, Repr

/-! Managed API access is deliberately separate from guest filesystem policy. -/
inductive WorkspaceApiAccess where
  | managedReadWrite
  deriving DecidableEq, Repr

/-- Opaque references identify downstream policy without interpreting chmod or ACLs. -/
structure FilePolicyRef where
  digest : PolicyDigest
  deriving DecidableEq, Repr

/-- Opaque identity for the exact committed snapshot manifest. -/
structure SnapshotManifestRef where
  digest : ManifestDigest
  deriving DecidableEq, Repr

structure RestoreSource where
  snapshot : SnapshotId
  manifest : SnapshotManifestRef
  deriving DecidableEq, Repr

inductive Protocol where
  | tcp
  | udp
  deriving DecidableEq, Repr

structure Endpoint where
  address : String
  prefixLength : Nat
  protocol : Protocol
  port : Nat
  deriving DecidableEq, Repr

inductive NetworkPolicy where
  | denyAll
  | staticAllowlist (endpoints : List Endpoint)
  deriving DecidableEq, Repr

inductive OperationInput where
  | create (workspace : FsId) (network : NetworkPolicy) (filePolicy : FilePolicyRef)
  | pause (generation : Generation)
  | resume (source : RestoreSource)
  | kill (generation : Generation)
  deriving DecidableEq, Repr

inductive OperationResult where
  | running (generation : Generation)
  | paused (source : RestoreSource)
  | destroyed
  deriving DecidableEq, Repr

inductive OperationKind where
  | create
  | pause
  | resume
  | kill
  deriving DecidableEq, Repr

inductive OperationStatus where
  | pending
  | outcomeUnknown
  | succeeded
  deriving DecidableEq, Repr

structure Operation where
  id : OperationId
  epoch : Epoch
  kind : OperationKind
  status : OperationStatus
  deriving DecidableEq, Repr

inductive Capability where
  | exec (sid : SandboxId) (generation : Generation) (vm : VmId)
  | fileRead (sid : SandboxId) (generation : Generation) (fs : FsId)
      (access : WorkspaceApiAccess)
  | fileWrite (sid : SandboxId) (generation : Generation) (fs : FsId)
      (access : WorkspaceApiAccess)
  | ingress (sid : SandboxId) (generation : Generation) (vm : VmId)
      (slot : NetSlotId)
  | egress (sid : SandboxId) (generation : Generation) (vm : VmId)
      (slot : NetSlotId) (policy : NetworkPolicy)
  | resume (sid : SandboxId) (source : RestoreSource)
  deriving DecidableEq, Repr

inductive Authority where
  | closed
  | running (sid : SandboxId) (generation : Generation) (vm : VmId)
      (workspace : FsId) (access : WorkspaceApiAccess) (slot : NetSlotId)
      (policy : NetworkPolicy)
  | paused (sid : SandboxId) (source : RestoreSource)
  deriving DecidableEq, Repr

def Authority.allows : Authority → Capability → Prop
  | .closed, _ => False
  | .running sid generation vm workspace access slot policy, capability =>
      capability = .exec sid generation vm ∨
      capability = .fileRead sid generation workspace access ∨
      capability = .fileWrite sid generation workspace access ∨
      capability = .ingress sid generation vm slot ∨
      capability = .egress sid generation vm slot policy
  | .paused sid source, capability => capability = .resume sid source

def Authority.withoutResume : Authority → Authority
  | .paused _ _ => .closed
  | authority => authority

def authorized (actor : Actor) (owner : TenantId) : Prop :=
  actor = .tenant owner ∨ actor = .systemController

inductive Command where
  | installFence (sid : SandboxId) (epoch : Epoch) (terminal : Bool)
  | prepareVm (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId) (slot : NetSlotId) (source : Option RestoreSource)
  | activateVm (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId)
  | freezeVm (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId)
  | applyFilePolicy (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (workspace : FsId) (policy : FilePolicyRef)
  | cleanupVm (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId)
  | cleanupSlot (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (slot : NetSlotId)
  deriving DecidableEq, Repr

/-- The symbolic command itself is a collision-free deterministic effect ID. -/
structure EffectId where
  command : Command
  deriving DecidableEq, Repr

inductive EffectStatus where
  | committed
  | dispatched
  | acknowledged (result : ResultDigest)
  deriving DecidableEq, Repr

structure OperationKey where
  sandbox : SandboxId
  operation : OperationId
  deriving DecidableEq, Repr

structure OperationLedgerRecord where
  sandbox : SandboxId
  operation : Operation
  input : OperationInput
  result : Option OperationResult
  deriving DecidableEq, Repr

/-! Labels make traces inspectable without putting operational logic in data. -/
inductive Event where
  | acceptCreate (actor : Actor) (owner : TenantId) (sid : SandboxId)
      (workspace : FsId) (filePolicy : FilePolicyRef)
      (policy : NetworkPolicy) (operation : OperationId)
  | retryOperation (actor : Actor) (sid : SandboxId) (operation : OperationId)
      (kind : OperationKind) (input : OperationInput)
  | queryOperation (actor : Actor) (sid : SandboxId) (operation : OperationId)
      (status : OperationStatus) (result : Option OperationResult)
  | dispatchEffect (effect : EffectId)
  | acknowledgeEffect (effect : EffectId) (result : ResultDigest)
  | installFence (sid : SandboxId)
  | dispatchPrepare (sid : SandboxId) (vm : VmId) (slot : NetSlotId)
      (source : Option RestoreSource)
  | prepareVm (sid : SandboxId) (vm : VmId) (slot : NetSlotId)
      (source : Option RestoreSource)
  | configureNetwork (sid : SandboxId) (slot : NetSlotId)
  | activateVm (sid : SandboxId) (vm : VmId)
  | applyFilePolicy (sid : SandboxId) (generation : Generation)
  | publishRunning (sid : SandboxId)
  | acceptPause (actor : Actor) (sid : SandboxId) (operation : OperationId)
  | freezeVm (sid : SandboxId) (vm : VmId)
  | sealSnapshot (sid : SandboxId) (snapshot : SnapshotId)
      (manifest : SnapshotManifestRef)
  | commitSnapshot (snapshot : SnapshotId)
  | stopVm (sid : SandboxId) (vm : VmId)
  | scheduleCleanupVm (sid : SandboxId) (generation : Generation) (vm : VmId)
  | scheduleCleanupSlot (sid : SandboxId) (generation : Generation)
      (slot : NetSlotId)
  | cleanupVm (sid : SandboxId) (generation : Generation) (vm : VmId)
  | cleanupSlot (sid : SandboxId) (generation : Generation) (slot : NetSlotId)
  | publishPaused (sid : SandboxId) (snapshot : SnapshotId)
  | acceptResume (actor : Actor) (sid : SandboxId) (operation : OperationId)
  | acceptKill (actor : Actor) (sid : SandboxId) (operation : OperationId)
  | publishDestroyed (sid : SandboxId)
  | rpcTimeout (sid : SandboxId)
  | controllerCrash
  | controllerRecover
  | duplicateOrLateDelivery (command : Command)
  | exec (actor : Actor) (sid : SandboxId)
  | fileRead (actor : Actor) (sid : SandboxId) (generation : Generation)
  | fileWrite (actor : Actor) (sid : SandboxId) (generation : Generation)
  | ingress (actor : Actor) (sid : SandboxId)
  | egress (sid : SandboxId) (destination : Endpoint)
  deriving DecidableEq, Repr

end SandboxModel
