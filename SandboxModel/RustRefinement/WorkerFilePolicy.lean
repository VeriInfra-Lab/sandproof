import SandboxModel.RustRefinement.WorkerAdmissionChecks

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance workerFileOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerFileLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerFileEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq
local instance workerFilePolicyDecEq : DecidableEq SandboxControllerRust.model.FilePolicyRef := by
  intro left right
  if same : filePolicyRef left = filePolicyRef right then
    exact isTrue (filePolicyRef_injective same)
  else exact isFalse (fun equal => same (congrArg filePolicyRef equal))

theorem worker_sandbox_some {α : Sort u} {value : Option SandboxRecord}
    {record : SandboxRecord} (found : value = some record)
    (missing : value = none → α) (present : (r : SandboxRecord) → value = some r → α) :
    Controller.handlePrepareVmCommand.match_1 (fun _ => α) value missing present = present record found := by
  cases found
  rfl
theorem worker_sandbox_none {α : Sort u} {value : Option SandboxRecord}
    (found : value = none)
    (missing : value = none → α) (present : (r : SandboxRecord) → value = some r → α) :
    Controller.handlePrepareVmCommand.match_1 (fun _ => α) value missing present = missing found := by
  cases found
  rfl

macro "worker_policy_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, publication_starting,
    publication_record_epoch, publication_record_generation, publication_record_workspace, publication_record_filePolicy,
    filePolicyRef_eq_iff, fsId_eq_iff, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "worker_policy_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | worker_policy_guard)
    | (rw [dif_neg]; rotate_left; solve | worker_policy_guard)))

theorem handleWorker_filePolicy_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (valid : ResourceIndexValid model) (room : StoreHeadroom rust.applied_file_policies)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch generation : U64)
    (workspace : SandboxControllerRust.ids.FsId) (policy : SandboxControllerRust.model.FilePolicyRef)
    (attempt : U64)
    (queued : model.inflight (effectId ⟨⟨.ApplyFilePolicy sid epoch generation workspace policy⟩, attempt⟩)) :
    ∃ result, SandboxControllerRust.worker.handle_worker rust ⟨⟨.ApplyFilePolicy sid epoch generation workspace policy⟩, attempt⟩ = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId ⟨⟨.ApplyFilePolicy sid epoch generation workspace policy⟩, attempt⟩, queued⟩) := by
  let id : SandboxControllerRust.model.EffectId := ⟨⟨.ApplyFilePolicy sid epoch generation workspace policy⟩, attempt⟩
  change ∃ result, SandboxControllerRust.worker.handle_worker rust id = ok result ∧
    WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId id, queued⟩)
  by_cases head : currentEffectId model (effectId id).command = effectId id
  · by_cases awaiting : model.effects (effectId id) = some .dispatched ∨ model.effects (effectId id) = some .outcomeUnknown
    · have currentEval := generated_workerCurrent_eq rust model rep id head
      have containsEval := generated_workerQueued_eq rust model rep id queued
      have getEffect := generated_get_eq SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId effectId_partialEq_lawful rust.effects id
      have awaitingEval := generated_workerAwaiting_refines rust model rep id
      have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful rust.sandboxes sid
      have sandboxLookup := storeRep_lookup_map rep.sandboxes sid
      have getFence := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful rust.worker_fences sid
      unfold SandboxControllerRust.worker.handle_worker
      simp only [command_clone_eq, currentEval, generated_workerId_ne_self, containsEval, getEffect, awaitingEval, awaiting, decide_true, bind_tc_ok, Bool.false_eq_true, if_false, if_true]
      dsimp only [id]
      simp_rw [getSandbox, getFence, generated_phase_ne, world_clone_eq]
      simp only [bind_tc_ok]
      simp_rw [publication_fence_store_ne rust model rep]
      simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq, filePolicyRef_eq_lawful, bind_tc_ok]
      cases found : storeLookup rust.sandboxes sid with
      | none =>
        have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
        refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
        unfold Controller.handleWorker
        rw [dif_pos head, dif_pos awaiting]
        unfold Controller.handleWorkerCommand
        dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
        erw [worker_sandbox_none modelFound]
        rfl
      | some record =>
        have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
        let applied : SandboxControllerRust.model.AppliedFilePolicy := ⟨sid, record.generation, record.workspace, record.file_policy⟩
        obtain ⟨policies, putEval, policiesRep⟩ := WP.spec_imp_exists
          (generated_put_preserves_storeRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
            sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => appliedFilePolicy a = b)
            rust.applied_file_policies model.appliedFilePolicies rep.appliedFilePolicies sid applied (appliedFilePolicy applied) rfl room)
        have nextRep : WorkerWorldRep { rust with applied_file_policies := policies }
            (applyFilePolicyResult model (sandboxId sid) (sandboxRecord record)) := by
          exact { rep with appliedFilePolicies := policiesRep }
        by_cases starting : phase record.phase = .starting
        case' pos =>
          cases terminal : record.terminal
          case' false =>
            by_cases sameEpoch : epoch = record.epoch
            case' pos =>
              by_cases sameGeneration : generation = record.generation
              case' pos =>
                by_cases sameWorkspace : workspace = record.workspace
                case' pos =>
                  by_cases samePolicy : policy = record.file_policy
                  case' pos =>
                    by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, false⟩
        all_goals
          conv =>
            enter [1, result, 1, 1]
            simp [*, -valid, bind_tc_ok, applied, publication_starting]
          refine ⟨_, rfl, ?_⟩
          unfold Controller.handleWorker
          rw [dif_pos head, dif_pos awaiting]
          unfold Controller.handleWorkerCommand
          dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
          erw [worker_sandbox_some modelFound]
          worker_policy_steps
          first | exact ⟨rfl, nextRep⟩ | rfl
    · obtain ⟨evaluated, related⟩ := handleWorker_notAwaiting_refines rust model rep valid id head queued awaiting
      exact ⟨.Err .NotAwaiting, evaluated, related⟩
  · refine ⟨.Err .StaleAttempt, handleWorker_stale_rejected rust model rep id head, ?_⟩
    unfold Controller.handleWorker
    rw [dif_neg head]
    rfl

end SandboxModel.RustRefinement
