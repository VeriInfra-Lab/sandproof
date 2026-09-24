import SandboxModel.RustRefinement.SchedulerReserveStartupUpdate
import SandboxModel.RustRefinement.SchedulerReservePause

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance schedulerReserveStartupOperationKeyDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerReserveStartupLogicalEffectIdDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerReserveStartupEffectIdDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq
local instance schedulerReserveStartupRestoreSourceDecEq : DecidableEq SandboxControllerRust.model.RestoreSource := schedulerRestoreSourceDecEq

macro "reserve_startup_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, publication_starting, lifecycle_record_vm,
    lifecycle_record_operation, publication_record_epoch, publication_record_generation,
    operation_id_field, scheduler_key_sandbox, scheduler_key_operation, operationId_injective.eq_iff,
    scheduler_plan_key, scheduler_plan_epoch, scheduler_plan_generation,
    scheduler_plan_sealed, scheduler_plan_vm, scheduler_plan_slot, publication_record_restore, scheduler_plan_restore, restoreSource_option_map_eq_iff,
    Option.map_eq_none_iff, Option.map_some, Option.map_none, reduceCtorEq, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "reserve_startup_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | reserve_startup_guard)
    | (rw [dif_neg]; rotate_left; solve | reserve_startup_guard)))
macro "finish_reserve_startup" up:ident found:ident current:ident plan:ident vmfound:ident slotfound:ident : tactic => `(tactic| (
  conv =>
    enter [1, result, 1, 1]
    simp [*, bind_tc_ok, publication_starting, core.option.Option.is_some]
  refine ⟨_, rfl, ?_⟩
  unfold Controller.handleReserveStartup
  rw [dif_pos $up]
  erw [publication_sandbox_some $found]
  reserve_startup_steps
  try erw [publication_operation_some $current]
  try erw [publication_operation_none $current]
  reserve_startup_steps
  try erw [scheduler_plan_some $plan]
  try erw [scheduler_plan_none $plan]
  reserve_startup_steps
  try erw [scheduler_vm_fresh_some $vmfound]
  try erw [scheduler_vm_fresh_none $vmfound]
  try erw [scheduler_slot_fresh_some $slotfound]
  try erw [scheduler_slot_fresh_none $slotfound]
  first | exact ⟨by simp [modelEvent, *], by assumption⟩ | rfl))

theorem handleReserveStartup_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (vmRoom : U64Headroom rust.next_vm_id)
    (slotRoom : U64Headroom rust.next_slot_id)
    (planRoom : StoreHeadroom rust.operation_plans)
    (key : SandboxControllerRust.model.OperationKey) (listsRoom : StartupPlanHeadroom rust key) :
    ∃ result, SandboxControllerRust.scheduler.handle_reserve_startup rust key = ok result ∧
      SchedulerResultRep model result (Controller.handleReserveStartup model (operationKey key)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes key.sandbox
  have sandboxLookup := storeRep_lookup_map rep.sandboxes key.sandbox
  have getPlan := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operation_plans key
  have planLookup := storeRep_lookup_map rep.operationPlans key
  have containsVm := generated_contains_key_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId
    vmId_partialEq_lawful rust.vms rust.next_vm_id
  have vmLookup := storeRep_lookup_map rep.vms rust.next_vm_id
  have containsSlot := generated_contains_key_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId
    netSlotId_partialEq_lawful rust.net_slots rust.next_slot_id
  have slotLookup := storeRep_lookup_map rep.netSlots rust.next_slot_id
  have cursor : vmId rust.next_vm_id = ⟨model.nextVmId⟩ := by simp [vmId, rep.nextVmId]
  have slotCursor : netSlotId rust.next_slot_id = ⟨model.nextSlotId⟩ := by simp [netSlotId, rep.nextSlotId]
  rw [cursor] at vmLookup
  rw [slotCursor] at slotLookup
  have sourceEq := generated_option_eq
    SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource scheduler_restore_eq_lawful
  unfold SandboxControllerRust.scheduler.handle_reserve_startup
  simp_rw [getSandbox, getPlan, containsVm, containsSlot, generated_phase_ne]
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    scheduler_key_eq, sourceEq, SandboxControllerRust.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq, bind_tc_ok]
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp only [up, Bool.false_eq_true, if_false]
    · unfold Controller.handleReserveStartup
      rw [dif_neg (by simp only [modelUp, Bool.false_eq_true, not_false_eq_true])]
      rfl
  cases found : storeLookup rust.sandboxes key.sandbox with
  | none =>
    have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
    refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
    unfold Controller.handleReserveStartup
    rw [dif_pos modelUp]
    erw [publication_sandbox_none modelFound]
    rfl
  | some record =>
    have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
    by_cases starting : phase record.phase = .starting
    case' pos =>
      cases terminal : record.terminal
      case' false =>
        cases current : record.current_operation
        all_goals have modelCurrent := congrArg (Option.map operation) current
        case' some op =>
          by_cases currentId : op.id = key.operation
          case' pos =>
            cases planFound : storeLookup rust.operation_plans key
            all_goals have modelPlan := planLookup.trans (congrArg (Option.map operationPlan) planFound)
            case' some plan =>
              by_cases sameKey : operationKey plan.key = operationKey key
              case' pos =>
                by_cases sameEpoch : plan.epoch = record.epoch
                case' pos =>
                  by_cases sameGeneration : plan.generation = record.generation
                  case' pos =>
                    cases sealed : plan.sealed
                    case' false =>
                      cases planVm : plan.vm
                      case' none =>
                        cases slot : plan.slot
                        case' none =>
                          by_cases source : plan.restore_from = record.restore_from
                          case' pos =>
                            obtain ⟨stopsRoom, closesRoom⟩ := listsRoom plan planFound
                            obtain ⟨next, nextEval, nextRep⟩ :=
                              generated_reserveStartupResources_refines rust model rep vmRoom slotRoom planRoom key plan stopsRoom closesRoom
                            cases vmFound : storeLookup rust.vms rust.next_vm_id
                            all_goals have modelVm := vmLookup.trans (congrArg (Option.map vm) vmFound)
                            case' none =>
                              cases slotFound : storeLookup rust.net_slots rust.next_slot_id
                              all_goals have modelSlot := slotLookup.trans (congrArg (Option.map netSlot) slotFound)
    all_goals (focus finish_reserve_startup modelUp modelFound modelCurrent modelPlan modelVm modelSlot)

end SandboxModel.RustRefinement
