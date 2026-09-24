import SandboxModel.RustRefinement.SchedulerComplete

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance workerAdmissionOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance workerAdmissionLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance workerAdmissionEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

theorem generated_workerCurrent_eq
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (id : SandboxControllerRust.model.EffectId)
    (head : currentEffectId model (effectId id).command = effectId id) :
    SandboxControllerRust.worker.current_effect_id rust id.logical.command = ok id := by
  obtain ⟨current, evaluated, same⟩ := WP.spec_imp_exists (generated_currentEffectId_refines rep id.logical.command)
  have equal : current = id := effectId_injective (same.trans head)
  simpa only [equal] using evaluated

theorem generated_workerQueued_eq
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (id : SandboxControllerRust.model.EffectId) (queued : model.inflight (effectId id)) :
    SandboxControllerRust.worker.contains SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
      (alloc.vec.Vec.deref rust.inflight) id = ok true := by
  obtain ⟨found, evaluated, same⟩ := WP.spec_imp_exists
    (generated_workerContainsEffect_refines (alloc.vec.Vec.deref rust.inflight) id)
  have member := (rep.inflight id).mpr queued
  have equal : found = true := by simpa [alloc.vec.Vec.deref, member] using same
  simpa only [equal] using evaluated

theorem generated_workerId_ne_self (id : SandboxControllerRust.model.EffectId) :
    core.cmp.PartialEq.ne.trait_default SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId id id =
      ok false := by
  unfold core.cmp.PartialEq.ne.trait_default core.cmp.PartialEq.ne.default
  rw [effectId_partialEq_lawful id id]
  simp

theorem generated_workerAwaiting_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (id : SandboxControllerRust.model.EffectId) :
    SandboxControllerRust.worker.effect_awaiting (storeLookup rust.effects id) =
      ok (decide (model.effects (effectId id) = some .dispatched ∨
        model.effects (effectId id) = some .outcomeUnknown)) := by
  have lookup := storeRep_lookup_map rep.effects id
  change model.effects (effectId id) = _ at lookup
  cases found : storeLookup rust.effects id with
  | none => simp [SandboxControllerRust.worker.effect_awaiting, lookup, found]
  | some status => cases status <;> simp [SandboxControllerRust.worker.effect_awaiting, lookup, found, effectStatus]

theorem handleWorker_notAwaiting_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : WorkerWorldRep rust model)
    (valid : ResourceIndexValid model) (id : SandboxControllerRust.model.EffectId)
    (head : currentEffectId model (effectId id).command = effectId id)
    (queued : model.inflight (effectId id))
    (blocked : ¬ (model.effects (effectId id) = some .dispatched ∨ model.effects (effectId id) = some .outcomeUnknown)) :
    SandboxControllerRust.worker.handle_worker rust id = ok (.Err .NotAwaiting) ∧
      WorkerResultRep model (.Err .NotAwaiting) (Controller.handleWorker model valid ⟨effectId id, queued⟩) := by
  have current := generated_workerCurrent_eq rust model rep id head
  have contains := generated_workerQueued_eq rust model rep id queued
  have get := generated_get_eq SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
    effectId_partialEq_lawful rust.effects id
  have awaiting := generated_workerAwaiting_refines rust model rep id
  constructor
  · simp [SandboxControllerRust.worker.handle_worker, current, contains, get, awaiting,
      generated_workerId_ne_self, blocked]
  · unfold Controller.handleWorker
    rw [dif_pos head, dif_neg blocked]
    rfl

end SandboxModel.RustRefinement
