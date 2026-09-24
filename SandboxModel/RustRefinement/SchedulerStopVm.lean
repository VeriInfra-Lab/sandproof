import SandboxModel.RustRefinement.SchedulerCleanupSlot

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance schedulerStopVmOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerStopVmLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerStopVmEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem generated_stoppableVm_refines (status : SandboxControllerRust.model.VmStatus) :
    SandboxControllerRust.scheduler.stoppable_vm_status status = ok (Controller.stoppableVmStatusCheck (vmStatus status)) := by
  cases status <;> rfl

macro "stop_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, publication_revoking, lifecycle_record_operation, publication_record_epoch, publication_record_generation,
    operation_id_field, scheduler_key_mk, scheduler_vm_mem,
    running_vm_sandbox, running_vm_generation, running_vm_status,
    scheduler_plan_key, scheduler_plan_epoch, scheduler_plan_generation,
    scheduler_stopped, Option.map_some, Option.map_none, reduceCtorEq,
    sandboxId_eq_iff, u64ToNat_eq_iff, vmId_injective.eq_iff,
    List.mem_map, exists_eq_left,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true,
    true_and, and_true, false_and, and_false]))
macro "stop_model_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | stop_guard)
    | (rw [dif_neg]; rotate_left; solve | stop_guard)))
macro "finish_stop_vm" up:ident found:ident current:ident plan:ident vmfound:ident : tactic => `(tactic| (
  conv =>
    enter [1, result, 1, 1]
    simp [*, bind_tc_ok, scheduler_key_mk, scheduler_stopped, alloc.vec.Vec.deref]
  refine ⟨_, rfl, ?_⟩
  unfold Controller.handleScheduleStopVm
  rw [dif_pos $up, publication_sandbox_some $found]
  stop_model_steps
  try erw [publication_operation_some $current]
  try erw [publication_operation_none $current]
  try erw [scheduler_plan_some $plan]
  try erw [scheduler_plan_none $plan]
  stop_model_steps
  try erw [publication_vm_some $vmfound]
  try erw [publication_vm_none $vmfound]
  stop_model_steps
  first | exact ⟨rfl, by assumption⟩ | rfl))

theorem handleScheduleStopVm_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (capacity : EffectCapacity rust 1)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : U64)
    (vid : SandboxControllerRust.ids.VmId) :
    ∃ result, SandboxControllerRust.scheduler.handle_schedule_stop_vm rust sid generation vid = ok result ∧
      SchedulerResultRep model result
        (Controller.handleScheduleStopVm model (sandboxId sid) (u64ToNat generation) (vmId vid)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have sandboxLookup := storeRep_lookup_map rep.sandboxes sid
  have getVm := generated_get_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId
    vmId_partialEq_lawful rust.vms vid
  have vmLookup := storeRep_lookup_map rep.vms vid
  have allPlans := fun key => generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operation_plans key
  have allMembers := fun values => generated_sliceContains_refines
    SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful values vid
  have allFences := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.worker_fences sid
  have allEffects := fun cmd => generated_effectSucceeded_refines rust model rep cmd
  unfold SandboxControllerRust.scheduler.handle_schedule_stop_vm
  simp_rw [getSandbox, getVm, allPlans, allMembers, allFences, allEffects, generated_stoppableVm_refines, world_clone_eq]
  simp only [bind_tc_ok, command]
  simp_rw [publication_fence_store_ne rust model rep, generated_phase_ne]
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    scheduler_key_eq, publication_vmStatus_eq,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    SandboxControllerRust.transition.operation_key, bind_tc_ok]
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp only [up, Bool.false_eq_true, if_false]
    · unfold Controller.handleScheduleStopVm
      rw [dif_neg (by simp only [modelUp, Bool.false_eq_true, not_false_eq_true])]
      rfl
  cases found : storeLookup rust.sandboxes sid with
  | none =>
    have modelFound : model.sandboxes (sandboxId sid) = none := by simpa only [found, Option.map_none] using sandboxLookup
    refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
    unfold Controller.handleScheduleStopVm
    rw [dif_pos modelUp, publication_sandbox_none modelFound]
    rfl
  | some record =>
    have modelFound : model.sandboxes (sandboxId sid) = some (sandboxRecord record) := by
      simpa only [found, Option.map_some] using sandboxLookup
    obtain ⟨next, nextEval, nextRep, _⟩ :=
      generated_enqueue_refines rust model rep capacity (.StopVm sid record.epoch generation vid)
    by_cases revoking : phase record.phase = .revoking
    case' pos =>
      cases terminal : record.terminal
      case' true =>
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
              by_cases sameEpoch : plan.epoch = record.epoch
              case' pos =>
                by_cases sameGeneration : plan.generation = record.generation
                case' pos =>
                  by_cases planned : vid ∈ plan.stop_vms.val
                  case' pos =>
                    by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, true⟩
                    case' pos =>
                      cases fenceSucceeded : effectSucceededCheck model (.installFence (sandboxId sid) (u64ToNat record.epoch) true)
                      case' true =>
                        cases vmFound : storeLookup rust.vms vid
                        all_goals have modelVm := vmLookup.trans (congrArg (Option.map vm) vmFound)
                        case' some machine =>
                          by_cases owner : machine.sandbox = sid
                          case' pos =>
                            by_cases version : machine.generation = generation
                            case' pos =>
                              cases stoppable : Controller.stoppableVmStatusCheck (vmStatus machine.status)
    all_goals (focus finish_stop_vm modelUp modelFound modelCurrent modelPlan modelVm)

end SandboxModel.RustRefinement
