import SandboxModel.Lifecycle.PauseDispatch.Composition
import SandboxModel.Lifecycle.PauseDispatch.Witness
import SandboxModel.Lifecycle.PauseStop.CommitAcceptance

open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

namespace SandboxModel.Lifecycle.PauseDispatch.Acceptance
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
    filesystems := (SandboxModel.Lifecycle.PausePublication.Fixture.world false).filesystems
    snapshots := entry 5#u64 snapshot
    net_slots := (SandboxModel.Lifecycle.PausePublication.Fixture.world false).net_slots
    applied_file_policies := (SandboxModel.Lifecycle.PausePublication.Fixture.world false).applied_file_policies
    worker_fences := entry 1#u64 ⟨8#u64,false⟩
    operations := (SandboxModel.Lifecycle.PausePublication.Fixture.world false).operations
    operation_plans := entry key plan
    resource_indexes := entry 1#u64 index
    logical_effects := beforeReceipts.logical_effects
    effect_keys := beforeReceipts.effect_keys
    effects := beforeReceipts.effects
    next_vm_id := 3#u64
    next_slot_id := 4#u64
    next_snapshot_id := 6#u64
    next_manifest_digest := 8#u64
    inflight := beforeReceipts.inflight
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
  classical
  have logicalRead : store.Store.get_copy model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      model.LogicalEffectRecord.Insts.CoreMarkerCopy world.logical_effects ⟨command⟩ = .ok (some logicalRecord) := by
    rw [SandboxModel.Lifecycle.PausePublication.get_copy_agrees,
      SandboxModel.Lifecycle.PausePublication.get_exact_at model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
        ⟨command⟩ (fun row => SandboxModel.Lifecycle.PausePublication.logical_query_eq row command (.stop _ _ _ _))]
    simp [world,beforeReceipts,SandboxModel.Lifecycle.PausePublication.Fixture.world,
      SandboxModel.Lifecycle.PausePublication.Fixture.five,tableLookup,command,logicalRecord,
      SandboxModel.Lifecycle.PausePublication.Fixture.fence,SandboxModel.Lifecycle.PausePublication.Fixture.freeze,
      SandboxModel.Lifecycle.PausePublication.Fixture.sealing,SandboxModel.Lifecycle.PausePublication.Fixture.commit,
      SandboxModel.Lifecycle.PausePublication.Fixture.stop,SandboxModel.Lifecycle.PausePublication.Fixture.logicalRecord,
      key,SandboxModel.Lifecycle.PausePublication.Fixture.key]
  have statusRead : store.Store.get model.EffectId.Insts.CoreCmpPartialEqEffectId world.effects id = .ok (some .Dispatched) := by
    rw [SandboxModel.Lifecycle.PausePublication.get_exact_at model.EffectId.Insts.CoreCmpPartialEqEffectId id
      (fun row => SandboxModel.Lifecycle.PausePublication.effect_query_eq row command 0#u64 (.stop _ _ _ _))]
    simp [world,beforeReceipts,SandboxModel.Lifecycle.PausePublication.Fixture.five,tableLookup,id,command,
      SandboxModel.Lifecycle.PausePublication.Fixture.id,SandboxModel.Lifecycle.PausePublication.Fixture.fence,
      SandboxModel.Lifecycle.PausePublication.Fixture.freeze,SandboxModel.Lifecycle.PausePublication.Fixture.sealing,
      SandboxModel.Lifecycle.PausePublication.Fixture.commit,SandboxModel.Lifecycle.PausePublication.Fixture.stop]
  have presence : worker.contains model.EffectId.Insts.CoreCmpPartialEqEffectId
      (alloc.vec.Vec.deref world.inflight) id = .ok true := by
    have same := id_same
    simp only [id,command] at same
    unfold worker.contains worker.contains_loop
    unfold loop
    simp [worker.contains_loop.body,world,beforeReceipts,
      SandboxModel.Lifecycle.PausePublication.Fixture.id,SandboxModel.Lifecycle.PausePublication.Fixture.stop,
      alloc.vec.Vec.deref,Slice.len,Slice.index_usize,core.cmp.impls.PartialEqShared.eq,id,command,same]
  have current : worker.current_effect_id world command = .ok id := by
    simp only [worker.current_effect_id,model_Command_clone_exact,
      logicalRead,bind_tc_ok]
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
    Bool.false_eq_true,if_false,presence,if_true,
    statusRead,worker.effect_awaiting,
    late]
  simp only [command,valid_stop_accepts,bind_tc_ok]

theorem after_stop_is_receipt_input : afterStop = beforeReceipts := by rfl

theorem actual_stop_then_receipts_then_publish :
    ∃ stopped middle next published,
      worker.pause_stop.record world id = .ok (some (.Ok stopped)) ∧
      effect.acknowledge_effect stopped.next id (.Succeeded 12#u64) = .ok middle ∧
      effect.acknowledge_effect middle (SandboxModel.Lifecycle.PausePublication.Fixture.id
        SandboxModel.Lifecycle.PausePublication.Fixture.commit) (.Succeeded 12#u64) = .ok next ∧
      publication.handle_publish_paused next 1#u64 source = .ok (.Ok published) := by
  obtain ⟨middle,next,published,stopAck,commitAck,publication⟩ := normal_two_receipts_then_publish
  exact ⟨⟨.StopVm 1#u64 2#u64,afterStop⟩,middle,next,published,valid_record_accepts,
    stopAck,commitAck,publication⟩

end SandboxModel.Lifecycle.PauseDispatch.Acceptance

namespace SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit
open SandboxModel.Lifecycle.PausePublication
open SandboxModel.Lifecycle.PausePublication.Fixture
open SandboxModel.Lifecycle.PauseStop.CommitAcceptance (finite_loop_continue finite_loop_finish)

def two (a b : T) : alloc.vec.Vec T := alloc.vec.Vec.from [a,b] (by scalar_tac)
def three (a b c : T) : alloc.vec.Vec T := alloc.vec.Vec.from [a,b,c] (by scalar_tac)
def four (a b c d : T) : alloc.vec.Vec T := alloc.vec.Vec.from [a,b,c,d] (by scalar_tac)
def five (a b c d e : T) : alloc.vec.Vec T := alloc.vec.Vec.from [a,b,c,d,e] (by scalar_tac)
def one (a : T) : alloc.vec.Vec T := alloc.vec.Vec.from [a] (by scalar_tac)
def commitCommand : model.Command := .CommitSnapshot 1#u64 8#u64 9#u64 5#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.source.manifest
def commitId : model.EffectId := ⟨⟨commitCommand⟩,0#u64⟩
def stopId : model.EffectId := SandboxModel.Lifecycle.PauseDispatch.Acceptance.id
def priorKeys : alloc.vec.Vec model.EffectId :=
  four (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze) (Fixture.id Fixture.sealing) commitId

theorem add_0 : (0#usize + 1#usize : Result Usize) = .ok 1#usize := by
  obtain ⟨next,evaluated,post⟩ := WP.spec_imp_exists
    (UScalar.add_spec (x := 0#usize) (y := 1#usize) (by scalar_tac))
  have same : next = 1#usize := by scalar_tac
  simpa only [same] using evaluated

theorem add_1 : (1#usize + 1#usize : Result Usize) = .ok 2#usize := by
  obtain ⟨next,evaluated,post⟩ := WP.spec_imp_exists
    (UScalar.add_spec (x := 1#usize) (y := 1#usize) (by scalar_tac))
  have same : next = 2#usize := by scalar_tac
  simpa only [same] using evaluated

theorem add_2 : (2#usize + 1#usize : Result Usize) = .ok 3#usize := by
  obtain ⟨next,evaluated,post⟩ := WP.spec_imp_exists
    (UScalar.add_spec (x := 2#usize) (y := 1#usize) (by scalar_tac))
  have same : next = 3#usize := by scalar_tac
  simpa only [same] using evaluated

theorem add_3 : (3#usize + 1#usize : Result Usize) = .ok 4#usize := by
  obtain ⟨next,evaluated,post⟩ := WP.spec_imp_exists
    (UScalar.add_spec (x := 3#usize) (y := 1#usize) (by scalar_tac))
  have same : next = 4#usize := by scalar_tac
  simpa only [same] using evaluated

theorem push_1 (a value : T) :
    alloc.vec.Vec.push (one a) value = .ok (two a value) := by
  have room : 2 ≤ U32.max := by scalar_tac
  simp [alloc.vec.Vec.push,one,two,room]

theorem push_2 (a b value : T) :
    alloc.vec.Vec.push (two a b) value = .ok (three a b value) := by
  have room : 3 ≤ U32.max := by scalar_tac
  simp [alloc.vec.Vec.push,two,three,room]

theorem push_3 (a b c value : T) :
    alloc.vec.Vec.push (three a b c) value = .ok (four a b c value) := by
  have room : 4 ≤ U32.max := by scalar_tac
  simp [alloc.vec.Vec.push,three,four,room]

theorem push_4 (a b c d value : T) :
    alloc.vec.Vec.push (four a b c d) value = .ok (five a b c d value) := by
  have room : 5 ≤ U32.max := by scalar_tac
  simp [alloc.vec.Vec.push,four,five,room]

theorem prior_0_different : model.EffectId.Insts.CoreCmpPartialEqEffectId.eq (Fixture.id Fixture.fence) stopId = .ok false := by
  classical
  have compared := effect_query_eq (Fixture.id Fixture.fence) SandboxModel.Lifecycle.PauseDispatch.Acceptance.command 0#u64 (.stop 1#u64 8#u64 9#u64 2#u64)
  simpa [Fixture.id,Fixture.fence,Fixture.freeze,Fixture.sealing,commitId,commitCommand,
    stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.id,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,model.EffectId.mk.injEq,model.LogicalEffectId.mk.injEq] using compared

theorem prior_1_different : model.EffectId.Insts.CoreCmpPartialEqEffectId.eq (Fixture.id Fixture.freeze) stopId = .ok false := by
  classical
  have compared := effect_query_eq (Fixture.id Fixture.freeze) SandboxModel.Lifecycle.PauseDispatch.Acceptance.command 0#u64 (.stop 1#u64 8#u64 9#u64 2#u64)
  simpa [Fixture.id,Fixture.fence,Fixture.freeze,Fixture.sealing,commitId,commitCommand,
    stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.id,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,model.EffectId.mk.injEq,model.LogicalEffectId.mk.injEq] using compared

theorem prior_2_different : model.EffectId.Insts.CoreCmpPartialEqEffectId.eq (Fixture.id Fixture.sealing) stopId = .ok false := by
  classical
  have compared := effect_query_eq (Fixture.id Fixture.sealing) SandboxModel.Lifecycle.PauseDispatch.Acceptance.command 0#u64 (.stop 1#u64 8#u64 9#u64 2#u64)
  simpa [Fixture.id,Fixture.fence,Fixture.freeze,Fixture.sealing,commitId,commitCommand,
    stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.id,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,model.EffectId.mk.injEq,model.LogicalEffectId.mk.injEq] using compared

theorem prior_3_different : model.EffectId.Insts.CoreCmpPartialEqEffectId.eq (commitId) stopId = .ok false := by
  classical
  have compared := effect_query_eq (commitId) SandboxModel.Lifecycle.PauseDispatch.Acceptance.command 0#u64 (.stop 1#u64 8#u64 9#u64 2#u64)
  simpa [Fixture.id,Fixture.fence,Fixture.freeze,Fixture.sealing,commitId,commitCommand,
    stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.id,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,model.EffectId.mk.injEq,model.LogicalEffectId.mk.injEq] using compared

theorem queue_insert_four : transition.insert_unique model.EffectId.Insts.CoreCloneClone
    model.EffectId.Insts.CoreCmpPartialEqEffectId priorKeys stopId =
    .ok (five stopId (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze) (Fixture.id Fixture.sealing) commitId) := by
  have scan0 : transition.insert_unique_loop0.body model.EffectId.Insts.CoreCmpPartialEqEffectId
      priorKeys stopId 0#usize false = .ok (.cont (1#usize,false)) := by
    simp [transition.insert_unique_loop0.body,priorKeys,four,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,prior_0_different,add_0]
  have scan1 : transition.insert_unique_loop0.body model.EffectId.Insts.CoreCmpPartialEqEffectId
      priorKeys stopId 1#usize false = .ok (.cont (2#usize,false)) := by
    simp [transition.insert_unique_loop0.body,priorKeys,four,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,prior_1_different,add_1]
  have scan2 : transition.insert_unique_loop0.body model.EffectId.Insts.CoreCmpPartialEqEffectId
      priorKeys stopId 2#usize false = .ok (.cont (3#usize,false)) := by
    simp [transition.insert_unique_loop0.body,priorKeys,four,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,prior_2_different,add_2]
  have scan3 : transition.insert_unique_loop0.body model.EffectId.Insts.CoreCmpPartialEqEffectId
      priorKeys stopId 3#usize false = .ok (.cont (4#usize,false)) := by
    simp [transition.insert_unique_loop0.body,priorKeys,four,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,prior_3_different,add_3]
  have scanEnd : transition.insert_unique_loop0.body model.EffectId.Insts.CoreCmpPartialEqEffectId
      priorKeys stopId 4#usize false = .ok (.done false) := by
    simp [transition.insert_unique_loop0.body,priorKeys,four,alloc.vec.Vec.len]
  have scanned : transition.insert_unique_loop0 model.EffectId.Insts.CoreCmpPartialEqEffectId
      priorKeys stopId 0#usize false = .ok false := by
    unfold transition.insert_unique_loop0
    rw [finite_loop_continue _ _ _ scan0]
    rw [finite_loop_continue _ _ _ scan1]
    rw [finite_loop_continue _ _ _ scan2]
    rw [finite_loop_continue _ _ _ scan3]
    exact finite_loop_finish _ _ _ scanEnd
  have copy0 : transition.insert_unique_loop1.body model.EffectId.Insts.CoreCloneClone
      priorKeys (one stopId) 0#usize = .ok (.cont (two stopId (Fixture.id Fixture.fence),1#usize)) := by
    simp [transition.insert_unique_loop1.body,priorKeys,four,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,model_EffectId_clone_exact,
      push_1,add_0]
  have copy1 : transition.insert_unique_loop1.body model.EffectId.Insts.CoreCloneClone
      priorKeys (two stopId (Fixture.id Fixture.fence)) 1#usize = .ok (.cont (three stopId (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze),2#usize)) := by
    simp [transition.insert_unique_loop1.body,priorKeys,four,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,model_EffectId_clone_exact,
      push_2,add_1]
  have copy2 : transition.insert_unique_loop1.body model.EffectId.Insts.CoreCloneClone
      priorKeys (three stopId (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze)) 2#usize = .ok (.cont (four stopId (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze) (Fixture.id Fixture.sealing),3#usize)) := by
    simp [transition.insert_unique_loop1.body,priorKeys,four,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,model_EffectId_clone_exact,
      push_3,add_2]
  have copy3 : transition.insert_unique_loop1.body model.EffectId.Insts.CoreCloneClone
      priorKeys (four stopId (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze) (Fixture.id Fixture.sealing)) 3#usize = .ok (.cont (five stopId (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze) (Fixture.id Fixture.sealing) commitId,4#usize)) := by
    have pushed := push_4 stopId (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze) (Fixture.id Fixture.sealing) commitId
    simp only [four] at pushed
    simp [transition.insert_unique_loop1.body,priorKeys,four,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,model_EffectId_clone_exact,
      pushed,add_3]
  have copyEnd : transition.insert_unique_loop1.body model.EffectId.Insts.CoreCloneClone
      priorKeys (five stopId (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze) (Fixture.id Fixture.sealing) commitId) 4#usize = .ok (.done (five stopId (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze) (Fixture.id Fixture.sealing) commitId)) := by
    simp [transition.insert_unique_loop1.body,priorKeys,four,alloc.vec.Vec.len]
  have copied : transition.insert_unique_loop1 model.EffectId.Insts.CoreCloneClone
      priorKeys (one stopId) 0#usize = .ok (five stopId (Fixture.id Fixture.fence) (Fixture.id Fixture.freeze) (Fixture.id Fixture.sealing) commitId) := by
    unfold transition.insert_unique_loop1
    rw [finite_loop_continue _ _ _ copy0]
    rw [finite_loop_continue _ _ _ copy1]
    rw [finite_loop_continue _ _ _ copy2]
    rw [finite_loop_continue _ _ _ copy3]
    exact finite_loop_finish _ _ _ copyEnd
  have pushed : alloc.vec.Vec.push (alloc.vec.Vec.new model.EffectId) stopId = .ok (one stopId) := by
    have room : 1 ≤ U32.max := by scalar_tac
    simp [alloc.vec.Vec.push,alloc.vec.Vec.new,one,room]
  simp only [transition.insert_unique,scanned,bind_tc_ok,Bool.false_eq_true,if_false,pushed,copied]

noncomputable section
local instance acceptanceDecidable1 : DecidableEq model.EffectId := Classical.decEq _
local instance acceptanceDecidable2 : DecidableEq model.LogicalEffectId := Classical.decEq _

def priorEffects : store.Store model.EffectId model.EffectStatus :=
  ⟨four (Fixture.id Fixture.fence,Fixture.success) (Fixture.id Fixture.freeze,Fixture.success)
    (Fixture.id Fixture.sealing,Fixture.success) (commitId,.OutcomeUnknown)⟩
def priorLogical : store.Store model.LogicalEffectId model.LogicalEffectRecord :=
  ⟨four (⟨Fixture.fence⟩,SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord) (⟨Fixture.freeze⟩,SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord)
    (⟨Fixture.sealing⟩,SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord) (⟨commitCommand⟩,SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord)⟩
def queued : model.World := {SandboxModel.Lifecycle.PauseDispatch.Acceptance.world with effects :=
  ⟨five (Fixture.id Fixture.fence,Fixture.success) (Fixture.id Fixture.freeze,Fixture.success)
    (Fixture.id Fixture.sealing,Fixture.success) (commitId,.OutcomeUnknown) (stopId,.Committed)⟩}
def beforeHandoff : capture.CaptureHandoff := ⟨.Unknown,.Capturing,true,false,none,none⟩
def beforePlan : model.OperationPlan := {SandboxModel.Lifecycle.PauseDispatch.Acceptance.plan with capture := some beforeHandoff}
def beforeSnapshot : model.Snapshot := {SandboxModel.Lifecycle.PauseDispatch.Acceptance.snapshot with stage := .LocalSealed,immutable := false}
def before : model.World := {queued with
  operation_plans := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry SandboxModel.Lifecycle.PauseDispatch.Acceptance.key beforePlan,
  snapshots := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry 5#u64 beforeSnapshot,
  logical_effects := priorLogical,effects := priorEffects,effect_keys := priorKeys,inflight := one commitId}
def staged : model.World := {before with operation_plans := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry SandboxModel.Lifecycle.PauseDispatch.Acceptance.key SandboxModel.Lifecycle.PauseDispatch.Acceptance.plan}
def registeredWorld : model.World := {staged with snapshots := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry 5#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.snapshot}
def committedWorld : model.World := {registeredWorld with
  logical_effects := queued.logical_effects,effects := queued.effects,effect_keys := queued.effect_keys}

theorem fresh_stop_initially_absent :
    tableLookup stopId before.effects.entries.val = none ∧
    tableLookup ⟨SandboxModel.Lifecycle.PauseDispatch.Acceptance.command⟩ before.logical_effects.entries.val = none := by
  simp [before,priorEffects,priorLogical,four,tableLookup,stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.id,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,
    commitId,commitCommand,Fixture.id,Fixture.fence,Fixture.freeze,Fixture.sealing]

theorem put_fresh_effect_row : store.Store.put model.EffectId.Insts.CoreCmpPartialEqEffectId
    priorEffects stopId .Committed = .ok queued.effects := by
  apply put_with_exact_entries model.EffectId.Insts.CoreCmpPartialEqEffectId stopId
    (fun row => effect_query_eq row SandboxModel.Lifecycle.PauseDispatch.Acceptance.command 0#u64 (.stop _ _ _ _))
  · apply Or.inr
    simp only [priorEffects,four,alloc.vec.Vec.from_val,List.length_cons,List.length_nil]
    scalar_tac
  · simp [priorEffects,queued,four,five,tablePut,stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.id,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,
      commitId,commitCommand,Fixture.id,Fixture.fence,Fixture.freeze,Fixture.sealing]

theorem put_fresh_logical_row : store.Store.put model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    priorLogical ⟨SandboxModel.Lifecycle.PauseDispatch.Acceptance.command⟩ SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord = .ok queued.logical_effects := by
  apply put_with_exact_entries model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨SandboxModel.Lifecycle.PauseDispatch.Acceptance.command⟩
    (fun row => logical_query_eq row SandboxModel.Lifecycle.PauseDispatch.Acceptance.command (.stop _ _ _ _))
  · apply Or.inr
    simp only [priorLogical,four,alloc.vec.Vec.from_val,List.length_cons,List.length_nil]
    scalar_tac
  · simp [priorLogical,queued,SandboxModel.Lifecycle.PauseDispatch.Acceptance.world,beforeReceipts,Fixture.world,Fixture.five,
      four,tablePut,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,commitCommand,Fixture.fence,Fixture.freeze,Fixture.sealing,Fixture.commit,
      Fixture.stop,SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord,Fixture.logicalRecord,SandboxModel.Lifecycle.PauseDispatch.Acceptance.key,Fixture.key,SandboxModel.Lifecycle.PauseDispatch.Acceptance.source]

theorem stop_effect_absent : store.Store.contains_key model.EffectId.Insts.CoreCmpPartialEqEffectId
    registeredWorld.effects stopId = .ok false := by
  have query := get_exact_at model.EffectId.Insts.CoreCmpPartialEqEffectId stopId
    (fun row => effect_query_eq row SandboxModel.Lifecycle.PauseDispatch.Acceptance.command 0#u64 (.stop _ _ _ _)) registeredWorld.effects
  have absent : tableLookup stopId registeredWorld.effects.entries.val = none := fresh_stop_initially_absent.1
  simp only [store.Store.contains_key,query,absent,bind_tc_ok,core.option.Option.is_some]
  rfl

theorem stop_logical_absent : store.Store.contains_key model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    registeredWorld.logical_effects ⟨SandboxModel.Lifecycle.PauseDispatch.Acceptance.command⟩ = .ok false := by
  have query := get_exact_at model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨SandboxModel.Lifecycle.PauseDispatch.Acceptance.command⟩
    (fun row => logical_query_eq row SandboxModel.Lifecycle.PauseDispatch.Acceptance.command (.stop _ _ _ _)) registeredWorld.logical_effects
  have absent : tableLookup ⟨SandboxModel.Lifecycle.PauseDispatch.Acceptance.command⟩ registeredWorld.logical_effects.entries.val = none := fresh_stop_initially_absent.2
  simp only [store.Store.contains_key,query,absent,bind_tc_ok,core.option.Option.is_some]
  rfl

theorem inflight_insert : transition.insert_unique model.EffectId.Insts.CoreCloneClone
    model.EffectId.Insts.CoreCmpPartialEqEffectId (one commitId) stopId = .ok (two stopId commitId) := by
  exact SandboxModel.Lifecycle.PauseStop.CommitAcceptance.queue_insert

theorem actual_fresh_commit_effect : transition.commit_effect registeredWorld SandboxModel.Lifecycle.PauseDispatch.Acceptance.command = .ok committedWorld := by
  have origin : transition.inferred_effect_origin
      {registeredWorld with effects := queued.effects,effect_keys := queued.effect_keys} SandboxModel.Lifecycle.PauseDispatch.Acceptance.command =
      .ok (.Operation SandboxModel.Lifecycle.PauseDispatch.Acceptance.key) := by
    exact SandboxModel.Lifecycle.PauseStop.CommitAcceptance.inferred_original_operation _ rfl
  have keysWrite : transition.insert_unique model.EffectId.Insts.CoreCloneClone
      model.EffectId.Insts.CoreCmpPartialEqEffectId priorKeys stopId = .ok queued.effect_keys := by
    exact queue_insert_four
  have initial : model.EffectId.initial SandboxModel.Lifecycle.PauseDispatch.Acceptance.command = .ok stopId := rfl
  have retryRecord : ({
      origin := model.EffectOrigin.Operation SandboxModel.Lifecycle.PauseDispatch.Acceptance.key,
      current_attempt := 0#u64,
      remaining_retries := transition.DEFAULT_RETRY_LIMIT,
      result := none} : model.LogicalEffectRecord) =
      SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord := by simp [transition.DEFAULT_RETRY_LIMIT,SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord]
  simp only [transition.commit_effect,model_Command_clone_exact,initial,stop_effect_absent,
    stop_logical_absent,bind_tc_ok,Bool.false_eq_true,if_false,model_EffectId_clone_exact]
  simp only [show registeredWorld.effects = priorEffects from rfl,put_fresh_effect_row,
    show registeredWorld.effect_keys = priorKeys from rfl,keysWrite,bind_tc_ok]
  rw [origin]
  simp only [bind_tc_ok,retryRecord,
    show registeredWorld.logical_effects = priorLogical from rfl,put_fresh_logical_row]
  rfl

end
noncomputable section
local instance acceptanceDecidable3 : DecidableEq model.EffectId := Classical.decEq _
local instance acceptanceDecidable4 : DecidableEq model.LogicalEffectId := Classical.decEq _

theorem enqueue_fresh_stop : transition.enqueue registeredWorld SandboxModel.Lifecycle.PauseDispatch.Acceptance.command = .ok queued := by
  have initial : model.EffectId.initial SandboxModel.Lifecycle.PauseDispatch.Acceptance.command = .ok stopId := rfl
  simp only [transition.enqueue,model_Command_clone_exact,initial,actual_fresh_commit_effect,bind_tc_ok]
  change (do
    let inflight ← transition.insert_unique model.EffectId.Insts.CoreCloneClone
      model.EffectId.Insts.CoreCmpPartialEqEffectId (one commitId) stopId
    .ok {committedWorld with inflight := inflight}) = .ok queued
  simp only [inflight_insert,bind_tc_ok]
  rfl

theorem captured_before : transition.current_capture before 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.record = .ok (some beforeHandoff) := by
  simp [current_capture_exact,currentCaptureView,before,SandboxModel.Lifecycle.PauseDispatch.Acceptance.record,SandboxModel.Lifecycle.PauseDispatch.Acceptance.operation,beforePlan,SandboxModel.Lifecycle.PauseDispatch.Acceptance.plan,
    SandboxModel.Lifecycle.PauseDispatch.Acceptance.key,SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry,SandboxModel.Lifecycle.PauseDispatch.Acceptance.one,tableLookup]

theorem bundle_observed : capture.CaptureHandoff.observe_bundle beforeHandoff = .ok (some SandboxModel.Lifecycle.PauseDispatch.Acceptance.handoff) := by
  simp [beforeHandoff,SandboxModel.Lifecycle.PauseDispatch.Acceptance.handoff,capture.CaptureHandoff.observe_bundle,
    capture.CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,capture.CaptureCommit.read_discriminant,
    capture.RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,capture.RestoreEligibility.read_discriminant]

theorem register_snapshot_existing (state : model.World)
    (indexes : state.resource_indexes = SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.index) :
    worker.register_snapshot state 1#u64 5#u64 = .ok state := by
  have read := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.index (by rfl)
  have resource : model.World.resource_index state 1#u64 = .ok SandboxModel.Lifecycle.PauseDispatch.Acceptance.index := by
    simp [model.World.resource_index,indexes,read,core.option.OptionShared0T.cloned,
      core.option.Option.unwrap_or_default]
  have inserted := SandboxModel.Lifecycle.PauseDispatch.Acceptance.insert_one ids.SnapshotId.Insts.CoreCloneClone
    ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId 5#u64 (by rfl)
  have written := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry_put ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.index SandboxModel.Lifecycle.PauseDispatch.Acceptance.index (by rfl)
  simp only [worker.register_snapshot,resource,bind_tc_ok,
    show SandboxModel.Lifecycle.PauseDispatch.Acceptance.index.snapshots = SandboxModel.Lifecycle.PauseDispatch.Acceptance.one 5#u64 from rfl,inserted]
  change (do
    let updated ← store.Store.put ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      state.resource_indexes 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.index
    .ok {state with resource_indexes := updated}) = .ok state
  simp only [indexes,written,bind_tc_ok]
  rw [← indexes]

theorem snapshot_registered : worker.put_snapshot staged 5#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.snapshot = .ok registeredWorld := by
  have registered := register_snapshot_existing staged rfl
  have written := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry_put ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId 5#u64 beforeSnapshot SandboxModel.Lifecycle.PauseDispatch.Acceptance.snapshot (by rfl)
  simp only [worker.put_snapshot,show SandboxModel.Lifecycle.PauseDispatch.Acceptance.snapshot.sandbox = 1#u64 from rfl,registered,bind_tc_ok,
    show staged.snapshots = SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry 5#u64 beforeSnapshot from rfl,written,registeredWorld]

theorem normal_commit_accepts : worker.pause_stop.commit_snapshot before
    1#u64 8#u64 9#u64 5#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.source.manifest = .ok (.Ok ⟨.CommitSnapshot 5#u64,queued⟩) := by
  have rowRead := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.record (by rfl)
  have snapshotRead := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry_get ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId 5#u64 beforeSnapshot (by rfl)
  have planRead := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry_get model.OperationKey.Insts.CoreCmpPartialEqOperationKey SandboxModel.Lifecycle.PauseDispatch.Acceptance.key beforePlan SandboxModel.Lifecycle.PauseDispatch.Acceptance.key_same
  have currentPlan : worker.current_plan before 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.record = .ok (.Ok (SandboxModel.Lifecycle.PauseDispatch.Acceptance.key,beforePlan)) := by
    simp only [worker.current_plan,show SandboxModel.Lifecycle.PauseDispatch.Acceptance.record.current_operation = some SandboxModel.Lifecycle.PauseDispatch.Acceptance.operation from rfl,
      transition.operation_key,bind_tc_ok,show SandboxModel.Lifecycle.PauseDispatch.Acceptance.operation.id = 6#u64 from rfl,
      show before.operation_plans = SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry SandboxModel.Lifecycle.PauseDispatch.Acceptance.key beforePlan from rfl]
    change (do
      let found ← store.Store.get model.OperationKey.Insts.CoreCmpPartialEqOperationKey
        (SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry SandboxModel.Lifecycle.PauseDispatch.Acceptance.key beforePlan) SandboxModel.Lifecycle.PauseDispatch.Acceptance.key
      match found with
      | none => .ok (core.result.Result.Err worker.WorkerReject.PlanNotFound)
      | some plan => .ok (core.result.Result.Ok (SandboxModel.Lifecycle.PauseDispatch.Acceptance.key,plan))) = .ok (.Ok (SandboxModel.Lifecycle.PauseDispatch.Acceptance.key,beforePlan))
    simp only [planRead,bind_tc_ok]
  have plansWrite := SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry_put model.OperationKey.Insts.CoreCmpPartialEqOperationKey SandboxModel.Lifecycle.PauseDispatch.Acceptance.key beforePlan SandboxModel.Lifecycle.PauseDispatch.Acceptance.plan SandboxModel.Lifecycle.PauseDispatch.Acceptance.key_same
  simp only [worker.pause_stop.commit_snapshot,
    show before.snapshots = SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry 5#u64 beforeSnapshot from rfl,snapshotRead,bind_tc_ok,
    show before.sandboxes = SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.record from rfl,rowRead,currentPlan,
    core.result.Result.Insts.CoreOpsTry.branch,captured_before,core.option.Option.and_then,
    P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffOptionCaptureHandoff.call_once,bundle_observed,
    model_World_clone_exact,model_OperationPlan_clone_exact,
    show {beforeSnapshot with stage := .Durable,immutable := true} = SandboxModel.Lifecycle.PauseDispatch.Acceptance.snapshot from rfl,
    SandboxModel.Lifecycle.PauseDispatch.Acceptance.fixture_permitted]
  simp [SandboxModel.Lifecycle.PauseDispatch.Acceptance.record,SandboxModel.Lifecycle.PauseDispatch.Acceptance.operation,beforeSnapshot,SandboxModel.Lifecycle.PauseDispatch.Acceptance.snapshot,beforePlan,SandboxModel.Lifecycle.PauseDispatch.Acceptance.plan,SandboxModel.Lifecycle.PauseDispatch.Acceptance.source,
    core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
    ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.FsId.Insts.CoreCmpPartialEqFsId.eq]
  change ((do
    let plans ← store.Store.put model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      (SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry SandboxModel.Lifecycle.PauseDispatch.Acceptance.key beforePlan) SandboxModel.Lifecycle.PauseDispatch.Acceptance.key SandboxModel.Lifecycle.PauseDispatch.Acceptance.plan
    let material ← worker.put_snapshot {before with operation_plans := plans} 5#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.snapshot
    let next ← transition.enqueue material SandboxModel.Lifecycle.PauseDispatch.Acceptance.command
    .ok (.Ok ({event := .CommitSnapshot 5#u64,next := next} : worker.WorkerDecision))) :
      Result (core.result.Result worker.WorkerDecision worker.WorkerReject)) =
    .ok (.Ok ⟨.CommitSnapshot 5#u64,queued⟩)
  simp only [plansWrite,bind_tc_ok]
  change ((do
    let material ← worker.put_snapshot staged 5#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.snapshot
    let next ← transition.enqueue material SandboxModel.Lifecycle.PauseDispatch.Acceptance.command
    .ok (.Ok ({event := .CommitSnapshot 5#u64,next := next} : worker.WorkerDecision))) :
      Result (core.result.Result worker.WorkerDecision worker.WorkerReject)) = _
  rw [snapshot_registered]
  simp only [bind_tc_ok,enqueue_fresh_stop]

theorem commit_id_same : model.EffectId.Insts.CoreCmpPartialEqEffectId.eq commitId commitId = .ok true := by
  have compared := effect_query_eq commitId commitCommand 0#u64 (.commit _ _ _ _ _)
  simpa [commitId] using compared

theorem actual_commit_record_accepts : worker.pause_stop.record before commitId =
    .ok (some (.Ok ⟨.CommitSnapshot 5#u64,queued⟩)) := by
  classical
  have logicalRead : store.Store.get_copy model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      model.LogicalEffectRecord.Insts.CoreMarkerCopy before.logical_effects ⟨commitCommand⟩ = .ok (some SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord) := by
    rw [get_copy_agrees,get_exact_at model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨commitCommand⟩
      (fun row => logical_query_eq row commitCommand (.commit _ _ _ _ _))]
    simp [before,priorLogical,four,tableLookup,commitCommand,Fixture.fence,Fixture.freeze,Fixture.sealing]
  have statusRead : store.Store.get model.EffectId.Insts.CoreCmpPartialEqEffectId before.effects commitId = .ok (some .OutcomeUnknown) := by
    rw [get_exact_at model.EffectId.Insts.CoreCmpPartialEqEffectId commitId
      (fun row => effect_query_eq row commitCommand 0#u64 (.commit _ _ _ _ _))]
    simp [before,priorEffects,four,tableLookup,commitId,commitCommand,Fixture.id,Fixture.fence,Fixture.freeze,Fixture.sealing]
  have presence : worker.contains model.EffectId.Insts.CoreCmpPartialEqEffectId (alloc.vec.Vec.deref before.inflight) commitId = .ok true := by
    change worker.contains model.EffectId.Insts.CoreCmpPartialEqEffectId (alloc.vec.Vec.deref (SandboxModel.Lifecycle.PauseDispatch.Acceptance.one commitId)) commitId = .ok true
    exact SandboxModel.Lifecycle.PauseDispatch.Acceptance.contains_one _ _ commit_id_same
  have current : worker.current_effect_id before commitCommand = .ok commitId := by
    simp only [worker.current_effect_id,model_Command_clone_exact,logicalRead,bind_tc_ok]
    rfl
  have late := SandboxModel.Lifecycle.PauseStop.pausing_late_route_none before commitId 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.record rfl
    (by simp [before,queued,SandboxModel.Lifecycle.PauseDispatch.Acceptance.world,SandboxModel.Lifecycle.PauseDispatch.Acceptance.entry,SandboxModel.Lifecycle.PauseDispatch.Acceptance.one,tableLookup]) rfl
  have same : core.cmp.PartialEq.ne.trait_default model.EffectId.Insts.CoreCmpPartialEqEffectId commitId commitId = .ok false := by
    simp [core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,commit_id_same]
  simp only [worker.pause_stop.record,commitId,model.EffectId.command,commitCommand,bind_tc_ok]
  change (do let response ← worker.pause_stop.execute before commitId; .ok (some response)) = _
  simp only [worker.pause_stop.execute,model_Command_clone_exact,
    show commitId.logical.command = commitCommand from rfl,current,same,bind_tc_ok,
    Bool.false_eq_true,if_false,presence,if_true,statusRead,worker.effect_awaiting,late]
  simp only [commitCommand,normal_commit_accepts,bind_tc_ok]

end
noncomputable section
local instance acceptanceDecidable5 : DecidableEq model.EffectId := Classical.decEq _

theorem dispatch_changes_exact_registered_stop : effect.dispatch_effect queued stopId = .ok SandboxModel.Lifecycle.PauseDispatch.Acceptance.world := by
  have write : store.Store.put model.EffectId.Insts.CoreCmpPartialEqEffectId queued.effects stopId .Dispatched =
      .ok SandboxModel.Lifecycle.PauseDispatch.Acceptance.world.effects := by
    apply put_with_exact_entries model.EffectId.Insts.CoreCmpPartialEqEffectId stopId
      (fun row => effect_query_eq row SandboxModel.Lifecycle.PauseDispatch.Acceptance.command 0#u64 (.stop _ _ _ _))
    · apply Or.inr
      simp only [queued,five,alloc.vec.Vec.from_val,List.length_cons,List.length_nil]
      scalar_tac
    · simp [queued,SandboxModel.Lifecycle.PauseDispatch.Acceptance.world,beforeReceipts,Fixture.five,five,tablePut,
        stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.id,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,commitId,commitCommand,
        Fixture.id,Fixture.fence,Fixture.freeze,Fixture.sealing,Fixture.commit,Fixture.stop,
        SandboxModel.Lifecycle.PauseDispatch.Acceptance.source]
  simp only [effect.dispatch_effect,model_World_clone_exact,model.EffectId.command,
    stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.id,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,model_Command_clone_exact,bind_tc_ok]
  change ((do
    let effects ← store.Store.put model.EffectId.Insts.CoreCmpPartialEqEffectId queued.effects stopId .Dispatched
    .ok {queued with effects}) : Result model.World) = .ok SandboxModel.Lifecycle.PauseDispatch.Acceptance.world
  rw [write]
  simp only [bind_tc_ok]
  rfl

theorem actual_registered_stop_dispatches : effect.dispatch_stop queued stopId =
    .ok (some (.Ok ⟨.DispatchEffect stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.world⟩)) := by
  have status : store.Store.get model.EffectId.Insts.CoreCmpPartialEqEffectId queued.effects stopId = .ok (some .Committed) := by
    rw [get_exact_at model.EffectId.Insts.CoreCmpPartialEqEffectId stopId
      (fun row => effect_query_eq row SandboxModel.Lifecycle.PauseDispatch.Acceptance.command 0#u64 (.stop _ _ _ _))]
    simp [queued,five,tableLookup,stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.id,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,
      commitId,commitCommand,Fixture.id,Fixture.fence,Fixture.freeze,Fixture.sealing]
  have decoded : model.EffectId.command stopId = .ok SandboxModel.Lifecycle.PauseDispatch.Acceptance.command := rfl
  have startup : effect.startup_dispatch_revoked queued SandboxModel.Lifecycle.PauseDispatch.Acceptance.command = .ok false := by
    exact stop_startup_not_revoked queued 1#u64 8#u64 9#u64 2#u64
  have allowed : capture.continuation.capture_dispatch_allowed queued stopId = .ok true := by
    exact stop_capture_dispatch_allowed queued 1#u64 8#u64 9#u64 2#u64 0#u64
  have eligible : effect.dispatch_eligible queued stopId = .ok (.Ok ⟨.DispatchEffect stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.world⟩) := by
    simp [effect.dispatch_eligible,show queued.controller_up = true from rfl,status,
      decoded,startup,allowed,model_EffectId_clone_exact,dispatch_changes_exact_registered_stop]
  simp only [effect.dispatch_stop,decoded,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,model_EffectId_clone_exact,eligible,bind_tc_ok]

end
noncomputable section
local instance acceptanceDecidable6 : DecidableEq model.EffectId := Classical.decEq _
local instance acceptanceDecidable7 : DecidableEq model.LogicalEffectId := Classical.decEq _

theorem full_stop_created :
    tableLookup stopId queued.effects.entries.val = some .Committed ∧
    tableLookup ⟨SandboxModel.Lifecycle.PauseDispatch.Acceptance.command⟩ queued.logical_effects.entries.val = some SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord ∧
    queued.effect_keys.val = stopId :: priorKeys.val ∧
    queued.inflight.val = [stopId,commitId] := by
  classical
  simp [queued,SandboxModel.Lifecycle.PauseDispatch.Acceptance.world,beforeReceipts,Fixture.world,priorKeys,four,five,Fixture.five,
    tableLookup,stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.id,SandboxModel.Lifecycle.PauseDispatch.Acceptance.command,commitId,commitCommand,Fixture.id,
    Fixture.fence,Fixture.freeze,Fixture.sealing,Fixture.commit,Fixture.stop,
    SandboxModel.Lifecycle.PauseDispatch.Acceptance.source,Fixture.logicalRecord,SandboxModel.Lifecycle.PauseDispatch.Acceptance.logicalRecord,Fixture.key,SandboxModel.Lifecycle.PauseDispatch.Acceptance.key]

end

end SandboxModel.Lifecycle.PauseDispatch.Acceptance.FreshCommit

namespace SandboxModel.Lifecycle.PauseDispatch.Acceptance
theorem fresh_commit_dispatch_stop_receipts_publish :
    ∃ committed dispatched stopped middle next published,
      worker.pause_stop.record FreshCommit.before FreshCommit.commitId = .ok (some (.Ok committed)) ∧
      effect.dispatch_stop committed.next FreshCommit.stopId = .ok (some (.Ok dispatched)) ∧
      worker.pause_stop.record dispatched.next FreshCommit.stopId = .ok (some (.Ok stopped)) ∧
      effect.acknowledge_effect stopped.next FreshCommit.stopId (.Succeeded 12#u64) = .ok middle ∧
      effect.acknowledge_effect middle FreshCommit.commitId (.Succeeded 12#u64) = .ok next ∧
      publication.handle_publish_paused next 1#u64 SandboxModel.Lifecycle.PauseDispatch.Acceptance.source = .ok (.Ok published) := by
  obtain ⟨stopped,middle,next,published,stop,stopAck,commitAck,publication⟩ := SandboxModel.Lifecycle.PauseDispatch.Acceptance.actual_stop_then_receipts_then_publish
  exact ⟨⟨.CommitSnapshot 5#u64,FreshCommit.queued⟩,
    ⟨.DispatchEffect FreshCommit.stopId,SandboxModel.Lifecycle.PauseDispatch.Acceptance.world⟩,stopped,middle,next,published,
    FreshCommit.actual_commit_record_accepts,FreshCommit.actual_registered_stop_dispatches,
    stop,stopAck,commitAck,publication⟩

end SandboxModel.Lifecycle.PauseDispatch.Acceptance
