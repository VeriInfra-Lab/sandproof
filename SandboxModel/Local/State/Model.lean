import Std

/-! Independent current local values. Scalars are natural numbers, vectors and
    stores are ordered lists (including duplicate keys). All current variants
    are retained; this file imports neither Aeneas nor generated Rust types.
    Numeric bounds, store uniqueness and product reachability are separate
    obligations, not silently encoded as validity assumptions. -/
namespace SandboxModel.Local.State

inductive model.Protocol where
| Tcp : model.Protocol
| Udp : model.Protocol
  deriving DecidableEq, Repr

structure model.Endpoint where
  address : String
  prefix_length : Nat
  protocol : model.Protocol
  port : Nat
  deriving DecidableEq, Repr

abbrev ids.TenantId := Nat

inductive model.Actor where
| Tenant :
  ids.TenantId →
  model.Actor
| SystemController : model.Actor
  deriving DecidableEq, Repr

abbrev ids.NetSlotId := Nat

abbrev ids.VmId := Nat

abbrev ids.SandboxId := Nat

inductive auxiliary.AccessInput where
| Exec :
  model.Actor →
  ids.SandboxId →
  ids.VmId →
  auxiliary.AccessInput
| FileRead :
  model.Actor →
  ids.SandboxId →
  Nat →
  auxiliary.AccessInput
| FileWrite :
  model.Actor →
  ids.SandboxId →
  Nat →
  auxiliary.AccessInput
| Ingress :
  model.Actor →
  ids.SandboxId →
  ids.VmId →
  ids.NetSlotId →
  auxiliary.AccessInput
| Egress :
  ids.SandboxId →
  ids.VmId →
  ids.NetSlotId →
  model.Endpoint →
  auxiliary.AccessInput
  deriving DecidableEq, Repr

inductive auxiliary.AuxiliaryEvent where
| RpcTimeout :
  ids.SandboxId →
  auxiliary.AuxiliaryEvent
| Exec :
  model.Actor →
  ids.SandboxId →
  auxiliary.AuxiliaryEvent
| FileRead :
  model.Actor →
  ids.SandboxId →
  Nat →
  auxiliary.AuxiliaryEvent
| FileWrite :
  model.Actor →
  ids.SandboxId →
  Nat →
  auxiliary.AuxiliaryEvent
| Ingress :
  model.Actor →
  ids.SandboxId →
  auxiliary.AuxiliaryEvent
| Egress :
  ids.SandboxId →
  model.Endpoint →
  auxiliary.AuxiliaryEvent
  deriving DecidableEq, Repr

inductive auxiliary.AuxiliaryReject where
| SandboxNotFound : auxiliary.AuxiliaryReject
| OperationUnavailable : auxiliary.AuxiliaryReject
| OperationAlreadyFinal : auxiliary.AuxiliaryReject
| LedgerMismatch : auxiliary.AuxiliaryReject
| Unauthorized : auxiliary.AuxiliaryReject
| WrongPhase : auxiliary.AuxiliaryReject
| WrongGeneration : auxiliary.AuxiliaryReject
| CapabilityDenied : auxiliary.AuxiliaryReject
| DestinationDenied : auxiliary.AuxiliaryReject
  deriving DecidableEq, Repr

structure store.Store (K : Type) (V : Type) where
  entries : List (K × V)
  deriving DecidableEq, Repr

structure model.WorkerFence where
  epoch : Nat
  terminal : Bool
  deriving DecidableEq, Repr

abbrev ids.PolicyDigest := Nat

structure model.FilePolicyRef where
  digest : ids.PolicyDigest
  deriving DecidableEq, Repr

abbrev ids.FsId := Nat

structure model.AppliedFilePolicy where
  sandbox : ids.SandboxId
  generation : Nat
  workspace : ids.FsId
  policy : model.FilePolicyRef
  deriving DecidableEq, Repr

structure model.Ipv4Cidr where
  network : Nat
  prefix_length : Nat
  deriving DecidableEq, Repr

inductive model.NetworkPolicy where
| DenyAll : model.NetworkPolicy
| StaticAllowlist :
  List model.Endpoint →
  model.NetworkPolicy
| Ipv4 :
  Bool →
  List model.Ipv4Cidr →
  List model.Ipv4Cidr →
  model.NetworkPolicy
  deriving DecidableEq, Repr

