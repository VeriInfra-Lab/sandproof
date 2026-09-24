import SandboxModel.RustRefinement.WorkerCleanupVm
import SandboxModel.RustRefinement.WorkerCleanupSlot

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance workerNetworkOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerNetworkLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerNetworkEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq
local instance workerNetworkPolicyDecEq : DecidableEq SandboxControllerRust.model.NetworkPolicy := by
  intro left right
  if same : networkPolicy left = networkPolicy right then exact isTrue (networkPolicy_injective same)
  else exact isFalse (fun equal => same (congrArg networkPolicy equal))

macro "worker_network_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, lifecycle_record_owner, publication_starting,
    publication_record_epoch, publication_record_generation, publication_record_policy,
    worker_record_slot_eq, worker_record_vm_eq, running_slot_owner, running_slot_sandbox,
    running_slot_generation, running_slot_vm, running_slot_policy,
    tenantId_injective.eq_iff, sandboxId_eq_iff, vmId_injective.eq_iff, networkPolicy_eq_iff, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "worker_network_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | worker_network_guard)
    | (rw [dif_neg]; rotate_left; solve | worker_network_guard)))

theorem handleWorker_network_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (valid : ResourceIndexValid model) (room : HandlerMachineHeadroom rust)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch generation : U64)
    (vid : SandboxControllerRust.ids.VmId) (slotId : SandboxControllerRust.ids.NetSlotId)
    (policy : SandboxControllerRust.model.NetworkPolicy) (attempt : U64)
    (queued : model.inflight (effectId ⟨⟨.ConfigureNetwork sid epoch generation vid slotId policy⟩, attempt⟩)) :
    ∃ result, SandboxControllerRust.worker.handle_worker rust ⟨⟨.ConfigureNetwork sid epoch generation vid slotId policy⟩, attempt⟩ = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId ⟨⟨.ConfigureNetwork sid epoch generation vid slotId policy⟩, attempt⟩, queued⟩) := by
  let id : SandboxControllerRust.model.EffectId := ⟨⟨.ConfigureNetwork sid epoch generation vid slotId policy⟩, attempt⟩
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
      have getSlot := generated_get_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful rust.net_slots slotId
      have slotLookup := storeRep_lookup_map rep.netSlots slotId
      have getFence := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful rust.worker_fences sid
      have optionVm := generated_option_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful
      have optionSlot := generated_option_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful
      unfold SandboxControllerRust.worker.handle_worker
      simp only [command_clone_eq, currentEval, generated_workerId_ne_self, containsEval, getEffect, awaitingEval, awaiting, decide_true, bind_tc_ok, Bool.false_eq_true, if_false, if_true]
      dsimp only [id]
      simp_rw [getSandbox, getSlot, getFence, generated_phase_ne, world_clone_eq, netSlot_clone_eq]
      simp only [bind_tc_ok]
      simp_rw [publication_fence_store_ne rust model rep]
      simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        SandboxControllerRust.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
        SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
        networkPolicy_eq_lawful, optionVm, optionSlot, bind_tc_ok]
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
        by_cases starting : phase record.phase = .starting
        case' pos =>
          cases terminal : record.terminal
          case' false =>
            by_cases sameEpoch : epoch = record.epoch
            case' pos =>
              by_cases sameGeneration : generation = record.generation
              case' pos =>
                by_cases samePolicy : policy = record.policy
                case' pos =>
                  cases slotFound : storeLookup rust.net_slots slotId
                  all_goals have modelSlot := slotLookup.trans (congrArg (Option.map netSlot) slotFound)
                  case' some slot =>
                    obtain ⟨next, nextEval, nextRep⟩ := generated_workerPutSlot_refines rust model rep room slotId { slot with configured := true }
                    have configuredRep : WorkerWorldRep next (configureSlot model (netSlotId slotId) (netSlot slot)) := nextRep
                    simp_rw [nextEval]
                    by_cases currentSlot : record.net_slot = some slotId
                    case' pos =>
                      by_cases currentVm : record.current_vm = some vid
                      case' pos =>
                        by_cases owner : slot.owner = record.owner
                        case' pos =>
                          by_cases sandbox : slot.sandbox = sid
                          case' pos =>
                            by_cases version : slot.generation = record.generation
                            case' pos =>
                              by_cases machine : slot.vm = vid
                              case' pos =>
                                by_cases policySame : slot.policy = record.policy
                                case' pos =>
                                  by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, false⟩
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
          worker_network_steps
          try erw [worker_slot_some modelSlot]
          try erw [worker_slot_none modelSlot]
          worker_network_steps
          first | exact ⟨rfl, configuredRep⟩ | rfl
    · obtain ⟨evaluated, related⟩ := handleWorker_notAwaiting_refines rust model rep valid id head queued awaiting
      exact ⟨.Err .NotAwaiting, evaluated, related⟩
  · refine ⟨.Err .StaleAttempt, handleWorker_stale_rejected rust model rep id head, ?_⟩
    unfold Controller.handleWorker
    rw [dif_neg head]
    rfl

end SandboxModel.RustRefinement
