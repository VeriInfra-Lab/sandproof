import SandboxModel.Lifecycle.PauseStop.Safety

/-! A non-vacuous interface-domain acceptance witness for the real correct
production entry. It is not a V9 reachability result. -/
namespace SandboxModel.Lifecycle.PauseStop.Acceptance
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

def one (value : T) : alloc.vec.Vec T := alloc.vec.Vec.from [value] (by scalar_tac)
def empty : store.Store K V := ⟨alloc.vec.Vec.new _⟩
def entry (key : K) (value : V) : store.Store K V := ⟨one (key,value)⟩
def source : model.RestoreSource := ⟨5#u64, ⟨7#u64⟩⟩
def operation : model.Operation := ⟨6#u64,8#u64,.Pause,.Pending⟩
def handoff : capture.CaptureHandoff := ⟨.Registered,.Recoverable,true,true,none,none⟩
def record : model.SandboxRecord :=
  ⟨10#u64,.Pausing,false,8#u64,9#u64,some 2#u64,4#u64,.ManagedReadWrite,⟨11#u64⟩,
   none,none,some 3#u64,.DenyAll,.Closed,
   .Running 1#u64 9#u64 2#u64 4#u64 .ManagedReadWrite 3#u64 .DenyAll,some operation⟩
def snapshot : model.Snapshot := ⟨10#u64,1#u64,9#u64,4#u64,source.manifest,.Durable,true⟩
def vm : model.Vm := ⟨10#u64,1#u64,9#u64,.Frozen⟩
def key : model.OperationKey := ⟨1#u64,6#u64⟩
def plan : model.OperationPlan :=
  ⟨key,8#u64,9#u64,some 2#u64,some 3#u64,none,some source,some handoff,
   one 2#u64,one 3#u64,3#u64,true⟩
def index : model.ResourceIndex := ⟨one 2#u64,one 3#u64,one 5#u64⟩
def command : model.Command := .StopVm 1#u64 8#u64 9#u64 2#u64
def id : model.EffectId := ⟨⟨command⟩,0#u64⟩
def logicalRecord : model.LogicalEffectRecord := ⟨.Operation key,0#u64,3#u64,none⟩
def world : model.World :=
  { maintenance := none
    sandboxes := entry 1#u64 record
    vms := entry 2#u64 vm
    filesystems := empty
    snapshots := entry 5#u64 snapshot
    net_slots := empty
    applied_file_policies := empty
    worker_fences := entry 1#u64 ⟨8#u64,false⟩
    operations := empty
    operation_plans := entry key plan
    resource_indexes := entry 1#u64 index
    logical_effects := entry ⟨command⟩ logicalRecord
    effect_keys := one id
    effects := entry id .OutcomeUnknown
    next_vm_id := 3#u64
    next_slot_id := 4#u64
    next_snapshot_id := 6#u64
    next_manifest_digest := 8#u64
    inflight := one id
    controller_up := true }