structure model.NetSlot where
  owner : ids.TenantId
  sandbox : ids.SandboxId
  generation : Nat
  vm : ids.VmId
  policy : model.NetworkPolicy
  configured : Bool
  active : Bool
  reclaimed : Bool
  deriving DecidableEq, Repr

abbrev ids.ManifestDigest := Nat

structure model.SnapshotManifestRef where
  digest : ids.ManifestDigest
  deriving DecidableEq, Repr

inductive model.SnapshotStage where
| LocalSealed : model.SnapshotStage
| Durable : model.SnapshotStage
  deriving DecidableEq, Repr

structure model.Snapshot where
  owner : ids.TenantId
  sandbox : ids.SandboxId
  source_generation : Nat
  workspace : ids.FsId
  manifest : model.SnapshotManifestRef
  stage : model.SnapshotStage
  immutable : Bool
  deriving DecidableEq, Repr

structure model.Filesystem where
  owner : ids.TenantId
  sandbox : ids.SandboxId
  created_generation : Nat
  deriving DecidableEq, Repr

inductive model.VmStatus where
| Prepared : model.VmStatus
| Executing : model.VmStatus
| Frozen : model.VmStatus
| Stopped : model.VmStatus
| Reclaimed : model.VmStatus
  deriving DecidableEq, Repr

structure model.Vm where
  owner : ids.TenantId
  sandbox : ids.SandboxId
  generation : Nat
  status : model.VmStatus
  deriving DecidableEq, Repr

abbrev ids.SnapshotId := Nat

structure model.RestoreSource where
  snapshot : ids.SnapshotId
  manifest : model.SnapshotManifestRef
  deriving DecidableEq, Repr

inductive model.WorkspaceApiAccess where
| ManagedReadWrite : model.WorkspaceApiAccess
  deriving DecidableEq, Repr

inductive model.Authority where
| Closed : model.Authority
| Running :
  ids.SandboxId →
  Nat →
  ids.VmId →
  ids.FsId →
  model.WorkspaceApiAccess →
  ids.NetSlotId →
  model.NetworkPolicy →
  model.Authority
| Paused :
  ids.SandboxId →
  model.RestoreSource →
  model.Authority
  deriving DecidableEq, Repr

inductive model.FailureKind where
| Cancelled : model.FailureKind
| DeadlineExceeded : model.FailureKind
| ResourceFailure : model.FailureKind
| PolicyFailure : model.FailureKind
| StorageFailure : model.FailureKind
| InternalFailure : model.FailureKind
  deriving DecidableEq, Repr

inductive model.OperationStatus where
| Pending : model.OperationStatus
| OutcomeUnknown : model.OperationStatus
| Compensating :
  model.FailureKind →
  model.OperationStatus
| Succeeded : model.OperationStatus
| Failed :
  model.FailureKind →
  model.OperationStatus
  deriving DecidableEq, Repr

inductive model.OperationKind where
| Create : model.OperationKind
| Pause : model.OperationKind
| Resume : model.OperationKind
| Kill : model.OperationKind
  deriving DecidableEq, Repr

abbrev ids.OperationId := Nat

structure model.Operation where
  id : ids.OperationId
  epoch : Nat
  kind : model.OperationKind
  status : model.OperationStatus
  deriving DecidableEq, Repr

inductive model.Phase where
| Starting : model.Phase
| Running : model.Phase
| Pausing : model.Phase
| Paused : model.Phase
| Revoking : model.Phase
| Destroyed : model.Phase
  deriving DecidableEq, Repr

structure model.SandboxRecord where
  owner : ids.TenantId
  phase : model.Phase
  terminal : Bool
  epoch : Nat
  generation : Nat
  current_vm : Option ids.VmId
  workspace : ids.FsId
  workspace_access : model.WorkspaceApiAccess
  file_policy : model.FilePolicyRef
  durable_snapshot : Option model.RestoreSource
  restore_from : Option model.RestoreSource
  net_slot : Option ids.NetSlotId
  policy : model.NetworkPolicy
  desired : model.Authority
  effective : model.Authority
  current_operation : Option model.Operation
  deriving DecidableEq, Repr

inductive model.OperationResult where
| Running : Nat → model.OperationResult
| Paused :
  model.RestoreSource →
  model.OperationResult
| Destroyed : model.OperationResult
| Failed :
  model.FailureKind →
  model.OperationResult
  deriving DecidableEq, Repr

inductive model.OperationInput where
| Create :
  ids.FsId →
  model.NetworkPolicy →
  model.FilePolicyRef →
  model.OperationInput
