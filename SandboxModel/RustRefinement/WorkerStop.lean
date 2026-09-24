import SandboxModel.RustRefinement.WorkerSnapshotChecks

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance workerStopOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerStopLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerStopEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

@[simp] theorem worker_source_snapshot (source : SandboxControllerRust.model.RestoreSource) :
    (restoreSource source).snapshot = snapshotId source.snapshot := rfl

@[simp] theorem worker_stoppable_frozen : Controller.stoppableVmStatusCheck .frozen = true := rfl

macro "worker_stop_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, lifecycle_record_owner, publication_starting,
    publication_record_epoch, publication_record_generation, publication_record_policy,
    worker_record_vm_eq,
    running_vm_owner, running_vm_sandbox, running_vm_generation, running_vm_status,
    publication_pausing, publication_executing, publication_revoking, scheduler_frozen,
    worker_stoppable_iff, worker_stoppable_frozen, lifecycle_record_operation, scheduler_plan_target,
    operation_id_field, scheduler_key_mk, worker_source_snapshot,
    scheduler_snapshot_sandbox, scheduler_snapshot_generation, scheduler_snapshot_stage,
    worker_snapshot_immutable, publication_durable, Option.map_some, Option.map_none, reduceCtorEq,
    tenantId_injective.eq_iff, sandboxId_eq_iff, vmId_injective.eq_iff, networkPolicy_eq_iff, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false] <;> assumption))
macro "worker_stop_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | worker_stop_guard)
    | (rw [dif_neg]; rotate_left; solve | worker_stop_guard)))

