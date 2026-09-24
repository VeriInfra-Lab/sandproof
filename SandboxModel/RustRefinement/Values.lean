import SandboxModel.RustRefinement.World

/-!
Concrete value correspondence between the Aeneas translation of the Rust
controller and `SandboxModel`.

The generated controller uses bounded machine integers and vectors.  The model
uses natural numbers and lists.  These functions are the single, explicit
translation boundary for every value persisted in `World`; the accompanying
injectivity lemmas justify using translated compound values as abstract store
keys.  Scalar identifier embeddings and the smallest state enums are defined
in `Store` (where the generic store relation first needs them) and re-exported
through this module.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std

/-! ## Primitive values -/

def filePolicyRef (value : SandboxControllerRust.model.FilePolicyRef) :
    FilePolicyRef :=
  ⟨policyDigest value.digest⟩

def snapshotManifestRef
    (value : SandboxControllerRust.model.SnapshotManifestRef) :
    SnapshotManifestRef :=
  ⟨manifestDigest value.digest⟩

def restoreSource (value : SandboxControllerRust.model.RestoreSource) :
    RestoreSource :=
  ⟨snapshotId value.snapshot, snapshotManifestRef value.manifest⟩

def protocol : SandboxControllerRust.model.Protocol → Protocol
  | .Tcp => .tcp
  | .Udp => .udp

def endpoint (value : SandboxControllerRust.model.Endpoint) : Endpoint where
  address := value.address
  prefixLength := u64ToNat value.prefix_length
  protocol := protocol value.protocol
  port := u64ToNat value.port

def networkPolicy : SandboxControllerRust.model.NetworkPolicy → NetworkPolicy
  | .DenyAll => .denyAll
  | .StaticAllowlist endpoints =>
      .staticAllowlist (endpoints.val.map endpoint)

def failureKind : SandboxControllerRust.model.FailureKind → FailureKind
  | .Cancelled => .cancelled
  | .DeadlineExceeded => .deadlineExceeded
  | .ResourceFailure => .resourceFailure
  | .PolicyFailure => .policyFailure
  | .StorageFailure => .storageFailure
  | .InternalFailure => .internalFailure

/-! ## Operations and authority -/

def operationInput : SandboxControllerRust.model.OperationInput → OperationInput
  | .Create workspace network filePolicy =>
      .create (fsId workspace) (networkPolicy network) (filePolicyRef filePolicy)
  | .Pause generation => .pause (u64ToNat generation)
  | .Resume source => .resume (restoreSource source)
  | .Kill generation => .kill (u64ToNat generation)

def operationResult : SandboxControllerRust.model.OperationResult → OperationResult
  | .Running generation => .running (u64ToNat generation)
  | .Paused source => .paused (restoreSource source)
  | .Destroyed => .destroyed
  | .Failed failure => .failed (failureKind failure)

def operationKind : SandboxControllerRust.model.OperationKind → OperationKind
  | .Create => .create
  | .Pause => .pause
  | .Resume => .resume
  | .Kill => .kill

def operationStatus : SandboxControllerRust.model.OperationStatus → OperationStatus
  | .Pending => .pending
  | .OutcomeUnknown => .outcomeUnknown
  | .Compensating failure => .compensating (failureKind failure)
  | .Succeeded => .succeeded
  | .Failed failure => .failed (failureKind failure)

def operation (value : SandboxControllerRust.model.Operation) : Operation where
  id := operationId value.id
  epoch := u64ToNat value.epoch
  kind := operationKind value.kind
  status := operationStatus value.status

def authority : SandboxControllerRust.model.Authority → Authority
  | .Closed => .closed
  | .Running sid generation vm workspace access slot policy =>
      .running (sandboxId sid) (u64ToNat generation) (vmId vm)
        (fsId workspace) (workspaceApiAccess access) (netSlotId slot)
        (networkPolicy policy)
  | .Paused sid source => .paused (sandboxId sid) (restoreSource source)

/-! ## Commands and durable effect values -/

def command : SandboxControllerRust.model.Command → Command
  | .InstallFence sid epoch terminal =>
      .installFence (sandboxId sid) (u64ToNat epoch) terminal
  | .PrepareVm sid epoch generation vm slot source =>
      .prepareVm (sandboxId sid) (u64ToNat epoch) (u64ToNat generation)
        (vmId vm) (netSlotId slot) (source.map restoreSource)
  | .ConfigureNetwork sid epoch generation vm slot policy =>
      .configureNetwork (sandboxId sid) (u64ToNat epoch) (u64ToNat generation)
        (vmId vm) (netSlotId slot) (networkPolicy policy)
  | .ActivateVm sid epoch generation vm =>
      .activateVm (sandboxId sid) (u64ToNat epoch) (u64ToNat generation)
        (vmId vm)
  | .FreezeVm sid epoch generation vm =>
      .freezeVm (sandboxId sid) (u64ToNat epoch) (u64ToNat generation)
        (vmId vm)
  | .ApplyFilePolicy sid epoch generation workspace policy =>
      .applyFilePolicy (sandboxId sid) (u64ToNat epoch) (u64ToNat generation)
        (fsId workspace) (filePolicyRef policy)
  | .SealSnapshot sid epoch generation vm workspace snapshot manifest =>
      .sealSnapshot (sandboxId sid) (u64ToNat epoch) (u64ToNat generation)
        (vmId vm) (fsId workspace) (snapshotId snapshot)
        (snapshotManifestRef manifest)
  | .CommitSnapshot sid epoch generation snapshot manifest =>
      .commitSnapshot (sandboxId sid) (u64ToNat epoch) (u64ToNat generation)
        (snapshotId snapshot) (snapshotManifestRef manifest)
  | .StopVm sid epoch generation vm =>
      .stopVm (sandboxId sid) (u64ToNat epoch) (u64ToNat generation) (vmId vm)
  | .CleanupVm sid generation vm =>
      .cleanupVm (sandboxId sid) (u64ToNat generation) (vmId vm)
  | .CleanupSlot sid generation slot =>
      .cleanupSlot (sandboxId sid) (u64ToNat generation) (netSlotId slot)

