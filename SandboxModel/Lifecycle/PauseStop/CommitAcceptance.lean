import SandboxModel.Lifecycle.PauseStop.Composition
import SandboxModel.Lifecycle.PauseStop.Acceptance

namespace SandboxModel.Lifecycle.PauseStop.CommitAcceptance
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller
open SandboxModel.Lifecycle.PausePublication
open SandboxModel.Lifecycle.PauseStop.Acceptance

noncomputable section
local instance : DecidableEq model.EffectId := Classical.decEq _
local instance : DecidableEq model.LogicalEffectId := Classical.decEq _

/-! A finite interface-domain witness, not a V9 reachable-history theorem.
The original Stop effect/logical rows are absent. The real registration and
enqueue must create both rows and add the exact Stop to effect_keys/inflight. -/
def two (left right : T) : alloc.vec.Vec T := alloc.vec.Vec.from [left,right] (by scalar_tac)
def commitCommand : model.Command := .CommitSnapshot 1#u64 8#u64 9#u64 5#u64 source.manifest
def commitId : model.EffectId := ⟨⟨commitCommand⟩,0#u64⟩
def stopId : model.EffectId := Acceptance.id
def beforeHandoff : capture.CaptureHandoff := ⟨.Unknown,.Capturing,true,false,none,none⟩
def beforePlan : model.OperationPlan := {plan with capture := some beforeHandoff}
def beforeSnapshot : model.Snapshot := {snapshot with stage := .LocalSealed,immutable := false}
def before : model.World := {world with
  operation_plans := entry key beforePlan,
  snapshots := entry 5#u64 beforeSnapshot,
  logical_effects := entry ⟨commitCommand⟩ logicalRecord,
  effect_keys := one commitId,
  effects := entry commitId .OutcomeUnknown,
  inflight := one commitId}