theorem handleWorker_stop_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (valid : ResourceIndexValid model) (room : HandlerMachineHeadroom rust)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch generation : U64)
    (vid : SandboxControllerRust.ids.VmId) (attempt : U64)
    (queued : model.inflight (effectId ⟨⟨.StopVm sid epoch generation vid⟩, attempt⟩)) :
    ∃ result, SandboxControllerRust.worker.handle_worker rust ⟨⟨.StopVm sid epoch generation vid⟩, attempt⟩ = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId ⟨⟨.StopVm sid epoch generation vid⟩, attempt⟩, queued⟩) := by
  let id : SandboxControllerRust.model.EffectId := ⟨⟨.StopVm sid epoch generation vid⟩, attempt⟩
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
      have allPlans := fun key => generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operationKey_partialEq_lawful rust.operation_plans key
      have allSnapshots := fun snapId => generated_get_eq SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId snapshotId_partialEq_lawful rust.snapshots snapId
      have optionVm := generated_option_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful
      unfold SandboxControllerRust.worker.handle_worker
      simp only [command_clone_eq, currentEval, generated_workerId_ne_self, containsEval, getEffect, awaitingEval, awaiting, decide_true, bind_tc_ok, Bool.false_eq_true, if_false, if_true]
      dsimp only [id]
      simp_rw [getSandbox, getVm, getFence, allSnapshots, generated_phase_eq, world_clone_eq,
        generated_workerStoppable_refines,
        SandboxControllerRust.worker.current_plan, SandboxControllerRust.transition.operation_key, allPlans]
      simp only [bind_tc_ok]
      simp_rw [publication_fence_store_ne rust model rep]
      simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
        publication_vmStatus_eq, publication_snapshotStage_eq, optionVm, bind_tc_ok]
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
        cases vmFound : storeLookup rust.vms vid
        all_goals have modelVm := vmLookup.trans (congrArg (Option.map vm) vmFound)
        case' some machine =>
          obtain ⟨next, nextEval, nextRep⟩ := generated_workerPutVm_refines rust model rep room vid { machine with status := .Stopped }
          have stoppedRep : WorkerWorldRep next (stopResource model (vmId vid) (vm machine)) := nextRep
          simp_rw [nextEval]
          by_cases sameEpoch : epoch = record.epoch
          case' pos =>
            by_cases sameGeneration : generation = machine.generation
            case' pos =>
              by_cases sandbox : machine.sandbox = sid
              case' pos =>
                cases stoppable : Controller.stoppableVmStatusCheck (vmStatus machine.status)
                case' true =>
                  by_cases pausing : phase record.phase = .pausing
                  case' pos =>
                    by_cases currentVm : record.current_vm = some vid
                    case' pos =>
                      cases terminal : record.terminal
                      case' false =>
                        by_cases frozen : vmStatus machine.status = .frozen
                        case' pos =>
                          by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, false⟩
                          case' pos =>
                            cases current : record.current_operation
                            all_goals have modelCurrent := congrArg (Option.map operation) current
                            case' some op =>
                              have planLookup := storeRep_lookup_map rep.operationPlans (SandboxControllerRust.model.OperationKey.mk sid op.id)
                              change model.operationPlans (SandboxModel.operationKey (sandboxId sid) (operationId op.id)) = _ at planLookup
                              cases planFound : storeLookup rust.operation_plans ⟨sid, op.id⟩
                              all_goals have modelPlan := planLookup.trans (congrArg (Option.map operationPlan) planFound)
                              case' some plan =>
                                cases target : plan.target_snapshot
                                all_goals have modelTarget := congrArg (Option.map restoreSource) target
                                case' some source =>
                                  have snapshotLookup := storeRep_lookup_map rep.snapshots source.snapshot
                                  cases snapshotFound : storeLookup rust.snapshots source.snapshot
                                  all_goals have modelSnapshot := snapshotLookup.trans (congrArg (Option.map snapshot) snapshotFound)
                                  case' some snap =>
                                    by_cases snapshotSandbox : snap.sandbox = sid
                                    case' pos =>
                                      by_cases version : snap.source_generation = record.generation
                                      case' pos =>
                                        by_cases durable : snapshotStage snap.stage = .durable
                                        case' pos =>
                                          cases immutable : snap.immutable
                  all_goals
                    try
                      have notPausing : phase record.phase ≠ .pausing := by assumption
                      by_cases revoking : phase record.phase = .revoking
                      case' pos =>
                        cases terminal : record.terminal
                        case' true =>
                          by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, true⟩
        all_goals
          conv =>
            enter [1, result, 1, 1]
            simp [*, -valid, bind_tc_ok, scheduler_key_mk, scheduler_frozen, publication_pausing, publication_revoking, publication_durable, Controller.stoppableVmStatusCheck,
              core.result.Result.Insts.CoreOpsTry.branch,
              core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual, core.convert.FromSame]
          refine ⟨_, rfl, ?_⟩
          unfold Controller.handleWorker
          rw [dif_pos head, dif_pos awaiting]
          unfold Controller.handleWorkerCommand
          dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
          erw [worker_sandbox_some modelFound]
          try erw [worker_vm_some modelVm]
          try erw [worker_vm_none modelVm]
          worker_stop_steps
          try rw [lifecycle_record_operation]
          try erw [worker_prepare_operation_some modelCurrent]
          try erw [worker_prepare_operation_none modelCurrent]
          try erw [worker_prepare_plan_some modelPlan]
          try erw [worker_prepare_plan_none modelPlan]
          try rw [scheduler_plan_target]
          try erw [worker_prepare_durable_some modelTarget]
          try erw [worker_prepare_durable_none modelTarget]
          try rw [worker_source_snapshot]
          try erw [worker_snapshot_some modelSnapshot]
          try erw [worker_snapshot_none modelSnapshot]
          worker_stop_steps
          first | exact ⟨rfl, stoppedRep⟩ | rfl
    · obtain ⟨evaluated, related⟩ := handleWorker_notAwaiting_refines rust model rep valid id head queued awaiting
      exact ⟨.Err .NotAwaiting, evaluated, related⟩
  · refine ⟨.Err .StaleAttempt, handleWorker_stale_rejected rust model rep id head, ?_⟩
    unfold Controller.handleWorker
    rw [dif_neg head]
    rfl

end SandboxModel.RustRefinement