| Pause : Nat → model.OperationInput
| Resume :
  model.RestoreSource →
  model.OperationInput
| Kill : Nat → model.OperationInput
  deriving DecidableEq, Repr

structure model.OperationLedgerRecord where
  sandbox : ids.SandboxId
  operation : model.Operation
  input : model.OperationInput
  result : Option model.OperationResult
  deriving DecidableEq, Repr

structure model.OperationKey where
  sandbox : ids.SandboxId
  operation : ids.OperationId
  deriving DecidableEq, Repr

abbrev ids.ResultDigest := Nat

structure model.StopCoverage where
  operation : model.OperationKey
  epoch : Nat
  attempt : Nat
  result : ids.ResultDigest
  deriving DecidableEq, Repr

inductive model.LogicalEffectResult where
| Succeeded :
  Nat →
  ids.ResultDigest →
  model.LogicalEffectResult
| Failed :
  Nat →
  model.FailureKind →
  model.LogicalEffectResult
| RetiredByStop :
  Nat →
  model.StopCoverage →
  model.LogicalEffectResult
  deriving DecidableEq, Repr

inductive model.EffectOrigin where
| Operation :
  model.OperationKey →
  model.EffectOrigin
| Maintenance :
  ids.SandboxId →
  model.EffectOrigin
  deriving DecidableEq, Repr

structure model.LogicalEffectRecord where
  origin : model.EffectOrigin
  current_attempt : Nat
  remaining_retries : Nat
  result : Option model.LogicalEffectResult
  deriving DecidableEq, Repr

structure model.ResourceIndex where
  vms : List ids.VmId
  slots : List ids.NetSlotId
  snapshots : List ids.SnapshotId
  deriving DecidableEq, Repr

structure model.OperationPlan where
  key : model.OperationKey
  epoch : Nat
  generation : Nat
  vm : Option ids.VmId
  slot : Option ids.NetSlotId
  restore_from : Option model.RestoreSource
  target_snapshot : Option model.RestoreSource
  stop_vms : List ids.VmId
  close_slots : List ids.NetSlotId
  retry_limit : Nat
  sealed : Bool
  deriving DecidableEq, Repr

inductive model.EffectOutcome where
| Succeeded :
  ids.ResultDigest →
  model.EffectOutcome
| Failed :
  model.FailureKind →
  model.EffectOutcome
  deriving DecidableEq, Repr

inductive model.EffectStatus where
| Committed : model.EffectStatus
| Dispatched : model.EffectStatus
| OutcomeUnknown : model.EffectStatus
| Acknowledged :
  model.EffectOutcome →
  model.EffectStatus
| RetiredByStop :
  model.StopCoverage →
  model.EffectStatus
  deriving DecidableEq, Repr

inductive model.Command where
| InstallFence :
  ids.SandboxId →
  Nat →
  Bool →
  model.Command
| PrepareVm :
  ids.SandboxId →
  Nat →
  Nat →
  ids.VmId →
  ids.NetSlotId →
  Option model.RestoreSource →
  model.Command
| ConfigureNetwork :
  ids.SandboxId →
  Nat →
  Nat →
  ids.VmId →
  ids.NetSlotId →
  model.NetworkPolicy →
  model.Command
| ActivateVm :
  ids.SandboxId →
  Nat →
  Nat →
  ids.VmId →
  model.Command
| FreezeVm :
  ids.SandboxId →
  Nat →
  Nat →
  ids.VmId →
  model.Command
| ApplyFilePolicy :
  ids.SandboxId →
  Nat →
  Nat →
  ids.FsId →
  model.FilePolicyRef →
  model.Command
| SealSnapshot :
  ids.SandboxId →
  Nat →
  Nat →
  ids.VmId →
  ids.FsId →
  ids.SnapshotId →
  model.SnapshotManifestRef →
  model.Command
| CommitSnapshot :
  ids.SandboxId →
  Nat →
  Nat →
  ids.SnapshotId →
  model.SnapshotManifestRef →
  model.Command
| StopVm :
  ids.SandboxId →
  Nat →
  Nat →
  ids.VmId →
  model.Command
| AbortPrepare :
  ids.SandboxId →
  Nat →
  Nat →
  ids.VmId →
  ids.NetSlotId →
  model.Command
| CleanupVm :
  ids.SandboxId →
  Nat →
  ids.VmId →
  model.Command