def logicalEffectId (value : SandboxControllerRust.model.LogicalEffectId) :
    LogicalEffectId :=
  ⟨command value.command⟩

def effectId (value : SandboxControllerRust.model.EffectId) : EffectId :=
  ⟨logicalEffectId value.logical, u64ToNat value.attempt⟩

def effectOutcome : SandboxControllerRust.model.EffectOutcome → EffectOutcome
  | .Succeeded result => .succeeded (resultDigest result)
  | .Failed failure => .failed (failureKind failure)

def effectStatus : SandboxControllerRust.model.EffectStatus → EffectStatus
  | .Committed => .committed
  | .Dispatched => .dispatched
  | .OutcomeUnknown => .outcomeUnknown
  | .Acknowledged outcome => .acknowledged (effectOutcome outcome)

def operationKey (value : SandboxControllerRust.model.OperationKey) : OperationKey :=
  ⟨sandboxId value.sandbox, operationId value.operation⟩

def effectOrigin : SandboxControllerRust.model.EffectOrigin → EffectOrigin
  | .Operation key => .operation (operationKey key)
  | .Maintenance sid => .maintenance (sandboxId sid)

def logicalEffectResult :
    SandboxControllerRust.model.LogicalEffectResult → LogicalEffectResult
  | .Succeeded attempt result =>
      .succeeded (u64ToNat attempt) (resultDigest result)
  | .Failed attempt failure =>
      .failed (u64ToNat attempt) (failureKind failure)

def logicalEffectRecord
    (value : SandboxControllerRust.model.LogicalEffectRecord) :
    LogicalEffectRecord where
  origin := effectOrigin value.origin
  currentAttempt := u64ToNat value.current_attempt
  remainingRetries := u64ToNat value.remaining_retries
  result := value.result.map logicalEffectResult

/-! ## Persistent records -/

def operationPlan (value : SandboxControllerRust.model.OperationPlan) :
    OperationPlan where
  key := operationKey value.key
  epoch := u64ToNat value.epoch
  generation := u64ToNat value.generation
  vm := value.vm.map vmId
  slot := value.slot.map netSlotId
  restoreFrom := value.restore_from.map restoreSource
  targetSnapshot := value.target_snapshot.map restoreSource
  stopVms := value.stop_vms.val.map vmId
  closeSlots := value.close_slots.val.map netSlotId
  retryLimit := u64ToNat value.retry_limit
  sealed := value.sealed

def resourceIndex (value : SandboxControllerRust.model.ResourceIndex) :
    ResourceIndex where
  vms := value.vms.val.map vmId
  slots := value.slots.val.map netSlotId
  snapshots := value.snapshots.val.map snapshotId

def operationLedger
    (value : SandboxControllerRust.model.OperationLedgerRecord) :
    OperationLedgerRecord where
  sandbox := sandboxId value.sandbox
  operation := operation value.operation
  input := operationInput value.input
  result := value.result.map operationResult

def sandboxRecord (value : SandboxControllerRust.model.SandboxRecord) :
    SandboxRecord where
  owner := tenantId value.owner
  phase := phase value.phase
  terminal := value.terminal
  epoch := u64ToNat value.epoch
  generation := u64ToNat value.generation
  currentVm := value.current_vm.map vmId
  workspace := fsId value.workspace
  workspaceAccess := workspaceApiAccess value.workspace_access
  filePolicy := filePolicyRef value.file_policy
  durableSnapshot := value.durable_snapshot.map restoreSource
  restoreFrom := value.restore_from.map restoreSource
  netSlot := value.net_slot.map netSlotId
  policy := networkPolicy value.policy
  desired := authority value.desired
  effective := authority value.effective
  currentOperation := value.current_operation.map operation

def vm (value : SandboxControllerRust.model.Vm) : Vm where
  owner := tenantId value.owner
  sandbox := sandboxId value.sandbox
  generation := u64ToNat value.generation
  status := vmStatus value.status

def filesystem (value : SandboxControllerRust.model.Filesystem) : Filesystem where
  owner := tenantId value.owner
  sandbox := sandboxId value.sandbox
  createdGeneration := u64ToNat value.created_generation

def snapshot (value : SandboxControllerRust.model.Snapshot) : Snapshot where
  owner := tenantId value.owner
  sandbox := sandboxId value.sandbox
  sourceGeneration := u64ToNat value.source_generation
  workspace := fsId value.workspace
  manifest := snapshotManifestRef value.manifest
  stage := snapshotStage value.stage
  immutable := value.immutable

def netSlot (value : SandboxControllerRust.model.NetSlot) : NetSlot where
  owner := tenantId value.owner
  sandbox := sandboxId value.sandbox
  generation := u64ToNat value.generation
  vm := vmId value.vm
  policy := networkPolicy value.policy
  configured := value.configured
  active := value.active
  reclaimed := value.reclaimed

def appliedFilePolicy
    (value : SandboxControllerRust.model.AppliedFilePolicy) :
    AppliedFilePolicy where
  sandbox := sandboxId value.sandbox
  generation := u64ToNat value.generation
  workspace := fsId value.workspace
  policy := filePolicyRef value.policy

def workerFence (value : SandboxControllerRust.model.WorkerFence) : WorkerFence where
  epoch := u64ToNat value.epoch
  terminal := value.terminal

/-! ## Observable events -/

