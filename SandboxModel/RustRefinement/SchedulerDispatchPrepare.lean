import SandboxModel.RustRefinement.SchedulerSealSnapshot

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance schedulerDispatchPrepareOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerDispatchPrepareLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerDispatchPrepareEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq
local instance schedulerDispatchPrepareSourceDecEq : DecidableEq SandboxControllerRust.model.RestoreSource := schedulerRestoreSourceDecEq

theorem scheduler_vm_fresh_some {α : Sort u} {value : Option Vm}
    {record : Vm} (found : value = some record)
    (present : (r : Vm) → value = some r → α) (missing : value = none → α) :
    Controller.handleReserveStartup.match_3 (fun _ => α) value present missing = present record found := by
  cases found
  rfl
theorem scheduler_vm_fresh_none {α : Sort u} {value : Option Vm}
    (found : value = none)
    (present : (r : Vm) → value = some r → α) (missing : value = none → α) :
    Controller.handleReserveStartup.match_3 (fun _ => α) value present missing = missing found := by
  cases found
  rfl
theorem scheduler_slot_fresh_some {α : Sort u} {value : Option NetSlot}
    {record : NetSlot} (found : value = some record)
    (present : (r : NetSlot) → value = some r → α) (missing : value = none → α) :
    Controller.handleReserveStartup.match_1 (fun _ => α) value present missing = present record found := by
  cases found
  rfl
theorem scheduler_slot_fresh_none {α : Sort u} {value : Option NetSlot}
    (found : value = none)
    (present : (r : NetSlot) → value = some r → α) (missing : value = none → α) :
    Controller.handleReserveStartup.match_1 (fun _ => α) value present missing = missing found := by
  cases found
  rfl

macro "prepare_guard" : tactic => `(tactic| (simp only [*, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, publication_starting,
    lifecycle_record_operation, publication_record_epoch, publication_record_generation,
    publication_record_restore, scheduler_plan_restore, restoreSource_option_map_eq_iff,
    operation_id_field, scheduler_key_mk, scheduler_plan_key, scheduler_plan_epoch, scheduler_plan_generation,
    scheduler_plan_sealed, scheduler_plan_vm_eq, scheduler_plan_slot_eq,
    Option.map_some, Option.map_none, reduceCtorEq, u64ToNat_eq_iff,
    if_true, if_false, dif_pos, dif_neg, decide_true, decide_false,
    Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
    ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true, true_and, and_true, false_and, and_false]))
macro "prepare_model_steps" : tactic => `(tactic| (
  repeat' first
    | (rw [dif_pos]; rotate_left; solve | prepare_guard)
    | (rw [dif_neg]; rotate_left; solve | prepare_guard)))
macro "finish_prepare" up:ident found:ident current:ident plan:ident vmfound:ident slot:ident : tactic => `(tactic| (
  conv =>
    enter [1, result, 1, 1]
    simp [*, bind_tc_ok, scheduler_key_mk, publication_starting]
  refine ⟨_, rfl, ?_⟩
  unfold Controller.handleDispatchPrepare
  rw [dif_pos $up, publication_sandbox_some $found]
  prepare_model_steps
  try erw [publication_operation_some $current]
  try erw [publication_operation_none $current]
  try erw [scheduler_plan_some $plan]
  try erw [scheduler_plan_none $plan]
  prepare_model_steps
  try erw [scheduler_vm_fresh_some $vmfound]
  try erw [scheduler_vm_fresh_none $vmfound]
  try erw [scheduler_slot_fresh_some $slot]
  try erw [scheduler_slot_fresh_none $slot]
  prepare_model_steps
  first | exact ⟨rfl, by assumption⟩ | rfl))

theorem handleDispatchPrepare_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (capacity : EffectCapacity rust 1)
    (sid : SandboxControllerRust.ids.SandboxId) (vid : SandboxControllerRust.ids.VmId)
    (slotId : SandboxControllerRust.ids.NetSlotId) :
    ∃ result, SandboxControllerRust.scheduler.handle_dispatch_prepare rust sid vid slotId = ok result ∧
      SchedulerResultRep model result
        (Controller.handleDispatchPrepare model (sandboxId sid) (vmId vid) (netSlotId slotId)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have sandboxLookup := storeRep_lookup_map rep.sandboxes sid
  have allPlans := fun key => generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operation_plans key
  have allFences := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.worker_fences sid
  have containsVm := generated_contains_key_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId
    vmId_partialEq_lawful rust.vms vid
  have vmLookup := storeRep_lookup_map rep.vms vid
  have containsSlot := generated_contains_key_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId
    netSlotId_partialEq_lawful rust.net_slots slotId
  have slotLookup := storeRep_lookup_map rep.netSlots slotId
  have allReady := fun record => generated_restoreSourceReady_refines rust model rep sid record
  have sourceEq := generated_option_eq
    SandboxControllerRust.model.RestoreSource.Insts.CoreCmpPartialEqRestoreSource scheduler_restore_eq_lawful
  have vmEq := generated_option_eq SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful
  have slotEq := generated_option_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful
  unfold SandboxControllerRust.scheduler.handle_dispatch_prepare
  simp_rw [getSandbox, allPlans, allFences, containsVm, containsSlot, allReady, world_clone_eq]
  simp only [bind_tc_ok, command]
  simp_rw [publication_fence_store_ne rust model rep, generated_phase_ne]
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    scheduler_key_eq, sourceEq, vmEq, slotEq, SandboxControllerRust.transition.operation_key, bind_tc_ok]
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp only [up, Bool.false_eq_true, if_false]
    · unfold Controller.handleDispatchPrepare
      rw [dif_neg (by simp only [modelUp, Bool.false_eq_true, not_false_eq_true])]
      rfl
  cases found : storeLookup rust.sandboxes sid with
  | none =>
    have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
    refine ⟨.Err .SandboxNotFound, rfl, ?_⟩
    unfold Controller.handleDispatchPrepare
    rw [dif_pos modelUp, publication_sandbox_none modelFound]
    rfl
  | some record =>
    have modelFound := sandboxLookup.trans (congrArg (Option.map sandboxRecord) found)
    obtain ⟨next, nextEval, nextRep, _⟩ :=
      generated_enqueue_refines rust model rep capacity (.PrepareVm sid record.epoch record.generation vid slotId record.restore_from)
    by_cases starting : phase record.phase = .starting
    case' pos =>
      cases terminal : record.terminal
      case' false =>
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
                  cases sealed : plan.sealed
                  case' true =>
                    by_cases plannedVm : plan.vm = some vid
                    case' pos =>
                      by_cases plannedSlot : plan.slot = some slotId
                      case' pos =>
                        by_cases source : plan.restore_from = record.restore_from
                        case' pos =>
                          by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, false⟩
                          case' pos =>
                            cases vmFound : storeLookup rust.vms vid
                            all_goals have modelVm := vmLookup.trans (congrArg (Option.map vm) vmFound)
                            case' none =>
                              cases slotFound : storeLookup rust.net_slots slotId
                              all_goals have modelSlot := slotLookup.trans (congrArg (Option.map netSlot) slotFound)
                              case' none =>
                                cases ready : Controller.restoreSourceReadyCheck model (sandboxId sid) (sandboxRecord record)
    all_goals (focus finish_prepare modelUp modelFound modelCurrent modelPlan modelVm modelSlot)

end SandboxModel.RustRefinement
