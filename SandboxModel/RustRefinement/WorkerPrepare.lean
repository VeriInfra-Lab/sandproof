import SandboxModel.RustRefinement.WorkerPrepareChecks

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance workerPrepareOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerPrepareLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerPrepareEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq
local instance workerPrepareSourceDecEq : DecidableEq SandboxControllerRust.model.RestoreSource := schedulerRestoreSourceDecEq

theorem worker_record_durable (record : SandboxControllerRust.model.SandboxRecord) :
    (sandboxRecord record).durableSnapshot = record.durable_snapshot.map restoreSource := rfl

macro "worker_prepare_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, publication_starting,
    lifecycle_record_operation, publication_record_epoch, publication_record_generation,
    publication_record_restore, scheduler_plan_restore, restoreSource_option_map_eq_iff,
    operation_id_field, scheduler_key_mk, scheduler_plan_key, scheduler_plan_epoch, scheduler_plan_generation,
    scheduler_plan_sealed, scheduler_plan_vm_eq, scheduler_plan_slot_eq,
    restoreSource_injective.eq_iff, Option.map_some, Option.map_none, reduceCtorEq, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "worker_prepare_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | worker_prepare_guard)
    | (rw [dif_neg]; rotate_left; solve | worker_prepare_guard)))

theorem handleWorker_prepare_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (valid : ResourceIndexValid model) (room : HandlerMachineHeadroom rust) (capacity : WorkerPrepareCapacity rust)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch generation : U64)
    (vid : SandboxControllerRust.ids.VmId) (slotId : SandboxControllerRust.ids.NetSlotId)
    (source : Option SandboxControllerRust.model.RestoreSource) (attempt : U64)
    (queued : model.inflight (effectId ⟨⟨.PrepareVm sid epoch generation vid slotId source⟩, attempt⟩)) :
    ∃ result, SandboxControllerRust.worker.handle_worker rust ⟨⟨.PrepareVm sid epoch generation vid slotId source⟩, attempt⟩ = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId ⟨⟨.PrepareVm sid epoch generation vid slotId source⟩, attempt⟩, queued⟩) := by
  let id : SandboxControllerRust.model.EffectId := ⟨⟨.PrepareVm sid epoch generation vid slotId source⟩, attempt⟩
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
      have fenceLookup := storeRep_lookup_map rep.workerFences sid
      have hasVm := generated_contains_key_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful rust.vms vid
      have vmLookup := storeRep_lookup_map rep.vms vid
      have hasSlot := generated_contains_key_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful rust.net_slots slotId
      have slotLookup := storeRep_lookup_map rep.netSlots slotId
      have allPlans := fun key => generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operationKey_partialEq_lawful rust.operation_plans key
      have optionVm := generated_option_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful
      have optionSlot := generated_option_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful
      have optionSource := generated_option_eq SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource scheduler_restore_eq_lawful
      have allUsable := fun record selected => generated_workerUsableSnapshot_refines rust model rep sid record selected
      unfold SandboxControllerRust.worker.handle_worker
      simp only [command_clone_eq, currentEval, generated_workerId_ne_self, containsEval, getEffect, awaitingEval, awaiting, decide_true, bind_tc_ok, Bool.false_eq_true, if_false, if_true]
      dsimp only [id]
      simp_rw [getSandbox, getFence, hasVm, hasSlot, generated_phase_ne, allUsable,
        SandboxControllerRust.worker.current_plan, SandboxControllerRust.transition.operation_key, allPlans]
      simp only [bind_tc_ok]
      simp_rw [publication_fence_store_ne rust model rep]
      simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        scheduler_key_eq, optionVm, optionSlot, optionSource, bind_tc_ok]
      cases found : storeLookup rust.sandboxes sid with
      | none =>
        have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
        refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
        unfold Controller.handleWorker
        rw [dif_pos head, dif_pos awaiting]
        unfold Controller.handleWorkerCommand
        dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
        unfold Controller.handlePrepareVmCommand
        erw [worker_sandbox_none modelFound]
        rfl
      | some record =>
        have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
        obtain ⟨next, nextEval, nextRep⟩ := generated_workerPrepareResources_refines rust model rep room capacity sid record vid slotId
        simp_rw [nextEval]
        by_cases starting : phase record.phase = .starting
        case' pos =>
          cases terminal : record.terminal
          case' false =>
            by_cases sameEpoch : epoch = record.epoch
            case' pos =>
              by_cases sameGeneration : generation = record.generation
              case' pos =>
                cases current : record.current_operation
                all_goals have modelCurrent := congrArg (Option.map operation) current
                case' some op =>
                  have planLookup := storeRep_lookup_map rep.operationPlans ⟨sid, op.id⟩
                  change model.operationPlans (SandboxModel.operationKey (sandboxId sid) (operationId op.id)) = _ at planLookup
                  cases planFound : storeLookup rust.operation_plans ⟨sid, op.id⟩
                  all_goals have modelPlan := planLookup.trans (congrArg (Option.map operationPlan) planFound)
                  case' some plan =>
                    by_cases planKey : operationKey plan.key = SandboxModel.operationKey (sandboxId sid) (operationId op.id)
                    case' pos =>
                      by_cases planEpoch : plan.epoch = record.epoch
                      case' pos =>
                        by_cases planGeneration : plan.generation = record.generation
                        case' pos =>
                          cases sealed : plan.sealed
                          case' true =>
                            by_cases plannedVm : plan.vm = some vid
                            case' pos =>
                              by_cases plannedSlot : plan.slot = some slotId
                              case' pos =>
                                by_cases plannedSource : plan.restore_from = source
                                case' pos =>
                                  cases fenceFound : storeLookup rust.worker_fences sid
                                  all_goals have modelFence := fenceLookup.trans (congrArg (Option.map workerFence) fenceFound)
                                  case' some fence =>
                                    by_cases fenced : workerFence fence = ⟨u64ToNat record.epoch, false⟩
                                    case' pos =>
                                      cases vmFound : storeLookup rust.vms vid
                                      all_goals have modelVm := vmLookup.trans (congrArg (Option.map vm) vmFound)
                                      case' none =>
                                        cases slotFound : storeLookup rust.net_slots slotId
                                        all_goals have modelSlot := slotLookup.trans (congrArg (Option.map netSlot) slotFound)
                                        case' none =>
                                          by_cases selectedSource : source = record.restore_from
                                          case' pos =>
                                            have storedSource := selectedSource.symm
                                            clear selectedSource
                                            cases source
                                            case' some selected =>
                                              cases durable : record.durable_snapshot
                                              all_goals have modelDurable := congrArg (Option.map restoreSource) durable
                                              case' some saved =>
                                                by_cases sameSource : saved = selected
                                                case' pos =>
                                                  cases usable : Controller.usableSnapshotCheck model (sandboxId sid) (sandboxRecord record) (restoreSource selected)
        all_goals
          conv =>
            enter [1, result, 1, 1]
            simp [*, -valid, bind_tc_ok, publication_starting, scheduler_key_mk,
              core.result.Result.Insts.CoreOpsTry.branch,
              core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual, core.convert.FromSame]
          refine ⟨_, rfl, ?_⟩
          unfold Controller.handleWorker
          rw [dif_pos head, dif_pos awaiting]
          unfold Controller.handleWorkerCommand
          dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
          unfold Controller.handlePrepareVmCommand
          erw [worker_sandbox_some modelFound]
          worker_prepare_steps
          try rw [lifecycle_record_operation]
          try erw [worker_prepare_operation_some modelCurrent]
          try erw [worker_prepare_operation_none modelCurrent]
          try erw [worker_prepare_plan_some modelPlan]
          try erw [worker_prepare_plan_none modelPlan]
          worker_prepare_steps
          try erw [worker_prepare_fence_some modelFence]
          try erw [worker_prepare_fence_none modelFence]
          worker_prepare_steps
          try erw [worker_prepare_freshVm_some modelVm]
          try erw [worker_prepare_freshVm_none modelVm]
          try erw [worker_prepare_freshSlot_some modelSlot]
          try erw [worker_prepare_freshSlot_none modelSlot]
          worker_prepare_steps
          try dsimp only [Option.map, Controller.handlePrepareVmCommand.match_13]
          try rw [worker_record_durable]
          try erw [worker_prepare_durable_some modelDurable]
          try erw [worker_prepare_durable_none modelDurable]
          worker_prepare_steps
          first | exact ⟨rfl, nextRep⟩ | rfl
    · obtain ⟨evaluated, related⟩ := handleWorker_notAwaiting_refines rust model rep valid id head queued awaiting
      exact ⟨.Err .NotAwaiting, evaluated, related⟩
  · refine ⟨.Err .StaleAttempt, handleWorker_stale_rejected rust model rep id head, ?_⟩
    unfold Controller.handleWorker
    rw [dif_neg head]
    rfl

end SandboxModel.RustRefinement