/-- Translate events emitted by the unified request, scheduler, and effect
handlers.  Worker, publication, and auxiliary handlers have distinct Rust
event types and therefore use the separate translations below. -/
def modelEvent : SandboxControllerRust.model.Event → Event
  | .AcceptCreate actorValue owner sid workspace filePolicy policy operation =>
      .acceptCreate (actor actorValue) (tenantId owner) (sandboxId sid)
        (fsId workspace) (filePolicyRef filePolicy) (networkPolicy policy)
        (operationId operation)
  | .AcceptPause actorValue sid operation =>
      .acceptPause (actor actorValue) (sandboxId sid) (operationId operation)
  | .AcceptResume actorValue sid operation =>
      .acceptResume (actor actorValue) (sandboxId sid) (operationId operation)
  | .AcceptKill actorValue sid operation =>
      .acceptKill (actor actorValue) (sandboxId sid) (operationId operation)
  | .RetryOperation actorValue sid operation kind input =>
      .retryOperation (actor actorValue) (sandboxId sid) (operationId operation)
        (operationKind kind) (operationInput input)
  | .QueryOperation actorValue sid operation status result =>
      .queryOperation (actor actorValue) (sandboxId sid) (operationId operation)
        (operationStatus status) (result.map operationResult)
  | .BeginCompensation sid failure =>
      .beginCompensation (sandboxId sid) (failureKind failure)
  | .ReserveStartupResources key vm slot =>
      .reserveStartupResources (operationKey key) (vmId vm) (netSlotId slot)
  | .ReservePauseSnapshot key snapshot manifest =>
      .reservePauseSnapshot (operationKey key) (snapshotId snapshot)
        (snapshotManifestRef manifest)
  | .DispatchPrepare sid vm slot source =>
      .dispatchPrepare (sandboxId sid) (vmId vm) (netSlotId slot)
        (source.map restoreSource)
  | .ScheduleSealSnapshot sid snapshot manifest =>
      .scheduleSealSnapshot (sandboxId sid) (snapshotId snapshot)
        (snapshotManifestRef manifest)
  | .ScheduleCommitSnapshot sid snapshot =>
      .scheduleCommitSnapshot (sandboxId sid) (snapshotId snapshot)
  | .ScheduleStopVm sid generation vm =>
      .scheduleStopVm (sandboxId sid) (u64ToNat generation) (vmId vm)
  | .ScheduleCleanupVm sid generation vm =>
      .scheduleCleanupVm (sandboxId sid) (u64ToNat generation) (vmId vm)
  | .ScheduleCleanupSlot sid generation slot =>
      .scheduleCleanupSlot (sandboxId sid) (u64ToNat generation) (netSlotId slot)
  | .DispatchEffect effect => .dispatchEffect (effectId effect)
  | .AcknowledgeEffect effect outcome =>
      .acknowledgeEffect (effectId effect) (effectOutcome outcome)
  | .EffectTimeout effect => .effectTimeout (effectId effect)
  | .StartNextAttempt previous next =>
      .startNextAttempt (effectId previous) (effectId next)
  | .FinalizeEffectFailure effect failure =>
      .finalizeEffectFailure (effectId effect) (failureKind failure)

/-- Translate events emitted by the worker handler. -/
def workerEvent : SandboxControllerRust.worker.WorkerEvent → Event
  | .InstallFence sid => .installFence (sandboxId sid)
  | .PrepareVm sid vm slot source =>
      .prepareVm (sandboxId sid) (vmId vm) (netSlotId slot)
        (source.map restoreSource)
  | .ConfigureNetwork sid slot =>
      .configureNetwork (sandboxId sid) (netSlotId slot)
  | .ActivateVm sid vm => .activateVm (sandboxId sid) (vmId vm)
  | .ApplyFilePolicy sid generation =>
      .applyFilePolicy (sandboxId sid) (u64ToNat generation)
  | .FreezeVm sid vm => .freezeVm (sandboxId sid) (vmId vm)
  | .SealSnapshot sid snapshot manifest =>
      .sealSnapshot (sandboxId sid) (snapshotId snapshot)
        (snapshotManifestRef manifest)
  | .CommitSnapshot snapshot => .commitSnapshot (snapshotId snapshot)
  | .StopVm sid vm => .stopVm (sandboxId sid) (vmId vm)
  | .CleanupVm sid generation vm =>
      .cleanupVm (sandboxId sid) (u64ToNat generation) (vmId vm)
  | .CleanupSlot sid generation slot =>
      .cleanupSlot (sandboxId sid) (u64ToNat generation) (netSlotId slot)

/-- Translate events emitted by the publication handler. -/
def publicationEvent : SandboxControllerRust.publication.PublicationEvent → Event
  | .PublishRunning sid => .publishRunning (sandboxId sid)
  | .PublishPaused sid snapshot =>
      .publishPaused (sandboxId sid) (snapshotId snapshot)
  | .PublishDestroyed sid => .publishDestroyed (sandboxId sid)
  | .PublishFailure sid failure =>
      .publishFailure (sandboxId sid) (failureKind failure)

/-- Translate events emitted by the auxiliary handler.  Recovery-monitor
events intentionally live with the runtime refinement instead. -/
def auxiliaryEvent : SandboxControllerRust.auxiliary.AuxiliaryEvent → Event
  | .RpcTimeout sid => .rpcTimeout (sandboxId sid)
  | .Exec actorValue sid => .exec (actor actorValue) (sandboxId sid)
  | .FileRead actorValue sid generation =>
      .fileRead (actor actorValue) (sandboxId sid) (u64ToNat generation)
  | .FileWrite actorValue sid generation =>
      .fileWrite (actor actorValue) (sandboxId sid) (u64ToNat generation)
  | .Ingress actorValue sid => .ingress (actor actorValue) (sandboxId sid)
  | .Egress sid destination => .egress (sandboxId sid) (endpoint destination)

/-! ## Injective key embeddings -/

