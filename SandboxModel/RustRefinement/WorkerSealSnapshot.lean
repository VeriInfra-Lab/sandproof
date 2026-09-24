import SandboxModel.RustRefinement.WorkerSnapshotChecks
import SandboxModel.RustRefinement.WorkerSnapshotUpdate

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance workerSealOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerSealLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerSealEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq
local instance workerSealSourceDecEq : DecidableEq SandboxControllerRust.model.RestoreSource := schedulerRestoreSourceDecEq

macro "worker_seal_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_owner, lifecycle_record_operation, publication_pausing,
    publication_record_epoch, publication_record_generation, publication_record_workspace,
    worker_record_vm_eq, operation_id_field, scheduler_key_mk,
    scheduler_plan_key, scheduler_plan_epoch, scheduler_plan_generation, scheduler_plan_sealed,
    scheduler_plan_vm_eq, scheduler_plan_target_eq,
    running_vm_owner, running_vm_sandbox, running_vm_generation, running_vm_status, scheduler_frozen,
    tenantId_injective.eq_iff, sandboxId_eq_iff, fsId_eq_iff, u64ToNat_eq_iff,
    Option.map_some, Option.map_none, reduceCtorEq,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "worker_seal_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | worker_seal_guard)
    | (rw [dif_neg]; rotate_left; solve | worker_seal_guard)))

