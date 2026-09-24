import SandboxModel.RustRefinement.WorkerSnapshotChecks
import SandboxModel.RustRefinement.WorkerSnapshotUpdate

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance workerCommitOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerCommitLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerCommitEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

macro "worker_commit_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_vm, publication_pausing,
    publication_record_epoch, publication_record_generation, publication_record_workspace,
    scheduler_snapshot_sandbox, scheduler_snapshot_generation, scheduler_snapshot_workspace,
    scheduler_snapshot_stage, scheduler_snapshot_manifest,
    scheduler_localSealed, Option.map_some, Option.map_none, reduceCtorEq,
    sandboxId_eq_iff, fsId_eq_iff, u64ToNat_eq_iff, snapshotManifestRef_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "worker_commit_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | worker_commit_guard)
    | (rw [dif_neg]; rotate_left; solve | worker_commit_guard)))

theorem handleWorker_commitSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (valid : ResourceIndexValid model) (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 1)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch generation : U64)
    (snapId : SandboxControllerRust.ids.SnapshotId) (manifest : SandboxControllerRust.model.SnapshotManifestRef) (attempt : U64)
    (queued : model.inflight (effectId ⟨⟨.CommitSnapshot sid epoch generation snapId manifest⟩, attempt⟩)) :
    ∃ result, SandboxControllerRust.worker.handle_worker rust ⟨⟨.CommitSnapshot sid epoch generation snapId manifest⟩, attempt⟩ = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId ⟨⟨.CommitSnapshot sid epoch generation snapId manifest⟩, attempt⟩, queued⟩) := by
  let id : SandboxControllerRust.model.EffectId := ⟨⟨.CommitSnapshot sid epoch generation snapId manifest⟩, attempt⟩
  change ∃ result, SandboxControllerRust.worker.handle_worker rust id = ok result ∧
    WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId id, queued⟩)
  by_cases head : currentEffectId model (effectId id).command = effectId id
  · by_cases awaiting : model.effects (effectId id) = some .dispatched ∨ model.effects (effectId id) = some .outcomeUnknown
    · have currentEval := generated_workerCurrent_eq rust model rep id head
      have containsEval := generated_workerQueued_eq rust model rep id queued
      have getEffect := generated_get_eq SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId effectId_partialEq_lawful rust.effects id
      have awaitingEval := generated_workerAwaiting_refines rust model rep id
      have getSnapshot := generated_get_eq SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId snapshotId_partialEq_lawful rust.snapshots snapId
      have snapshotLookup := storeRep_lookup_map rep.snapshots snapId
      have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful rust.sandboxes sid
      have sandboxLookup := storeRep_lookup_map rep.sandboxes sid
      unfold SandboxControllerRust.worker.handle_worker
      simp only [command_clone_eq, currentEval, generated_workerId_ne_self, containsEval, getEffect, awaitingEval, awaiting, decide_true, bind_tc_ok, Bool.false_eq_true, if_false, if_true]
      dsimp only [id]
      simp_rw [getSnapshot, getSandbox, generated_phase_ne, world_clone_eq]
      simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
        SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
        snapshotManifestRef_eq_lawful, publication_snapshotStage_eq, bind_tc_ok]
      cases found : storeLookup rust.snapshots snapId with
      | none =>
        have modelFound := snapshotLookup.trans (congrArg (Option.map snapshot) found)
        refine ⟨.Err .ResourceNotFound, rfl, ?_⟩
        unfold Controller.handleWorker
        rw [dif_pos head, dif_pos awaiting]
        unfold Controller.handleWorkerCommand
        dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
        erw [worker_snapshot_none modelFound]
        rfl
      | some snap =>
        have modelFound := snapshotLookup.trans (congrArg (Option.map snapshot) found)
        obtain ⟨indices, snapshots, putEval, putRep⟩ := generated_workerPutSnapshot_refines rust model rep room snapId { snap with stage := .Durable, immutable := true }
        let durable := { rust with resource_indexes := indices, snapshots := snapshots }
        have durableRep : WorkerWorldRep durable (makeSnapshotDurable model (snapshotId snapId) (snapshot snap)) := putRep
        have durableRoom : EffectCapacity durable 1 := ⟨capacity.effects, capacity.logical, capacity.keys, capacity.inflight⟩
        simp_rw [putEval]
        cases sandboxFound : storeLookup rust.sandboxes sid
        all_goals have modelSandbox := sandboxLookup.trans (congrArg (Option.map sandboxRecord) sandboxFound)
        case' some record =>
          by_cases sameEpoch : epoch = record.epoch
          case' pos =>
            by_cases sameGeneration : generation = record.generation
            case' pos =>
              by_cases sameManifest : manifest = snap.manifest
              case' pos =>
                by_cases sandbox : snap.sandbox = sid
                case' pos =>
                  by_cases pausing : phase record.phase = .pausing
                  case' pos =>
                    cases current : record.current_vm
                    all_goals have modelCurrent := congrArg (Option.map vmId) current
                    case' some vid =>
                      obtain ⟨next, enqueueEval, nextRep, _⟩ := generated_enqueue_refines durable _ durableRep durableRoom (.StopVm sid record.epoch record.generation vid)
                      dsimp only [durable] at enqueueEval
                      simp only [bind_tc_ok, current]
                      simp_rw [enqueueEval]
                      by_cases version : snap.source_generation = record.generation
                      case' pos =>
                        by_cases workspace : snap.workspace = record.workspace
                        case' pos =>
                          by_cases sealed : snapshotStage snap.stage = .localSealed
        all_goals
          conv =>
            enter [1, result, 1, 1]
            simp [*, -valid, bind_tc_ok, scheduler_localSealed, publication_pausing]
          refine ⟨_, rfl, ?_⟩
          unfold Controller.handleWorker
          rw [dif_pos head, dif_pos awaiting]
          unfold Controller.handleWorkerCommand
          dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
          erw [worker_snapshot_some modelFound]
          try erw [worker_sandbox_some modelSandbox]
          try erw [worker_sandbox_none modelSandbox]
          worker_commit_steps
          try rw [lifecycle_record_vm]
          try erw [worker_vmId_some modelCurrent]
          try erw [worker_vmId_none modelCurrent]
          worker_commit_steps
          first | exact ⟨rfl, nextRep⟩ | rfl
    · obtain ⟨evaluated, related⟩ := handleWorker_notAwaiting_refines rust model rep valid id head queued awaiting
      exact ⟨.Err .NotAwaiting, evaluated, related⟩
  · refine ⟨.Err .StaleAttempt, handleWorker_stale_rejected rust model rep id head, ?_⟩
    unfold Controller.handleWorker
    rw [dif_neg head]
    rfl

end SandboxModel.RustRefinement
