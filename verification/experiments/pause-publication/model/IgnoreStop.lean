import PauseStopWrong.Funs
import SandboxModel.Foundations.Cloning
import SandboxModel.Foundations.Model
import SandboxModel.Lifecycle.PausePublication.Conditions
import SandboxModel.ProofAuditCommand

/-! Real Rust Stop-guard mutant, generated independently with the original tools.
The finite interface-domain witness is not a V9 reachability theorem.
Clone facts concern ordinary values under the registered backend/library boundary. -/
namespace SandboxModel.Lifecycle.PausePublication.IgnoreStop
open Aeneas Aeneas.Std PauseStopWrongRust SandboxModel.Foundations
open PauseStopWrongRust.sandbox_controller_core

def one (value : T) : alloc.vec.Vec T := alloc.vec.Vec.from [value] (by scalar_tac)
def five (a b c d e : T) : alloc.vec.Vec T := alloc.vec.Vec.from [a,b,c,d,e] (by scalar_tac)
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
def vm : model.Vm := ⟨10#u64,1#u64,9#u64,.Stopped⟩
def slot : model.NetSlot := ⟨10#u64,1#u64,9#u64,2#u64,.DenyAll,true,false,false⟩
def key : model.OperationKey := ⟨1#u64,6#u64⟩
def ledger : model.OperationLedgerRecord := ⟨1#u64,operation,.Pause 9#u64,none⟩
def plan : model.OperationPlan :=
  ⟨key,8#u64,9#u64,some 2#u64,some 3#u64,none,some source,some handoff,
   one 2#u64,one 3#u64,3#u64,true⟩
