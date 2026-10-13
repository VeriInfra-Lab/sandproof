import SandproofNetwork.Funs
namespace SandboxModel.Access
open Aeneas Aeneas.Std SandproofNetworkRust

@[simp] def CloneExact (dictionary : core.clone.Clone T) : Prop :=
  ∀ value, dictionary.clone value = .ok value

@[simp] theorem vec_clone_exact (dictionary : core.clone.Clone T) (values : alloc.vec.Vec T)
    (exactClone : CloneExact dictionary) :
    alloc.vec.CloneVec.clone dictionary values = .ok values := by
  obtain ⟨copied, copiedEq, valueEq⟩ := WP.spec_imp_exists
    (Slice.clone_spec (s := values.slice) (fun value _ => exactClone value))
  simp only [alloc.vec.CloneVec.clone, copiedEq]
  cases values
  cases valueEq
  simp

@[simp] theorem option_clone_exact (dictionary : core.clone.Clone T) (value : Option T)
    (exactClone : CloneExact dictionary) :
    core.option.Option.Insts.CoreCloneClone.clone dictionary value = .ok value := by
  unfold CloneExact at exactClone
  cases value <;> simp [core.option.Option.Insts.CoreCloneClone.clone, exactClone]

@[simp] theorem store_clone_exact (keyClone : core.clone.Clone K) (valueClone : core.clone.Clone V)
    (store : sandbox_controller_core.store.Store K V)
    (_keysExact : CloneExact keyClone) (_valuesExact : CloneExact valueClone) :
    sandbox_controller_core.store.Store.Insts.CoreCloneClone.clone keyClone valueClone store =
      .ok store := by
  have tupleExact : CloneExact (BuiltinClone (K × V)) := fun _ => rfl
  simp [sandbox_controller_core.store.Store.Insts.CoreCloneClone.clone,
    vec_clone_exact (BuiltinClone (K × V)) store.entries tupleExact]

@[simp] theorem model_WorkerFence_clone_exact (value : sandbox_controller_core.model.WorkerFence) :
    sandbox_controller_core.model.WorkerFence.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_AppliedFilePolicy_clone_exact (value : sandbox_controller_core.model.AppliedFilePolicy) :
    sandbox_controller_core.model.AppliedFilePolicy.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_Ipv4Cidr_clone_exact (value : sandbox_controller_core.model.Ipv4Cidr) :
    sandbox_controller_core.model.Ipv4Cidr.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_Protocol_clone_exact (value : sandbox_controller_core.model.Protocol) :
    sandbox_controller_core.model.Protocol.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_Endpoint_clone_exact (value : sandbox_controller_core.model.Endpoint) :
    sandbox_controller_core.model.Endpoint.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.model.Endpoint.Insts.CoreCloneClone.clone, lift]

@[simp] theorem model_NetworkPolicy_clone_exact (value : sandbox_controller_core.model.NetworkPolicy) :
    sandbox_controller_core.model.NetworkPolicy.Insts.CoreCloneClone.clone value = .ok value := by
  cases value <;> simp [sandbox_controller_core.model.NetworkPolicy.Insts.CoreCloneClone.clone, lift]

@[simp] theorem ids_TenantId_clone_exact (value : sandbox_controller_core.ids.TenantId) :
    sandbox_controller_core.ids.TenantId.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem ids_VmId_clone_exact (value : sandbox_controller_core.ids.VmId) :
    sandbox_controller_core.ids.VmId.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem ids_SandboxId_clone_exact (value : sandbox_controller_core.ids.SandboxId) :
    sandbox_controller_core.ids.SandboxId.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_NetSlot_clone_exact (value : sandbox_controller_core.model.NetSlot) :
    sandbox_controller_core.model.NetSlot.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.model.NetSlot.Insts.CoreCloneClone.clone, lift]

@[simp] theorem model_Snapshot_clone_exact (value : sandbox_controller_core.model.Snapshot) :
    sandbox_controller_core.model.Snapshot.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_Filesystem_clone_exact (value : sandbox_controller_core.model.Filesystem) :
    sandbox_controller_core.model.Filesystem.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_Vm_clone_exact (value : sandbox_controller_core.model.Vm) :
    sandbox_controller_core.model.Vm.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_RestoreSource_clone_exact (value : sandbox_controller_core.model.RestoreSource) :
    sandbox_controller_core.model.RestoreSource.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_WorkspaceApiAccess_clone_exact (value : sandbox_controller_core.model.WorkspaceApiAccess) :
    sandbox_controller_core.model.WorkspaceApiAccess.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem ids_NetSlotId_clone_exact (value : sandbox_controller_core.ids.NetSlotId) :
    sandbox_controller_core.ids.NetSlotId.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem ids_FsId_clone_exact (value : sandbox_controller_core.ids.FsId) :
    sandbox_controller_core.ids.FsId.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_Authority_clone_exact (value : sandbox_controller_core.model.Authority) :
    sandbox_controller_core.model.Authority.Insts.CoreCloneClone.clone value = .ok value := by
  cases value <;> simp [sandbox_controller_core.model.Authority.Insts.CoreCloneClone.clone, lift]

