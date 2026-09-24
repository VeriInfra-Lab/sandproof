import SandboxModel.RustRefinement.SchedulerCleanupVm

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance schedulerCleanupSlotOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerCleanupSlotLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerCleanupSlotEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

@[simp] theorem scheduler_slot_reclaimed (value : SandboxControllerRust.model.NetSlot) :
    (netSlot value).reclaimed = value.reclaimed := rfl

macro "slot_cleanup_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_operation, publication_record_epoch, publication_record_generation,
    operation_id_field, scheduler_key_mk, scheduler_slot_mem,
    running_slot_sandbox, running_slot_generation, running_slot_active, scheduler_slot_reclaimed,
    scheduler_plan_key, scheduler_plan_epoch, scheduler_plan_generation,
    scheduler_stopped, Option.map_some, Option.map_none, reduceCtorEq,
    sandboxId_eq_iff, u64ToNat_eq_iff, netSlotId_injective.eq_iff,
    List.mem_map, exists_eq_left,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true,
    true_and, and_true, false_and, and_false]))
macro "slot_cleanup_model_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | slot_cleanup_guard)
    | (rw [dif_neg]; rotate_left; solve | slot_cleanup_guard)))
macro "finish_cleanup_slot" up:ident found:ident current:ident plan:ident vmfound:ident : tactic => `(tactic| (
  conv =>
    enter [1, result, 1, 1]
    simp [*, bind_tc_ok, scheduler_key_mk, scheduler_stopped, alloc.vec.Vec.deref]
  refine ⟨_, rfl, ?_⟩
  unfold Controller.handleScheduleCleanupSlot
  rw [dif_pos $up, publication_sandbox_some $found]
  try erw [publication_operation_some $current]
  try erw [publication_operation_none $current]
  try erw [scheduler_plan_some $plan]
  try erw [scheduler_plan_none $plan]
  slot_cleanup_model_steps
  try erw [publication_slot_some $vmfound]
  try erw [publication_slot_none $vmfound]
  slot_cleanup_model_steps
  first | exact ⟨rfl, by assumption⟩ | rfl))

theorem handleScheduleCleanupSlot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (capacity : EffectCapacity rust 1)
    (sid : SandboxControllerRust.ids.SandboxId) (generation : U64)
    (vid : SandboxControllerRust.ids.NetSlotId) :
    ∃ result, SandboxControllerRust.scheduler.handle_schedule_cleanup_slot rust sid generation vid = ok result ∧
      SchedulerResultRep model result
        (Controller.handleScheduleCleanupSlot model (sandboxId sid) (u64ToNat generation) (netSlotId vid)) := by
  obtain ⟨next, nextEval, nextRep, _⟩ :=
    generated_enqueue_refines rust model rep capacity (.CleanupSlot sid generation vid)
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have sandboxLookup := storeRep_lookup_map rep.sandboxes sid
  have getVm := generated_get_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId
    netSlotId_partialEq_lawful rust.net_slots vid
  have vmLookup := storeRep_lookup_map rep.netSlots vid
  have allPlans := fun key => generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operation_plans key
  have allMembers := fun values => generated_sliceContains_refines
    SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful values vid
  have notCurrentCheck := generated_notCurrentSlot_refines rust model rep sid vid
  unfold SandboxControllerRust.scheduler.handle_schedule_cleanup_slot
  simp_rw [getSandbox, getVm, allPlans, allMembers, notCurrentCheck, world_clone_eq]
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    scheduler_key_eq,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    SandboxControllerRust.transition.operation_key, bind_tc_ok]
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp only [up, Bool.false_eq_true, if_false]
    · unfold Controller.handleScheduleCleanupSlot
      rw [dif_neg (by simp only [modelUp, Bool.false_eq_true, not_false_eq_true])]
      rfl
  cases found : storeLookup rust.sandboxes sid with
  | none =>
    have modelFound : model.sandboxes (sandboxId sid) = none := by simpa only [found, Option.map_none] using sandboxLookup
    refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
    unfold Controller.handleScheduleCleanupSlot
    rw [dif_pos modelUp, publication_sandbox_none modelFound]
    rfl
  | some record =>
    have modelFound : model.sandboxes (sandboxId sid) = some (sandboxRecord record) := by
      simpa only [found, Option.map_some] using sandboxLookup
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
              by_cases planned : vid ∈ plan.close_slots.val
              case' pos =>
                cases vmFound : storeLookup rust.net_slots vid
                all_goals have modelVm := vmLookup.trans (congrArg (Option.map netSlot) vmFound)
                case' some machine =>
                  by_cases owner : machine.sandbox = sid
                  case' pos =>
                    by_cases version : machine.generation = generation
                    case' pos =>
                      cases inactive : machine.active
                      case' false =>
                        cases reclaimed : machine.reclaimed
                        case' false =>
                          cases notCurrent : Controller.notCurrentSlotCheck model (sandboxId sid) (netSlotId vid)
    all_goals (focus finish_cleanup_slot modelUp modelFound modelCurrent modelPlan modelVm)

end SandboxModel.RustRefinement
