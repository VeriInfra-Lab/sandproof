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
abbrev AttemptNo := Nat

/-- One protocol-wide retry budget.  Operation plans and newly committed
    logical effects must read the same value so the persisted plan is the
    unique bound used by execution and liveness proofs. -/
def defaultRetryLimit : Nat := 3

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

/-- Failures are protocol data, not exceptions hidden outside the state
    machine.  The constructors deliberately describe controller-visible
    classes rather than adapter-specific error strings. -/
inductive FailureKind where
  | cancelled
  | deadlineExceeded
  | resourceFailure
  | policyFailure
  | storageFailure
  | internalFailure
  deriving DecidableEq, Repr

/-- Protocol-owned retry classification.  Adapter strings never decide this
    policy.  Cancellation and policy rejection are terminal; transient
    resource, storage, deadline, and internal failures may consume a retry. -/
def FailureKind.retryable : FailureKind → Bool
  | .cancelled | .policyFailure => false
  | .deadlineExceeded | .resourceFailure | .storageFailure | .internalFailure => true

inductive OperationResult where
  | running (generation : Generation)
  | paused (source : RestoreSource)
  | destroyed
  | failed (failure : FailureKind)
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
  | compensating (failure : FailureKind)
  | succeeded
  | failed (failure : FailureKind)
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
  | configureNetwork (sid : SandboxId) (epoch : Epoch)
      (generation : Generation) (vm : VmId) (slot : NetSlotId)
      (policy : NetworkPolicy)
  | activateVm (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId)
  | freezeVm (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId)
  | applyFilePolicy (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (workspace : FsId) (policy : FilePolicyRef)
  | sealSnapshot (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId) (workspace : FsId) (snapshot : SnapshotId)
      (manifest : SnapshotManifestRef)
  | commitSnapshot (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (snapshot : SnapshotId) (manifest : SnapshotManifestRef)
  | stopVm (sid : SandboxId) (epoch : Epoch) (generation : Generation)
      (vm : VmId)
  | cleanupVm (sid : SandboxId) (generation : Generation) (vm : VmId)
  | cleanupSlot (sid : SandboxId) (generation : Generation)
      (slot : NetSlotId)
  deriving DecidableEq, Repr

def Command.sandbox : Command → SandboxId
  | .installFence sid _ _
  | .prepareVm sid _ _ _ _ _
  | .configureNetwork sid _ _ _ _ _
  | .activateVm sid _ _ _
  | .freezeVm sid _ _ _
  | .applyFilePolicy sid _ _ _ _
  | .sealSnapshot sid _ _ _ _ _ _
  | .commitSnapshot sid _ _ _ _
  | .stopVm sid _ _ _
  | .cleanupVm sid _ _
  | .cleanupSlot sid _ _ => sid

def Command.epoch? : Command → Option Epoch
  | .installFence _ epoch _
  | .prepareVm _ epoch _ _ _ _
  | .configureNetwork _ epoch _ _ _ _
  | .activateVm _ epoch _ _
  | .freezeVm _ epoch _ _
  | .applyFilePolicy _ epoch _ _ _
  | .sealSnapshot _ epoch _ _ _ _ _
  | .commitSnapshot _ epoch _ _ _
  | .stopVm _ epoch _ _ => some epoch
  | .cleanupVm _ _ _ | .cleanupSlot _ _ _ => none

/-- Stable identity of one logical worker action.  Retries do not change this
    identity; they only advance the enclosing `EffectId.attempt`. -/
structure LogicalEffectId where
  command : Command
  deriving DecidableEq, Repr

/-- Identity of one exact execution attempt of a logical worker action. -/
structure EffectId where
  logical : LogicalEffectId
  attempt : AttemptNo
  deriving DecidableEq, Repr

def EffectId.command (id : EffectId) : Command :=
  id.logical.command

/-- A worker receipt records either a successful command result or a
    controller-visible failure class.  Timeouts are deliberately not outcomes:
    after a timeout the real outcome remains unknown and may still arrive. -/
inductive EffectOutcome where
  | succeeded (result : ResultDigest)
  | failed (failure : FailureKind)
  deriving DecidableEq, Repr

inductive EffectStatus where
  | committed
  | dispatched
  | outcomeUnknown
  | acknowledged (outcome : EffectOutcome)
  deriving DecidableEq, Repr

structure OperationKey where
  sandbox : SandboxId
  operation : OperationId
  deriving DecidableEq, Repr

/-- Resource choices and the finite cleanup domain for one accepted operation.
    The plan may be filled exactly once and is sealed before a worker action can
    use its resource identities. -/
structure OperationPlan where
  key : OperationKey
  epoch : Epoch
  generation : Generation
  vm : Option VmId
  slot : Option NetSlotId
  restoreFrom : Option RestoreSource
  targetSnapshot : Option RestoreSource
  stopVms : List VmId
  closeSlots : List NetSlotId
  retryLimit : Nat
  sealed : Bool
  deriving DecidableEq, Repr

/-- Finite enumeration of every resource ever registered for one sandbox.
    Reclaimed resources remain listed so recovery and publication can inspect
    the complete lifecycle history. -/
structure ResourceIndex where
  vms : List VmId
  slots : List NetSlotId
  snapshots : List SnapshotId
  deriving DecidableEq, Repr

def ResourceIndex.empty : ResourceIndex := ⟨[], [], []⟩

inductive EffectOrigin where
  | operation (key : OperationKey)
  | maintenance (sandbox : SandboxId)
  deriving DecidableEq, Repr

inductive LogicalEffectResult where
  | succeeded (attempt : AttemptNo) (result : ResultDigest)
  | failed (attempt : AttemptNo) (failure : FailureKind)
  deriving DecidableEq, Repr

/-- Durable retry head for one logical action.  Individual attempt outcomes
    remain immutable in `World.effects`; this record only selects the head. -/
structure LogicalEffectRecord where
  origin : EffectOrigin
  currentAttempt : AttemptNo
  remainingRetries : Nat
  result : Option LogicalEffectResult
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
  | acknowledgeEffect (effect : EffectId) (outcome : EffectOutcome)
  | effectTimeout (effect : EffectId)
  | startNextAttempt (previous next : EffectId)
  | finalizeEffectFailure (effect : EffectId) (failure : FailureKind)
  | reserveStartupResources (key : OperationKey) (vm : VmId)
      (slot : NetSlotId)
  | reservePauseSnapshot (key : OperationKey) (snapshot : SnapshotId)
      (manifest : SnapshotManifestRef)
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
  | scheduleSealSnapshot (sid : SandboxId) (snapshot : SnapshotId)
      (manifest : SnapshotManifestRef)
  | sealSnapshot (sid : SandboxId) (snapshot : SnapshotId)
      (manifest : SnapshotManifestRef)
  | scheduleCommitSnapshot (sid : SandboxId) (snapshot : SnapshotId)
  | commitSnapshot (snapshot : SnapshotId)
  | scheduleStopVm (sid : SandboxId) (generation : Generation) (vm : VmId)
  | stopVm (sid : SandboxId) (vm : VmId)
  | scheduleCleanupVm (sid : SandboxId) (generation : Generation) (vm : VmId)
  | scheduleCleanupSlot (sid : SandboxId) (generation : Generation)
      (slot : NetSlotId)
  | cleanupVm (sid : SandboxId) (generation : Generation) (vm : VmId)
  | cleanupSlot (sid : SandboxId) (generation : Generation) (slot : NetSlotId)
  | publishPaused (sid : SandboxId) (snapshot : SnapshotId)
  | acceptResume (actor : Actor) (sid : SandboxId) (operation : OperationId)
  | acceptKill (actor : Actor) (sid : SandboxId) (operation : OperationId)
  | beginCompensation (sid : SandboxId) (failure : FailureKind)
  | publishDestroyed (sid : SandboxId)
  | publishFailure (sid : SandboxId) (failure : FailureKind)
  | rpcTimeout (sid : SandboxId)
  | controllerCrash
  | controllerRecover
  | duplicateOrLateDelivery (effect : EffectId)
  | exec (actor : Actor) (sid : SandboxId)
  | fileRead (actor : Actor) (sid : SandboxId) (generation : Generation)
  | fileWrite (actor : Actor) (sid : SandboxId) (generation : Generation)
  | ingress (actor : Actor) (sid : SandboxId)
  | egress (sid : SandboxId) (destination : Endpoint)
  deriving DecidableEq, Repr

end SandboxModel