| CleanupSlot :
  ids.SandboxId →
  Nat →
  ids.NetSlotId →
  model.Command
  deriving DecidableEq, Repr

structure model.LogicalEffectId where
  command : model.Command
  deriving DecidableEq, Repr

structure model.EffectId where
  logical : model.LogicalEffectId
  attempt : Nat
  deriving DecidableEq, Repr

abbrev maintenance.windows.WindowId := Nat

structure maintenance.windows.WindowRecord where
  id : maintenance.windows.WindowId
  tenant : ids.TenantId
  issued_at_ms : Nat
  expires_at_ms : Nat
  deriving DecidableEq, Repr

structure maintenance.windows.WindowLimits where
  max_open_windows : Nat
  max_duration_ms : Nat
  deriving DecidableEq, Repr

structure maintenance.windows.WindowState where
  limits : maintenance.windows.WindowLimits
  issued_through : Option Nat
  time_floor_ms : Nat
  «open» : List
    maintenance.windows.WindowRecord
  deriving DecidableEq, Repr

inductive maintenance.slots.HostSlotPhase where
| Occupied : maintenance.slots.HostSlotPhase
| Releasing : maintenance.slots.HostSlotPhase
| Free : maintenance.slots.HostSlotPhase
  deriving DecidableEq, Repr

structure maintenance.slots.HostBinding where
  owner : ids.TenantId
  sandbox : ids.SandboxId
  generation : Nat
  vm : ids.VmId
  network : ids.NetSlotId
  deriving DecidableEq, Repr

structure maintenance.slots.HostSlotRecord where
  binding : maintenance.slots.HostBinding
  phase : maintenance.slots.HostSlotPhase
  deriving DecidableEq, Repr

abbrev maintenance.slots.HostSlotId := Nat

structure maintenance.retirement.RetainedOperation
  where
  owner : ids.TenantId
  ledger : model.OperationLedgerRecord
  expires_at : Nat
  deriving DecidableEq, Repr

structure maintenance.MaintenanceState where
  sandbox_frontier : Option Nat
  workspace_frontier : Option Nat
  retained : store.Store
    model.OperationKey
    maintenance.retirement.RetainedOperation
  time_floor : Nat
  max_retained_results : Nat
  max_host_slots : Nat
  slots : store.Store
    maintenance.slots.HostSlotId
    maintenance.slots.HostSlotRecord
  windows : maintenance.windows.WindowState
  deriving DecidableEq, Repr

structure model.World where
  maintenance : Option maintenance.MaintenanceState
  sandboxes : store.Store
    ids.SandboxId
    model.SandboxRecord
  vms : store.Store ids.VmId
    model.Vm
  filesystems : store.Store
    ids.FsId model.Filesystem
  snapshots : store.Store
    ids.SnapshotId
    model.Snapshot
  net_slots : store.Store
    ids.NetSlotId model.NetSlot
  applied_file_policies : store.Store
    ids.SandboxId
    model.AppliedFilePolicy
  worker_fences : store.Store
    ids.SandboxId
    model.WorkerFence
  operations : store.Store
    model.OperationKey
    model.OperationLedgerRecord
  operation_plans : store.Store
    model.OperationKey
    model.OperationPlan
  resource_indexes : store.Store
    ids.SandboxId
    model.ResourceIndex
  logical_effects : store.Store
    model.LogicalEffectId
    model.LogicalEffectRecord
  effect_keys : List model.EffectId
  effects : store.Store
    model.EffectId
    model.EffectStatus
  next_vm_id : Nat
  next_slot_id : Nat
  next_snapshot_id : Nat
  next_manifest_digest : Nat
  inflight : List model.EffectId
  controller_up : Bool
  deriving DecidableEq, Repr

structure auxiliary.AuxiliaryDecision where
  event : auxiliary.AuxiliaryEvent
  next : model.World
  deriving DecidableEq, Repr

inductive scheduler.SchedulerInput where
| ReserveStartup :
  model.OperationKey →
  scheduler.SchedulerInput
| ReservePauseSnapshot :
  model.OperationKey →
  scheduler.SchedulerInput
| DispatchPrepare :
  ids.SandboxId →
  ids.VmId →
  ids.NetSlotId →
  scheduler.SchedulerInput
| SealSnapshot :
  ids.SandboxId →
  ids.SnapshotId →
  model.SnapshotManifestRef →
  scheduler.SchedulerInput
| CommitSnapshot :
  ids.SandboxId →
  ids.SnapshotId →
  scheduler.SchedulerInput
