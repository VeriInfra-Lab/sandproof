import SandboxModel.RustRefinement.WorkerFenceUpdate
import SandboxModel.RustRefinement.WorkerCleanupSlot

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance workerFenceOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerFenceLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerFenceEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem worker_slotId_some {α : Sort u} {value : Option NetSlotId}
    {record : NetSlotId} (found : value = some record)
    (missing : value = none → α) (present : (r : NetSlotId) → value = some r → α) :
    Controller.handleWorkerCommand.match_3 (fun _ => α) value missing present = present record found := by cases found; rfl
theorem worker_slotId_none {α : Sort u} {value : Option NetSlotId}
    (found : value = none)
    (missing : value = none → α) (present : (r : NetSlotId) → value = some r → α) :
    Controller.handleWorkerCommand.match_3 (fun _ => α) value missing present = missing found := by cases found; rfl

macro "worker_fence_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, publication_starting, publication_record_epoch,
    running_slot_sandbox, sandboxId_eq_iff, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "worker_fence_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | worker_fence_guard)
    | (rw [dif_neg]; rotate_left; solve | worker_fence_guard)))

theorem handleWorker_fence_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (valid : ResourceIndexValid model) (room : HandlerMachineHeadroom rust)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch : U64) (terminal : Bool) (attempt : U64)
    (queued : model.inflight (effectId ⟨⟨.InstallFence sid epoch terminal⟩, attempt⟩)) :
    ∃ result, SandboxControllerRust.worker.handle_worker rust ⟨⟨.InstallFence sid epoch terminal⟩, attempt⟩ = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId ⟨⟨.InstallFence sid epoch terminal⟩, attempt⟩, queued⟩) := by
  let id : SandboxControllerRust.model.EffectId := ⟨⟨.InstallFence sid epoch terminal⟩, attempt⟩
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
      have getSlot := fun slot => generated_get_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful rust.net_slots slot
      have advance := generated_workerFenceCanAdvance_refines rust model rep sid epoch terminal
      unfold SandboxControllerRust.worker.handle_worker
      simp only [command_clone_eq, currentEval, generated_workerId_ne_self, containsEval, getEffect, awaitingEval, awaiting, decide_true, bind_tc_ok, Bool.false_eq_true, if_false, if_true]
      dsimp only [id]
      simp_rw [getSandbox, getFence, getSlot, generated_phase_eq, generated_workerRevocationPhase]
      simp only [advance, core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq, bind_tc_ok]
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
        obtain ⟨started, startEval, startRep⟩ := generated_workerStartFence_refines rust model rep room.workerFences sid record
        obtain ⟨revoked, revokeEval, revokeRep⟩ := generated_workerRevocationFence_refines rust model rep room sid record
        simp_rw [startEval, revokeEval]
        by_cases sameEpoch : epoch = record.epoch
        case' pos =>
          by_cases sameTerminal : terminal = record.terminal
          case' pos =>
            by_cases monotone : fenceCanAdvance (model.workerFences (sandboxId sid)) (u64ToNat record.epoch) record.terminal
            case' pos =>
              by_cases starting : phase record.phase = .starting
              case' pos =>
                cases live : record.terminal
                all_goals simp only [live] at monotone
              case' neg =>
                by_cases revoking : phase record.phase = .pausing ∨ phase record.phase = .revoking
                case' pos =>
                  cases slotChoice : record.net_slot
                  all_goals have modelChoice := congrArg (Option.map netSlotId) slotChoice
                  case' some slotId =>
                    have slotLookup := storeRep_lookup_map rep.netSlots slotId
                    cases slotFound : storeLookup rust.net_slots slotId
                    all_goals have modelSlot := slotLookup.trans (congrArg (Option.map netSlot) slotFound)
                    case' some slot => by_cases owned : slot.sandbox = sid
        all_goals
          conv =>
            enter [1, result, 1, 1]
            simp [*, -valid, bind_tc_ok, publication_starting]
          refine ⟨_, rfl, ?_⟩
          unfold Controller.handleWorker
          rw [dif_pos head, dif_pos awaiting]
          unfold Controller.handleWorkerCommand
          dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
          erw [worker_sandbox_some modelFound]
          worker_fence_steps
          try rw [publication_record_slot]
          try erw [worker_slotId_some modelChoice]
          try erw [worker_slotId_none modelChoice]
          try erw [worker_slot_some modelSlot]
          try erw [worker_slot_none modelSlot]
          worker_fence_steps
          first | exact ⟨rfl, startRep⟩ | exact ⟨rfl, revokeRep⟩ | rfl
    · obtain ⟨evaluated, related⟩ := handleWorker_notAwaiting_refines rust model rep valid id head queued awaiting
      exact ⟨.Err .NotAwaiting, evaluated, related⟩
  · refine ⟨.Err .StaleAttempt, handleWorker_stale_rejected rust model rep id head, ?_⟩
    unfold Controller.handleWorker
    rw [dif_neg head]
    rfl

end SandboxModel.RustRefinement
