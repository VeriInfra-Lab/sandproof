import SandboxModel.RustRefinement.WorkerCleanupVm
import SandboxModel.RustRefinement.WorkerFence

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance workerFreezeOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerFreezeLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerFreezeEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

macro "worker_freeze_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, lifecycle_record_owner, publication_starting,
    publication_record_epoch, publication_record_generation, publication_record_policy,
    worker_record_slot_eq, worker_record_vm_eq, running_slot_owner, running_slot_sandbox,
    running_slot_generation, running_slot_vm, running_slot_policy,
    running_vm_owner, running_vm_sandbox, running_vm_generation, running_vm_status,
    publication_pausing, publication_executing,
    tenantId_injective.eq_iff, sandboxId_eq_iff, vmId_injective.eq_iff, networkPolicy_eq_iff, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "worker_freeze_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | worker_freeze_guard)
    | (rw [dif_neg]; rotate_left; solve | worker_freeze_guard)))

theorem handleWorker_freeze_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (valid : ResourceIndexValid model) (room : HandlerMachineHeadroom rust)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch generation : U64)
    (vid : SandboxControllerRust.ids.VmId) (attempt : U64)
    (queued : model.inflight (effectId ⟨⟨.FreezeVm sid epoch generation vid⟩, attempt⟩)) :
    ∃ result, SandboxControllerRust.worker.handle_worker rust ⟨⟨.FreezeVm sid epoch generation vid⟩, attempt⟩ = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId ⟨⟨.FreezeVm sid epoch generation vid⟩, attempt⟩, queued⟩) := by
  let id : SandboxControllerRust.model.EffectId := ⟨⟨.FreezeVm sid epoch generation vid⟩, attempt⟩
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
      have getVm := generated_get_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful rust.vms vid
      have vmLookup := storeRep_lookup_map rep.vms vid
      have getFence := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful rust.worker_fences sid
      have optionVm := generated_option_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful
      unfold SandboxControllerRust.worker.handle_worker
      simp only [command_clone_eq, currentEval, generated_workerId_ne_self, containsEval, getEffect, awaitingEval, awaiting, decide_true, bind_tc_ok, Bool.false_eq_true, if_false, if_true]
      dsimp only [id]
      simp_rw [getSandbox, getVm, getFence, generated_phase_ne, world_clone_eq]
      simp only [bind_tc_ok]
      simp_rw [publication_fence_store_ne rust model rep]
      simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        SandboxControllerRust.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
        SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
        publication_vmStatus_eq, optionVm, bind_tc_ok]
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
        by_cases pausing : phase record.phase = .pausing
        case' pos =>
          by_cases sameEpoch : epoch = record.epoch
          case' pos =>
            by_cases sameGeneration : generation = record.generation
            case' pos =>
              by_cases currentVm : record.current_vm = some vid
              case' pos =>
                cases vmFound : storeLookup rust.vms vid
                all_goals have modelVm := vmLookup.trans (congrArg (Option.map vm) vmFound)
                case' some machine =>
                  obtain ⟨next, nextEval, nextRep⟩ := generated_workerPutVm_refines rust model rep room vid { machine with status := .Frozen }
                  have frozenRep : WorkerWorldRep next (setVm model (vmId vid) { (vm machine) with status := .frozen }) := nextRep
                  simp_rw [nextEval]
                  by_cases owner : machine.owner = record.owner
                  case' pos =>
                    by_cases sandbox : machine.sandbox = sid
                    case' pos =>
                      by_cases version : machine.generation = record.generation
                      case' pos =>
                        by_cases executing : vmStatus machine.status = .executing
                        case' pos =>
                          by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, false⟩
        all_goals
          conv =>
            enter [1, result, 1, 1]
            simp [*, -valid, bind_tc_ok, publication_starting, publication_pausing, publication_executing]
          refine ⟨_, rfl, ?_⟩
          unfold Controller.handleWorker
          rw [dif_pos head, dif_pos awaiting]
          unfold Controller.handleWorkerCommand
          dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
          erw [worker_sandbox_some modelFound]
          worker_freeze_steps
          try erw [worker_vm_some modelVm]
          try erw [worker_vm_none modelVm]
          worker_freeze_steps
          first | exact ⟨rfl, frozenRep⟩ | rfl
    · obtain ⟨evaluated, related⟩ := handleWorker_notAwaiting_refines rust model rep valid id head queued awaiting
      exact ⟨.Err .NotAwaiting, evaluated, related⟩
  · refine ⟨.Err .StaleAttempt, handleWorker_stale_rejected rust model rep id head, ?_⟩
    unfold Controller.handleWorker
    rw [dif_neg head]
    rfl

end SandboxModel.RustRefinement