def index : model.ResourceIndex := ⟨one 2#u64,one 3#u64,one 5#u64⟩
def fence : model.Command := .InstallFence 1#u64 8#u64 false
def freeze : model.Command := .FreezeVm 1#u64 8#u64 9#u64 2#u64
def sealing : model.Command := .SealSnapshot 1#u64 8#u64 9#u64 2#u64 4#u64 5#u64 ⟨7#u64⟩
def commit : model.Command := .CommitSnapshot 1#u64 8#u64 9#u64 5#u64 ⟨7#u64⟩
def stop : model.Command := .StopVm 1#u64 8#u64 9#u64 2#u64
def id (command : model.Command) : model.EffectId := ⟨⟨command⟩,0#u64⟩
def logicalRecord : model.LogicalEffectRecord := ⟨.Operation key,0#u64,3#u64,none⟩
def success : model.EffectStatus := .Acknowledged (.Succeeded 12#u64)
def world (hasStop : Bool) : model.World :=
  { maintenance := none
    sandboxes := entry 1#u64 record
    vms := entry 2#u64 vm
    filesystems := entry 4#u64 ⟨10#u64,1#u64,9#u64⟩
    snapshots := entry 5#u64 snapshot
    net_slots := entry 3#u64 slot
    applied_file_policies := entry 1#u64 ⟨1#u64,9#u64,4#u64,⟨11#u64⟩⟩
    worker_fences := entry 1#u64 ⟨8#u64,false⟩
    operations := entry key ledger
    operation_plans := entry key plan
    resource_indexes := entry 1#u64 index
    logical_effects := ⟨five (⟨fence⟩,logicalRecord) (⟨freeze⟩,logicalRecord)
      (⟨sealing⟩,logicalRecord) (⟨commit⟩,logicalRecord) (⟨stop⟩,logicalRecord)⟩
    effect_keys := five (id fence) (id freeze) (id sealing) (id commit) (id stop)
    effects := ⟨five (id fence,success) (id freeze,success) (id sealing,success)
      (id commit,success) (id stop,if hasStop then success else .OutcomeUnknown)⟩
    next_vm_id := 3#u64
    next_slot_id := 4#u64
    next_snapshot_id := 6#u64
    next_manifest_digest := 8#u64
    inflight := alloc.vec.Vec.new _
    controller_up := true }


theorem entry_get (eq : core.cmp.PartialEq K K) (key : K) (value : V)
    (same : eq.eq key key = .ok true) :
    store.Store.get eq (entry key value) key = .ok (some value) := by
  unfold store.Store.get store.Store.get_loop
  unfold loop
  simp [store.Store.get_loop.body,entry,one,alloc.vec.Vec.len,
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

theorem fixture_keys_clone :
    alloc.vec.CloneVec.clone model.EffectId.Insts.CoreCloneClone (world false).effect_keys =
      .ok (world false).effect_keys := by
  have copies (value : model.EffectId) (member : value ∈ (world false).effect_keys.val) :
      model.EffectId.Insts.CoreCloneClone.clone value = .ok value := by
    change value ∈ [id fence,id freeze,id sealing,id commit,id stop] at member
    simp only [List.mem_cons, List.not_mem_nil, or_false] at member
    rcases member with rfl | rfl | rfl | rfl | rfl
    all_goals simp [id,fence,freeze,sealing,commit,stop,
      model.EffectId.Insts.CoreCloneClone.clone,
      model.LogicalEffectId.Insts.CoreCloneClone.clone,
      model.Command.Insts.CoreCloneClone.clone,
      ids.SandboxId.Insts.CoreCloneClone.clone, ids.VmId.Insts.CoreCloneClone.clone,
      ids.FsId.Insts.CoreCloneClone.clone, ids.SnapshotId.Insts.CoreCloneClone.clone,
      model.SnapshotManifestRef.Insts.CoreCloneClone.clone, lift]
  obtain ⟨copied, evaluated, same⟩ := WP.spec_imp_exists
    (Slice.clone_spec (s := (world false).effect_keys.slice) (fun value member => copies value member))
  simp only [alloc.vec.CloneVec.clone, evaluated]
  cases same
  simp

theorem fixture_world_clone :
    model.World.Insts.CoreCloneClone.clone (world false) = .ok (world false) := by
  have copy_sandboxes := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (ids.SandboxId × model.SandboxRecord)) (world false).sandboxes.entries (fun _ => rfl)
  have copy_vms := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (ids.VmId × model.Vm)) (world false).vms.entries (fun _ => rfl)
  have copy_filesystems := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (ids.FsId × model.Filesystem)) (world false).filesystems.entries (fun _ => rfl)
  have copy_snapshots := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (ids.SnapshotId × model.Snapshot)) (world false).snapshots.entries (fun _ => rfl)
  have copy_net_slots := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (ids.NetSlotId × model.NetSlot)) (world false).net_slots.entries (fun _ => rfl)
  have copy_applied_file_policies := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (ids.SandboxId × model.AppliedFilePolicy)) (world false).applied_file_policies.entries (fun _ => rfl)
  have copy_worker_fences := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (ids.SandboxId × model.WorkerFence)) (world false).worker_fences.entries (fun _ => rfl)
  have copy_operations := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (model.OperationKey × model.OperationLedgerRecord)) (world false).operations.entries (fun _ => rfl)
  have copy_operation_plans := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (model.OperationKey × model.OperationPlan)) (world false).operation_plans.entries (fun _ => rfl)
  have copy_resource_indexes := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (ids.SandboxId × model.ResourceIndex)) (world false).resource_indexes.entries (fun _ => rfl)
  have copy_logical_effects := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (model.LogicalEffectId × model.LogicalEffectRecord)) (world false).logical_effects.entries (fun _ => rfl)
  have copy_effects := SandboxModel.Foundations.vec_clone_exact
    (BuiltinClone (model.EffectId × model.EffectStatus)) (world false).effects.entries (fun _ => rfl)
  have noMaintenance : (world false).maintenance = none := rfl
  have noInflight : (world false).inflight = alloc.vec.Vec.new _ := rfl
  have inflightCopy : alloc.vec.CloneVec.clone model.EffectId.Insts.CoreCloneClone
      (world false).inflight = .ok (world false).inflight := by
    have copies (value : model.EffectId) (member : value ∈ (world false).inflight.val) :
        model.EffectId.Insts.CoreCloneClone.clone value = .ok value := by
      simp [noInflight] at member
    obtain ⟨copied, evaluated, same⟩ := WP.spec_imp_exists
      (Slice.clone_spec (s := (world false).inflight.slice) (fun value member => copies value member))
    simp only [alloc.vec.CloneVec.clone, evaluated]
    cases same
    simp
  simp only [model.World.Insts.CoreCloneClone.clone, store.Store.Insts.CoreCloneClone.clone,
    copy_sandboxes, copy_vms, copy_filesystems, copy_snapshots, copy_net_slots, copy_applied_file_policies, copy_worker_fences, copy_operations, copy_operation_plans, copy_resource_indexes, copy_logical_effects, copy_effects, noMaintenance,
    core.option.Option.Insts.CoreCloneClone.clone, fixture_keys_clone, inflightCopy,
    bind_tc_ok, lift, core.clone.impls.CloneU64.clone, core.clone.impls.CloneBool.clone]
  all_goals rfl


theorem loop_continue (body : A → Result (ControlFlow A B)) (position next : A)
    (ran : body position = .ok (.cont next)) : loop body position = loop body next := by
  conv_lhs => unfold loop
  rw [ran]
  simp

theorem loop_finish (body : A → Result (ControlFlow A B)) (position : A) (out : B)
    (ran : body position = .ok (.done out)) : loop body position = .ok out := by
  conv_lhs => unfold loop
  rw [ran]
  simp

