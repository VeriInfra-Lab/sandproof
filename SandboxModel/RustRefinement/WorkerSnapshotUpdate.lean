import SandboxModel.RustRefinement.WorkerVmUpdate

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance workerSnapshotUpdateOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerSnapshotUpdateLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerSnapshotUpdateEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem generated_workerRegisterSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (sid : SandboxControllerRust.ids.SandboxId)
    (vid : SandboxControllerRust.ids.SnapshotId) :
    ∃ indices, SandboxControllerRust.worker.register_snapshot rust sid vid = ok { rust with resource_indexes := indices } ∧
      WorkerWorldRep { rust with resource_indexes := indices } (registerSnapshot model (sandboxId sid) (snapshotId vid)) := by
  obtain ⟨index, indexEval, indexSame, _, _, snapshotRoom⟩ := generated_workerResourceIndex_refines rust model rep room sid
  obtain ⟨values, valuesEval, valuesSame⟩ := WP.spec_imp_exists
    (generated_workerInsertUnique_refines SandboxControllerRust.ids.SnapshotId.Insts.CoreCloneClone (by intros; rfl)
      SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId snapshotId_partialEq_lawful index.snapshots vid snapshotRoom)
  let updated : SandboxControllerRust.model.ResourceIndex := { index with snapshots := values }
  obtain ⟨indices, putEval, indicesRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_defaultStoreRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => resourceIndex a = b)
      rust.resource_indexes model.resourceIndexes ResourceIndex.empty rep.resourceIndexes
      sid updated (resourceIndex updated) rfl room.resourceIndexes)
  refine ⟨indices, ?_, ?_⟩
  · simp [SandboxControllerRust.worker.register_snapshot, indexEval, valuesEval, putEval, updated]
  · have updatedSame : resourceIndex updated =
        { (model.resourceIndexes (sandboxId sid)) with snapshots := insertUnique (snapshotId vid) (model.resourceIndexes (sandboxId sid)).snapshots } := by
      rw [← indexSame]
      simp [updated, resourceIndex, valuesSame, map_insertUnique_of_injective snapshotId_injective]
    exact { rep with resourceIndexes := by simpa only [updatedSame, registerSnapshot, setResourceIndex, valueRelations] using indicesRep }

theorem generated_workerPutSnapshot_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (vid : SandboxControllerRust.ids.SnapshotId)
    (value : SandboxControllerRust.model.Snapshot) :
    ∃ indices snapshots, SandboxControllerRust.worker.put_snapshot rust vid value = ok { rust with resource_indexes := indices, snapshots := snapshots } ∧
      WorkerWorldRep { rust with resource_indexes := indices, snapshots := snapshots } (setSnapshot model (snapshotId vid) (snapshot value)) := by
  obtain ⟨indices, registerEval, registerRep⟩ := generated_workerRegisterSnapshot_refines rust model rep room value.sandbox vid
  obtain ⟨snapshots, putEval, snapshotsRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.SnapshotId.Insts.CoreCmpPartialEqSnapshotId
      snapshotId_partialEq_lawful snapshotId snapshotId_injective (fun a b => snapshot a = b)
      rust.snapshots model.snapshots rep.snapshots vid value (snapshot value) rfl room.snapshots)
  refine ⟨indices, snapshots, ?_, ?_⟩
  · simp [SandboxControllerRust.worker.put_snapshot, registerEval, putEval]
  · exact { registerRep with snapshots := snapshotsRep }

end SandboxModel.RustRefinement