@[simp] theorem model_Operation_clone_exact (value : sandbox_controller_core.model.Operation) :
    sandbox_controller_core.model.Operation.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_FilePolicyRef_clone_exact (value : sandbox_controller_core.model.FilePolicyRef) :
    sandbox_controller_core.model.FilePolicyRef.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_Phase_clone_exact (value : sandbox_controller_core.model.Phase) :
    sandbox_controller_core.model.Phase.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_SandboxRecord_clone_exact (value : sandbox_controller_core.model.SandboxRecord) :
    sandbox_controller_core.model.SandboxRecord.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.model.SandboxRecord.Insts.CoreCloneClone.clone, lift]

@[simp] theorem model_OperationResult_clone_exact (value : sandbox_controller_core.model.OperationResult) :
    sandbox_controller_core.model.OperationResult.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_OperationInput_clone_exact (value : sandbox_controller_core.model.OperationInput) :
    sandbox_controller_core.model.OperationInput.Insts.CoreCloneClone.clone value = .ok value := by
  cases value <;> simp [sandbox_controller_core.model.OperationInput.Insts.CoreCloneClone.clone, lift]

@[simp] theorem model_OperationLedgerRecord_clone_exact (value : sandbox_controller_core.model.OperationLedgerRecord) :
    sandbox_controller_core.model.OperationLedgerRecord.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.model.OperationLedgerRecord.Insts.CoreCloneClone.clone]

@[simp] theorem model_LogicalEffectRecord_clone_exact (value : sandbox_controller_core.model.LogicalEffectRecord) :
    sandbox_controller_core.model.LogicalEffectRecord.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem ids_SnapshotId_clone_exact (value : sandbox_controller_core.ids.SnapshotId) :
    sandbox_controller_core.ids.SnapshotId.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_ResourceIndex_clone_exact (value : sandbox_controller_core.model.ResourceIndex) :
    sandbox_controller_core.model.ResourceIndex.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.model.ResourceIndex.Insts.CoreCloneClone.clone]

@[simp] theorem model_OperationKey_clone_exact (value : sandbox_controller_core.model.OperationKey) :
    sandbox_controller_core.model.OperationKey.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_OperationPlan_clone_exact (value : sandbox_controller_core.model.OperationPlan) :
    sandbox_controller_core.model.OperationPlan.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.model.OperationPlan.Insts.CoreCloneClone.clone, lift]

@[simp] theorem model_EffectStatus_clone_exact (value : sandbox_controller_core.model.EffectStatus) :
    sandbox_controller_core.model.EffectStatus.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_SnapshotManifestRef_clone_exact (value : sandbox_controller_core.model.SnapshotManifestRef) :
    sandbox_controller_core.model.SnapshotManifestRef.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem model_Command_clone_exact (value : sandbox_controller_core.model.Command) :
    sandbox_controller_core.model.Command.Insts.CoreCloneClone.clone value = .ok value := by
  cases value <;> simp [sandbox_controller_core.model.Command.Insts.CoreCloneClone.clone, lift]

@[simp] theorem model_LogicalEffectId_clone_exact (value : sandbox_controller_core.model.LogicalEffectId) :
    sandbox_controller_core.model.LogicalEffectId.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.model.LogicalEffectId.Insts.CoreCloneClone.clone]

@[simp] theorem model_EffectId_clone_exact (value : sandbox_controller_core.model.EffectId) :
    sandbox_controller_core.model.EffectId.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.model.EffectId.Insts.CoreCloneClone.clone, lift]

@[simp] theorem maintenance_windows_WindowRecord_clone_exact (value : sandbox_controller_core.maintenance.windows.WindowRecord) :
    sandbox_controller_core.maintenance.windows.WindowRecord.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem maintenance_windows_WindowLimits_clone_exact (value : sandbox_controller_core.maintenance.windows.WindowLimits) :
    sandbox_controller_core.maintenance.windows.WindowLimits.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem maintenance_windows_WindowState_clone_exact (value : sandbox_controller_core.maintenance.windows.WindowState) :
    sandbox_controller_core.maintenance.windows.WindowState.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.maintenance.windows.WindowState.Insts.CoreCloneClone.clone, lift]

@[simp] theorem maintenance_slots_HostSlotRecord_clone_exact (value : sandbox_controller_core.maintenance.slots.HostSlotRecord) :
    sandbox_controller_core.maintenance.slots.HostSlotRecord.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem maintenance_slots_HostSlotId_clone_exact (value : sandbox_controller_core.maintenance.slots.HostSlotId) :
    sandbox_controller_core.maintenance.slots.HostSlotId.Insts.CoreCloneClone.clone value = .ok value := by
  rfl

@[simp] theorem maintenance_retirement_RetainedOperation_clone_exact (value : sandbox_controller_core.maintenance.retirement.RetainedOperation) :
    sandbox_controller_core.maintenance.retirement.RetainedOperation.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.maintenance.retirement.RetainedOperation.Insts.CoreCloneClone.clone, lift]

@[simp] theorem maintenance_MaintenanceState_clone_exact (value : sandbox_controller_core.maintenance.MaintenanceState) :
    sandbox_controller_core.maintenance.MaintenanceState.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.maintenance.MaintenanceState.Insts.CoreCloneClone.clone, lift]

@[simp] theorem model_World_clone_exact (value : sandbox_controller_core.model.World) :
    sandbox_controller_core.model.World.Insts.CoreCloneClone.clone value = .ok value := by
  cases value
  simp [sandbox_controller_core.model.World.Insts.CoreCloneClone.clone, lift]

end SandboxModel.Access