@[simp] theorem add_one_0 : (0#usize + 1#usize : Result Usize) = .ok 1#usize := by
  obtain ⟨next, evaluated, post⟩ := WP.spec_imp_exists
    (UScalar.add_spec (x := 0#usize) (y := 1#usize) (by scalar_tac))
  have same : next = 1#usize := by scalar_tac
  simpa only [same] using evaluated

@[simp] theorem add_one_1 : (1#usize + 1#usize : Result Usize) = .ok 2#usize := by
  obtain ⟨next, evaluated, post⟩ := WP.spec_imp_exists
    (UScalar.add_spec (x := 1#usize) (y := 1#usize) (by scalar_tac))
  have same : next = 2#usize := by scalar_tac
  simpa only [same] using evaluated

@[simp] theorem add_one_2 : (2#usize + 1#usize : Result Usize) = .ok 3#usize := by
  obtain ⟨next, evaluated, post⟩ := WP.spec_imp_exists
    (UScalar.add_spec (x := 2#usize) (y := 1#usize) (by scalar_tac))
  have same : next = 3#usize := by scalar_tac
  simpa only [same] using evaluated

@[simp] theorem add_one_3 : (3#usize + 1#usize : Result Usize) = .ok 4#usize := by
  obtain ⟨next, evaluated, post⟩ := WP.spec_imp_exists
    (UScalar.add_spec (x := 3#usize) (y := 1#usize) (by scalar_tac))
  have same : next = 4#usize := by scalar_tac
  simpa only [same] using evaluated

section Lookups
attribute [local simp] store.Store.get_copy_loop.body
  world
  five
  id
  fence
  freeze
  sealing
  commit
  stop
  alloc.vec.Vec.len
  alloc.vec.Vec.index_slice_index
  alloc.vec.Vec.index_usize
  core.cmp.impls.PartialEqShared.eq
  model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq
  model.EffectId.Insts.CoreCmpPartialEqEffectId.eq
  model.Command.Insts.CoreCmpPartialEqCommand.eq
  model.Command.read_discriminant
  ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq
  ids.VmId.Insts.CoreCmpPartialEqVmId.eq
  ids.FsId.Insts.CoreCmpPartialEqFsId.eq
  ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq
  model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq
  ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq
  core.cmp.impls.PartialEqBool.eq
  lift
  store.Store.get_loop.body

theorem logical_lookup (query : model.Command)
    (member : query ∈ [fence,freeze,sealing,commit,stop]) :
    store.Store.get_copy model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      model.LogicalEffectRecord.Insts.CoreMarkerCopy (world false).logical_effects ⟨query⟩ =
      .ok (some logicalRecord) := by
  simp only [List.mem_cons, List.not_mem_nil, or_false] at member
  rcases member with rfl | rfl | rfl | rfl | rfl
  · unfold store.Store.get_copy store.Store.get_copy_loop
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨fence⟩ 0#usize = .ok (.done (some logicalRecord)) := by
      simp
    exact loop_finish _ _ _ ran

  · unfold store.Store.get_copy store.Store.get_copy_loop
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨freeze⟩ 0#usize = .ok (.cont 1#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨freeze⟩ 1#usize = .ok (.done (some logicalRecord)) := by
      simp
    exact loop_finish _ _ _ ran

  · unfold store.Store.get_copy store.Store.get_copy_loop
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨sealing⟩ 0#usize = .ok (.cont 1#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨sealing⟩ 1#usize = .ok (.cont 2#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨sealing⟩ 2#usize = .ok (.done (some logicalRecord)) := by
      simp
    exact loop_finish _ _ _ ran

  · unfold store.Store.get_copy store.Store.get_copy_loop
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨commit⟩ 0#usize = .ok (.cont 1#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨commit⟩ 1#usize = .ok (.cont 2#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨commit⟩ 2#usize = .ok (.cont 3#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨commit⟩ 3#usize = .ok (.done (some logicalRecord)) := by
      simp
    exact loop_finish _ _ _ ran

  · unfold store.Store.get_copy store.Store.get_copy_loop
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨stop⟩ 0#usize = .ok (.cont 1#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨stop⟩ 1#usize = .ok (.cont 2#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨stop⟩ 2#usize = .ok (.cont 3#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨stop⟩ 3#usize = .ok (.cont 4#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_copy_loop.body model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId (world false).logical_effects ⟨stop⟩ 4#usize = .ok (.done (some logicalRecord)) := by
      simp
    exact loop_finish _ _ _ ran

theorem effect_lookup (query : model.Command) (status : model.EffectStatus)
    (member : (query,status) ∈ [(fence,success),(freeze,success),(sealing,success),
      (commit,success),(stop,.OutcomeUnknown)]) :
    store.Store.get model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects
      (id query) = .ok (some status) := by
  simp only [List.mem_cons, List.not_mem_nil, or_false, Prod.mk.injEq] at member
  rcases member with ⟨rfl,rfl⟩ | ⟨rfl,rfl⟩ | ⟨rfl,rfl⟩ | ⟨rfl,rfl⟩ | ⟨rfl,rfl⟩
  · unfold store.Store.get store.Store.get_loop
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id fence) 0#usize = .ok (.done (some success)) := by
      simp
    exact loop_finish _ _ _ ran

  · unfold store.Store.get store.Store.get_loop
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id freeze) 0#usize = .ok (.cont 1#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id freeze) 1#usize = .ok (.done (some success)) := by
      simp
    exact loop_finish _ _ _ ran

  · unfold store.Store.get store.Store.get_loop
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id sealing) 0#usize = .ok (.cont 1#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id sealing) 1#usize = .ok (.cont 2#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id sealing) 2#usize = .ok (.done (some success)) := by
      simp
    exact loop_finish _ _ _ ran

  · unfold store.Store.get store.Store.get_loop
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id commit) 0#usize = .ok (.cont 1#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id commit) 1#usize = .ok (.cont 2#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id commit) 2#usize = .ok (.cont 3#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id commit) 3#usize = .ok (.done (some success)) := by
      simp
    exact loop_finish _ _ _ ran

  · unfold store.Store.get store.Store.get_loop
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id stop) 0#usize = .ok (.cont 1#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id stop) 1#usize = .ok (.cont 2#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id stop) 2#usize = .ok (.cont 3#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id stop) 3#usize = .ok (.cont 4#usize) := by
      simp
    rw [loop_continue _ _ _ ran]
    have ran : store.Store.get_loop.body model.EffectId.Insts.CoreCmpPartialEqEffectId (world false).effects (id stop) 4#usize = .ok (.done (some .OutcomeUnknown)) := by
      simp
    exact loop_finish _ _ _ ran


end Lookups

theorem fixture_effect_run (query : model.Command) (status : model.EffectStatus)
    (member : (query,status) ∈ [(fence,success),(freeze,success),(sealing,success),
      (commit,success),(stop,.OutcomeUnknown)]) :
    effect.effect_succeeded_check (world false) query =
      .ok (match status with | .Acknowledged (.Succeeded _) => true | _ => false) := by
  have commands : query ∈ [fence,freeze,sealing,commit,stop] := by
    simp only [List.mem_cons,List.not_mem_nil,or_false,Prod.mk.injEq] at member ⊢
    rcases member with ⟨rfl,rfl⟩ | ⟨rfl,rfl⟩ | ⟨rfl,rfl⟩ | ⟨rfl,rfl⟩ | ⟨rfl,rfl⟩ <;> simp
  have copied : model.Command.Insts.CoreCloneClone.clone query = .ok query := by
    simp only [List.mem_cons,List.not_mem_nil,or_false] at commands
    rcases commands with rfl | rfl | rfl | rfl | rfl
    all_goals simp [fence,freeze,sealing,commit,stop, model.Command.Insts.CoreCloneClone.clone,
      ids.SandboxId.Insts.CoreCloneClone.clone, ids.VmId.Insts.CoreCloneClone.clone,
      ids.FsId.Insts.CoreCloneClone.clone, ids.SnapshotId.Insts.CoreCloneClone.clone,
      model.SnapshotManifestRef.Insts.CoreCloneClone.clone,lift]
  have logical := logical_lookup query commands
  have receipt := effect_lookup query status member
  simp only [effect.effect_succeeded_check,copied,logical,logicalRecord,bind_tc_ok]
  have exactReceipt : store.Store.get model.EffectId.Insts.CoreCmpPartialEqEffectId
      (world false).effects ⟨⟨query⟩,0#u64⟩ = .ok (some status) := receipt
  rw [exactReceipt]
  simp only [bind_tc_ok]
  cases status <;> try rfl
  rename_i outcome
  cases outcome <;> rfl


theorem fixture_index : model.World.resource_index (world false) 1#u64 = .ok index := by
  have stored := entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 index (by rfl)
  have vms := SandboxModel.Foundations.vec_clone_exact ids.VmId.Insts.CoreCloneClone index.vms (fun _ => rfl)
  have slots := SandboxModel.Foundations.vec_clone_exact ids.NetSlotId.Insts.CoreCloneClone index.slots (fun _ => rfl)
  have snapshots := SandboxModel.Foundations.vec_clone_exact ids.SnapshotId.Insts.CoreCloneClone index.snapshots (fun _ => rfl)
  have copied : model.ResourceIndex.Insts.CoreCloneClone.clone index = .ok index := by
    simp only [model.ResourceIndex.Insts.CoreCloneClone.clone,vms,slots,snapshots,bind_tc_ok]
  change (do
    let found ← store.Store.get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId (entry 1#u64 index) 1#u64
    let cloned ← core.option.OptionShared0T.cloned model.ResourceIndex.Insts.CoreCloneClone found
    core.option.Option.unwrap_or_default model.ResourceIndex.Insts.CoreDefaultDefault cloned) = _
  simp [stored,core.option.OptionShared0T.cloned,core.option.Option.Insts.CoreCloneClone.clone,copied,core.option.Option.unwrap_or_default]

theorem fixture_vms_closed : publication.indexed_vms_closed (world false) 1#u64 = .ok true := by
  have stored := entry_get ids.VmId.Insts.CoreCmpPartialEqVmId 2#u64 vm (by rfl)
  change store.Store.get ids.VmId.Insts.CoreCmpPartialEqVmId (world false).vms 2#u64 = .ok (some vm) at stored
  have first : publication.indexed_vms_closed_loop.body (world false) index 0#usize =
      .ok (.cont 1#usize) := by
    simp [publication.indexed_vms_closed_loop.body,index,one,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,stored]
    simp [vm,
      core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      model.VmStatus.Insts.CoreCmpPartialEqVmStatus.eq,model.VmStatus.read_discriminant]
  have last : publication.indexed_vms_closed_loop.body (world false) index 1#usize =
      .ok (.done true) := by
    simp [publication.indexed_vms_closed_loop.body,index,one,alloc.vec.Vec.len]
  simp only [publication.indexed_vms_closed,fixture_index,bind_tc_ok,
    publication.indexed_vms_closed_loop]
  rw [loop_continue _ _ _ first]
  exact loop_finish _ _ _ last

theorem fixture_slots_closed : publication.indexed_slots_closed (world false) 1#u64 = .ok true := by
  have stored := entry_get ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId 3#u64 slot (by rfl)
  change store.Store.get ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId (world false).net_slots 3#u64 = .ok (some slot) at stored
  have first : publication.indexed_slots_closed_loop.body (world false) index 0#usize =
      .ok (.cont 1#usize) := by
    simp [publication.indexed_slots_closed_loop.body,index,one,alloc.vec.Vec.len,
      alloc.vec.Vec.index_slice_index,alloc.vec.Vec.index_usize,stored]
    rfl
  have last : publication.indexed_slots_closed_loop.body (world false) index 1#usize =
      .ok (.done true) := by
    simp [publication.indexed_slots_closed_loop.body,index,one,alloc.vec.Vec.len]
  simp only [publication.indexed_slots_closed,fixture_index,bind_tc_ok,
    publication.indexed_slots_closed_loop]
  rw [loop_continue _ _ _ first]
  exact loop_finish _ _ _ last


def paused : model.SandboxRecord :=
  { record with
    phase := .Paused
    current_vm := none
    durable_snapshot := some source
    restore_from := none
    net_slot := none
    desired := .Paused 1#u64 source
    effective := .Paused 1#u64 source
    current_operation := some {operation with status := .Succeeded} }

def afterPublication : model.World :=
  { (world false) with sandboxes := entry 1#u64 paused, operations := entry key {ledger with operation := {operation with status := .Succeeded}, result := some (.Paused source)} }

theorem fixture_record_clone : model.SandboxRecord.Insts.CoreCloneClone.clone record = .ok record := by
  simp [record,operation,core.option.Option.Insts.CoreCloneClone.clone,lift,
    model.SandboxRecord.Insts.CoreCloneClone.clone,
    model.Phase.Insts.CoreCloneClone.clone,
    model.WorkspaceApiAccess.Insts.CoreCloneClone.clone,
    model.FilePolicyRef.Insts.CoreCloneClone.clone,
    model.NetworkPolicy.Insts.CoreCloneClone.clone,
    model.Authority.Insts.CoreCloneClone.clone,
    model.Operation.Insts.CoreCloneClone.clone,
    ids.SandboxId.Insts.CoreCloneClone.clone,
    ids.TenantId.Insts.CoreCloneClone.clone,
    ids.VmId.Insts.CoreCloneClone.clone,
    ids.FsId.Insts.CoreCloneClone.clone,
    ids.NetSlotId.Insts.CoreCloneClone.clone]

theorem fixture_paused_clone : model.SandboxRecord.Insts.CoreCloneClone.clone paused = .ok paused := by
  simp [record,paused,operation,source,core.option.Option.Insts.CoreCloneClone.clone,lift,
    model.SandboxRecord.Insts.CoreCloneClone.clone,
    model.Phase.Insts.CoreCloneClone.clone,
    model.WorkspaceApiAccess.Insts.CoreCloneClone.clone,
    model.FilePolicyRef.Insts.CoreCloneClone.clone,
    model.NetworkPolicy.Insts.CoreCloneClone.clone,
    model.Authority.Insts.CoreCloneClone.clone,
    model.Operation.Insts.CoreCloneClone.clone,
    model.RestoreSource.Insts.CoreCloneClone.clone,
    ids.SandboxId.Insts.CoreCloneClone.clone,
    ids.TenantId.Insts.CoreCloneClone.clone,
    ids.FsId.Insts.CoreCloneClone.clone,
]

theorem fixture_ledger_clone : model.OperationLedgerRecord.Insts.CoreCloneClone.clone ledger = .ok ledger := by
  simp [ledger,operation,model.OperationLedgerRecord.Insts.CoreCloneClone.clone,
    model.OperationInput.Insts.CoreCloneClone.clone,model.Operation.Insts.CoreCloneClone.clone,
    core.option.Option.Insts.CoreCloneClone.clone,lift,ids.SandboxId.Insts.CoreCloneClone.clone]

theorem fixture_publish : publication.publish_paused (world false) 1#u64 record source =
    .ok afterPublication := by
  have sandboxWrite := entry_put ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64
    record paused (by rfl)
  have operationRead := entry_get model.OperationKey.Insts.CoreCmpPartialEqOperationKey key
    ledger (by simp [key,model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
      ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq])
  have operationWrite := entry_put model.OperationKey.Insts.CoreCmpPartialEqOperationKey key
    ledger {ledger with operation := {operation with status := .Succeeded},result := some (.Paused source)} (by simp [key,model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
      ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq])
  simp only [publication.publish_paused,fixture_world_clone,fixture_record_clone,bind_tc_ok]
  have currentOp : record.current_operation = some operation := rfl
  simp only [currentOp,bind_tc_ok]
  change (do
    let authority ← model.Authority.Insts.CoreCloneClone.clone (.Paused 1#u64 source)
    let row ← model.SandboxRecord.Insts.CoreCloneClone.clone
      {paused with desired := authority}
    let rows ← store.Store.put ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      (entry 1#u64 record) 1#u64 row
    publication.update_operation_result {world false with sandboxes := rows}
      1#u64 {operation with status := .Succeeded} (.Paused source)) = _
  have authorityCopy : model.Authority.Insts.CoreCloneClone.clone (.Paused 1#u64 source) =
      .ok (.Paused 1#u64 source) := by
    simp [source,model.Authority.Insts.CoreCloneClone.clone,
      ids.SandboxId.Insts.CoreCloneClone.clone,model.RestoreSource.Insts.CoreCloneClone.clone]
  simp only [authorityCopy,bind_tc_ok]
  change (do
    let row ← model.SandboxRecord.Insts.CoreCloneClone.clone paused
    let rows ← store.Store.put ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      (entry 1#u64 record) 1#u64 row
    publication.update_operation_result {world false with sandboxes := rows}
      1#u64 {operation with status := .Succeeded} (.Paused source)) = _
  simp only [fixture_paused_clone,bind_tc_ok,sandboxWrite]
  unfold publication.update_operation_result
  simp only [transition.operation_key,bind_tc_ok]
  change (do
    let found ← store.Store.get model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      (entry key ledger) key
    match found with
    | none => .ok {world false with sandboxes := entry 1#u64 paused}
    | some stored => do
      let cloned ← model.OperationLedgerRecord.Insts.CoreCloneClone.clone stored
      let rows ← store.Store.put model.OperationKey.Insts.CoreCmpPartialEqOperationKey
        (entry key ledger) key {cloned with operation := {operation with status := .Succeeded}, result := some (.Paused source)}
      .ok {world false with sandboxes := entry 1#u64 paused,operations := rows}) = _
  simp only [operationRead,bind_tc_ok,fixture_ledger_clone,operationWrite]
  rfl


theorem fixture_capture : transition.current_capture (world false) 1#u64 record = .ok (some handoff) := by
  have stored := entry_get model.OperationKey.Insts.CoreCmpPartialEqOperationKey key plan
    (by simp [key,model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
      ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq])
  change store.Store.get model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    (world false).operation_plans ⟨1#u64,6#u64⟩ = .ok (some plan) at stored
  simp only [transition.current_capture,record,operation,
    core.option.Option.Insts.CoreOpsTry_traitTry.branch,transition.operation_key]
  simp only [core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
    model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,model.OperationKind.read_discriminant,
    bind_tc_ok]
  simp [stored]
  simp [plan,key,
    model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
    ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq]

theorem wrong_publication_accepts : publication.handle_publish_paused (world false) 1#u64 source =
    .ok (.Ok ⟨.PublishPaused 1#u64 source.snapshot,afterPublication⟩) := by
  have rowRead := entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64 record (by rfl)
  change store.Store.get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId (world false).sandboxes
    1#u64 = .ok (some record) at rowRead
  have snapshotRead := entry_get ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId 5#u64 snapshot (by rfl)
  change store.Store.get ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId (world false).snapshots
    source.snapshot = .ok (some snapshot) at snapshotRead
  have fenceRead := entry_get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId 1#u64
    (⟨8#u64,false⟩ : model.WorkerFence) (by rfl)
  change store.Store.get ids.SandboxId.Insts.CoreCmpPartialEqSandboxId (world false).worker_fences
    1#u64 = .ok (some ⟨record.epoch,false⟩) at fenceRead
  have ledgerRead := entry_get model.OperationKey.Insts.CoreCmpPartialEqOperationKey key ledger
    (by simp [key,model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
      ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq])
  change store.Store.get model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    (world false).operations ⟨1#u64,operation.id⟩ = .ok (some ledger) at ledgerRead
  have planRead := entry_get model.OperationKey.Insts.CoreCmpPartialEqOperationKey key plan
    (by simp [key,model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq,
      ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq])
  change store.Store.get model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    (world false).operation_plans ⟨1#u64,operation.id⟩ = .ok (some plan) at planRead
  have phaseCheck : core.cmp.PartialEq.ne.trait_default model.Phase.Insts.CoreCmpPartialEqPhase
      record.phase .Pausing = .ok false := by
    simp [record,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      model.Phase.Insts.CoreCmpPartialEqPhase.eq,model.Phase.read_discriminant]
  have ownerCheck : core.cmp.PartialEq.ne.trait_default ids.TenantId.Insts.CoreCmpPartialEqTenantId
      snapshot.owner record.owner = .ok false := by
    simp [snapshot,record,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq]
  have sessionCheck : core.cmp.PartialEq.ne.trait_default ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      snapshot.sandbox 1#u64 = .ok false := by
    simp [snapshot,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
  have workspaceCheck : core.cmp.PartialEq.ne.trait_default ids.FsId.Insts.CoreCmpPartialEqFsId
      snapshot.workspace record.workspace = .ok false := by
    simp [snapshot,record,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      ids.FsId.Insts.CoreCmpPartialEqFsId.eq]
  have manifestCheck : core.cmp.PartialEq.ne.trait_default model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef
      snapshot.manifest source.manifest = .ok false := by
    simp [snapshot,source,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
      ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq]
  have stageCheck : core.cmp.PartialEq.ne.trait_default model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage
      snapshot.stage .Durable = .ok false := by
    simp [snapshot,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      model.SnapshotStage.Insts.CoreCmpPartialEqSnapshotStage.eq,model.SnapshotStage.read_discriminant]
  have fenceCheck : core.cmp.PartialEq.ne.trait_default
      (core.option.Option.Insts.CoreCmpPartialEqOption
        (core.cmp.PartialEqShared model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence))
      (some ⟨record.epoch,false⟩) (some ⟨record.epoch,false⟩) = .ok false := by
    simp [record,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      core.option.Option.Insts.CoreCmpPartialEqOption.eq,core.cmp.impls.PartialEqShared.eq,
      model.WorkerFence.Insts.CoreCmpPartialEqWorkerFence.eq]
  have ledgerSession : core.cmp.PartialEq.ne.trait_default ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      ledger.sandbox 1#u64 = .ok false := by
    simp [ledger,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]
  have ledgerOperation : core.cmp.PartialEq.ne.trait_default model.Operation.Insts.CoreCmpPartialEqOperation
      ledger.operation operation = .ok false := by
    simp [ledger,operation,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      model.Operation.Insts.CoreCmpPartialEqOperation.eq,ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq,
      model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,model.OperationKind.read_discriminant,
      model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus.eq,model.OperationStatus.read_discriminant]
  have targetCheck : core.cmp.PartialEq.ne.trait_default
      (core.option.Option.Insts.CoreCmpPartialEqOption model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource)
      plan.target_snapshot (some source) = .ok false := by
    simp [plan,source,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      core.option.Option.Insts.CoreCmpPartialEqOption.eq,model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource.eq,
      ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId.eq,
      model.SnapshotManifestRef.Insts.CoreCmpPartialEqSnapshotManifestRef.eq,
      ids.ManifestDigest.Insts.CoreCmpPartialEqManifestDigest.eq]
  have vmCheck : core.cmp.PartialEq.ne.trait_default
      (core.option.Option.Insts.CoreCmpPartialEqOption ids.VmId.Insts.CoreCmpPartialEqVmId)
      plan.vm (some 2#u64) = .ok false := by
    simp [plan,core.cmp.PartialEq.ne.trait_default,core.cmp.PartialEq.ne.default,
      core.option.Option.Insts.CoreCmpPartialEqOption.eq,ids.VmId.Insts.CoreCmpPartialEqVmId.eq]
  have fenceRun := fixture_effect_run fence success (by simp)
  change effect.effect_succeeded_check (world false) (.InstallFence 1#u64 record.epoch false) = .ok true at fenceRun
  have freezeRun := fixture_effect_run freeze success (by simp)
  change effect.effect_succeeded_check (world false) (.FreezeVm 1#u64 record.epoch record.generation 2#u64) = .ok true at freezeRun
  have sealRun := fixture_effect_run sealing success (by simp)
  change effect.effect_succeeded_check (world false) (.SealSnapshot 1#u64 record.epoch record.generation 2#u64 record.workspace source.snapshot source.manifest) = .ok true at sealRun
  have commitRun := fixture_effect_run commit success (by simp)
  change effect.effect_succeeded_check (world false) (.CommitSnapshot 1#u64 record.epoch record.generation source.snapshot source.manifest) = .ok true at commitRun
  have stopRun := fixture_effect_run stop .OutcomeUnknown (by simp)
  change effect.effect_succeeded_check (world false) (.StopVm 1#u64 record.epoch record.generation 2#u64) = .ok false at stopRun
  have up : (world false).controller_up = true := rfl
  have terminal : record.terminal = false := rfl
  have generation : (snapshot.source_generation != record.generation) = false := by simp [snapshot,record]
  have immutable : snapshot.immutable = true := rfl
  have currentVm : record.current_vm = some 2#u64 := rfl
  have currentOp : record.current_operation = some operation := rfl
  have noResult : core.option.Option.is_some ledger.result = false := rfl
  have mayStop : capture.CaptureHandoff.may_stop_original handoff = .ok true := by
    simp [handoff,capture.CaptureHandoff.may_stop_original,
      capture.CaptureCommit.Insts.CoreCmpPartialEqCaptureCommit.eq,capture.CaptureCommit.read_discriminant,
      capture.RestoreEligibility.Insts.CoreCmpPartialEqRestoreEligibility.eq,capture.RestoreEligibility.read_discriminant]
  simp only [publication.handle_publish_paused,up,if_true,rowRead,bind_tc_ok,phaseCheck,
    Bool.false_eq_true,if_false,terminal,snapshotRead,ownerCheck,sessionCheck,generation,
    workspaceCheck,manifestCheck,stageCheck,immutable,fenceRead,fenceCheck,currentVm,
    fenceRun,freezeRun,sealRun,commitRun,stopRun,fixture_vms_closed,fixture_slots_closed,
    currentOp,transition.operation_key,ledgerRead,ledgerSession,ledgerOperation,noResult,
    planRead,targetCheck,vmCheck,fixture_capture,SandproofFoundationsRust.core.option.Option.is_some_and,
    P.Insts.CoreOpsFunctionFnOnceTupleCaptureHandoffBool.call_once,mayStop,fixture_publish]


abbrev Input := ids.SandboxId × model.RestoreSource

def publishes (world : model.World) (input : Input) : Prop :=
  ∃ decision, publication.handle_publish_paused world input.1 input.2 = .ok (.Ok decision)

noncomputable def currentAttempt (world : model.World) (command : model.Command) : U64 := by
  classical
  exact match tableLookup ⟨command⟩ world.logical_effects.entries.val with
    | none => 0#u64
    | some row => row.current_attempt

/-- Identical full-key/current-attempt logical receipt condition to the positive environment. -/
noncomputable def currentStopAcknowledged (world : model.World) (input : Input) : Prop := by
  classical
  exact ∃ record vm receipt,
    tableLookup input.1 world.sandboxes.entries.val = some record ∧
    record.current_vm = some vm ∧
    tableLookup ⟨⟨.StopVm input.1 record.epoch record.generation vm⟩,
        currentAttempt world (.StopVm input.1 record.epoch record.generation vm)⟩
      world.effects.entries.val = some (.Acknowledged (.Succeeded receipt))

theorem fixture_no_stop : ¬ currentStopAcknowledged (world false) (1#u64,source) := by
  classical
  simp [currentStopAcknowledged,currentAttempt,world,entry,one,five,id,record,
    logicalRecord,fence,freeze,sealing,commit,stop,tableLookup]

theorem same_property_rejects : ¬ RequiresStop publishes currentStopAcknowledged := by
  intro claimed
  have acknowledgment := claimed (world false) (1#u64,source)
    ⟨⟨.PublishPaused 1#u64 source.snapshot,afterPublication⟩,wrong_publication_accepts⟩
  exact fixture_no_stop acknowledgment

end SandboxModel.Lifecycle.PausePublication.IgnoreStop

#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.entry_get
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.entry_put
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_keys_clone
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_world_clone
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.loop_continue
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.loop_finish
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.add_one_0
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.add_one_1
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.add_one_2
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.add_one_3
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.logical_lookup
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.effect_lookup
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_effect_run
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_index
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_vms_closed
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_slots_closed
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_record_clone
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_paused_clone
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_ledger_clone
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_publish
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_capture
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.wrong_publication_accepts
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.fixture_no_stop
#audit_controller_proof SandboxModel.Lifecycle.PausePublication.IgnoreStop.same_property_rejects