theorem handleWorker_sealSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (valid : ResourceIndexValid model) (room : HandlerMachineHeadroom rust)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch generation : U64)
    (vid : SandboxControllerRust.ids.VmId) (workspace : SandboxControllerRust.ids.FsId)
    (snapId : SandboxControllerRust.ids.SnapshotId) (manifest : SandboxControllerRust.model.SnapshotManifestRef) (attempt : U64)
    (queued : model.inflight (effectId ⟨⟨.SealSnapshot sid epoch generation vid workspace snapId manifest⟩, attempt⟩)) :
    ∃ result, SandboxControllerRust.worker.handle_worker rust ⟨⟨.SealSnapshot sid epoch generation vid workspace snapId manifest⟩, attempt⟩ = ok result ∧
      WorkerResultRep model result (Controller.handleWorker model valid ⟨effectId ⟨⟨.SealSnapshot sid epoch generation vid workspace snapId manifest⟩, attempt⟩, queued⟩) := by
  let id : SandboxControllerRust.model.EffectId := ⟨⟨.SealSnapshot sid epoch generation vid workspace snapId manifest⟩, attempt⟩
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
      have hasSnapshot := generated_contains_key_eq SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId snapshotId_partialEq_lawful rust.snapshots snapId
      have snapshotLookup := storeRep_lookup_map rep.snapshots snapId
      have optionVm := generated_option_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful
      have optionSource := generated_option_eq SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource scheduler_restore_eq_lawful
      unfold SandboxControllerRust.worker.handle_worker
      simp only [command_clone_eq, currentEval, generated_workerId_ne_self, containsEval, getEffect, awaitingEval, awaiting, decide_true, bind_tc_ok, Bool.false_eq_true, if_false, if_true]
      dsimp only [id]
      simp_rw [getSandbox, getVm, getFence, hasSnapshot, generated_phase_ne, world_clone_eq,
        SandboxControllerRust.worker.current_plan, SandboxControllerRust.transition.operation_key, allPlans]
      simp only [bind_tc_ok]
      simp_rw [publication_fence_store_ne rust model rep]
      simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        SandboxControllerRust.ids.TenantId.Insts.CoreCmpPartialEqTenantId.eq,
        SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
        SandboxControllerRust.ids.FsId.Insts.CoreCmpPartialEqFsId.eq,
        scheduler_key_eq, publication_vmStatus_eq, optionVm, optionSource, bind_tc_ok]
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
        obtain ⟨indices, snapshots, nextEval, nextRep⟩ := generated_workerPutSnapshot_refines rust model rep room snapId
          ⟨record.owner, sid, record.generation, record.workspace, manifest, .LocalSealed, false⟩
        have sealedRep : WorkerWorldRep { rust with resource_indexes := indices, snapshots := snapshots }
            (sealSnapshot model (snapshotId snapId) (snapshotManifestRef manifest) (sandboxRecord record) (sandboxId sid)) := nextRep
        simp_rw [nextEval]
        by_cases pausing : phase record.phase = .pausing
        case' pos =>
          by_cases sameEpoch : epoch = record.epoch
          case' pos =>
            by_cases sameGeneration : generation = record.generation
            case' pos =>
              by_cases sameWorkspace : workspace = record.workspace
              case' pos =>
                cases current : record.current_operation
                all_goals have modelCurrent := congrArg (Option.map operation) current
                case' some op =>
                  have planLookup := storeRep_lookup_map rep.operationPlans (SandboxControllerRust.model.OperationKey.mk sid op.id)
                  change model.operationPlans (SandboxModel.operationKey (sandboxId sid) (operationId op.id)) = _ at planLookup
                  cases planFound : storeLookup rust.operation_plans ⟨sid, op.id⟩
                  all_goals have modelPlan := planLookup.trans (congrArg (Option.map operationPlan) planFound)
                  case' some plan =>
                    by_cases sameKey : operationKey plan.key = SandboxModel.operationKey (sandboxId sid) (operationId op.id)
                    case' pos =>
                      by_cases planEpoch : plan.epoch = record.epoch
                      case' pos =>
                        by_cases planGeneration : plan.generation = record.generation
                        case' pos =>
                          cases sealed : plan.sealed
                          case' true =>
                            by_cases plannedVm : plan.vm = some vid
                            case' pos =>
                              by_cases target : plan.target_snapshot = some ⟨snapId, manifest⟩
                              case' pos =>
                                cases vmFound : storeLookup rust.vms vid
                                all_goals have modelVm := vmLookup.trans (congrArg (Option.map vm) vmFound)
                                case' some machine =>
                                  by_cases currentVm : record.current_vm = some vid
                                  case' pos =>
                                    by_cases owner : machine.owner = record.owner
                                    case' pos =>
                                      by_cases sandbox : machine.sandbox = sid
                                      case' pos =>
                                        by_cases version : machine.generation = record.generation
                                        case' pos =>
                                          by_cases frozen : vmStatus machine.status = .frozen
                                          case' pos =>
                                            by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, false⟩
                                            case' pos =>
                                              cases fresh : storeLookup rust.snapshots snapId
                                              all_goals have modelFresh := snapshotLookup.trans (congrArg (Option.map snapshot) fresh)
        all_goals
          conv =>
            enter [1, result, 1, 1]
            simp [*, -valid, bind_tc_ok, scheduler_key_mk, scheduler_frozen, publication_pausing,
              core.result.Result.Insts.CoreOpsTry.branch,
              core.result.Result.Insts.CoreOpsTryTraitFromResidualResultInfallible.from_residual, core.convert.FromSame]
          refine ⟨_, rfl, ?_⟩
          unfold Controller.handleWorker
          rw [dif_pos head, dif_pos awaiting]
          unfold Controller.handleWorkerCommand
          dsimp only [id, effectId, logicalEffectId, EffectId.command, command]
          erw [worker_sandbox_some modelFound]
          worker_seal_steps
          try rw [lifecycle_record_operation]
          try erw [worker_prepare_operation_some modelCurrent]
          try erw [worker_prepare_operation_none modelCurrent]
          try erw [worker_prepare_plan_some modelPlan]
          try erw [worker_prepare_plan_none modelPlan]
          worker_seal_steps
          try erw [worker_vm_some modelVm]
          try erw [worker_vm_none modelVm]
          worker_seal_steps
          try erw [worker_snapshot_fresh_some modelFresh]
          try erw [worker_snapshot_fresh_none modelFresh]
          first | exact ⟨rfl, sealedRep⟩ | rfl
    · obtain ⟨evaluated, related⟩ := handleWorker_notAwaiting_refines rust model rep valid id head queued awaiting
      exact ⟨.Err .NotAwaiting, evaluated, related⟩
  · refine ⟨.Err .StaleAttempt, handleWorker_stale_rejected rust model rep id head, ?_⟩
    unfold Controller.handleWorker
    rw [dif_neg head]
    rfl

end SandboxModel.RustRefinement
