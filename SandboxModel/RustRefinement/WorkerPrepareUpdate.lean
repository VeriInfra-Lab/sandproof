import SandboxModel.RustRefinement.WorkerIndexedSlot

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance workerPrepareUpdateOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerPrepareUpdateLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerPrepareUpdateEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

/-- Sufficient finite capacity for three enqueues and two index writes.
The index bound is conservative: the second write replaces the same key. -/
structure WorkerPrepareCapacity (rust : SandboxControllerRust.model.World) : Prop where
  commands : EffectCapacity rust 3
  indexes : rust.resource_indexes.entries.val.length + 2 ≤ Usize.max

theorem generated_workerPrepareResources_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : WorkerPrepareCapacity rust)
    (sid : SandboxControllerRust.ids.SandboxId) (record : SandboxControllerRust.model.SandboxRecord)
    (vid : SandboxControllerRust.ids.VmId) (slotId : SandboxControllerRust.ids.NetSlotId) :
    ∃ next, SandboxControllerRust.worker.prepare_resources rust sid record vid slotId = ok next ∧
      WorkerWorldRep next (prepareResources model (sandboxId sid) (sandboxRecord record) (vmId vid) (netSlotId slotId)) := by
  obtain ⟨index, indexEval, indexSame, vmRoom, slotRoom, _⟩ := generated_workerResourceIndex_refines rust model rep room sid
  obtain ⟨values, valuesEval, valuesSame⟩ := WP.spec_imp_exists
    (generated_workerInsertUnique_refines SandboxControllerRust.ids.VmId.Insts.CoreCloneClone (by intros; rfl)
      SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId vmId_partialEq_lawful index.vms vid vmRoom)
  let updatedIndex : SandboxControllerRust.model.ResourceIndex := { index with vms := values }
  obtain ⟨indices, indexPut, indicesRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_defaultStoreRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => resourceIndex a = b)
      rust.resource_indexes model.resourceIndexes ResourceIndex.empty rep.resourceIndexes
      sid updatedIndex (resourceIndex updatedIndex) rfl room.resourceIndexes)
  have updatedSame : resourceIndex updatedIndex =
      { (model.resourceIndexes (sandboxId sid)) with vms := insertUnique (vmId vid) (model.resourceIndexes (sandboxId sid)).vms } := by
    rw [← indexSame]
    simp [updatedIndex, resourceIndex, valuesSame, map_insertUnique_of_injective vmId_injective]
  have indicesRoom : StoreHeadroom indices := by
    have bound := generated_put_length_bound SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful rust.resource_indexes indices sid updatedIndex room.resourceIndexes indexPut
    have limit := capacity.indexes
    unfold StoreHeadroom
    omega
  have indexFound : storeLookup indices sid = some updatedIndex := by
    obtain ⟨store, evaluated, entries⟩ := WP.spec_imp_exists
      (generated_put_refines SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
        sandboxId_partialEq_lawful rust.resource_indexes sid updatedIndex room.resourceIndexes)
    have same := Result.ok_injective (indexPut.symm.trans evaluated)
    subst store
    simp [storeLookup, entries, lookup_associationPut]
  let machine : SandboxControllerRust.model.Vm := ⟨record.owner, sid, record.generation, .Prepared⟩
  obtain ⟨vms, vmPut, vmsRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.VmId.Insts.CoreCmpPartialEqVmId
      vmId_partialEq_lawful vmId vmId_injective (fun a b => vm a = b)
      rust.vms model.vms rep.vms vid machine (vm machine) rfl room.vms)
  let withVm := { rust with resource_indexes := indices, vms := vms }
  let withVmModel := setVm model (vmId vid) (vm machine)
  have vmRep : WorkerWorldRep withVm withVmModel := { rep with
    resourceIndexes := by simpa only [withVm, withVmModel, updatedSame, setVm, registerVm, setResourceIndex, machine, vm, valueRelations] using indicesRep
    vms := vmsRep }
  have vmEval : SandboxControllerRust.worker.put_vm rust vid machine = ok withVm := by
    simp [SandboxControllerRust.worker.put_vm, SandboxControllerRust.worker.register_vm,
      machine, indexEval, valuesEval, indexPut, updatedIndex, vmPut, withVm]
  have nextIndexEval : SandboxControllerRust.model.World.resource_index withVm sid = ok updatedIndex := by
    have get := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful indices sid
    simp [SandboxControllerRust.model.World.resource_index, withVm, get, indexFound,
      core.option.OptionShared0T.cloned, core.option.Option.unwrap_or_default]
  have nextIndexSame : resourceIndex updatedIndex = withVmModel.resourceIndexes (sandboxId sid) := by
    simp [withVmModel, setVm, machine, vm, updatedSame]
  obtain ⟨bothIndices, registerEval, registerRep⟩ := generated_workerRegisterSlot_atIndex withVm withVmModel vmRep
    indicesRoom sid updatedIndex nextIndexEval nextIndexSame slotRoom slotId
  let slot : SandboxControllerRust.model.NetSlot := ⟨record.owner, sid, record.generation, vid, record.policy, false, false, false⟩
  obtain ⟨slots, slotPut, slotsRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId
      netSlotId_partialEq_lawful netSlotId netSlotId_injective (fun a b => netSlot a = b)
      rust.net_slots model.netSlots rep.netSlots slotId slot (netSlot slot) rfl room.netSlots)
  let withSlot := { withVm with resource_indexes := bothIndices, net_slots := slots }
  let withSlotModel := setNetSlot withVmModel (netSlotId slotId) (netSlot slot)
  have slotRep : WorkerWorldRep withSlot withSlotModel := { registerRep with netSlots := slotsRep }
  have slotEval : SandboxControllerRust.worker.put_slot withVm slotId slot = ok withSlot := by
    simp [SandboxControllerRust.worker.put_slot, slot, registerEval, slotPut, withSlot, withVm]
  let updated := { record with current_vm := some vid, net_slot := some slotId }
  obtain ⟨sandboxes, sandboxPut, sandboxesRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => sandboxRecord a = b)
      rust.sandboxes model.sandboxes rep.sandboxes sid updated (sandboxRecord updated) rfl room.sandboxes)
  let prepared := { withSlot with sandboxes := sandboxes }
  let preparedModel := setSandbox withSlotModel (sandboxId sid) (sandboxRecord updated)
  have preparedRep : WorkerWorldRep prepared preparedModel := { slotRep with sandboxes := sandboxesRep }
  have commandRoom : EffectCapacity prepared 3 := by
    exact ⟨capacity.commands.effects, capacity.commands.logical, capacity.commands.keys, capacity.commands.inflight⟩
  obtain ⟨configured, configureEval, configureRep, room2⟩ := generated_enqueue_refines prepared preparedModel preparedRep commandRoom
    (.ConfigureNetwork sid record.epoch record.generation vid slotId record.policy)
  obtain ⟨activated, activateEval, activateRep, room1⟩ := generated_enqueue_refines configured _ configureRep room2
    (.ActivateVm sid record.epoch record.generation vid)
  obtain ⟨next, policyEval, nextRep, _⟩ := generated_enqueue_refines activated _ activateRep room1
    (.ApplyFilePolicy sid record.epoch record.generation record.workspace record.file_policy)
  refine ⟨next, ?_, ?_⟩
  · have vmEval' := vmEval
    have slotEval' := slotEval
    dsimp only [machine] at vmEval'
    dsimp only [slot] at slotEval'
    simp only [SandboxControllerRust.worker.prepare_resources, world_clone_eq, network_policy_clone_eq,
      sandboxRecord_clone_eq, bind_tc_ok, vmEval', slotEval']
    change (do
      let sandboxes' ← SandboxControllerRust.store.Store.put SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId rust.sandboxes sid updated
      let configured' ← SandboxControllerRust.transition.enqueue { withSlot with sandboxes := sandboxes' } (.ConfigureNetwork sid record.epoch record.generation vid slotId record.policy)
      let activated' ← SandboxControllerRust.transition.enqueue configured' (.ActivateVm sid record.epoch record.generation vid)
      SandboxControllerRust.transition.enqueue activated' (.ApplyFilePolicy sid record.epoch record.generation record.workspace record.file_policy)) = ok next
    simp only [prepared] at configureEval
    simp only [sandboxPut, bind_tc_ok, configureEval, activateEval, policyEval]
  · exact nextRep

end SandboxModel.RustRefinement