theorem tenantId_injective : Function.Injective tenantId := by
  intro left right same
  apply u64ToNat_injective
  exact congrArg TenantId.value same

theorem vmId_injective : Function.Injective vmId := by
  intro left right same
  apply u64ToNat_injective
  exact congrArg VmId.value same

theorem snapshotId_injective : Function.Injective snapshotId := by
  intro left right same
  apply u64ToNat_injective
  exact congrArg SnapshotId.value same

theorem netSlotId_injective : Function.Injective netSlotId := by
  intro left right same
  apply u64ToNat_injective
  exact congrArg NetSlotId.value same

theorem policyDigest_injective : Function.Injective policyDigest := by
  intro left right same
  apply u64ToNat_injective
  exact congrArg PolicyDigest.value same

theorem manifestDigest_injective : Function.Injective manifestDigest := by
  intro left right same
  apply u64ToNat_injective
  exact congrArg ManifestDigest.value same

theorem resultDigest_injective : Function.Injective resultDigest := by
  intro left right same
  apply u64ToNat_injective
  exact congrArg ResultDigest.value same

@[simp] theorem u64ToNat_eq_iff {left right : Std.U64} :
    u64ToNat left = u64ToNat right ↔ left = right :=
  u64ToNat_injective.eq_iff

theorem protocol_injective : Function.Injective protocol := by
  intro left right same
  cases left <;> cases right <;> simp_all [protocol]

theorem filePolicyRef_injective : Function.Injective filePolicyRef := by
  intro left right same
  cases left
  cases right
  simp_all [filePolicyRef, policyDigest]

theorem snapshotManifestRef_injective : Function.Injective snapshotManifestRef := by
  intro left right same
  cases left
  cases right
  simp_all [snapshotManifestRef, manifestDigest]

theorem restoreSource_injective : Function.Injective restoreSource := by
  intro left right same
  cases left with
  | mk leftSnapshot leftManifest =>
      cases right with
      | mk rightSnapshot rightManifest =>
          simp only [restoreSource, RestoreSource.mk.injEq] at same
          have snapshotSame : leftSnapshot = rightSnapshot :=
            snapshotId_injective same.1
          have manifestSame : leftManifest = rightManifest :=
            snapshotManifestRef_injective same.2
          simp_all

private theorem listMapInjective (injective : Function.Injective f) :
    Function.Injective (List.map f) := by
  intro left right same
  induction left generalizing right with
  | nil => cases right <;> simp_all
  | cons head tail ih =>
      cases right with
      | nil => simp at same
      | cons other rest =>
          simp only [List.map, List.cons.injEq] at same
          exact congrArg₂ List.cons (injective same.1) (ih same.2)

theorem endpoint_injective : Function.Injective endpoint := by
  intro left right same
  cases left with
  | mk leftAddress leftPrefix leftProtocol leftPort =>
      cases right with
      | mk rightAddress rightPrefix rightProtocol rightPort =>
          simp only [endpoint, Endpoint.mk.injEq] at same
          have protocolSame : leftProtocol = rightProtocol :=
            protocol_injective same.2.2.1
          simp_all

theorem networkPolicy_injective : Function.Injective networkPolicy := by
  intro left right same
  cases left with
  | DenyAll => cases right <;> simp_all [networkPolicy]
  | StaticAllowlist endpoints =>
      cases right with
      | DenyAll => simp_all [networkPolicy]
      | StaticAllowlist other =>
          simp only [networkPolicy, NetworkPolicy.staticAllowlist.injEq] at same
          apply congrArg SandboxControllerRust.model.NetworkPolicy.StaticAllowlist
          apply alloc.vec.Vec.ext
          exact listMapInjective endpoint_injective same

@[simp] theorem sandboxId_eq_iff {left right} :
    sandboxId left = sandboxId right ↔ left = right :=
  sandboxId_injective.eq_iff

@[simp] theorem vmId_eq_iff {left right} :
    vmId left = vmId right ↔ left = right :=
  vmId_injective.eq_iff

@[simp] theorem fsId_eq_iff {left right} :
    fsId left = fsId right ↔ left = right :=
  fsId_injective.eq_iff

@[simp] theorem snapshotId_eq_iff {left right} :
    snapshotId left = snapshotId right ↔ left = right :=
  snapshotId_injective.eq_iff

@[simp] theorem netSlotId_eq_iff {left right} :
    netSlotId left = netSlotId right ↔ left = right :=
  netSlotId_injective.eq_iff

@[simp] theorem operationId_eq_iff {left right} :
    operationId left = operationId right ↔ left = right :=
  operationId_injective.eq_iff

@[simp] theorem filePolicyRef_eq_iff {left right} :
    filePolicyRef left = filePolicyRef right ↔ left = right :=
  filePolicyRef_injective.eq_iff

@[simp] theorem snapshotManifestRef_eq_iff {left right} :
    snapshotManifestRef left = snapshotManifestRef right ↔ left = right :=
  snapshotManifestRef_injective.eq_iff

@[simp] theorem restoreSource_eq_iff {left right} :
    restoreSource left = restoreSource right ↔ left = right :=
  restoreSource_injective.eq_iff

@[simp] theorem networkPolicy_eq_iff {left right} :
    networkPolicy left = networkPolicy right ↔ left = right :=
  networkPolicy_injective.eq_iff

@[simp] theorem restoreSource_option_map_eq_iff
    {left right : Option SandboxControllerRust.model.RestoreSource} :
    left.map restoreSource = right.map restoreSource ↔ left = right :=
  (Option.map_injective restoreSource_injective).eq_iff

theorem command_injective : Function.Injective command := by
  intro left right same
  cases left <;> cases right <;> simp_all [command]

@[simp] theorem command_eq_iff {left right} :
    command left = command right ↔ left = right :=
  command_injective.eq_iff

