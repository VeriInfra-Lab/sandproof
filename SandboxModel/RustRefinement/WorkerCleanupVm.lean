import SandboxModel.RustRefinement.WorkerVmUpdate
import SandboxModel.RustRefinement.WorkerFilePolicy

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance workerCleanupVmOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerCleanupVmLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerCleanupVmEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem worker_vm_some {α : Sort u} {value : Option Vm}
    {record : Vm} (found : value = some record)
    (missing : value = none → α) (present : (r : Vm) → value = some r → α) :
    Controller.handleWorkerCommand.match_7 (fun _ => α) value missing present = present record found := by
  cases found
  rfl
theorem worker_vm_none {α : Sort u} {value : Option Vm}
    (found : value = none)
    (missing : value = none → α) (present : (r : Vm) → value = some r → α) :
    Controller.handleWorkerCommand.match_7 (fun _ => α) value missing present = missing found := by
  cases found
  rfl

theorem worker_record_vm_eq (record : SandboxControllerRust.model.SandboxRecord) (vid : SandboxControllerRust.ids.VmId) :
    (sandboxRecord record).currentVm = some (vmId vid) ↔ record.current_vm = some vid := by
  simpa only [lifecycle_record_vm, Option.map_some] using scheduler_vm_option_eq record.current_vm (some vid)

macro "worker_cleanup_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    running_vm_sandbox, running_vm_generation, running_vm_status,
    worker_record_vm_eq, scheduler_stopped, Option.map_some, Option.map_none,
    sandboxId_eq_iff, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "worker_cleanup_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | worker_cleanup_guard)
    | (rw [dif_neg]; rotate_left; solve | worker_cleanup_guard)))

theorem handleWorker_cleanupVm_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (valid : ResourceIndexValid model) (room : HandlerMachineHeadroom rust)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : U64)
    (vid : SandboxControllerRust.ids.VmId) (attempt : U64)
    (queued : model.inflight (effectId ⟨⟨.CleanupVm sid generation vid⟩, attempt⟩)) :
    ∃ result, SandboxControllerRust.worker.handle_worker rust ⟨⟨.CleanupVm sid generation vid⟩, attempt⟩ = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId ⟨⟨.CleanupVm sid generation vid⟩, attempt⟩, queued⟩) := by
  let id : SandboxControllerRust.model.EffectId := ⟨⟨.CleanupVm sid generation vid⟩, attempt⟩
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
      have optionEq := generated_option_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful
      unfold SandboxControllerRust.worker.handle_worker
      simp only [command_clone_eq, currentEval, generated_workerId_ne_self, containsEval, getEffect, awaitingEval, awaiting, decide_true, bind_tc_ok, Bool.false_eq_true, if_false, if_true]
      dsimp only [id]
      simp_rw [getVm, getSandbox, world_clone_eq]
      simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq, publication_vmStatus_eq, optionEq, bind_tc_ok]
      cases found : storeLookup rust.vms vid with
      | none =>
        have modelFound := vmLookup.trans (congrArg (Option.map vm) found)
        refine ⟨.Err .ResourceNotFound, rfl, ?_⟩
        unfold Controller.handleWorker
        rw [dif_pos head, dif_pos awaiting]
        unfold Controller.handleWorkerCommand
        dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
        erw [worker_vm_none modelFound]
        rfl
      | some machine =>
        have modelFound := vmLookup.trans (congrArg (Option.map vm) found)
        obtain ⟨next, nextEval, nextRep⟩ := generated_workerPutVm_refines rust model rep room vid { machine with status := .Reclaimed }
        have reclaimedRep : WorkerWorldRep next (reclaimVm model (vmId vid) (vm machine)) := nextRep
        simp_rw [nextEval]
        by_cases owner : machine.sandbox = sid
        case' pos =>
          by_cases version : machine.generation = generation
          case' pos =>
            by_cases stopped : vmStatus machine.status = .stopped
            case' pos =>
              cases sandboxFound : storeLookup rust.sandboxes sid
              all_goals have modelSandbox := sandboxLookup.trans (congrArg (Option.map sandboxRecord) sandboxFound)
              case' some record =>
                by_cases current : record.current_vm = some vid
        all_goals
          conv =>
            enter [1, result, 1, 1]
            simp [*, -valid, bind_tc_ok, scheduler_stopped]
          refine ⟨_, rfl, ?_⟩
          unfold Controller.handleWorker
          rw [dif_pos head, dif_pos awaiting]
          unfold Controller.handleWorkerCommand
          dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
          erw [worker_vm_some modelFound]
          worker_cleanup_steps
          try erw [worker_sandbox_some modelSandbox]
          try erw [worker_sandbox_none modelSandbox]
          worker_cleanup_steps
          first | exact ⟨rfl, reclaimedRep⟩ | rfl
    · obtain ⟨evaluated, related⟩ := handleWorker_notAwaiting_refines rust model rep valid id head queued awaiting
      exact ⟨.Err .NotAwaiting, evaluated, related⟩
  · refine ⟨.Err .StaleAttempt, handleWorker_stale_rejected rust model rep id head, ?_⟩
    unfold Controller.handleWorker
    rw [dif_neg head]
    rfl

end SandboxModel.RustRefinement
