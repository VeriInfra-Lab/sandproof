import SandboxModel.Lifecycle.PauseDispatch.Publication
import SandboxModel.Lifecycle.PausePublication.Witness

namespace SandboxModel.Lifecycle.PauseDispatch
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

noncomputable section
open SandboxModel.Lifecycle.PausePublication
open SandboxModel.Lifecycle.PausePublication.Fixture
local instance witnessDecidable1 : DecidableEq model.LogicalEffectId := Classical.decEq _
local instance witnessDecidable2 : DecidableEq model.EffectId := Classical.decEq _

/-- Both receipts are genuinely missing in this finite post-Stop input domain. -/
def beforeReceipts : model.World := {Fixture.world false with
  inflight := alloc.vec.Vec.from [Fixture.id Fixture.stop,Fixture.id Fixture.commit] (by scalar_tac),
  effect_keys := Fixture.five (Fixture.id Fixture.stop) (Fixture.id Fixture.fence)
    (Fixture.id Fixture.freeze) (Fixture.id Fixture.sealing) (Fixture.id Fixture.commit), effects :=
  ⟨Fixture.five (Fixture.id Fixture.fence,Fixture.success) (Fixture.id Fixture.freeze,Fixture.success)
    (Fixture.id Fixture.sealing,Fixture.success) (Fixture.id Fixture.commit,.OutcomeUnknown)
    (Fixture.id Fixture.stop,.Dispatched)⟩}

noncomputable def normal_rest : ReadyRest beforeReceipts 1#u64 source := by
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
  all_goals dsimp [beforeReceipts,world,source,operation,handoff,record,snapshot,vm,slot,key,ledger,plan,index,
    fence,freeze,sealing,commit,stop,Fixture.id,logicalRecord,success,one,five,entry,empty,
    effectValue,currentAttempt,attemptOf,caller_index_view,caller_index_empty,vmClosed,slotClosed]
  all_goals first | rfl | simp [tableLookup]

theorem normal_two_receipts_then_publish :
    ∃ middle next decision,
      effect.acknowledge_effect beforeReceipts (Fixture.id Fixture.stop) (.Succeeded 12#u64) = .ok middle ∧
      effect.acknowledge_effect middle (Fixture.id Fixture.commit) (.Succeeded 12#u64) = .ok next ∧
      publication.handle_publish_paused next 1#u64 Fixture.source = .ok (.Ok decision) := by
  classical
  have stopRead : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      beforeReceipts.logical_effects ⟨Fixture.stop⟩ = .ok (some Fixture.logicalRecord) := by
    rw [get_exact_at model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨Fixture.stop⟩
      (fun row => logical_query_eq row Fixture.stop (.stop _ _ _ _))]
    simp [beforeReceipts,Fixture.world,Fixture.five,Fixture.stop,Fixture.fence,Fixture.freeze,Fixture.sealing,Fixture.commit,tableLookup]
  have commitRead : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      beforeReceipts.logical_effects ⟨Fixture.commit⟩ = .ok (some Fixture.logicalRecord) := by
    rw [get_exact_at model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨Fixture.commit⟩
      (fun row => logical_query_eq row Fixture.commit (.commit _ _ _ _ _))]
    simp [beforeReceipts,Fixture.world,Fixture.five,Fixture.stop,Fixture.fence,Fixture.freeze,Fixture.sealing,Fixture.commit,tableLookup]
  apply stop_commit_receipts_publish beforeReceipts 1#u64 Fixture.source normal_rest
    0#u64 0#u64 12#u64 12#u64 Fixture.logicalRecord Fixture.logicalRecord stopRead commitRead rfl rfl
  all_goals simp [normal_rest,beforeReceipts,Fixture.world,Fixture.five,Fixture.id,Fixture.stop,Fixture.commit,
    Fixture.fence,Fixture.freeze,Fixture.sealing,Fixture.record,Fixture.source,tableLookup]

end

end SandboxModel.Lifecycle.PauseDispatch