theorem logicalEffectId_injective : Function.Injective logicalEffectId := by
  intro left right same
  cases left
  cases right
  simp_all [logicalEffectId]

@[simp] theorem logicalEffectId_eq_iff {left right} :
    logicalEffectId left = logicalEffectId right ↔ left = right :=
  logicalEffectId_injective.eq_iff

theorem effectId_injective : Function.Injective effectId := by
  intro left right same
  cases left
  cases right
  simp_all [effectId]

theorem operationKey_injective : Function.Injective operationKey := by
  intro left right same
  cases left
  cases right
  simp_all [operationKey]

/-! ## Concrete world relation and initial world -/

def valueRelations : WorldValueRelations where
  sandboxRecord rust model := sandboxRecord rust = model
  vm rust model := vm rust = model
  filesystem rust model := filesystem rust = model
  snapshot rust model := snapshot rust = model
  netSlot rust model := netSlot rust = model
  appliedFilePolicy rust model := appliedFilePolicy rust = model
  workerFence rust model := workerFence rust = model
  operationKey := operationKey
  operationKeyInjective := operationKey_injective
  operationLedger rust model := operationLedger rust = model
  operationPlan rust model := operationPlan rust = model
  resourceIndex rust model := resourceIndex rust = model
  logicalEffectId := logicalEffectId
  logicalEffectIdInjective := logicalEffectId_injective
  logicalEffectRecord rust model := logicalEffectRecord rust = model
  effectId := effectId
  effectIdInjective := effectId_injective
  effectStatus rust model := effectStatus rust = model

/-- The Rust `World::default()` value, expressed directly in generated types.
    Aeneas does not emit that constructor because it lies outside the selected
    translation roots. -/
def emptyRustWorld : SandboxControllerRust.model.World where
  sandboxes := emptyStore _ _
  vms := emptyStore _ _
  filesystems := emptyStore _ _
  snapshots := emptyStore _ _
  net_slots := emptyStore _ _
  applied_file_policies := emptyStore _ _
  worker_fences := emptyStore _ _
  operations := emptyStore _ _
  operation_plans := emptyStore _ _
  resource_indexes := emptyStore _ _
  logical_effects := emptyStore _ _
  effect_keys := alloc.vec.Vec.new _
  effects := emptyStore _ _
  next_vm_id := 0#u64
  next_slot_id := 0#u64
  next_snapshot_id := 0#u64
  next_manifest_digest := 0#u64
  inflight := alloc.vec.Vec.new _
  controller_up := true

private def decidableEqOfInjective [DecidableEq ModelKey]
    (keyMap : RustKey → ModelKey) (injective : Function.Injective keyMap) :
    DecidableEq RustKey := by
  intro left right
  if same : keyMap left = keyMap right then
    exact isTrue (injective same)
  else
    exact isFalse (fun equal => same (congrArg keyMap equal))

def operationKeyDecidableEq :
    DecidableEq SandboxControllerRust.model.OperationKey :=
  decidableEqOfInjective operationKey operationKey_injective

def logicalEffectIdDecidableEq :
    DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  decidableEqOfInjective logicalEffectId logicalEffectId_injective

def effectIdDecidableEq :
    DecidableEq SandboxControllerRust.model.EffectId :=
  decidableEqOfInjective effectId effectId_injective

private def protocolDecidableEq :
    DecidableEq SandboxControllerRust.model.Protocol :=
  decidableEqOfInjective protocol protocol_injective

private def endpointDecidableEq :
    DecidableEq SandboxControllerRust.model.Endpoint :=
  decidableEqOfInjective endpoint endpoint_injective

private def networkPolicyDecidableEq :
    DecidableEq SandboxControllerRust.model.NetworkPolicy :=
  decidableEqOfInjective networkPolicy networkPolicy_injective

private def filePolicyRefDecidableEq :
    DecidableEq SandboxControllerRust.model.FilePolicyRef :=
  decidableEqOfInjective filePolicyRef filePolicyRef_injective

private def snapshotManifestRefDecidableEq :
    DecidableEq SandboxControllerRust.model.SnapshotManifestRef :=
  decidableEqOfInjective snapshotManifestRef snapshotManifestRef_injective

private def restoreSourceDecidableEq :
    DecidableEq SandboxControllerRust.model.RestoreSource :=
  decidableEqOfInjective restoreSource restoreSource_injective

private def commandDecidableEq :
    DecidableEq SandboxControllerRust.model.Command :=
  decidableEqOfInjective command command_injective

private theorem allM_zip_eq_of_lawful [DecidableEq α]
    (partialEq : core.cmp.PartialEq α α)
    (lawful : LawfulPartialEq partialEq) :
    ∀ left right : List α, left.length = right.length →
      List.allM
          (fun pair : α × α => partialEq.eq pair.1 pair.2)
          (List.zip left right) =
        Aeneas.Std.Result.ok (decide (left = right)) := by
  intro left
  induction left with
  | nil =>
      intro right sameLength
      cases right with
      | nil => rfl
      | cons _ _ => simp at sameLength
  | cons head tail ih =>
      intro right sameLength
      cases right with
      | nil => simp at sameLength
      | cons other rest =>
          have tailLength : tail.length = rest.length := by
            simpa using sameLength
          simp only [List.zip_cons_cons, List.allM_cons]
          rw [lawful head other]
          by_cases sameHead : head = other
          · subst other
            simpa [Aeneas.Std.instMonadResult] using ih rest tailLength
          · simp [sameHead]
            rfl

