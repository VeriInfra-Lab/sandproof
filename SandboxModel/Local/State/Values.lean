import SandboxModel.Local.State.ValueView

namespace SandboxModel.Local.State
open Aeneas Aeneas.Std

def model.Protocol.view : SandproofNetworkRust.sandbox_controller_core.model.Protocol → model.Protocol
  | .Tcp => .Tcp
  | .Udp => .Udp

theorem model.Protocol.view_injective : Function.Injective model.Protocol.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.Protocol.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Protocol model.Protocol :=
  ⟨model.Protocol.view, model.Protocol.view_injective⟩

def model.Endpoint.view (value : SandproofNetworkRust.sandbox_controller_core.model.Endpoint) : model.Endpoint := {
  address := ValueView.view value.address,
  prefix_length := ValueView.view value.prefix_length,
  protocol := ValueView.view value.protocol,
  port := ValueView.view value.port
}

theorem model.Endpoint.view_injective : Function.Injective model.Endpoint.view := by
  intro a b h
  cases a; cases b; simp_all [model.Endpoint.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Endpoint model.Endpoint :=
  ⟨model.Endpoint.view, model.Endpoint.view_injective⟩

def model.Actor.view : SandproofNetworkRust.sandbox_controller_core.model.Actor → model.Actor
  | .Tenant v0 => .Tenant (ValueView.view v0)
  | .SystemController => .SystemController

theorem model.Actor.view_injective : Function.Injective model.Actor.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.Actor.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Actor model.Actor :=
  ⟨model.Actor.view, model.Actor.view_injective⟩

def auxiliary.AccessInput.view : SandproofNetworkRust.sandbox_controller_core.auxiliary.AccessInput → auxiliary.AccessInput
  | .Exec v0 v1 v2 => .Exec (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .FileRead v0 v1 v2 => .FileRead (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .FileWrite v0 v1 v2 => .FileWrite (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .Ingress v0 v1 v2 v3 => .Ingress (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3)
  | .Egress v0 v1 v2 v3 => .Egress (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3)

theorem auxiliary.AccessInput.view_injective : Function.Injective auxiliary.AccessInput.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [auxiliary.AccessInput.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.auxiliary.AccessInput auxiliary.AccessInput :=
  ⟨auxiliary.AccessInput.view, auxiliary.AccessInput.view_injective⟩

def auxiliary.AuxiliaryEvent.view : SandproofNetworkRust.sandbox_controller_core.auxiliary.AuxiliaryEvent → auxiliary.AuxiliaryEvent
  | .RpcTimeout v0 => .RpcTimeout (ValueView.view v0)
  | .Exec v0 v1 => .Exec (ValueView.view v0) (ValueView.view v1)
  | .FileRead v0 v1 v2 => .FileRead (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .FileWrite v0 v1 v2 => .FileWrite (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .Ingress v0 v1 => .Ingress (ValueView.view v0) (ValueView.view v1)
  | .Egress v0 v1 => .Egress (ValueView.view v0) (ValueView.view v1)

theorem auxiliary.AuxiliaryEvent.view_injective : Function.Injective auxiliary.AuxiliaryEvent.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [auxiliary.AuxiliaryEvent.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.auxiliary.AuxiliaryEvent auxiliary.AuxiliaryEvent :=
  ⟨auxiliary.AuxiliaryEvent.view, auxiliary.AuxiliaryEvent.view_injective⟩

def auxiliary.AuxiliaryReject.view : SandproofNetworkRust.sandbox_controller_core.auxiliary.AuxiliaryReject → auxiliary.AuxiliaryReject
  | .SandboxNotFound => .SandboxNotFound
  | .OperationUnavailable => .OperationUnavailable
  | .OperationAlreadyFinal => .OperationAlreadyFinal
  | .LedgerMismatch => .LedgerMismatch
  | .Unauthorized => .Unauthorized
  | .WrongPhase => .WrongPhase
  | .WrongGeneration => .WrongGeneration
  | .CapabilityDenied => .CapabilityDenied
  | .DestinationDenied => .DestinationDenied

theorem auxiliary.AuxiliaryReject.view_injective : Function.Injective auxiliary.AuxiliaryReject.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [auxiliary.AuxiliaryReject.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.auxiliary.AuxiliaryReject auxiliary.AuxiliaryReject :=
  ⟨auxiliary.AuxiliaryReject.view, auxiliary.AuxiliaryReject.view_injective⟩

instance [ValueView K KM] [ValueView V VM] :
    ValueView (SandproofNetworkRust.sandbox_controller_core.store.Store K V) (store.Store KM VM) where
  view := fun table => ⟨ValueView.view table.entries⟩
  injective := by
    intro a b h
    cases a
    cases b
    simp_all

def model.WorkerFence.view (value : SandproofNetworkRust.sandbox_controller_core.model.WorkerFence) : model.WorkerFence := {
  epoch := ValueView.view value.epoch,
  terminal := ValueView.view value.terminal
}

theorem model.WorkerFence.view_injective : Function.Injective model.WorkerFence.view := by
  intro a b h
  cases a; cases b; simp_all [model.WorkerFence.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.WorkerFence model.WorkerFence :=
  ⟨model.WorkerFence.view, model.WorkerFence.view_injective⟩

def model.FilePolicyRef.view (value : SandproofNetworkRust.sandbox_controller_core.model.FilePolicyRef) : model.FilePolicyRef := {
  digest := ValueView.view value.digest
}

theorem model.FilePolicyRef.view_injective : Function.Injective model.FilePolicyRef.view := by
  intro a b h
  cases a; cases b; simp_all [model.FilePolicyRef.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.FilePolicyRef model.FilePolicyRef :=
  ⟨model.FilePolicyRef.view, model.FilePolicyRef.view_injective⟩

def model.AppliedFilePolicy.view (value : SandproofNetworkRust.sandbox_controller_core.model.AppliedFilePolicy) : model.AppliedFilePolicy := {
  sandbox := ValueView.view value.sandbox,
  generation := ValueView.view value.generation,
  workspace := ValueView.view value.workspace,
  policy := ValueView.view value.policy
}

theorem model.AppliedFilePolicy.view_injective : Function.Injective model.AppliedFilePolicy.view := by
  intro a b h
  cases a; cases b; simp_all [model.AppliedFilePolicy.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.AppliedFilePolicy model.AppliedFilePolicy :=
  ⟨model.AppliedFilePolicy.view, model.AppliedFilePolicy.view_injective⟩

def model.Ipv4Cidr.view (value : SandproofNetworkRust.sandbox_controller_core.model.Ipv4Cidr) : model.Ipv4Cidr := {
  network := ValueView.view value.network,
  prefix_length := ValueView.view value.prefix_length
}

theorem model.Ipv4Cidr.view_injective : Function.Injective model.Ipv4Cidr.view := by
  intro a b h
  cases a; cases b; simp_all [model.Ipv4Cidr.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Ipv4Cidr model.Ipv4Cidr :=
  ⟨model.Ipv4Cidr.view, model.Ipv4Cidr.view_injective⟩

def model.NetworkPolicy.view : SandproofNetworkRust.sandbox_controller_core.model.NetworkPolicy → model.NetworkPolicy
  | .DenyAll => .DenyAll
  | .StaticAllowlist v0 => .StaticAllowlist (ValueView.view v0)
  | .Ipv4 v0 v1 v2 => .Ipv4 (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)

theorem model.NetworkPolicy.view_injective : Function.Injective model.NetworkPolicy.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.NetworkPolicy.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.NetworkPolicy model.NetworkPolicy :=
  ⟨model.NetworkPolicy.view, model.NetworkPolicy.view_injective⟩

def model.NetSlot.view (value : SandproofNetworkRust.sandbox_controller_core.model.NetSlot) : model.NetSlot := {
  owner := ValueView.view value.owner,
  sandbox := ValueView.view value.sandbox,
  generation := ValueView.view value.generation,
  vm := ValueView.view value.vm,
  policy := ValueView.view value.policy,
  configured := ValueView.view value.configured,
  active := ValueView.view value.active,
  reclaimed := ValueView.view value.reclaimed
}

theorem model.NetSlot.view_injective : Function.Injective model.NetSlot.view := by
  intro a b h
  cases a; cases b; simp_all [model.NetSlot.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.NetSlot model.NetSlot :=
  ⟨model.NetSlot.view, model.NetSlot.view_injective⟩

def model.SnapshotManifestRef.view (value : SandproofNetworkRust.sandbox_controller_core.model.SnapshotManifestRef) : model.SnapshotManifestRef := {
  digest := ValueView.view value.digest
}

theorem model.SnapshotManifestRef.view_injective : Function.Injective model.SnapshotManifestRef.view := by
  intro a b h
  cases a; cases b; simp_all [model.SnapshotManifestRef.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.SnapshotManifestRef model.SnapshotManifestRef :=
  ⟨model.SnapshotManifestRef.view, model.SnapshotManifestRef.view_injective⟩

def model.SnapshotStage.view : SandproofNetworkRust.sandbox_controller_core.model.SnapshotStage → model.SnapshotStage
  | .LocalSealed => .LocalSealed
  | .Durable => .Durable

theorem model.SnapshotStage.view_injective : Function.Injective model.SnapshotStage.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.SnapshotStage.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.SnapshotStage model.SnapshotStage :=
  ⟨model.SnapshotStage.view, model.SnapshotStage.view_injective⟩

def model.Snapshot.view (value : SandproofNetworkRust.sandbox_controller_core.model.Snapshot) : model.Snapshot := {
  owner := ValueView.view value.owner,
  sandbox := ValueView.view value.sandbox,
  source_generation := ValueView.view value.source_generation,
  workspace := ValueView.view value.workspace,
  manifest := ValueView.view value.manifest,
  stage := ValueView.view value.stage,
  immutable := ValueView.view value.immutable
}

theorem model.Snapshot.view_injective : Function.Injective model.Snapshot.view := by
  intro a b h
  cases a; cases b; simp_all [model.Snapshot.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Snapshot model.Snapshot :=
  ⟨model.Snapshot.view, model.Snapshot.view_injective⟩

def model.Filesystem.view (value : SandproofNetworkRust.sandbox_controller_core.model.Filesystem) : model.Filesystem := {
  owner := ValueView.view value.owner,
  sandbox := ValueView.view value.sandbox,
  created_generation := ValueView.view value.created_generation
}

theorem model.Filesystem.view_injective : Function.Injective model.Filesystem.view := by
  intro a b h
  cases a; cases b; simp_all [model.Filesystem.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Filesystem model.Filesystem :=
  ⟨model.Filesystem.view, model.Filesystem.view_injective⟩

def model.VmStatus.view : SandproofNetworkRust.sandbox_controller_core.model.VmStatus → model.VmStatus
  | .Prepared => .Prepared
  | .Executing => .Executing
  | .Frozen => .Frozen
  | .Stopped => .Stopped
  | .Reclaimed => .Reclaimed

theorem model.VmStatus.view_injective : Function.Injective model.VmStatus.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.VmStatus.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.VmStatus model.VmStatus :=
  ⟨model.VmStatus.view, model.VmStatus.view_injective⟩

def model.Vm.view (value : SandproofNetworkRust.sandbox_controller_core.model.Vm) : model.Vm := {
  owner := ValueView.view value.owner,
  sandbox := ValueView.view value.sandbox,
  generation := ValueView.view value.generation,
  status := ValueView.view value.status
}

theorem model.Vm.view_injective : Function.Injective model.Vm.view := by
  intro a b h
  cases a; cases b; simp_all [model.Vm.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Vm model.Vm :=
  ⟨model.Vm.view, model.Vm.view_injective⟩

def model.RestoreSource.view (value : SandproofNetworkRust.sandbox_controller_core.model.RestoreSource) : model.RestoreSource := {
  snapshot := ValueView.view value.snapshot,
  manifest := ValueView.view value.manifest
}

theorem model.RestoreSource.view_injective : Function.Injective model.RestoreSource.view := by
  intro a b h
  cases a; cases b; simp_all [model.RestoreSource.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.RestoreSource model.RestoreSource :=
  ⟨model.RestoreSource.view, model.RestoreSource.view_injective⟩

def model.WorkspaceApiAccess.view : SandproofNetworkRust.sandbox_controller_core.model.WorkspaceApiAccess → model.WorkspaceApiAccess
  | .ManagedReadWrite => .ManagedReadWrite

theorem model.WorkspaceApiAccess.view_injective : Function.Injective model.WorkspaceApiAccess.view := by
  intro a b h
  cases a; cases b; simp_all [model.WorkspaceApiAccess.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.WorkspaceApiAccess model.WorkspaceApiAccess :=
  ⟨model.WorkspaceApiAccess.view, model.WorkspaceApiAccess.view_injective⟩

def model.Authority.view : SandproofNetworkRust.sandbox_controller_core.model.Authority → model.Authority
  | .Closed => .Closed
  | .Running v0 v1 v2 v3 v4 v5 v6 => .Running (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3) (ValueView.view v4) (ValueView.view v5) (ValueView.view v6)
  | .Paused v0 v1 => .Paused (ValueView.view v0) (ValueView.view v1)

theorem model.Authority.view_injective : Function.Injective model.Authority.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.Authority.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Authority model.Authority :=
  ⟨model.Authority.view, model.Authority.view_injective⟩

def model.FailureKind.view : SandproofNetworkRust.sandbox_controller_core.model.FailureKind → model.FailureKind
  | .Cancelled => .Cancelled
  | .DeadlineExceeded => .DeadlineExceeded
  | .ResourceFailure => .ResourceFailure
  | .PolicyFailure => .PolicyFailure
  | .StorageFailure => .StorageFailure
  | .InternalFailure => .InternalFailure

theorem model.FailureKind.view_injective : Function.Injective model.FailureKind.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.FailureKind.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.FailureKind model.FailureKind :=
  ⟨model.FailureKind.view, model.FailureKind.view_injective⟩

def model.OperationStatus.view : SandproofNetworkRust.sandbox_controller_core.model.OperationStatus → model.OperationStatus
  | .Pending => .Pending
  | .OutcomeUnknown => .OutcomeUnknown
  | .Compensating v0 => .Compensating (ValueView.view v0)
  | .Succeeded => .Succeeded
  | .Failed v0 => .Failed (ValueView.view v0)

theorem model.OperationStatus.view_injective : Function.Injective model.OperationStatus.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.OperationStatus.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.OperationStatus model.OperationStatus :=
  ⟨model.OperationStatus.view, model.OperationStatus.view_injective⟩

def model.OperationKind.view : SandproofNetworkRust.sandbox_controller_core.model.OperationKind → model.OperationKind
  | .Create => .Create
  | .Pause => .Pause
  | .Resume => .Resume
  | .Kill => .Kill

theorem model.OperationKind.view_injective : Function.Injective model.OperationKind.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.OperationKind.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.OperationKind model.OperationKind :=
  ⟨model.OperationKind.view, model.OperationKind.view_injective⟩

def model.Operation.view (value : SandproofNetworkRust.sandbox_controller_core.model.Operation) : model.Operation := {
  id := ValueView.view value.id,
  epoch := ValueView.view value.epoch,
  kind := ValueView.view value.kind,
  status := ValueView.view value.status
}

theorem model.Operation.view_injective : Function.Injective model.Operation.view := by
  intro a b h
  cases a; cases b; simp_all [model.Operation.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Operation model.Operation :=
  ⟨model.Operation.view, model.Operation.view_injective⟩

def model.Phase.view : SandproofNetworkRust.sandbox_controller_core.model.Phase → model.Phase
  | .Starting => .Starting
  | .Running => .Running
  | .Pausing => .Pausing
  | .Paused => .Paused
  | .Revoking => .Revoking
  | .Destroyed => .Destroyed

theorem model.Phase.view_injective : Function.Injective model.Phase.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.Phase.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Phase model.Phase :=
  ⟨model.Phase.view, model.Phase.view_injective⟩

def model.SandboxRecord.view (value : SandproofNetworkRust.sandbox_controller_core.model.SandboxRecord) : model.SandboxRecord := {
  owner := ValueView.view value.owner,
  phase := ValueView.view value.phase,
  terminal := ValueView.view value.terminal,
  epoch := ValueView.view value.epoch,
  generation := ValueView.view value.generation,
  current_vm := ValueView.view value.current_vm,
  workspace := ValueView.view value.workspace,
  workspace_access := ValueView.view value.workspace_access,
  file_policy := ValueView.view value.file_policy,
  durable_snapshot := ValueView.view value.durable_snapshot,
  restore_from := ValueView.view value.restore_from,
  net_slot := ValueView.view value.net_slot,
  policy := ValueView.view value.policy,
  desired := ValueView.view value.desired,
  effective := ValueView.view value.effective,
  current_operation := ValueView.view value.current_operation
}

theorem model.SandboxRecord.view_injective : Function.Injective model.SandboxRecord.view := by
  intro a b h
  cases a; cases b; simp_all [model.SandboxRecord.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.SandboxRecord model.SandboxRecord :=
  ⟨model.SandboxRecord.view, model.SandboxRecord.view_injective⟩

def model.OperationResult.view : SandproofNetworkRust.sandbox_controller_core.model.OperationResult → model.OperationResult
  | .Running v0 => .Running (ValueView.view v0)
  | .Paused v0 => .Paused (ValueView.view v0)
  | .Destroyed => .Destroyed
  | .Failed v0 => .Failed (ValueView.view v0)

theorem model.OperationResult.view_injective : Function.Injective model.OperationResult.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.OperationResult.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.OperationResult model.OperationResult :=
  ⟨model.OperationResult.view, model.OperationResult.view_injective⟩

def model.OperationInput.view : SandproofNetworkRust.sandbox_controller_core.model.OperationInput → model.OperationInput
  | .Create v0 v1 v2 => .Create (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .Pause v0 => .Pause (ValueView.view v0)
  | .Resume v0 => .Resume (ValueView.view v0)
  | .Kill v0 => .Kill (ValueView.view v0)

theorem model.OperationInput.view_injective : Function.Injective model.OperationInput.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.OperationInput.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.OperationInput model.OperationInput :=
  ⟨model.OperationInput.view, model.OperationInput.view_injective⟩

def model.OperationLedgerRecord.view (value : SandproofNetworkRust.sandbox_controller_core.model.OperationLedgerRecord) : model.OperationLedgerRecord := {
  sandbox := ValueView.view value.sandbox,
  operation := ValueView.view value.operation,
  input := ValueView.view value.input,
  result := ValueView.view value.result
}

theorem model.OperationLedgerRecord.view_injective : Function.Injective model.OperationLedgerRecord.view := by
  intro a b h
  cases a; cases b; simp_all [model.OperationLedgerRecord.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.OperationLedgerRecord model.OperationLedgerRecord :=
  ⟨model.OperationLedgerRecord.view, model.OperationLedgerRecord.view_injective⟩

def model.OperationKey.view (value : SandproofNetworkRust.sandbox_controller_core.model.OperationKey) : model.OperationKey := {
  sandbox := ValueView.view value.sandbox,
  operation := ValueView.view value.operation
}

theorem model.OperationKey.view_injective : Function.Injective model.OperationKey.view := by
  intro a b h
  cases a; cases b; simp_all [model.OperationKey.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.OperationKey model.OperationKey :=
  ⟨model.OperationKey.view, model.OperationKey.view_injective⟩

def model.StopCoverage.view (value : SandproofNetworkRust.sandbox_controller_core.model.StopCoverage) : model.StopCoverage := {
  operation := ValueView.view value.operation,
  epoch := ValueView.view value.epoch,
  attempt := ValueView.view value.attempt,
  result := ValueView.view value.result
}

theorem model.StopCoverage.view_injective : Function.Injective model.StopCoverage.view := by
  intro a b h
  cases a; cases b; simp_all [model.StopCoverage.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.StopCoverage model.StopCoverage :=
  ⟨model.StopCoverage.view, model.StopCoverage.view_injective⟩

def model.LogicalEffectResult.view : SandproofNetworkRust.sandbox_controller_core.model.LogicalEffectResult → model.LogicalEffectResult
  | .Succeeded v0 v1 => .Succeeded (ValueView.view v0) (ValueView.view v1)
  | .Failed v0 v1 => .Failed (ValueView.view v0) (ValueView.view v1)
  | .RetiredByStop v0 v1 => .RetiredByStop (ValueView.view v0) (ValueView.view v1)

theorem model.LogicalEffectResult.view_injective : Function.Injective model.LogicalEffectResult.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.LogicalEffectResult.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.LogicalEffectResult model.LogicalEffectResult :=
  ⟨model.LogicalEffectResult.view, model.LogicalEffectResult.view_injective⟩

def model.EffectOrigin.view : SandproofNetworkRust.sandbox_controller_core.model.EffectOrigin → model.EffectOrigin
  | .Operation v0 => .Operation (ValueView.view v0)
  | .Maintenance v0 => .Maintenance (ValueView.view v0)

theorem model.EffectOrigin.view_injective : Function.Injective model.EffectOrigin.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.EffectOrigin.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.EffectOrigin model.EffectOrigin :=
  ⟨model.EffectOrigin.view, model.EffectOrigin.view_injective⟩

def model.LogicalEffectRecord.view (value : SandproofNetworkRust.sandbox_controller_core.model.LogicalEffectRecord) : model.LogicalEffectRecord := {
  origin := ValueView.view value.origin,
  current_attempt := ValueView.view value.current_attempt,
  remaining_retries := ValueView.view value.remaining_retries,
  result := ValueView.view value.result
}

theorem model.LogicalEffectRecord.view_injective : Function.Injective model.LogicalEffectRecord.view := by
  intro a b h
  cases a; cases b; simp_all [model.LogicalEffectRecord.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.LogicalEffectRecord model.LogicalEffectRecord :=
  ⟨model.LogicalEffectRecord.view, model.LogicalEffectRecord.view_injective⟩

def model.ResourceIndex.view (value : SandproofNetworkRust.sandbox_controller_core.model.ResourceIndex) : model.ResourceIndex := {
  vms := ValueView.view value.vms,
  slots := ValueView.view value.slots,
  snapshots := ValueView.view value.snapshots
}

theorem model.ResourceIndex.view_injective : Function.Injective model.ResourceIndex.view := by
  intro a b h
  cases a; cases b; simp_all [model.ResourceIndex.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.ResourceIndex model.ResourceIndex :=
  ⟨model.ResourceIndex.view, model.ResourceIndex.view_injective⟩

def model.OperationPlan.view (value : SandproofNetworkRust.sandbox_controller_core.model.OperationPlan) : model.OperationPlan := {
  key := ValueView.view value.key,
  epoch := ValueView.view value.epoch,
  generation := ValueView.view value.generation,
  vm := ValueView.view value.vm,
  slot := ValueView.view value.slot,
  restore_from := ValueView.view value.restore_from,
  target_snapshot := ValueView.view value.target_snapshot,
  stop_vms := ValueView.view value.stop_vms,
  close_slots := ValueView.view value.close_slots,
  retry_limit := ValueView.view value.retry_limit,
  sealed := ValueView.view value.sealed
}

theorem model.OperationPlan.view_injective : Function.Injective model.OperationPlan.view := by
  intro a b h
  cases a; cases b; simp_all [model.OperationPlan.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.OperationPlan model.OperationPlan :=
  ⟨model.OperationPlan.view, model.OperationPlan.view_injective⟩

def model.EffectOutcome.view : SandproofNetworkRust.sandbox_controller_core.model.EffectOutcome → model.EffectOutcome
  | .Succeeded v0 => .Succeeded (ValueView.view v0)
  | .Failed v0 => .Failed (ValueView.view v0)

theorem model.EffectOutcome.view_injective : Function.Injective model.EffectOutcome.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.EffectOutcome.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.EffectOutcome model.EffectOutcome :=
  ⟨model.EffectOutcome.view, model.EffectOutcome.view_injective⟩

def model.EffectStatus.view : SandproofNetworkRust.sandbox_controller_core.model.EffectStatus → model.EffectStatus
  | .Committed => .Committed
  | .Dispatched => .Dispatched
  | .OutcomeUnknown => .OutcomeUnknown
  | .Acknowledged v0 => .Acknowledged (ValueView.view v0)
  | .RetiredByStop v0 => .RetiredByStop (ValueView.view v0)

theorem model.EffectStatus.view_injective : Function.Injective model.EffectStatus.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.EffectStatus.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.EffectStatus model.EffectStatus :=
  ⟨model.EffectStatus.view, model.EffectStatus.view_injective⟩

def model.Command.view : SandproofNetworkRust.sandbox_controller_core.model.Command → model.Command
  | .InstallFence v0 v1 v2 => .InstallFence (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .PrepareVm v0 v1 v2 v3 v4 v5 => .PrepareVm (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3) (ValueView.view v4) (ValueView.view v5)
  | .ConfigureNetwork v0 v1 v2 v3 v4 v5 => .ConfigureNetwork (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3) (ValueView.view v4) (ValueView.view v5)
  | .ActivateVm v0 v1 v2 v3 => .ActivateVm (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3)
  | .FreezeVm v0 v1 v2 v3 => .FreezeVm (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3)
  | .ApplyFilePolicy v0 v1 v2 v3 v4 => .ApplyFilePolicy (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3) (ValueView.view v4)
  | .SealSnapshot v0 v1 v2 v3 v4 v5 v6 => .SealSnapshot (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3) (ValueView.view v4) (ValueView.view v5) (ValueView.view v6)
  | .CommitSnapshot v0 v1 v2 v3 v4 => .CommitSnapshot (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3) (ValueView.view v4)
  | .StopVm v0 v1 v2 v3 => .StopVm (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3)
  | .AbortPrepare v0 v1 v2 v3 v4 => .AbortPrepare (ValueView.view v0) (ValueView.view v1) (ValueView.view v2) (ValueView.view v3) (ValueView.view v4)
  | .CleanupVm v0 v1 v2 => .CleanupVm (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)
  | .CleanupSlot v0 v1 v2 => .CleanupSlot (ValueView.view v0) (ValueView.view v1) (ValueView.view v2)

theorem model.Command.view_injective : Function.Injective model.Command.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [model.Command.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.Command model.Command :=
  ⟨model.Command.view, model.Command.view_injective⟩

def model.LogicalEffectId.view (value : SandproofNetworkRust.sandbox_controller_core.model.LogicalEffectId) : model.LogicalEffectId := {
  command := ValueView.view value.command
}

theorem model.LogicalEffectId.view_injective : Function.Injective model.LogicalEffectId.view := by
  intro a b h
  cases a; cases b; simp_all [model.LogicalEffectId.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.LogicalEffectId model.LogicalEffectId :=
  ⟨model.LogicalEffectId.view, model.LogicalEffectId.view_injective⟩

def model.EffectId.view (value : SandproofNetworkRust.sandbox_controller_core.model.EffectId) : model.EffectId := {
  logical := ValueView.view value.logical,
  attempt := ValueView.view value.attempt
}

theorem model.EffectId.view_injective : Function.Injective model.EffectId.view := by
  intro a b h
  cases a; cases b; simp_all [model.EffectId.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.model.EffectId model.EffectId :=
  ⟨model.EffectId.view, model.EffectId.view_injective⟩

def maintenance.windows.WindowRecord.view (value : SandproofNetworkRust.sandbox_controller_core.maintenance.windows.WindowRecord) : maintenance.windows.WindowRecord := {
  id := ValueView.view value.id,
  tenant := ValueView.view value.tenant,
  issued_at_ms := ValueView.view value.issued_at_ms,
  expires_at_ms := ValueView.view value.expires_at_ms
}

theorem maintenance.windows.WindowRecord.view_injective : Function.Injective maintenance.windows.WindowRecord.view := by
  intro a b h
  cases a; cases b; simp_all [maintenance.windows.WindowRecord.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.windows.WindowRecord maintenance.windows.WindowRecord :=
  ⟨maintenance.windows.WindowRecord.view, maintenance.windows.WindowRecord.view_injective⟩

def maintenance.windows.WindowLimits.view (value : SandproofNetworkRust.sandbox_controller_core.maintenance.windows.WindowLimits) : maintenance.windows.WindowLimits := {
  max_open_windows := ValueView.view value.max_open_windows,
  max_duration_ms := ValueView.view value.max_duration_ms
}

theorem maintenance.windows.WindowLimits.view_injective : Function.Injective maintenance.windows.WindowLimits.view := by
  intro a b h
  cases a; cases b; simp_all [maintenance.windows.WindowLimits.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.windows.WindowLimits maintenance.windows.WindowLimits :=
  ⟨maintenance.windows.WindowLimits.view, maintenance.windows.WindowLimits.view_injective⟩

def maintenance.windows.WindowState.view (value : SandproofNetworkRust.sandbox_controller_core.maintenance.windows.WindowState) : maintenance.windows.WindowState := {
  limits := ValueView.view value.limits,
  issued_through := ValueView.view value.issued_through,
  time_floor_ms := ValueView.view value.time_floor_ms,
  «open» := ValueView.view value.«open»
}

theorem maintenance.windows.WindowState.view_injective : Function.Injective maintenance.windows.WindowState.view := by
  intro a b h
  cases a; cases b; simp_all [maintenance.windows.WindowState.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.windows.WindowState maintenance.windows.WindowState :=
  ⟨maintenance.windows.WindowState.view, maintenance.windows.WindowState.view_injective⟩

def maintenance.slots.HostSlotPhase.view : SandproofNetworkRust.sandbox_controller_core.maintenance.slots.HostSlotPhase → maintenance.slots.HostSlotPhase
  | .Occupied => .Occupied
  | .Releasing => .Releasing
  | .Free => .Free

theorem maintenance.slots.HostSlotPhase.view_injective : Function.Injective maintenance.slots.HostSlotPhase.view := by
  intro a b h
  cases a <;> cases b <;> simp_all [maintenance.slots.HostSlotPhase.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.slots.HostSlotPhase maintenance.slots.HostSlotPhase :=
  ⟨maintenance.slots.HostSlotPhase.view, maintenance.slots.HostSlotPhase.view_injective⟩

def maintenance.slots.HostBinding.view (value : SandproofNetworkRust.sandbox_controller_core.maintenance.slots.HostBinding) : maintenance.slots.HostBinding := {
  owner := ValueView.view value.owner,
  sandbox := ValueView.view value.sandbox,
  generation := ValueView.view value.generation,
  vm := ValueView.view value.vm,
  network := ValueView.view value.network
}

theorem maintenance.slots.HostBinding.view_injective : Function.Injective maintenance.slots.HostBinding.view := by
  intro a b h
  cases a; cases b; simp_all [maintenance.slots.HostBinding.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.slots.HostBinding maintenance.slots.HostBinding :=
  ⟨maintenance.slots.HostBinding.view, maintenance.slots.HostBinding.view_injective⟩

def maintenance.slots.HostSlotRecord.view (value : SandproofNetworkRust.sandbox_controller_core.maintenance.slots.HostSlotRecord) : maintenance.slots.HostSlotRecord := {
  binding := ValueView.view value.binding,
  phase := ValueView.view value.phase
}

theorem maintenance.slots.HostSlotRecord.view_injective : Function.Injective maintenance.slots.HostSlotRecord.view := by
  intro a b h
  cases a; cases b; simp_all [maintenance.slots.HostSlotRecord.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.slots.HostSlotRecord maintenance.slots.HostSlotRecord :=
  ⟨maintenance.slots.HostSlotRecord.view, maintenance.slots.HostSlotRecord.view_injective⟩

def maintenance.retirement.RetainedOperation.view (value : SandproofNetworkRust.sandbox_controller_core.maintenance.retirement.RetainedOperation) : maintenance.retirement.RetainedOperation := {
  owner := ValueView.view value.owner,
  ledger := ValueView.view value.ledger,
  expires_at := ValueView.view value.expires_at
}

theorem maintenance.retirement.RetainedOperation.view_injective : Function.Injective maintenance.retirement.RetainedOperation.view := by
  intro a b h
  cases a; cases b; simp_all [maintenance.retirement.RetainedOperation.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.retirement.RetainedOperation maintenance.retirement.RetainedOperation :=
  ⟨maintenance.retirement.RetainedOperation.view, maintenance.retirement.RetainedOperation.view_injective⟩

def maintenance.MaintenanceState.view (value : SandproofNetworkRust.sandbox_controller_core.maintenance.MaintenanceState) : maintenance.MaintenanceState := {
  sandbox_frontier := ValueView.view value.sandbox_frontier,
  workspace_frontier := ValueView.view value.workspace_frontier,
  retained := ValueView.view value.retained,
  time_floor := ValueView.view value.time_floor,
  max_retained_results := ValueView.view value.max_retained_results,
  max_host_slots := ValueView.view value.max_host_slots,
  slots := ValueView.view value.slots,
  windows := ValueView.view value.windows
}

theorem maintenance.MaintenanceState.view_injective : Function.Injective maintenance.MaintenanceState.view := by
  intro a b h
  cases a; cases b; simp_all [maintenance.MaintenanceState.view]

instance : ValueView SandproofNetworkRust.sandbox_controller_core.maintenance.MaintenanceState maintenance.MaintenanceState :=
  ⟨maintenance.MaintenanceState.view, maintenance.MaintenanceState.view_injective⟩

end SandboxModel.Local.State
