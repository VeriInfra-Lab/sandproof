import SandboxModel.RustRefinement.WorkerVmUpdate

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance workerSlotUpdateOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerSlotUpdateLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerSlotUpdateEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem generated_workerRegisterSlot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (sid : SandboxControllerRust.ids.SandboxId)
    (vid : SandboxControllerRust.ids.NetSlotId) :
    ∃ indices, SandboxControllerRust.worker.register_slot rust sid vid = ok { rust with resource_indexes := indices } ∧
      WorkerWorldRep { rust with resource_indexes := indices } (registerSlot model (sandboxId sid) (netSlotId vid)) := by
  obtain ⟨index, indexEval, indexSame, _, slotRoom, _⟩ := generated_workerResourceIndex_refines rust model rep room sid
  obtain ⟨values, valuesEval, valuesSame⟩ := WP.spec_imp_exists
    (generated_workerInsertUnique_refines SandboxControllerRust.ids.NetSlotId.Insts.CoreCloneClone (by intros; rfl)
      SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful index.slots vid slotRoom)
  let updated : SandboxControllerRust.model.ResourceIndex := { index with slots := values }
  obtain ⟨indices, putEval, indicesRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_defaultStoreRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => resourceIndex a = b)
      rust.resource_indexes model.resourceIndexes ResourceIndex.empty rep.resourceIndexes
      sid updated (resourceIndex updated) rfl room.resourceIndexes)
  refine ⟨indices, ?_, ?_⟩
  · simp [SandboxControllerRust.worker.register_slot, indexEval, valuesEval, putEval, updated]
  · have updatedSame : resourceIndex updated =
        { (model.resourceIndexes (sandboxId sid)) with slots := insertUnique (netSlotId vid) (model.resourceIndexes (sandboxId sid)).slots } := by
      rw [← indexSame]
      simp [updated, resourceIndex, valuesSame, map_insertUnique_of_injective netSlotId_injective]
    exact { rep with resourceIndexes := by simpa only [updatedSame, registerSlot, setResourceIndex, valueRelations] using indicesRep }

theorem generated_workerPutSlot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (vid : SandboxControllerRust.ids.NetSlotId)
    (value : SandboxControllerRust.model.NetSlot) :
    ∃ next, SandboxControllerRust.worker.put_slot rust vid value = ok next ∧
      WorkerWorldRep next (setNetSlot model (netSlotId vid) (netSlot value)) := by
  obtain ⟨indices, registerEval, registerRep⟩ := generated_workerRegisterSlot_refines rust model rep room value.sandbox vid
  obtain ⟨slots, putEval, slotsRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId
      netSlotId_partialEq_lawful netSlotId netSlotId_injective (fun a b => netSlot a = b)
      rust.net_slots model.netSlots rep.netSlots vid value (netSlot value) rfl room.netSlots)
  refine ⟨{ rust with resource_indexes := indices, net_slots := slots }, ?_, ?_⟩
  · simp [SandboxControllerRust.worker.put_slot, registerEval, putEval]
  · exact { registerRep with netSlots := slotsRep }

end SandboxModel.RustRefinement
