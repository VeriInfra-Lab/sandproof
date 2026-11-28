import SandboxModel.ProofAuditCommand
import SandboxModel.Lifecycle.PauseDispatch.Conditions
import SandboxModel.Foundations.Model

import PauseDispatchWrong.Funs
import SandboxModel.Foundations.Cloning
/-! Clone bridges for the actual PauseDispatchWrongRust generated bodies.
The ordinary-value proofs follow the existing Access.Cloning argument, but
all project types and clone functions here belong to the current caller
extraction, including both extra CaptureHandoff fields. No old Network
implementation theorem is used to identify these new types.

The fixed Std aliases and the backend's tuple BuiltinClone remain the
registered library/tool boundary. These are value-preservation statements;
they provide no physical allocation, Drop, cancellation or World frame law.
-/
namespace SandboxModel.Lifecycle.PauseDispatch.WrongBinding
open Aeneas Aeneas.Std PauseDispatchWrongRust
open PauseDispatchWrongRust.sandbox_controller_core

abbrev CloneExact := @SandboxModel.Foundations.CloneExact

@[simp] theorem vec_clone_exact (dictionary : core.clone.Clone T) (values : alloc.vec.Vec T)
    (exactClone : CloneExact dictionary) :
    alloc.vec.CloneVec.clone dictionary values = .ok values := by
  exact SandboxModel.Foundations.vec_clone_exact dictionary values exactClone

@[simp] theorem option_clone_exact (dictionary : core.clone.Clone T) (value : Option T)
    (exactClone : CloneExact dictionary) :
    core.option.Option.Insts.CoreCloneClone.clone dictionary value = .ok value := by
  change ∀ value, dictionary.clone value = .ok value at exactClone
  cases value <;> simp [core.option.Option.Insts.CoreCloneClone.clone, exactClone]

/-- This extraction clones Store entries with BuiltinClone on pairs. The
explicit key/value laws delimit the supported ordinary-value dictionaries;
the conclusion is about this generated body, not arbitrary Rust Clone effects. -/
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

/-- The real derived Copy clone returns the whole six-field handoff value. -/
@[simp] theorem capture_CaptureHandoff_clone_exact
    (value : sandbox_controller_core.capture.CaptureHandoff) :
    sandbox_controller_core.capture.CaptureHandoff.Insts.CoreCloneClone.clone value = .ok value := by
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


open SandboxModel.Foundations

def one (value : T) : alloc.vec.Vec T := alloc.vec.Vec.from [value] (by scalar_tac)
def two (left right : T) : alloc.vec.Vec T := alloc.vec.Vec.from [left,right] (by scalar_tac)
def empty : store.Store K V := ⟨alloc.vec.Vec.new _⟩
def entry (key : K) (value : V) : store.Store K V := ⟨one (key,value)⟩
def input : SandboxModel.Lifecycle.PauseDispatch.StopInput := ⟨1#u64,8#u64,9#u64,2#u64,7#u64⟩
def id : model.EffectId := ⟨⟨.StopVm 1#u64 8#u64 9#u64 2#u64⟩,7#u64⟩
def wrongId : model.EffectId := ⟨⟨.StopVm 1#u64 8#u64 0#u64 2#u64⟩,7#u64⟩
def world : model.World :=
  { maintenance := none, sandboxes := empty, vms := empty, filesystems := empty,
    snapshots := empty, net_slots := empty, applied_file_policies := empty,
    worker_fences := empty, operations := empty, operation_plans := empty,
    resource_indexes := empty, logical_effects := empty, effect_keys := one id,
    effects := entry id .Committed, next_vm_id := 3#u64, next_slot_id := 4#u64,
    next_snapshot_id := 6#u64, next_manifest_digest := 8#u64,
    inflight := one id, controller_up := true }
def after : model.World := {world with effects := ⟨two (id,.Committed) (wrongId,.Dispatched)⟩}
def decision : effect.EffectDecision := ⟨.DispatchEffect id,after⟩

