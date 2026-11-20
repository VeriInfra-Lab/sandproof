import SandboxModel.Lifecycle.PausePublication.Acceptance

namespace SandboxModel.Lifecycle.PausePublication.Fixture
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

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

/-- A concrete nonempty interface-domain witness. No V9 reachability is claimed. -/
noncomputable def ready : Ready (world true) 1#u64 source := by
  refine {
    record := record
    snapshot := snapshot
    vm := 2#u64
    operation := operation
    ledger := ledger
    plan := plan
    handoff := handoff
    up := ?_
    stored := ?_
    phase := ?_
    terminal := ?_
    snapshotStored := ?_
    owner := ?_
    session := ?_
    generation := ?_
    workspace := ?_
    manifest := ?_
    stage := ?_
    immutable := ?_
    fence := ?_
    currentVm := ?_
    fenceSucceeded := ?_
    freezeSucceeded := ?_
    sealSucceeded := ?_
    commitSucceeded := ?_
    stopSucceeded := ?_
    vmsClosed := ?_
    slotsClosed := ?_
    currentOp := ?_
    ledgerStored := ?_
    ledgerSandbox := ?_
    ledgerOperation := ?_
    noResult := ?_
    planStored := ?_
    target := ?_
    planVm := ?_
    operationKind := ?_
    operationEpoch := ?_
    planKey := ?_
    planEpoch := ?_
    planGeneration := ?_
    planCapture := ?_
    registered := ?_
    recoverable := ?_
    sourcePresent := ?_ }
  all_goals dsimp [world,source,operation,handoff,record,snapshot,vm,slot,key,ledger,plan,index,
    fence,freeze,sealing,commit,stop,id,logicalRecord,success,one,five,entry,empty,
    effectValue,currentAttempt,attemptOf,caller_index_view,caller_index_empty,vmClosed,slotClosed]
  all_goals first | rfl | simp [tableLookup]

theorem normal_domain_nonempty :
    ∃ decision, publication.handle_publish_paused (world true) 1#u64 source = .ok (.Ok decision) ∧
      (caller_index_view (world true) 1#u64).vms.val ≠ [] ∧
      (caller_index_view (world true) 1#u64).slots.val ≠ [] := by
  obtain ⟨decision, accepted⟩ := normal_inputs_accept (world true) 1#u64 source ready
  refine ⟨decision, accepted, ?_, ?_⟩ <;>
    simp [caller_index_view,world,entry,one,index,tableLookup]

end SandboxModel.Lifecycle.PausePublication.Fixture