| StopVm :
  ids.SandboxId →
  Nat →
  ids.VmId →
  scheduler.SchedulerInput
| AbortPrepare :
  ids.SandboxId →
  Nat →
  ids.VmId →
  ids.NetSlotId →
  scheduler.SchedulerInput
| CleanupVm :
  ids.SandboxId →
  Nat →
  ids.VmId →
  scheduler.SchedulerInput
| CleanupSlot :
  ids.SandboxId →
  Nat →
  ids.NetSlotId →
  scheduler.SchedulerInput
  deriving DecidableEq, Repr

structure request.CompensationRequest where
  sandbox : ids.SandboxId
  failure : model.FailureKind
  evidence : Option model.EffectId
  deriving DecidableEq, Repr

structure request.QueryRequest where
  actor : model.Actor
  sandbox : ids.SandboxId
  operation : ids.OperationId
  deriving DecidableEq, Repr

structure request.ReplayRequest where
  actor : model.Actor
  sandbox : ids.SandboxId
  operation : ids.OperationId
  kind : model.OperationKind
  input : model.OperationInput
  deriving DecidableEq, Repr

structure request.KillRequest where
  actor : model.Actor
  sandbox : ids.SandboxId
  operation : ids.OperationId
  deriving DecidableEq, Repr

structure request.ResumeRequest where
  actor : model.Actor
  sandbox : ids.SandboxId
  operation : ids.OperationId
  deriving DecidableEq, Repr

structure request.PauseRequest where
  actor : model.Actor
  sandbox : ids.SandboxId
  operation : ids.OperationId
  deriving DecidableEq, Repr

structure request.CreateRequest where
  actor : model.Actor
  owner : ids.TenantId
  sandbox : ids.SandboxId
  workspace : ids.FsId
  file_policy : model.FilePolicyRef
  policy : model.NetworkPolicy
  operation : ids.OperationId
  deriving DecidableEq, Repr

inductive request.Request where
| Create :
  request.CreateRequest →
  request.Request
| Pause :
  request.PauseRequest →
  request.Request
| Resume :
  request.ResumeRequest →
  request.Request
| Kill :
  request.KillRequest →
  request.Request
| Replay :
  request.ReplayRequest →
  request.Request
| Query :
  request.QueryRequest →
  request.Request
| Compensate :
  request.CompensationRequest →
  request.Request
  deriving DecidableEq, Repr

inductive recovery_monitor.RuntimeInput where
| Crash : recovery_monitor.RuntimeInput
| Recover : recovery_monitor.RuntimeInput
| DuplicateOrLateDelivery :
  model.EffectId →
  recovery_monitor.RuntimeInput
  deriving DecidableEq, Repr

inductive publication.PublicationInput where
| Running :
  ids.SandboxId →
  publication.PublicationInput
| Paused :
  ids.SandboxId →
  model.RestoreSource →
  publication.PublicationInput
| Destroyed :
  ids.SandboxId →
  publication.PublicationInput
| Failed :
  ids.SandboxId →
  model.FailureKind →
  publication.PublicationInput
  deriving DecidableEq, Repr

structure maintenance.MaintenanceLimits where
  max_retained_results : Nat
  max_host_slots : Nat
  windows : maintenance.windows.WindowLimits
  deriving DecidableEq, Repr

inductive maintenance.windows.WindowInput where
| Issue :
  ids.TenantId →
  Nat →
  Nat →
  maintenance.windows.WindowInput
| Close :
  ids.TenantId →
  maintenance.windows.WindowId →
  Nat →
  maintenance.windows.WindowInput
| AdvanceTime :
  Nat →
  maintenance.windows.WindowInput
| Admit :
  ids.TenantId →
  maintenance.windows.WindowId →
  Nat →
  maintenance.windows.WindowInput
  deriving DecidableEq, Repr

inductive maintenance.slots.SlotInput where
| Bind :
  maintenance.slots.HostSlotId →
  Option ids.VmId →
  maintenance.slots.HostBinding →
  maintenance.slots.SlotInput
| BeginRelease :
  maintenance.slots.HostSlotId →
  maintenance.slots.HostBinding →
  maintenance.slots.SlotInput
| ConfirmRelease :
  maintenance.slots.HostSlotId →
  maintenance.slots.HostBinding →
  maintenance.slots.SlotInput