def staged : model.World := {before with operation_plans := entry key plan}
def registeredWorld : model.World := {staged with snapshots := entry 5#u64 snapshot}
def committedWorld : model.World := {registeredWorld with
  logical_effects := ⟨two (⟨commitCommand⟩,logicalRecord) (⟨command⟩,logicalRecord)⟩,
  effects := ⟨two (commitId,.OutcomeUnknown) (stopId,.Committed)⟩,
  effect_keys := two stopId commitId}
def queuedWorld : model.World := {committedWorld with inflight := two stopId commitId}

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

theorem commit_stop_different : model.EffectId.Insts.CoreCmpPartialEqEffectId.eq commitId stopId = .ok false := by
  classical
  have compared := effect_query_eq commitId command 0#u64 (.stop 1#u64 8#u64 9#u64 2#u64)
  simpa [commitId,commitCommand,stopId,Acceptance.id,command,model.EffectId.mk.injEq,model.LogicalEffectId.mk.injEq] using compared

theorem commit_same : model.EffectId.Insts.CoreCmpPartialEqEffectId.eq commitId commitId = .ok true := by
  classical
  have compared := effect_query_eq commitId commitCommand 0#u64
    (.commit 1#u64 8#u64 9#u64 5#u64 source.manifest)
  simpa only [show commitId = ⟨⟨commitCommand⟩,0#u64⟩ from rfl,decide_true] using compared

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

/-- Evaluate only the actual two iterations needed by this singleton vector. -/
theorem queue_insert : transition.insert_unique model.EffectId.Insts.CoreCloneClone
    model.EffectId.Insts.CoreCmpPartialEqEffectId (one commitId) stopId = .ok (two stopId commitId) := by
  have first : transition.insert_unique_loop0.body model.EffectId.Insts.CoreCmpPartialEqEffectId
      (one commitId) stopId 0#usize false = .ok (.cont (1#usize,false)) := by
    simp [transition.insert_unique_loop0.body,one,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,commit_stop_different,zero_add_one]
  have last : transition.insert_unique_loop0.body model.EffectId.Insts.CoreCmpPartialEqEffectId
      (one commitId) stopId 1#usize false = .ok (.done false) := by
    simp [transition.insert_unique_loop0.body,one,alloc.vec.Vec.len]
  have scan : transition.insert_unique_loop0 model.EffectId.Insts.CoreCmpPartialEqEffectId
      (one commitId) stopId 0#usize false = .ok false := by
    unfold transition.insert_unique_loop0
    rw [finite_loop_continue _ _ _ first]
    exact finite_loop_finish _ _ _ last
  have copyFirst : transition.insert_unique_loop1.body model.EffectId.Insts.CoreCloneClone
      (one commitId) (one stopId) 0#usize = .ok (.cont (two stopId commitId,1#usize)) := by
    simp [transition.insert_unique_loop1.body,one,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,zero_add_one]
    change ((do
      let updated ← alloc.vec.Vec.push (one stopId) commitId
      .ok (.cont (updated,1#usize))) : Result (ControlFlow
        ((alloc.vec.Vec model.EffectId) × Usize) (alloc.vec.Vec model.EffectId))) =
      .ok (.cont (two stopId commitId,1#usize))
    rw [push_one]
    simp only [bind_tc_ok]
  have copyLast : transition.insert_unique_loop1.body model.EffectId.Insts.CoreCloneClone
      (one commitId) (two stopId commitId) 1#usize = .ok (.done (two stopId commitId)) := by
    simp [transition.insert_unique_loop1.body,one,alloc.vec.Vec.len]
  have copied : transition.insert_unique_loop1 model.EffectId.Insts.CoreCloneClone
      (one commitId) (one stopId) 0#usize = .ok (two stopId commitId) := by
    unfold transition.insert_unique_loop1
    rw [finite_loop_continue _ _ _ copyFirst]
    exact finite_loop_finish _ _ _ copyLast
  simp only [transition.insert_unique,scan,bind_tc_ok,Bool.false_eq_true,if_false,push_empty,copied]

theorem logical_different : model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq
    ⟨commitCommand⟩ ⟨command⟩ = .ok false := by
  classical
  have compared := logical_query_eq ⟨commitCommand⟩ command (.stop 1#u64 8#u64 9#u64 2#u64)
  simpa [commitCommand,command,model.LogicalEffectId.mk.injEq] using compared

theorem before_stop_absent :
    tableLookup stopId before.effects.entries.val = none ∧
    tableLookup ⟨command⟩ before.logical_effects.entries.val = none := by
  classical
  simp [before,entry,one,tableLookup,stopId,Acceptance.id,commitId,commitCommand,command,
    model.EffectId.mk.injEq,model.LogicalEffectId.mk.injEq]

theorem stop_effect_absent : store.Store.contains_key model.EffectId.Insts.CoreCmpPartialEqEffectId
    registeredWorld.effects stopId = .ok false := by
  classical
  have queried := get_exact_at model.EffectId.Insts.CoreCmpPartialEqEffectId stopId
    (fun stored => effect_query_eq stored command 0#u64 (.stop 1#u64 8#u64 9#u64 2#u64))
    registeredWorld.effects
  have absent : tableLookup stopId registeredWorld.effects.entries.val = none := before_stop_absent.1
  simp only [store.Store.contains_key,queried,absent,bind_tc_ok,core.option.Option.is_some]
  rfl

theorem stop_logical_absent : store.Store.contains_key model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    registeredWorld.logical_effects ⟨command⟩ = .ok false := by
  classical
  have queried := get_exact_at model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨command⟩
    (fun stored => logical_query_eq stored command (.stop 1#u64 8#u64 9#u64 2#u64))
    registeredWorld.logical_effects
  have absent : tableLookup ⟨command⟩ registeredWorld.logical_effects.entries.val = none := before_stop_absent.2
  simp only [store.Store.contains_key,queried,absent,bind_tc_ok,core.option.Option.is_some]
  rfl

theorem inferred_original_operation (state : model.World)
    (sandboxes : state.sandboxes = entry 1#u64 record) :
    transition.inferred_effect_origin state command = .ok (.Operation key) := by
  have stored := entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 record (by rfl)
  simp only [transition.inferred_effect_origin,command,model.Command.sandbox,model.Command.epoch,
    sandboxes,stored,bind_tc_ok]
  simp [record,operation,transition.operation_key,key]

theorem fresh_commit_effect : transition.commit_effect registeredWorld command = .ok committedWorld := by
  have effectsWrite := put_fresh_one model.EffectId.Insts.CoreCmpPartialEqEffectId
    commitId stopId model.EffectStatus.OutcomeUnknown .Committed commit_stop_different
  have logicalWrite := put_fresh_one model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    ⟨commitCommand⟩ ⟨command⟩ logicalRecord logicalRecord logical_different
  have origin := inferred_original_operation
    {registeredWorld with
      effects := twoEntries commitId stopId .OutcomeUnknown .Committed,
      effect_keys := two stopId commitId} rfl
  have initial : model.EffectId.initial command = .ok stopId := rfl
  have retryRecord : ({
      origin := model.EffectOrigin.Operation key,
      current_attempt := 0#u64,
      remaining_retries := transition.DEFAULT_RETRY_LIMIT,
      result := none} : model.LogicalEffectRecord) =
      logicalRecord := by simp [transition.DEFAULT_RETRY_LIMIT,logicalRecord]
  simp only [transition.commit_effect,model_Command_clone_exact,initial,stop_effect_absent,
    stop_logical_absent,bind_tc_ok,Bool.false_eq_true,if_false,model_EffectId_clone_exact]
  simp only [show registeredWorld.effects = entry commitId .OutcomeUnknown from rfl,effectsWrite,
    show registeredWorld.effect_keys = one commitId from rfl,queue_insert,bind_tc_ok]
  rw [origin]
  simp only [bind_tc_ok,retryRecord]
  simp only [show registeredWorld.logical_effects = entry ⟨commitCommand⟩ logicalRecord from rfl,
    logicalWrite,bind_tc_ok]
  rfl

theorem enqueue_missing_stop : transition.enqueue registeredWorld command = .ok queuedWorld := by
  have initial : model.EffectId.initial command = .ok stopId := rfl
  simp only [transition.enqueue,model_Command_clone_exact,initial,fresh_commit_effect,bind_tc_ok]
  change (do
    let inflight ← transition.insert_unique model.EffectId.Insts.CoreCloneClone
      model.EffectId.Insts.CoreCmpPartialEqEffectId (one commitId) stopId
    .ok {committedWorld with inflight := inflight}) = .ok queuedWorld
  simp only [queue_insert,bind_tc_ok,queuedWorld]

theorem captured_before : transition.current_capture before 1#u64 record = .ok (some beforeHandoff) := by
  simp [current_capture_exact,currentCaptureView,before,world,record,operation,beforePlan,plan,
    key,entry,one,tableLookup]

theorem bundle_observed : capture.CaptureHandoff.observe_bundle beforeHandoff = .ok (some handoff) := by
  simp [beforeHandoff,handoff,capture.CaptureHandoff.observe_bundle,
    capture.CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,capture.CaptureCommit.read_discriminant,
    capture.RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,capture.RestoreEligibility.read_discriminant]

theorem register_snapshot_existing (state : model.World)
    (indexes : state.resource_indexes = entry 1#u64 index) :
    worker.register_snapshot state 1#u64 5#u64 = .ok state := by
  have read := entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 index (by rfl)
  have resource : model.World.resource_index state 1#u64 = .ok index := by
    simp [model.World.resource_index,indexes,read,core.option.OptionShared0T.cloned,
      core.option.Option.unwrap_or_default]
  have inserted := insert_one ids.SnapshotId.Insts.CoreCloneClone
    ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId 5#u64 (by rfl)
  have written := entry_put ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 index index (by rfl)
  simp only [worker.register_snapshot,resource,bind_tc_ok,
    show index.snapshots = one 5#u64 from rfl,inserted]
  change (do
    let updated ← store.Store.put ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      state.resource_indexes 1#u64 index
    .ok {state with resource_indexes := updated}) = .ok state
  simp only [indexes,written,bind_tc_ok]
  rw [← indexes]

theorem snapshot_registered : worker.put_snapshot staged 5#u64 snapshot = .ok registeredWorld := by
  have registered := register_snapshot_existing staged rfl
  have written := entry_put ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId 5#u64 beforeSnapshot snapshot (by rfl)
  simp only [worker.put_snapshot,show snapshot.sandbox = 1#u64 from rfl,registered,bind_tc_ok,
    show staged.snapshots = entry 5#u64 beforeSnapshot from rfl,written,registeredWorld]

theorem normal_commit_accepts : worker.pause_stop.commit_snapshot before
    1#u64 8#u64 9#u64 5#u64 source.manifest = .ok (.Ok ⟨.CommitSnapshot 5#u64,queuedWorld⟩) := by
  have rowRead := entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 record (by rfl)
  have snapshotRead := entry_get ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId 5#u64 beforeSnapshot (by rfl)
  have planRead := entry_get model.OperationKey.Insts.CoreCmpPartialEqOperationKey key beforePlan key_same
  have currentPlan : worker.current_plan before 1#u64 record = .ok (.Ok (key,beforePlan)) := by
    simp only [worker.current_plan,show record.current_operation = some operation from rfl,
      transition.operation_key,bind_tc_ok,show operation.id = 6#u64 from rfl,
      show before.operation_plans = entry key beforePlan from rfl]
    change (do
      let found ← store.Store.get model.OperationKey.Insts.CoreCmpPartialEqOperationKey
        (entry key beforePlan) key
      match found with
      | none => .ok (core.result.Result.Err worker.WorkerReject.PlanNotFound)
      | some plan => .ok (core.result.Result.Ok (key,plan))) = .ok (.Ok (key,beforePlan))
    simp only [planRead,bind_tc_ok]
  have plansWrite := entry_put model.OperationKey.Insts.CoreCmpPartialEqOperationKey key beforePlan plan key_same
  simp only [worker.pause_stop.commit_snapshot,
    show before.snapshots = entry 5#u64 beforeSnapshot from rfl,snapshotRead,bind_tc_ok,
    show before.sandboxes = entry 1#u64 record from rfl,rowRead,currentPlan,
    core.result.Result.Insts.CoreOpsTry.branch,captured_before,core.option.Option.and_then,
    P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffOptionCaptureHandoff.call_once,bundle_observed,
    model_World_clone_exact,model_OperationPlan_clone_exact,
    show {beforeSnapshot with stage := .Durable,immutable := true} = snapshot from rfl,
    fixture_permitted]
  simp [record,operation,beforeSnapshot,snapshot,beforePlan,plan,source,
    core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
    ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.FsId.Insts.CoreCmpPartialEqFsId.eq]
  change ((do
    let plans ← store.Store.put model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      (entry key beforePlan) key plan
    let material ← worker.put_snapshot {before with operation_plans := plans} 5#u64 snapshot
    let next ← transition.enqueue material command
    .ok (.Ok ({event := .CommitSnapshot 5#u64,next := next} : worker.WorkerDecision))) :
      Result (core.result.Result worker.WorkerDecision worker.WorkerReject)) =
    .ok (.Ok ⟨.CommitSnapshot 5#u64,queuedWorld⟩)
  simp only [plansWrite,bind_tc_ok]
  change ((do
    let material ← worker.put_snapshot staged 5#u64 snapshot
    let next ← transition.enqueue material command
    .ok (.Ok ({event := .CommitSnapshot 5#u64,next := next} : worker.WorkerDecision))) :
      Result (core.result.Result worker.WorkerDecision worker.WorkerReject)) = _
  rw [snapshot_registered]
  simp only [bind_tc_ok,enqueue_missing_stop]

theorem normal_record_commit_accepts : worker.pause_stop.record before commitId =
    .ok (some (.Ok ⟨.CommitSnapshot 5#u64,queuedWorld⟩)) := by
  have logicalSame : model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq
      ⟨commitCommand⟩ ⟨commitCommand⟩ = .ok true := by
    have compared := logical_query_eq ⟨commitCommand⟩ commitCommand
      (.commit 1#u64 8#u64 9#u64 5#u64 source.manifest)
    simpa using compared
  have logicalRead := entry_get_copy model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    model.LogicalEffectRecord.Insts.CoreMarkerCopy ⟨commitCommand⟩ logicalRecord logicalSame
  have statusRead := entry_get model.EffectId.Insts.CoreCmpPartialEqEffectId commitId
    model.EffectStatus.OutcomeUnknown commit_same
  have presence := contains_one model.EffectId.Insts.CoreCmpPartialEqEffectId commitId commit_same
  have current : worker.current_effect_id before commitCommand = .ok commitId := by
    simp only [worker.current_effect_id,model_Command_clone_exact,
      show before.logical_effects = entry ⟨commitCommand⟩ logicalRecord from rfl,logicalRead,bind_tc_ok]
    rfl
  have late := pausing_late_route_none before commitId 1#u64 record rfl
    (by simp [before,world,entry,one,tableLookup]) rfl
  have same : core.cmp.PartialEq.ne.trait_default model.EffectId.Insts.CoreCmpPartialEqEffectId
      commitId commitId = .ok false := by
    simp [core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,commit_same]
  simp only [worker.pause_stop.record,commitId,model.EffectId.command,commitCommand,bind_tc_ok]
  change (do let response ← worker.pause_stop.execute before commitId; .ok (some response)) = _
  simp only [worker.pause_stop.execute,model_Command_clone_exact,
    show commitId.logical.command = commitCommand from rfl,current,same,bind_tc_ok,
    Bool.false_eq_true,if_false,show before.inflight = one commitId from rfl,presence,if_true,
    show before.effects = entry commitId .OutcomeUnknown from rfl,statusRead,worker.effect_awaiting,late]
  simp only [commitCommand,normal_commit_accepts,bind_tc_ok]

/-- The concrete successful result contains newly created, fully bound rows;
    Committed is not an acknowledgement or a proof of eventual dispatch. -/
theorem fresh_stop_created :
    tableLookup stopId queuedWorld.effects.entries.val = some .Committed ∧
    tableLookup ⟨command⟩ queuedWorld.logical_effects.entries.val = some logicalRecord ∧
    logicalRecord.origin = .Operation key ∧ logicalRecord.current_attempt = 0#u64 ∧
    queuedWorld.effect_keys.val = [stopId,commitId] ∧
    queuedWorld.inflight.val = [stopId,commitId] := by
  simp [queuedWorld,committedWorld,two,tableLookup,stopId,Acceptance.id,commitId,commitCommand,command,
    model.EffectId.mk.injEq,model.LogicalEffectId.mk.injEq,logicalRecord]

end
end SandboxModel.Lifecycle.PauseStop.CommitAcceptance