theorem zero_add_one : (0#usize + 1#usize : Result Usize) = .ok 1#usize := by
  obtain ⟨next,evaluated,post⟩ := WP.spec_imp_exists
    (UScalar.add_spec (x := 0#usize) (y := 1#usize) (by scalar_tac))
  have same : next = 1#usize := by scalar_tac
  simpa only [same] using evaluated

theorem push_empty (value : T) :
    alloc.vec.Vec.push (alloc.vec.Vec.new T) value = .ok (one value) := by
  have room : 1 ≤ U32.max := by scalar_tac
  simp [alloc.vec.Vec.push,alloc.vec.Vec.new,one,room]

theorem push_one (left right : T) :
    alloc.vec.Vec.push (one left) right = .ok (two left right) := by
  have room : 2 ≤ U32.max := by scalar_tac
  simp [alloc.vec.Vec.push,one,two,room]

theorem finite_loop_continue (body : A → Result (ControlFlow A B)) (position next : A)
    (ran : body position = .ok (.cont next)) : loop body position = loop body next := by
  conv_lhs => unfold loop
  rw [ran]
  simp

theorem finite_loop_finish (body : A → Result (ControlFlow A B)) (position : A) (out : B)
    (ran : body position = .ok (.done out)) : loop body position = .ok out := by
  conv_lhs => unfold loop
  rw [ran]
  simp

def twoEntries (firstKey queryKey : K) (oldValue newValue : V) : store.Store K V :=
  ⟨two (firstKey,oldValue) (queryKey,newValue)⟩

/-- The one comparison actually executed suffices for this finite append. -/
theorem put_fresh_one (eq : core.cmp.PartialEq K K)
    (firstKey queryKey : K) (oldValue newValue : V)
    (different : eq.eq firstKey queryKey = .ok false) :
    store.Store.put eq (entry firstKey oldValue) queryKey newValue =
      .ok (twoEntries firstKey queryKey oldValue newValue) := by
  have first : store.Store.put_loop.body eq (entry firstKey oldValue) queryKey newValue 0#usize =
      .ok (.cont 1#usize) := by
    simp [store.Store.put_loop.body,entry,one,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,different,zero_add_one]
  have last : store.Store.put_loop.body eq (entry firstKey oldValue) queryKey newValue 1#usize =
      .ok (.done (two (firstKey,oldValue) (queryKey,newValue))) := by
    simp [store.Store.put_loop.body,entry,one,alloc.vec.Vec.len]
    change ((do
      let updated ← alloc.vec.Vec.push (one (firstKey,oldValue)) (queryKey,newValue)
      .ok (.done updated)) : Result (ControlFlow Usize (alloc.vec.Vec (K × V)))) =
      .ok (.done (two (firstKey,oldValue) (queryKey,newValue)))
    rw [push_one]
    simp only [bind_tc_ok]
  have scanned : store.Store.put_loop eq (entry firstKey oldValue) queryKey newValue 0#usize =
      .ok (two (firstKey,oldValue) (queryKey,newValue)) := by
    unfold store.Store.put_loop
    rw [finite_loop_continue _ _ _ first]
    exact finite_loop_finish _ _ _ last
  simp only [store.Store.put,scanned,bind_tc_ok]
  rfl

theorem entry_get (eq : core.cmp.PartialEq K K) (key : K) (value : V)
    (same : eq.eq key key = .ok true) :
    store.Store.get eq (entry key value) key = .ok (some value) := by
  unfold store.Store.get store.Store.get_loop
  unfold loop
  simp [store.Store.get_loop.body,entry,one,alloc.vec.Vec.len,
    alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,
    core.cmp.impls.PartialEqShared.eq,same]


theorem ids_compare : model.EffectId.Insts.CoreCmpPartialEqEffectId.eq id id = .ok true ∧
    model.EffectId.Insts.CoreCmpPartialEqEffectId.eq id wrongId = .ok false := by
  simp [id,wrongId,model.EffectId.Insts.CoreCmpPartialEqEffectId.eq,
    model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq,
    model.Command.Insts.CoreCmpPartialEqCommand.eq,model.Command.read_discriminant,
    ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.VmId.Insts.CoreCmpPartialEqVmId.eq,lift]

theorem actual_wrong_accepts : effect.dispatch_stop world id = .ok (some (.Ok decision)) := by
  have read := entry_get model.EffectId.Insts.CoreCmpPartialEqEffectId id model.EffectStatus.Committed ids_compare.1
  have write := put_fresh_one model.EffectId.Insts.CoreCmpPartialEqEffectId id wrongId
    model.EffectStatus.Committed model.EffectStatus.Dispatched ids_compare.2
  simp only [effect.dispatch_stop,id,model.EffectId.command,model_EffectId_clone_exact,bind_tc_ok]
  change (do let response ← effect.dispatch_eligible world id; .ok (some response)) = _
  simp only [effect.dispatch_eligible,show world.controller_up = true from rfl,if_true,
    show world.effects = entry id .Committed from rfl,read,bind_tc_ok,
    model.EffectId.command,show id.logical.command = .StopVm 1#u64 8#u64 9#u64 2#u64 from rfl,
    effect.startup_dispatch_revoked,effect.is_startup_command,Bool.false_eq_true,if_false,
    capture.continuation.capture_dispatch_allowed,capture.continuation.capture_command,
    model_EffectId_clone_exact,effect.dispatch_effect,model_World_clone_exact,
    model_Command_clone_exact,effect.misbound_stop]
  simp only [id,wrongId] at write ⊢
  rw [write]
  simp only [bind_tc_ok]
  rfl

noncomputable section
local instance : DecidableEq model.EffectId := Classical.decEq _

def actualAccepts (world : model.World) (input : SandboxModel.Lifecycle.PauseDispatch.StopInput)
    (decision : effect.EffectDecision) : Prop :=
  effect.dispatch_stop world ⟨⟨.StopVm input.sandbox input.epoch input.generation input.vm⟩,input.attempt⟩ =
    .ok (some (.Ok decision))

def bound (world : model.World) (input : SandboxModel.Lifecycle.PauseDispatch.StopInput)
    (decision : effect.EffectDecision) : Prop :=
  decision.event = .DispatchEffect ⟨⟨.StopVm input.sandbox input.epoch input.generation input.vm⟩,input.attempt⟩ ∧
  decision.next = {world with effects := decision.next.effects} ∧
  tableLookup ⟨⟨.StopVm input.sandbox input.epoch input.generation input.vm⟩,input.attempt⟩
    decision.next.effects.entries.val = some .Dispatched

theorem same_property_rejects :
    ¬ SandboxModel.Lifecycle.PauseDispatch.PreservesBoundStop actualAccepts bound := by
  intro safe
  have accepted : actualAccepts world input decision := actual_wrong_accepts
  have required := (safe world input decision accepted).2.2
  have unchanged : tableLookup id after.effects.entries.val = some .Committed := by
    simp [after,two,id,tableLookup]
  change tableLookup id after.effects.entries.val = some .Dispatched at required
  rw [unchanged] at required
  cases required

end
end SandboxModel.Lifecycle.PauseDispatch.WrongBinding

#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.vec_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.option_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.store_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_WorkerFence_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_AppliedFilePolicy_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_Ipv4Cidr_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_Protocol_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_Endpoint_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_NetworkPolicy_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.ids_TenantId_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.ids_VmId_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.ids_SandboxId_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_NetSlot_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_Snapshot_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_Filesystem_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_Vm_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_RestoreSource_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_WorkspaceApiAccess_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.ids_NetSlotId_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.ids_FsId_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_Authority_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_Operation_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_FilePolicyRef_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_Phase_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_SandboxRecord_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_OperationResult_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_OperationInput_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_OperationLedgerRecord_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_LogicalEffectRecord_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.ids_SnapshotId_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_ResourceIndex_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_OperationKey_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.capture_CaptureHandoff_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_OperationPlan_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_EffectStatus_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_SnapshotManifestRef_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_Command_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_LogicalEffectId_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_EffectId_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.maintenance_windows_WindowRecord_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.maintenance_windows_WindowLimits_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.maintenance_windows_WindowState_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.maintenance_slots_HostSlotRecord_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.maintenance_slots_HostSlotId_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.maintenance_retirement_RetainedOperation_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.maintenance_MaintenanceState_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.model_World_clone_exact
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.zero_add_one
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.push_empty
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.push_one
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.finite_loop_continue
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.finite_loop_finish
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.put_fresh_one
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.entry_get
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.ids_compare
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.actual_wrong_accepts
#audit_controller_proof SandboxModel.Lifecycle.PauseDispatch.WrongBinding.same_property_rejects