private theorem vecPartialEq_lawful [DecidableEq α]
    (partialEq : core.cmp.PartialEq α α)
    (lawful : LawfulPartialEq partialEq)
    (left right : alloc.vec.Vec α) :
    alloc.vec.partial_eq.PartialEqVec.eq partialEq left right =
      Aeneas.Std.Result.ok (decide (left = right)) := by
  unfold alloc.vec.partial_eq.PartialEqVec.eq
  by_cases sameLength : left.length = right.length
  · simp only [sameLength, if_pos]
    rw [allM_zip_eq_of_lawful partialEq lawful left.val right.val
      (by simpa using sameLength)]
    congr 2
    apply propext
    constructor
    · exact fun same => alloc.vec.Vec.ext left right same
    · exact fun same => congrArg alloc.vec.Vec.val same
  · have different : left ≠ right := by
      intro same
      exact sameLength (congrArg alloc.vec.Vec.length same)
    simp [sameLength, different]

private theorem protocol_partialEq_lawful
    (left right : SandboxControllerRust.model.Protocol) :
    SandboxControllerRust.model.Protocol.Insts.CoreCmpPartialEqProtocol.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide (left = right) (protocolDecidableEq left right)) := by
  cases left <;> cases right <;>
    simp [SandboxControllerRust.model.Protocol.Insts.CoreCmpPartialEqProtocol.eq,
      SandboxControllerRust.model.Protocol.read_discriminant]

private theorem endpoint_partialEq_lawful :
    @LawfulPartialEq SandboxControllerRust.model.Endpoint
      endpointDecidableEq
      SandboxControllerRust.model.Endpoint.Insts.CoreCmpPartialEqEndpoint := by
  intro left right
  cases left with
  | mk leftAddress leftPrefix leftProtocol leftPort =>
      cases right with
      | mk rightAddress rightPrefix rightProtocol rightPort =>
          simp only [
            SandboxControllerRust.model.Endpoint.Insts.CoreCmpPartialEqEndpoint.eq]
          by_cases samePrefix : leftPrefix = rightPrefix
          · subst rightPrefix
            by_cases samePort : leftPort = rightPort
            · subst rightPort
              by_cases sameAddress : leftAddress = rightAddress
              · subst rightAddress
                cases leftProtocol <;> cases rightProtocol <;>
                  simp [
                    alloc.string.String.Insts.CoreCmpPartialEqString.eq,
                    SandboxControllerRust.model.Protocol.Insts.CoreCmpPartialEqProtocol.eq,
                    SandboxControllerRust.model.Protocol.read_discriminant]
              · simp [sameAddress,
                  alloc.string.String.Insts.CoreCmpPartialEqString.eq]
            · simp [samePort]
          · simp [samePrefix]

private theorem networkPolicy_partialEq_lawful :
    @LawfulPartialEq SandboxControllerRust.model.NetworkPolicy
      networkPolicyDecidableEq
      SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy := by
  letI : DecidableEq SandboxControllerRust.model.Endpoint :=
    endpointDecidableEq
  intro left right
  cases left with
  | DenyAll =>
      cases right with
      | DenyAll =>
          simp [
            SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
            SandboxControllerRust.model.NetworkPolicy.read_discriminant]
      | StaticAllowlist rightEndpoints =>
          simp [
            SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
            SandboxControllerRust.model.NetworkPolicy.read_discriminant]
  | StaticAllowlist leftEndpoints =>
      cases right with
      | DenyAll =>
          simp [
            SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
            SandboxControllerRust.model.NetworkPolicy.read_discriminant]
      | StaticAllowlist rightEndpoints =>
          simp only [
            SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq,
            SandboxControllerRust.model.NetworkPolicy.read_discriminant,
            ↓reduceIte]
          rw [vecPartialEq_lawful
            SandboxControllerRust.model.Endpoint.Insts.CoreCmpPartialEqEndpoint
            endpoint_partialEq_lawful leftEndpoints rightEndpoints]
          congr 2
          apply propext
          simp

private theorem filePolicyRef_partialEq_lawful :
    @LawfulPartialEq SandboxControllerRust.model.FilePolicyRef
      filePolicyRefDecidableEq
      SandboxControllerRust.model.FilePolicyRef.Insts.CoreCmpPartialEqFilePolicyRef := by
  intro left right
  cases left
  cases right
  simp [SandboxControllerRust.model.FilePolicyRef.Insts.CoreCmpPartialEqFilePolicyRef.eq,
    SandboxControllerRust.ids.PolicyDigest.Insts.CoreCmpPartialEqPolicyDigest.eq]

private theorem snapshotManifestRef_partialEq_lawful :
    @LawfulPartialEq SandboxControllerRust.model.SnapshotManifestRef
      snapshotManifestRefDecidableEq
      SandboxControllerRust.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef := by
  intro left right
  cases left
  cases right
  simp [
    SandboxControllerRust.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
    SandboxControllerRust.ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq]

private theorem restoreSource_partialEq_lawful :
    @LawfulPartialEq SandboxControllerRust.model.RestoreSource
      restoreSourceDecidableEq
      SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource := by
  intro left right
  cases left with
  | mk leftSnapshot leftManifest =>
      cases right with
      | mk rightSnapshot rightManifest =>
          cases leftManifest with
          | mk leftDigest =>
            cases rightManifest with
            | mk rightDigest =>
              simp only [
                SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource.eq,
                SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq,
                SandboxControllerRust.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
                SandboxControllerRust.ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq]
              by_cases sameSnapshot : leftSnapshot = rightSnapshot <;>
              by_cases sameDigest : leftDigest = rightDigest <;>
                simp [sameSnapshot, sameDigest]

private theorem restoreSource_eq_lawful
    (left right : SandboxControllerRust.model.RestoreSource) :
    SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide (left = right) (restoreSourceDecidableEq left right)) :=
  restoreSource_partialEq_lawful left right

theorem networkPolicy_eq_lawful
    (left right : SandboxControllerRust.model.NetworkPolicy) :
    SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide (left = right) (networkPolicyDecidableEq left right)) :=
  networkPolicy_partialEq_lawful left right

