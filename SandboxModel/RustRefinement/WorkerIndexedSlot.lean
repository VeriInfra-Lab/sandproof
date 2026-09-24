import SandboxModel.RustRefinement.WorkerSlotUpdate

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance workerIndexedSlotOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerIndexedSlotLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerIndexedSlotEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem generated_workerRegisterSlot_atIndex
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (indexRoom : StoreHeadroom rust.resource_indexes)
    (sid : SandboxControllerRust.ids.SandboxId)
    (index : SandboxControllerRust.model.ResourceIndex)
    (indexEval : SandboxControllerRust.model.World.resource_index rust sid = ok index)
    (indexSame : resourceIndex index = model.resourceIndexes (sandboxId sid))
    (slotRoom : VecHeadroom index.slots)
    (vid : SandboxControllerRust.ids.NetSlotId) :
    ∃ indices, SandboxControllerRust.worker.register_slot rust sid vid = ok { rust with resource_indexes := indices } ∧
      WorkerWorldRep { rust with resource_indexes := indices } (registerSlot model (sandboxId sid) (netSlotId vid)) := by
  obtain ⟨values, valuesEval, valuesSame⟩ := WP.spec_imp_exists
    (generated_workerInsertUnique_refines SandboxControllerRust.ids.NetSlotId.Insts.CoreCloneClone (by intros; rfl)
      SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful index.slots vid slotRoom)
  let updated : SandboxControllerRust.model.ResourceIndex := { index with slots := values }
  obtain ⟨indices, putEval, indicesRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_defaultStoreRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => resourceIndex a = b)
      rust.resource_indexes model.resourceIndexes ResourceIndex.empty rep.resourceIndexes
      sid updated (resourceIndex updated) rfl indexRoom)
  refine ⟨indices, ?_, ?_⟩
  · simp [SandboxControllerRust.worker.register_slot, indexEval, valuesEval, putEval, updated]
  · have updatedSame : resourceIndex updated =
        { (model.resourceIndexes (sandboxId sid)) with slots := insertUnique (netSlotId vid) (model.resourceIndexes (sandboxId sid)).slots } := by
      rw [← indexSame]
      simp [updated, resourceIndex, valuesSame, map_insertUnique_of_injective netSlotId_injective]
    exact { rep with resourceIndexes := by simpa only [updatedSame, registerSlot, setResourceIndex, valueRelations] using indicesRep }

end SandboxModel.RustRefinement
