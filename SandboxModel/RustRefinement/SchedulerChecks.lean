import SandboxModel.RustRefinement.PublicationRunningChecks
import SandboxModel.RustRefinement.PublicationRunning
import SandboxModel.RustRefinement.SchedulerHandler

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance schedulerChecksOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerChecksLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerChecksEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem scheduler_key_eq (left right : SandboxControllerRust.model.OperationKey) :
    SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey.eq left right =
      ok (decide (operationKey left = operationKey right)) := by
  simpa only [operationKey_injective.eq_iff] using operationKey_partialEq_lawful left right

@[simp] theorem scheduler_key_mk (sid : SandboxControllerRust.ids.SandboxId)
    (op : SandboxControllerRust.ids.OperationId) :
    operationKey ⟨sid, op⟩ = SandboxModel.operationKey (sandboxId sid) (operationId op) := rfl

@[simp] theorem scheduler_vm_mem (plan : SandboxControllerRust.model.OperationPlan)
    (vid : SandboxControllerRust.ids.VmId) :
    vmId vid ∈ (operationPlan plan).stopVms ↔ vid ∈ plan.stop_vms.val := by
  simp [operationPlan, List.mem_map, vmId_injective.eq_iff]

@[simp] theorem scheduler_slot_mem (plan : SandboxControllerRust.model.OperationPlan)
    (slot : SandboxControllerRust.ids.NetSlotId) :
    netSlotId slot ∈ (operationPlan plan).closeSlots ↔ slot ∈ plan.close_slots.val := by
  simp [operationPlan, List.mem_map, netSlotId_injective.eq_iff]

theorem scheduler_plan_some {α : Sort u} {value : Option OperationPlan}
    {record : OperationPlan} (found : value = some record)
    (missing : value = none → α) (present : (r : OperationPlan) → value = some r → α) :
    Controller.handleReserveStartup.match_5 (fun _ => α) value missing present = present record found := by
  cases found
  rfl
theorem scheduler_plan_none {α : Sort u} {value : Option OperationPlan}
    (found : value = none)
    (missing : value = none → α) (present : (r : OperationPlan) → value = some r → α) :
    Controller.handleReserveStartup.match_5 (fun _ => α) value missing present = missing found := by
  cases found
  rfl

theorem generated_sliceContains_refines [DecidableEq T]
    (partialEq : core.cmp.PartialEq T T) (lawful : LawfulPartialEq partialEq)
    (values : Slice T) (value : T) :
    core.slice.Slice.contains partialEq values value = ok (decide (value ∈ values.val)) := by
  unfold core.slice.Slice.contains
  have aux : ∀ values : List T, List.anyM (partialEq.eq value) values = ok (decide (value ∈ values)) := by
    intro values
    induction values with
    | nil => rfl
    | cons head tail ih =>
      simp only [List.anyM]
      rw [lawful value head]
      by_cases same : value = head <;> simp [same, ih] <;> rfl
  exact aux values.val

theorem generated_notCurrentVm_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (vid : SandboxControllerRust.ids.VmId) :
    SandboxControllerRust.scheduler.not_current_vm_check rust sid vid =
      ok (Controller.notCurrentVmCheck model (sandboxId sid) (vmId vid)) := by
  have get := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have lookup := storeRep_lookup_map rep.sandboxes sid
  cases found : storeLookup rust.sandboxes sid with
  | none => simp [SandboxControllerRust.scheduler.not_current_vm_check, get, found,
      Controller.notCurrentVmCheck, lookup]
  | some record =>
    cases current : record.current_vm <;>
      simp [SandboxControllerRust.scheduler.not_current_vm_check, get, found, current,
        Controller.notCurrentVmCheck, lookup, lifecycle_record_vm,
        core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        core.option.Option.Insts.CoreCmpPartialEqOption.eq,
        SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId.eq,
        vmId_injective.eq_iff]

theorem generated_notCurrentSlot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (slot : SandboxControllerRust.ids.NetSlotId) :
    SandboxControllerRust.scheduler.not_current_slot_check rust sid slot =
      ok (Controller.notCurrentSlotCheck model (sandboxId sid) (netSlotId slot)) := by
  have get := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have lookup := storeRep_lookup_map rep.sandboxes sid
  cases found : storeLookup rust.sandboxes sid with
  | none => simp [SandboxControllerRust.scheduler.not_current_slot_check, get, found,
      Controller.notCurrentSlotCheck, lookup]
  | some record =>
    cases current : record.net_slot <;>
      simp [SandboxControllerRust.scheduler.not_current_slot_check, get, found, current,
        Controller.notCurrentSlotCheck, lookup, publication_record_slot,
        core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
        core.option.Option.Insts.CoreCmpPartialEqOption.eq,
        SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId.eq,
        netSlotId_injective.eq_iff]

end SandboxModel.RustRefinement
