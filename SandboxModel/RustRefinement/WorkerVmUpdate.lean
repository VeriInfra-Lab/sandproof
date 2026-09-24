import SandboxModel.RustRefinement.WorkerResourceStore

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance workerVmUpdateOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerVmUpdateLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerVmUpdateEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem generated_workerResourceIndex_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (sid : SandboxControllerRust.ids.SandboxId) :
    ∃ index, SandboxControllerRust.model.World.resource_index rust sid = ok index ∧
      resourceIndex index = model.resourceIndexes (sandboxId sid) ∧
      VecHeadroom index.vms ∧ VecHeadroom index.slots ∧ VecHeadroom index.snapshots := by
  obtain ⟨index, evaluated, mapped⟩ := generated_resourceIndex_refines rust model rep sid
  refine ⟨index, evaluated, mapped, ?_⟩
  have get := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.resource_indexes sid
  cases found : storeLookup rust.resource_indexes sid with
  | none =>
    simp [SandboxControllerRust.model.World.resource_index, get, found,
      core.option.OptionShared0T.cloned, core.option.Option.unwrap_or_default,
      SandboxControllerRust.model.ResourceIndex.Insts.CoreDefaultDefault.default,
      alloc.vec.Vec.Insts.CoreDefaultDefault.default] at evaluated
    subst index
    simp [VecHeadroom, alloc.vec.Vec.new]
    scalar_tac
  | some stored =>
    simp [SandboxControllerRust.model.World.resource_index, get, found,
      core.option.OptionShared0T.cloned, core.option.Option.unwrap_or_default] at evaluated
    subst index
    have member := storeLookup_some_mem found
    exact ⟨room.resourceIndexVms (sid, stored) member,
      room.resourceIndexSlots (sid, stored) member, room.resourceIndexSnapshots (sid, stored) member⟩

theorem generated_workerRegisterVm_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (sid : SandboxControllerRust.ids.SandboxId)
    (vid : SandboxControllerRust.ids.VmId) :
    ∃ indices, SandboxControllerRust.worker.register_vm rust sid vid = ok { rust with resource_indexes := indices } ∧
      WorkerWorldRep { rust with resource_indexes := indices } (registerVm model (sandboxId sid) (vmId vid)) := by
  obtain ⟨index, indexEval, indexSame, vmRoom, _, _⟩ := generated_workerResourceIndex_refines rust model rep room sid
  obtain ⟨values, valuesEval, valuesSame⟩ := WP.spec_imp_exists
    (generated_workerInsertUnique_refines SandboxControllerRust.ids.VmId.Insts.CoreCloneClone (by intros; rfl)
      SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful index.vms vid vmRoom)
  let updated : SandboxControllerRust.model.ResourceIndex := { index with vms := values }
  obtain ⟨indices, putEval, indicesRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_defaultStoreRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => resourceIndex a = b)
      rust.resource_indexes model.resourceIndexes ResourceIndex.empty rep.resourceIndexes
      sid updated (resourceIndex updated) rfl room.resourceIndexes)
  refine ⟨indices, ?_, ?_⟩
  · simp [SandboxControllerRust.worker.register_vm, indexEval, valuesEval, putEval, updated]
  · have updatedSame : resourceIndex updated =
        { (model.resourceIndexes (sandboxId sid)) with vms := insertUnique (vmId vid) (model.resourceIndexes (sandboxId sid)).vms } := by
      rw [← indexSame]
      simp [updated, resourceIndex, valuesSame, map_insertUnique_of_injective vmId_injective]
    exact { rep with resourceIndexes := by simpa only [updatedSame, registerVm, setResourceIndex, valueRelations] using indicesRep }

theorem generated_workerPutVm_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (vid : SandboxControllerRust.ids.VmId)
    (value : SandboxControllerRust.model.Vm) :
    ∃ next, SandboxControllerRust.worker.put_vm rust vid value = ok next ∧
      WorkerWorldRep next (setVm model (vmId vid) (vm value)) := by
  obtain ⟨indices, registerEval, registerRep⟩ := generated_workerRegisterVm_refines rust model rep room value.sandbox vid
  obtain ⟨vms, putEval, vmsRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId
      vmId_partialEq_lawful vmId vmId_injective (fun a b => vm a = b)
      rust.vms model.vms rep.vms vid value (vm value) rfl room.vms)
  refine ⟨{ rust with resource_indexes := indices, vms := vms }, ?_, ?_⟩
  · simp [SandboxControllerRust.worker.put_vm, registerEval, putEval]
  · exact { registerRep with vms := vmsRep }

end SandboxModel.RustRefinement