theorem filePolicyRef_eq_lawful
    (left right : SandboxControllerRust.model.FilePolicyRef) :
    SandboxControllerRust.model.FilePolicyRef.Insts.CoreCmpPartialEqFilePolicyRef.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide (left = right) (filePolicyRefDecidableEq left right)) :=
  filePolicyRef_partialEq_lawful left right

theorem snapshotManifestRef_eq_lawful
    (left right : SandboxControllerRust.model.SnapshotManifestRef) :
    SandboxControllerRust.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide (left = right)
          (snapshotManifestRefDecidableEq left right)) :=
  snapshotManifestRef_partialEq_lawful left right

private theorem option_match_different (left right : Option α)
    (noneDifferent : left = none → right ≠ none)
    (someDifferent : ∀ leftValue rightValue,
      left = some leftValue → right ≠ some rightValue) :
    left ≠ right := by
  intro same
  subst right
  cases left with
  | none => exact noneDifferent rfl rfl
  | some value => exact someDifferent value value rfl rfl

private theorem configureNetwork_policy_eq_lawful
    (sandbox : SandboxControllerRust.ids.SandboxId)
    (epoch generation : Std.U64) (vm : SandboxControllerRust.ids.VmId)
    (slot : SandboxControllerRust.ids.NetSlotId)
    (left right : SandboxControllerRust.model.NetworkPolicy) :
    SandboxControllerRust.model.NetworkPolicy.Insts.CoreCmpPartialEqNetworkPolicy.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide
          (SandboxControllerRust.model.Command.ConfigureNetwork sandbox epoch
              generation vm slot left =
            SandboxControllerRust.model.Command.ConfigureNetwork sandbox epoch
              generation vm slot right)
          (commandDecidableEq _ _)) := by
  rw [networkPolicy_eq_lawful]
  congr 2
  apply propext
  simp

private theorem applyFilePolicy_policy_eq_lawful
    (sandbox : SandboxControllerRust.ids.SandboxId)
    (epoch generation : Std.U64) (workspace : SandboxControllerRust.ids.FsId)
    (left right : SandboxControllerRust.model.FilePolicyRef) :
    SandboxControllerRust.model.FilePolicyRef.Insts.CoreCmpPartialEqFilePolicyRef.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide
          (SandboxControllerRust.model.Command.ApplyFilePolicy sandbox epoch
              generation workspace left =
            SandboxControllerRust.model.Command.ApplyFilePolicy sandbox epoch
              generation workspace right)
          (commandDecidableEq _ _)) := by
  rw [filePolicyRef_eq_lawful]
  congr 2
  apply propext
  simp

private theorem sealSnapshot_manifest_eq_lawful
    (sandbox : SandboxControllerRust.ids.SandboxId)
    (epoch generation : Std.U64) (vm : SandboxControllerRust.ids.VmId)
    (workspace : SandboxControllerRust.ids.FsId)
    (snapshot : SandboxControllerRust.ids.SnapshotId)
    (left right : SandboxControllerRust.model.SnapshotManifestRef) :
    SandboxControllerRust.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide
          (SandboxControllerRust.model.Command.SealSnapshot sandbox epoch generation
              vm workspace snapshot left =
            SandboxControllerRust.model.Command.SealSnapshot sandbox epoch generation
              vm workspace snapshot right)
          (commandDecidableEq _ _)) := by
  rw [snapshotManifestRef_eq_lawful]
  congr 2
  apply propext
  simp

private theorem commitSnapshot_manifest_eq_lawful
    (sandbox : SandboxControllerRust.ids.SandboxId)
    (epoch generation : Std.U64) (snapshot : SandboxControllerRust.ids.SnapshotId)
    (left right : SandboxControllerRust.model.SnapshotManifestRef) :
    SandboxControllerRust.model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide
          (SandboxControllerRust.model.Command.CommitSnapshot sandbox epoch
              generation snapshot left =
            SandboxControllerRust.model.Command.CommitSnapshot sandbox epoch
              generation snapshot right)
          (commandDecidableEq _ _)) := by
  rw [snapshotManifestRef_eq_lawful]
  congr 2
  apply propext
  simp

private theorem command_partialEq_lawful
    (left right : SandboxControllerRust.model.Command) :
    SandboxControllerRust.model.Command.Insts.CoreCmpPartialEqCommand.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide (left = right) (commandDecidableEq left right)) := by
  letI : DecidableEq SandboxControllerRust.model.Endpoint :=
    endpointDecidableEq
  letI : DecidableEq SandboxControllerRust.model.RestoreSource :=
    restoreSourceDecidableEq
  cases left <;> cases right <;>
    simp [
      SandboxControllerRust.model.Command.Insts.CoreCmpPartialEqCommand.eq,
      SandboxControllerRust.model.Command.read_discriminant,
      SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
      SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
      SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
      SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq,
      SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId.eq,
      Aeneas.Std.lift,
      core.cmp.impls.PartialEqBool.eq]
  all_goals
    repeat' first
      | split
      | simp_all
  all_goals
    subst_vars
    first
      | exact restoreSource_eq_lawful _ _
      | exact configureNetwork_policy_eq_lawful _ _ _ _ _ _ _
      | exact applyFilePolicy_policy_eq_lawful _ _ _ _ _ _
      | exact sealSnapshot_manifest_eq_lawful _ _ _ _ _ _ _ _
      | exact commitSnapshot_manifest_eq_lawful _ _ _ _ _ _
      | exact option_match_different _ _ (by assumption) (by assumption)

