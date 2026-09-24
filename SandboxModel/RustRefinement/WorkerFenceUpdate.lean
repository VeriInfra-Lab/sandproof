import SandboxModel.RustRefinement.WorkerIndexedSlot

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance workerFenceUpdateOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerFenceUpdateLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerFenceUpdateEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem generated_workerFenceCanAdvance_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (epoch : U64) (terminal : Bool) :
    SandboxControllerRust.worker.fence_can_advance (storeLookup rust.worker_fences sid) epoch terminal =
      ok (decide (fenceCanAdvance (model.workerFences (sandboxId sid)) (u64ToNat epoch) terminal)) := by
  have lookup := storeRep_lookup_map rep.workerFences sid
  change model.workerFences (sandboxId sid) = _ at lookup
  rw [lookup]
  cases found : storeLookup rust.worker_fences sid with
  | none => rfl
  | some fence =>
    by_cases bound : fence.epoch.val ≤ epoch.val <;> cases old : fence.terminal <;> cases terminal <;>
      simp [SandboxControllerRust.worker.fence_can_advance, fenceCanAdvance, workerFence, old, u64ToNat, bound]

theorem generated_workerRevocationPhase (value : SandboxControllerRust.model.Phase) :
    SandboxControllerRust.worker.revocation_phase value = ok (decide (phase value = .pausing ∨ phase value = .revoking)) := by
  cases value <;> rfl

theorem generated_workerStartFence_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (room : StoreHeadroom rust.worker_fences) (sid : SandboxControllerRust.ids.SandboxId)
    (record : SandboxControllerRust.model.SandboxRecord) :
    ∃ next, SandboxControllerRust.worker.apply_start_fence rust sid record = ok next ∧
      WorkerWorldRep next (applyStartFence model (sandboxId sid) (sandboxRecord record)) := by
  obtain ⟨fences, putEval, fencesRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => workerFence a = b)
      rust.worker_fences model.workerFences rep.workerFences sid ⟨record.epoch, false⟩
      ⟨u64ToNat record.epoch, false⟩ rfl room)
  exact ⟨{ rust with worker_fences := fences }, by simp [SandboxControllerRust.worker.apply_start_fence, putEval, world_clone_eq],
    { rep with workerFences := fencesRep }⟩

theorem generated_workerRevocationFence_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (sid : SandboxControllerRust.ids.SandboxId)
    (record : SandboxControllerRust.model.SandboxRecord) :
    ∃ next, SandboxControllerRust.worker.apply_revocation_fence rust sid record = ok next ∧
      WorkerWorldRep next (applyRevocationFence model (sandboxId sid) (sandboxRecord record)) := by
  obtain ⟨sandboxes, sandboxEval, sandboxesRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => sandboxRecord a = b)
      rust.sandboxes model.sandboxes rep.sandboxes sid { record with effective := .Closed }
      { (sandboxRecord record) with effective := .closed } rfl room.sandboxes)
  obtain ⟨fences, fenceEval, fencesRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => workerFence a = b)
      rust.worker_fences model.workerFences rep.workerFences sid ⟨record.epoch, record.terminal⟩
      ⟨u64ToNat record.epoch, record.terminal⟩ rfl room.workerFences)
  let base := { rust with sandboxes := sandboxes, worker_fences := fences }
  let baseModel := setFence (setSandbox model (sandboxId sid) { (sandboxRecord record) with effective := .closed })
    (sandboxId sid) ⟨u64ToNat record.epoch, record.terminal⟩
  have baseRep : WorkerWorldRep base baseModel := { rep with sandboxes := sandboxesRep, workerFences := fencesRep }
  unfold SandboxControllerRust.worker.apply_revocation_fence
  simp only [world_clone_eq, sandboxRecord_clone_eq, sandboxEval, fenceEval, bind_tc_ok]
  cases choice : record.net_slot with
  | none =>
    exact ⟨base, rfl, by simpa [applyRevocationFence, sandboxRecord, choice, baseModel] using baseRep⟩
  | some slotId =>
    have get := generated_get_eq SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful rust.net_slots slotId
    have lookup := storeRep_lookup_map rep.netSlots slotId
    change model.netSlots (netSlotId slotId) = _ at lookup
    simp only [get, bind_tc_ok]
    cases found : storeLookup rust.net_slots slotId with
    | none =>
      exact ⟨base, rfl, by simpa [applyRevocationFence, sandboxRecord, choice, lookup, found, baseModel] using baseRep⟩
    | some slot =>
      obtain ⟨index, indexEval, indexSame, _, slotRoom, _⟩ := generated_workerResourceIndex_refines rust model rep room slot.sandbox
      obtain ⟨indices, registerEval, registerRep⟩ := generated_workerRegisterSlot_atIndex base baseModel baseRep
        room.resourceIndexes slot.sandbox index indexEval indexSame slotRoom slotId
      obtain ⟨slots, putEval, slotsRep⟩ := WP.spec_imp_exists
        (generated_put_preserves_storeRep SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId
          netSlotId_partialEq_lawful netSlotId netSlotId_injective (fun a b => netSlot a = b)
          rust.net_slots model.netSlots rep.netSlots slotId { slot with active := false }
          { (netSlot slot) with active := false } rfl room.netSlots)
      have nextRep : WorkerWorldRep { base with resource_indexes := indices, net_slots := slots }
          (setNetSlot baseModel (netSlotId slotId) { (netSlot slot) with active := false }) :=
        { registerRep with netSlots := slotsRep }
      refine ⟨{ base with resource_indexes := indices, net_slots := slots }, ?_, ?_⟩
      · simpa only [netSlot_clone_eq, SandboxControllerRust.worker.put_slot, bind_tc_ok, base, registerEval, putEval]
      · simpa [applyRevocationFence, sandboxRecord, choice, lookup, found, baseModel] using nextRep

end SandboxModel.RustRefinement