| CheckOccupied :
  maintenance.slots.HostSlotId →
  maintenance.slots.HostBinding →
  maintenance.slots.SlotInput
  deriving DecidableEq, Repr

inductive maintenance.retirement.QuiescenceStatus where
| Unconfirmed : maintenance.retirement.QuiescenceStatus
| Confirmed : maintenance.retirement.QuiescenceStatus
  deriving DecidableEq, Repr

structure maintenance.retirement.QuiescenceCertificate
  where
  sandbox : ids.SandboxId
  owner : ids.TenantId
  epoch : Nat
  operation : ids.OperationId
  workspace : ids.FsId
  worker_drained :
    maintenance.retirement.QuiescenceStatus
  data_connections_drained :
    maintenance.retirement.QuiescenceStatus
  host_resources_reclaimed :
    maintenance.retirement.QuiescenceStatus
  external_pins_released :
    maintenance.retirement.QuiescenceStatus
  deriving DecidableEq, Repr

structure maintenance.retirement.RetireRequest where
  sandbox : ids.SandboxId
  owner : ids.TenantId
  epoch : Nat
  operation : ids.OperationId
  retain_until : Nat
  quiescence :
    maintenance.retirement.QuiescenceCertificate
  deriving DecidableEq, Repr

inductive maintenance.MaintenanceInput where
| Enable :
  maintenance.MaintenanceLimits →
  Nat →
  maintenance.MaintenanceInput
| Retire :
  maintenance.retirement.RetireRequest →
  maintenance.MaintenanceInput
| ExpireResults :
  Nat →
  maintenance.MaintenanceInput
| Slot :
  maintenance.slots.SlotInput →
  maintenance.MaintenanceInput
| Window :
  maintenance.windows.WindowInput →
  maintenance.MaintenanceInput
  deriving DecidableEq, Repr

inductive effect.EffectInput where
| Dispatch :
  model.EffectId →
  effect.EffectInput
| Receipt :
  model.EffectId →
  model.EffectOutcome →
  effect.EffectInput
| Timeout :
  model.EffectId →
  effect.EffectInput
| Retry :
  model.LogicalEffectId →
  model.FailureKind →
  effect.EffectInput
| FinalizeFailure :
  model.LogicalEffectId →
  model.FailureKind →
  effect.EffectInput
| RetireActivation :
  model.EffectId →
  model.EffectId →
  effect.EffectInput
  deriving DecidableEq, Repr

inductive controller.ControllerInput where
| Maintenance :
  maintenance.MaintenanceInput →
  controller.ControllerInput
| Request :
  request.Request →
  controller.ControllerInput
| Effect :
  effect.EffectInput →
  controller.ControllerInput
| Scheduler :
  scheduler.SchedulerInput →
  controller.ControllerInput
| Worker :
  model.EffectId →
  controller.ControllerInput
| Publication :
  publication.PublicationInput →
  controller.ControllerInput
| RpcTimeout :
  ids.SandboxId →
  controller.ControllerInput
| Access :
  auxiliary.AccessInput →
  controller.ControllerInput
| Runtime :
  recovery_monitor.RuntimeInput →
  controller.ControllerInput
  deriving DecidableEq, Repr

inductive maintenance.slots.SlotReject where
| OutOfRange : maintenance.slots.SlotReject
| BindingMismatch : maintenance.slots.SlotReject
| NotFree : maintenance.slots.SlotReject
| NotOccupied : maintenance.slots.SlotReject
| NotReleasing : maintenance.slots.SlotReject
| StaleIdentity : maintenance.slots.SlotReject
| IdentityAlreadyBound : maintenance.slots.SlotReject
  deriving DecidableEq, Repr

inductive recovery_monitor.RuntimeEvent where
| ControllerCrash : recovery_monitor.RuntimeEvent
| ControllerRecover : recovery_monitor.RuntimeEvent
| DuplicateOrLateDelivery :
  model.EffectId →
  recovery_monitor.RuntimeEvent
  deriving DecidableEq, Repr

inductive recovery_monitor.RuntimeReject where
| ControllerAlreadyDown :
  recovery_monitor.RuntimeReject
| ControllerAlreadyUp : recovery_monitor.RuntimeReject
| EffectNotFound : recovery_monitor.RuntimeReject
  deriving DecidableEq, Repr

structure recovery_monitor.RuntimeDecision where
  event : recovery_monitor.RuntimeEvent
  next : model.World
  deriving DecidableEq, Repr

end SandboxModel.Local.State