def afterStop : model.World := { world with vms := entry 2#u64 { vm with status := .Stopped } }

theorem entry_get (eq : core.cmp.PartialEq K K) (key : K) (value : V)
    (same : eq.eq key key = .ok true) :
    store.Store.get eq (entry key value) key = .ok (some value) := by
  unfold store.Store.get store.Store.get_loop
  unfold loop
  simp [store.Store.get_loop.body,entry,one,alloc.vec.Vec.len,
    alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,
    core.cmp.impls.PartialEqShared.eq,same]

theorem entry_get_copy (eq : core.cmp.PartialEq K K) (copy : core.marker.Copy V) (key : K) (value : V)
    (same : eq.eq key key = .ok true) :
    store.Store.get_copy eq copy (entry key value) key = .ok (some value) := by
  unfold store.Store.get_copy store.Store.get_copy_loop
  unfold loop
  simp [store.Store.get_copy_loop.body,entry,one,alloc.vec.Vec.len,
    alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,
    core.cmp.impls.PartialEqShared.eq,same]

theorem entry_put (eq : core.cmp.PartialEq K K) (key : K) (before after : V)
    (same : eq.eq key key = .ok true) :
    store.Store.put eq (entry key before) key after = .ok (entry key after) := by
  unfold store.Store.put store.Store.put_loop
  unfold loop
  simp [store.Store.put_loop.body,entry,one,alloc.vec.Vec.len,
    alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,
    alloc.vec.Vec.index_mut_usize,alloc.vec.Vec.set,same]

theorem contains_one (eq : core.cmp.PartialEq T T) (value : T)
    (same : eq.eq value value = .ok true) :
    worker.contains eq (alloc.vec.Vec.deref (one value)) value = .ok true := by
  unfold worker.contains worker.contains_loop
  unfold loop
  simp [worker.contains_loop.body,one,alloc.vec.Vec.deref,Slice.len,Slice.index_usize,
    core.cmp.impls.PartialEqShared.eq,same]

theorem insert_one (clone : core.clone.Clone T) (eq : core.cmp.PartialEq T T) (value : T)
    (same : eq.eq value value = .ok true) :
    worker.insert_unique clone eq (one value) value = .ok (one value) := by
  unfold worker.insert_unique worker.insert_unique_loop0
  unfold loop
  simp [worker.insert_unique_loop0.body,one,alloc.vec.Vec.len,
    alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,same]

theorem key_same : model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq key key = .ok true := by
  simp [key,model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
    ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq]

theorem command_same : model.Command.Insts.CoreCmpPartialEqCommand.eq command command = .ok true := by
  simp [command,model.Command.Insts.CoreCmpPartialEqCommand.eq,model.Command.read_discriminant,
    ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.VmId.Insts.CoreCmpPartialEqVmId.eq,lift]

theorem logical_same : model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq
    ⟨command⟩ ⟨command⟩ = .ok true := by
  simp only [model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq,command_same]

theorem id_same : model.EffectId.Insts.CoreCmpPartialEqEffectId.eq id id = .ok true := by
  simp [model.EffectId.Insts.CoreCmpPartialEqEffectId.eq,id,logical_same]

theorem fixture_capture : transition.current_capture world 1#u64 record = .ok (some handoff) := by
  have planRead := entry_get model.OperationKey.Insts.CoreCmpPartialEqOperationKey key plan key_same
  simp [transition.current_capture,record,operation,transition.operation_key,entry_get,
    core.option.Option.Insts.CoreOpsTry_traitTry.branch,SandproofFoundationsRust.core.option.Option.Insts.CoreOpsTry_traitTry.branch,world,key,plan,
    core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
    model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,model.OperationKind.read_discriminant,
    model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
    ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq]

theorem fixture_index : model.World.resource_index world 1#u64 = .ok index := by
  have stored := entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 index (by rfl)
  simp [model.World.resource_index,world,stored,core.option.OptionShared0T.cloned,
    core.option.Option.unwrap_or_default]

theorem fixture_register_vm : worker.register_vm world 1#u64 2#u64 = .ok world := by
  have inserted := insert_one ids.VmId.Insts.CoreCloneClone
    ids.VmId.Insts.CoreCmpPartialEqVmId 2#u64 (by rfl)
  have written := entry_put ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 index index (by rfl)
  simp only [worker.register_vm,fixture_index,bind_tc_ok,show index.vms = one 2#u64 from rfl,inserted]
  change (do
    let indexes ← store.Store.put ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      (entry 1#u64 index) 1#u64 index
    .ok {world with resource_indexes := indexes}) = .ok world
  simp only [written,bind_tc_ok]
  rfl

theorem fixture_put_vm : worker.put_vm world 2#u64 {vm with status := .Stopped} = .ok afterStop := by
  have written := entry_put ids.VmId.Insts.CoreCmpPartialEqVmId 2#u64 vm
    {vm with status := .Stopped} (by rfl)
  simp only [worker.put_vm,show vm.sandbox = 1#u64 from rfl,fixture_register_vm,bind_tc_ok,
    show world.vms = entry 2#u64 vm from rfl]
  change (do
    let vms ← store.Store.put ids.VmId.Insts.CoreCmpPartialEqVmId
      (entry 2#u64 vm) 2#u64 {vm with status := .Stopped}
    .ok {world with vms := vms}) = .ok afterStop
  simp only [written,bind_tc_ok]
  rfl

theorem fixture_permitted : capture.CaptureHandoff.may_stop_original handoff = .ok true := by
  simp [handoff,capture.CaptureHandoff.may_stop_original,
    capture.CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,capture.CaptureCommit.read_discriminant,
    capture.RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,capture.RestoreEligibility.read_discriminant]

theorem valid_stop_accepts : worker.pause_stop.stop_vm world 1#u64 8#u64 9#u64 2#u64 =
    .ok (.Ok ⟨.StopVm 1#u64 2#u64,afterStop⟩) := by
  have rowRead := entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 record (by rfl)
  have vmRead := entry_get ids.VmId.Insts.CoreCmpPartialEqVmId 2#u64 vm (by rfl)
  have fenceRead := entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64
    (⟨8#u64,false⟩ : model.WorkerFence) (by rfl)
  have planRead := entry_get model.OperationKey.Insts.CoreCmpPartialEqOperationKey key plan key_same
  have snapshotRead := entry_get ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId 5#u64 snapshot (by rfl)
  have currentPlan : worker.current_plan world 1#u64 record = .ok (.Ok (key,plan)) := by
    simp only [worker.current_plan,show record.current_operation = some operation from rfl,
      transition.operation_key,bind_tc_ok,show operation.id = 6#u64 from rfl,
      show world.operation_plans = entry key plan from rfl]
    change (do
      let found ← store.Store.get model.OperationKey.Insts.CoreCmpPartialEqOperationKey
        (entry key plan) key
      match found with
      | none => .ok (core.result.Result.Err worker.WorkerReject.PlanNotFound)
      | some plan => .ok (core.result.Result.Ok (key,plan))) = .ok (.Ok (key,plan))
    simp only [planRead,bind_tc_ok]
  simp only [worker.pause_stop.stop_vm,show world.sandboxes = entry 1#u64 record from rfl,
    rowRead,bind_tc_ok,show world.vms = entry 2#u64 vm from rfl,vmRead]
  simp only [show world.worker_fences = entry 1#u64 ⟨8#u64,false⟩ from rfl,fenceRead,
    currentPlan,core.result.Result.Insts.CoreOpsTry.branch,bind_tc_ok,fixture_capture,
    SandproofFoundationsRust.core.option.Option.is_some_and,
    P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffBool.call_once,
    fixture_permitted,
    model_World_clone_exact,fixture_put_vm]
  simp [record,vm,worker.stoppable_vm_status,
    core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
    ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    model.Phase.Insts.CoreCmpPartialEqPhase.eq,model.Phase.read_discriminant,
    core.option.Option.Insts.CoreCmpPartialEqOption.eq,ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
    model.VmStatus.Insts.CoreCmpPartialEqVmStatus.eq,model.VmStatus.read_discriminant,
    core.cmp.impls.PartialEqShared.eq,model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence.eq,
    plan,source,ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
    model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
    ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq,
    model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage.eq,model.SnapshotStage.read_discriminant]
  simp only [show world.snapshots = entry 5#u64 snapshot from rfl,snapshotRead,bind_tc_ok]
  simp [snapshot,source]

theorem valid_record_accepts : worker.pause_stop.record world id =
    .ok (some (.Ok ⟨.StopVm 1#u64 2#u64,afterStop⟩)) := by
  have logicalRead := entry_get_copy model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    model.LogicalEffectRecord.Insts.CoreMarkerCopy ⟨command⟩ logicalRecord logical_same
  have statusRead := entry_get model.EffectId.Insts.CoreCmpPartialEqEffectId id
    model.EffectStatus.OutcomeUnknown id_same
  have presence := contains_one model.EffectId.Insts.CoreCmpPartialEqEffectId id id_same
  have current : worker.current_effect_id world command = .ok id := by
    simp only [worker.current_effect_id,model_Command_clone_exact,
      show world.logical_effects = entry ⟨command⟩ logicalRecord from rfl,logicalRead,bind_tc_ok]
    rfl
  have late : worker.late_capture.record world id = .ok none := by
    have rowRead := entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 record (by rfl)
    simp only [worker.late_capture.record,id,model.EffectId.command,
      show command = .StopVm 1#u64 8#u64 9#u64 2#u64 from rfl,model.Command.sandbox,
      model.World.sandbox_record,show world.sandboxes = entry 1#u64 record from rfl,rowRead,
      bind_tc_ok,SandproofFoundationsRust.core.option.Option.Insts.CoreOpsTry_traitTry.branch,
      show record.terminal = false from rfl,Bool.false_eq_true,if_false]
  have same : core.cmp.PartialEq.ne.trait_default model.EffectId.Insts.CoreCmpPartialEqEffectId id id =
      .ok false := by
    simp [core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,id_same]
  simp only [worker.pause_stop.record,id,model.EffectId.command,command,bind_tc_ok]
  change (do let response ← worker.pause_stop.execute world id; .ok (some response)) = _
  simp only [worker.pause_stop.execute,model_Command_clone_exact,show id.logical.command = command from rfl,current,same,bind_tc_ok,
    Bool.false_eq_true,if_false,show world.inflight = one id from rfl,presence,if_true,
    show world.effects = entry id .OutcomeUnknown from rfl,statusRead,worker.effect_awaiting,
    late]
  simp only [command,valid_stop_accepts,bind_tc_ok]

def input : StopInput := ⟨1#u64,8#u64,9#u64,2#u64,0#u64⟩

theorem valid_input_accepted : stops world input := by
  exact ⟨record,⟨.StopVm 1#u64 2#u64,afterStop⟩,
    (by simp [input,world,entry,one,tableLookup]),rfl,valid_record_accepts⟩

theorem valid_input_has_permission : hasPermission world input :=
  record_stop_requires_permission world input valid_input_accepted

end SandboxModel.Lifecycle.PauseStop.Acceptance