theorem operationKey_partialEq_lawful :
    @LawfulPartialEq SandboxControllerRust.model.OperationKey
      operationKeyDecidableEq
      SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey := by
  intro left right
  cases left with
  | mk leftSandbox leftOperation =>
      cases right with
      | mk rightSandbox rightOperation =>
          simp only [
            SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
            SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
            SandboxControllerRust.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq]
          by_cases sameSandbox : leftSandbox = rightSandbox <;>
            simp [sameSandbox]

theorem logicalEffectId_partialEq_lawful :
    @LawfulPartialEq SandboxControllerRust.model.LogicalEffectId
      logicalEffectIdDecidableEq
      SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId := by
  intro left right
  cases left with
  | mk leftCommand =>
      cases right with
      | mk rightCommand =>
          simp only [
            SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq]
          rw [command_partialEq_lawful leftCommand rightCommand]
          congr 2
          apply propext
          simp

theorem logicalEffectId_eq_lawful
    (left right : SandboxControllerRust.model.LogicalEffectId) :
    SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq
        left right =
      Aeneas.Std.Result.ok
        (@decide (left = right) (logicalEffectIdDecidableEq left right)) :=
  logicalEffectId_partialEq_lawful left right

theorem effectId_partialEq_lawful :
    @LawfulPartialEq SandboxControllerRust.model.EffectId
      effectIdDecidableEq
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId := by
  intro left right
  cases left with
  | mk leftLogical leftAttempt =>
      cases right with
      | mk rightLogical rightAttempt =>
          simp only [
            SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId.eq]
          rw [logicalEffectId_eq_lawful leftLogical rightLogical]
          by_cases sameLogical : leftLogical = rightLogical <;>
            simp [sameLogical]

/-- The concrete Rust initial state represents the model's `emptyWorld` under
    the complete value correspondence. -/
theorem emptyWorld_rep :
    @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
      effectIdDecidableEq valueRelations emptyRustWorld SandboxModel.emptyWorld := by
  letI : DecidableEq SandboxControllerRust.model.OperationKey :=
    operationKeyDecidableEq
  letI : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
    logicalEffectIdDecidableEq
  letI : DecidableEq SandboxControllerRust.model.EffectId :=
    effectIdDecidableEq
  refine {
    sandboxes := emptyStore_rep sandboxId _
    vms := emptyStore_rep vmId _
    filesystems := emptyStore_rep fsId _
    snapshots := emptyStore_rep snapshotId _
    netSlots := emptyStore_rep netSlotId _
    appliedFilePolicies := emptyStore_rep sandboxId _
    workerFences := emptyStore_rep sandboxId _
    operations := emptyStore_rep operationKey _
    operationPlans := emptyStore_rep operationKey _
    resourceIndexes := ?_
    logicalEffects := emptyStore_rep logicalEffectId _
    effects := emptyStore_rep effectId _
    effectKeys := by simp [emptyRustWorld, SandboxModel.emptyWorld]
    effectKeysUnique := by simp [emptyRustWorld]
    inflight := by
      intro effect
      simp [emptyRustWorld, SandboxModel.emptyWorld]
    inflightSupport := by
      intro effect present
      simp [SandboxModel.emptyWorld] at present
    inflightUnique := by simp [emptyRustWorld]
    nextVmId := by simp [emptyRustWorld, u64ToNat, SandboxModel.emptyWorld]
    nextSlotId := by simp [emptyRustWorld, u64ToNat, SandboxModel.emptyWorld]
    nextSnapshotId := by simp [emptyRustWorld, u64ToNat, SandboxModel.emptyWorld]
    nextManifestDigest := by simp [emptyRustWorld, u64ToNat, SandboxModel.emptyWorld]
    controllerUp := by simp [emptyRustWorld, SandboxModel.emptyWorld] }
  refine {
    keyUnique := by simp [emptyRustWorld, emptyStore]
    lookupAgrees := ?_
    modelSupport := ?_
    capacity := by simp [emptyRustWorld, emptyStore] }
  · intro key
    simp [emptyRustWorld, storeLookup, associationLookup,
      SandboxModel.emptyWorld]
  · intro key notDefault
    simp [SandboxModel.emptyWorld] at notDefault

/-- The concrete empty state has room for every store/vector append and every
    counter increment required by the generated handlers. -/
theorem emptyRustWorld_machineHeadroom :
    HandlerMachineHeadroom emptyRustWorld := by
  refine {
    sandboxes := ?_
    vms := ?_
    filesystems := ?_
    snapshots := ?_
    netSlots := ?_
    appliedFilePolicies := ?_
    workerFences := ?_
    operations := ?_
    operationPlans := ?_
    resourceIndexes := ?_
    logicalEffects := ?_
    effects := ?_
    effectKeys := ?_
    inflight := ?_
    resourceIndexVms := ?_
    resourceIndexSlots := ?_
    resourceIndexSnapshots := ?_
    sandboxEpoch := ?_
    sandboxGeneration := ?_
    logicalAttempt := ?_
    cursors := {
      vm := ?_
      slot := ?_
      snapshot := ?_
      manifest := ?_ } }
  all_goals
    simp [emptyRustWorld, emptyStore, StoreHeadroom, VecHeadroom,
      U64Headroom] <;>
    scalar_tac

@[simp] theorem networkPolicy_denyAll :
    networkPolicy .DenyAll = .denyAll := rfl

@[simp] theorem command_installFence sid epoch terminal :
    command (.InstallFence sid epoch terminal) =
      .installFence (sandboxId sid) (u64ToNat epoch) terminal := rfl

@[simp] theorem effectId_command (id : SandboxControllerRust.model.EffectId) :
    (effectId id).command = command id.logical.command := rfl

@[simp] theorem effectStatus_committed :
    effectStatus .Committed = .committed := rfl

@[simp] theorem effectStatus_outcomeUnknown :
    effectStatus .OutcomeUnknown = .outcomeUnknown := rfl

end SandboxModel.RustRefinement
