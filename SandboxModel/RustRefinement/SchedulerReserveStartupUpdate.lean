import SandboxModel.RustRefinement.SchedulerReservePauseUpdate
import SandboxModel.RustRefinement.InsertUnique

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance schedulerReserveStartupUpdateOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance schedulerReserveStartupUpdateLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance schedulerReserveStartupUpdateEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

/-- Additional finite-vector capacity for a startup reservation's selected plan. -/
def StartupPlanHeadroom (rust : SandboxControllerRust.model.World)
    (key : SandboxControllerRust.model.OperationKey) : Prop :=
  ∀ plan, storeLookup rust.operation_plans key = some plan →
    VecHeadroom plan.stop_vms ∧ VecHeadroom plan.close_slots

theorem generated_reserveStartupResources_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : SchedulerWorldRep rust model)
    (vmRoom : U64Headroom rust.next_vm_id) (slotRoom : U64Headroom rust.next_slot_id)
    (planRoom : StoreHeadroom rust.operation_plans)
    (key : SandboxControllerRust.model.OperationKey) (plan : SandboxControllerRust.model.OperationPlan)
    (stopsRoom : VecHeadroom plan.stop_vms) (closesRoom : VecHeadroom plan.close_slots) :
    ∃ next, SandboxControllerRust.transition.reserve_startup_resources rust key plan = ok (some next) ∧
      SchedulerWorldRep next (reserveStartupResources model (operationKey key) (operationPlan plan)) := by
  obtain ⟨vmNotMax, nextVm, vmEval, vmSame⟩ := lifecycle_increment rust.next_vm_id vmRoom
  obtain ⟨slotNotMax, nextSlot, slotEval, slotSame⟩ := lifecycle_increment rust.next_slot_id slotRoom
  obtain ⟨stops, stopsEval, stopsSame⟩ := WP.spec_imp_exists
    (generated_insertUnique_generic SandboxControllerRust.ids.VmId.Insts.CoreCloneClone (by intros; rfl)
      SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful
      plan.stop_vms rust.next_vm_id stopsRoom)
  obtain ⟨closes, closesEval, closesSame⟩ := WP.spec_imp_exists
    (generated_insertUnique_generic SandboxControllerRust.ids.NetSlotId.Insts.CoreCloneClone (by intros; rfl)
      SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful
      plan.close_slots rust.next_slot_id closesRoom)
  let reserved : SandboxControllerRust.model.OperationPlan :=
    { plan with vm := some rust.next_vm_id, slot := some rust.next_slot_id, stop_vms := stops, close_slots := closes, sealed := true }
  obtain ⟨plans, plansEval, plansRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operationKey_partialEq_lawful operationKey operationKey_injective
      (fun a b => operationPlan a = b) rust.operation_plans model.operationPlans rep.operationPlans
      key reserved (operationPlan reserved) rfl planRoom)
  refine ⟨{ rust with operation_plans := plans, next_vm_id := nextVm, next_slot_id := nextSlot }, ?_, ?_⟩
  · simp [SandboxControllerRust.transition.reserve_startup_resources, vmNotMax, slotNotMax,
      vmEval, slotEval, stopsEval, closesEval, plansEval, world_clone_eq, reserved]
  · have targetSame : operationPlan reserved =
        { (operationPlan plan) with vm := some ⟨model.nextVmId⟩, slot := some ⟨model.nextSlotId⟩, stopVms := insertUnique ⟨model.nextVmId⟩ (operationPlan plan).stopVms, closeSlots := insertUnique ⟨model.nextSlotId⟩ (operationPlan plan).closeSlots, sealed := true } := by
      simp only [reserved, operationPlan, Option.map_some, stopsSame, closesSame,
        map_insertUnique_of_injective vmId_injective, map_insertUnique_of_injective netSlotId_injective]
      simp [vmId, netSlotId, rep.nextVmId, rep.nextSlotId]
    exact { rep with
      operationPlans := by simpa only [targetSame, reserveStartupResources, setOperationPlan, valueRelations] using plansRep
      nextVmId := vmSame.trans (congrArg (· + 1) rep.nextVmId)
      nextSlotId := slotSame.trans (congrArg (· + 1) rep.nextSlotId) }

end SandboxModel.RustRefinement
