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
  deriving DecidableEq, Repr

inductive SnapshotStage where
  | localSealed
  | durable
  deriving DecidableEq, Repr

/-! The first artifact deliberately supports one workspace policy constructor. -/
inductive WorkspaceMode where
  | readWrite
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
  | failed
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
      (mode : WorkspaceMode)
  | fileWrite (sid : SandboxId) (generation : Generation) (fs : FsId)
      (mode : WorkspaceMode)
  | ingress (sid : SandboxId) (generation : Generation) (vm : VmId)
      (slot : NetSlotId)
  | egress (sid : SandboxId) (generation : Generation) (vm : VmId)
      (slot : NetSlotId) (policy : NetworkPolicy)
  | resume (sid : SandboxId) (snapshot : SnapshotId)
  deriving DecidableEq, Repr

inductive Authority where
  | closed
  | running (sid : SandboxId) (generation : Generation) (vm : VmId)
      (workspace : FsId) (mode : WorkspaceMode) (slot : NetSlotId)
      (policy : NetworkPolicy)
  | paused (sid : SandboxId) (snapshot : SnapshotId)
  deriving DecidableEq, Repr

def Authority.allows : Authority → Capability → Prop
  | .closed, _ => False
  | .running sid generation vm workspace mode slot policy, capability =>
      capability = .exec sid generation vm ∨
      capability = .fileRead sid generation workspace mode ∨
      capability = .fileWrite sid generation workspace mode ∨
      capability = .ingress sid generation vm slot ∨
      capability = .egress sid generation vm slot policy
  | .paused sid snapshot, capability => capability = .resume sid snapshot

def Authority.withoutResume : Authority → Authority
  | .paused _ _ => .closed
  | authority => authority

def authorized (actor : Actor) (owner : TenantId) : Prop :=
  actor = .tenant owner ∨ actor = .systemController

inductive Command where
  | installFence (sid : SandboxId) (epoch : Epoch) (terminal : Bool)
  | prepareVm (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId) (slot : NetSlotId) (snapshot : Option SnapshotId)
  | activateVm (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId)
  | freezeVm (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId)
  deriving DecidableEq, Repr

/-! Labels make traces inspectable without putting operational logic in data. -/
inductive Event where
  | acceptCreate (actor : Actor) (owner : TenantId) (sid : SandboxId)
      (workspace : FsId) (policy : NetworkPolicy) (operation : OperationId)
  | installFence (sid : SandboxId)
  | dispatchPrepare (sid : SandboxId) (vm : VmId) (slot : NetSlotId)
      (snapshot : Option SnapshotId)
  | prepareVm (sid : SandboxId) (vm : VmId) (slot : NetSlotId)
      (snapshot : Option SnapshotId)
  | configureNetwork (sid : SandboxId) (slot : NetSlotId)
  | activateVm (sid : SandboxId) (vm : VmId)
  | publishRunning (sid : SandboxId)
  | acceptPause (actor : Actor) (sid : SandboxId) (operation : OperationId)
  | freezeVm (sid : SandboxId) (vm : VmId)
  | sealSnapshot (sid : SandboxId) (snapshot : SnapshotId)
  | commitSnapshot (snapshot : SnapshotId)
  | stopVm (sid : SandboxId) (vm : VmId)
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
