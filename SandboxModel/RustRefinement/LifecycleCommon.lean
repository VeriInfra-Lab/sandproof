import SandboxModel.RustRefinement.PublicationChecks
import SandboxModel.RustRefinement.PublicationUpdate

/-! Shared, actual extracted state updates used by lifecycle requests. -/

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

local instance lifecycleOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance lifecycleLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance lifecycleEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

/-- Space for a bounded number of new commands during one atomic reducer.
This is a machine-capacity condition, not a premise about semantic progress. -/
structure EffectCapacity (rust : SandboxControllerRust.model.World) (count : Nat) : Prop where
  effects : rust.effects.entries.val.length + count ≤ Usize.max
  logical : rust.logical_effects.entries.val.length + count ≤ Usize.max
  keys : rust.effect_keys.val.length + count ≤ Usize.max
  inflight : rust.inflight.val.length + count ≤ Usize.max

theorem EffectCapacity.mono (room : EffectCapacity rust count) (bound : smaller ≤ count) :
    EffectCapacity rust smaller := by
  rcases room with ⟨a,b,c,d⟩
  constructor <;> omega

theorem generated_put_length_bound [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (before after : SandboxControllerRust.store.Store K V) (key : K) (value : V)
    (room : StoreHeadroom before)
    (evaluated : SandboxControllerRust.store.Store.put partialEq before key value = ok after) :
    after.entries.val.length ≤ before.entries.val.length + 1 := by
  obtain ⟨next, nextEval, nextEntries⟩ := WP.spec_imp_exists
    (generated_put_refines partialEq lawful before key value room)
  have same := Result.ok_injective (evaluated.symm.trans nextEval)
  subst next
  rw [nextEntries]
  exact length_associationPut_le key value before.entries.val

@[simp] theorem operationInput_clone_eq (input : SandboxControllerRust.model.OperationInput) :
    SandboxControllerRust.model.OperationInput.Insts.CoreCloneClone.clone input = ok input := by
  cases input <;> simp [SandboxControllerRust.model.OperationInput.Insts.CoreCloneClone.clone,
    SandboxControllerRust.ids.FsId.Insts.CoreCloneClone.clone,
    SandboxControllerRust.model.FilePolicyRef.Insts.CoreCloneClone.clone,
    SandboxControllerRust.model.RestoreSource.Insts.CoreCloneClone.clone, lift]

theorem generated_initialOperationPlan_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (sid : SandboxControllerRust.ids.SandboxId) (op : SandboxControllerRust.model.Operation)
    (input : SandboxControllerRust.model.OperationInput) :
    ∃ plan, SandboxControllerRust.transition.initial_operation_plan rust sid op input = ok plan ∧
      operationPlan plan = initialOperationPlan model (sandboxId sid) (operation op) (operationInput input) := by
  obtain ⟨index, indexEval, indexSame⟩ := generated_resourceIndex_refines rust model rep sid
  have get := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have lookup := storeRep_lookup_map rep.sandboxes sid
  have kind : SandboxControllerRust.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq op.kind .Kill =
      ok (decide ((operationKind op.kind) = .kill)) := by
    cases op.kind <;> simp [SandboxControllerRust.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,
      SandboxControllerRust.model.OperationKind.read_discriminant, operationKind]
  cases found : storeLookup rust.sandboxes sid <;> cases input <;>
    simp [SandboxControllerRust.transition.initial_operation_plan, indexEval, get, kind,
      SandboxControllerRust.transition.operation_key,
      operationPlan, initialOperationPlan, operation, operationInput, operationKey, SandboxModel.operationKey,
      ← indexSame, resourceIndex, lookup, sandboxRecord, defaultRetryLimit,
      SandboxControllerRust.transition.DEFAULT_RETRY_LIMIT, u64ToNat, found]

theorem generated_storeNewOperation_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (operationRoom : StoreHeadroom rust.operations) (planRoom : StoreHeadroom rust.operation_plans)
    (capacity : EffectCapacity rust count)
    (sid : SandboxControllerRust.ids.SandboxId) (op : SandboxControllerRust.model.Operation)
    (input : SandboxControllerRust.model.OperationInput) :
    ∃ next, SandboxControllerRust.transition.store_new_operation rust sid op input = ok next ∧
      EffectWorldRep next (storeNewOperation model (sandboxId sid) (operation op) (operationInput input)) ∧
      EffectCapacity next count := by
  obtain ⟨plan, planEval, planSame⟩ := generated_initialOperationPlan_refines rust model rep sid op input
  let key : SandboxControllerRust.model.OperationKey := ⟨sid, op.id⟩
  let ledger : SandboxControllerRust.model.OperationLedgerRecord := ⟨sid, op, input, none⟩
  obtain ⟨operations, operationsEval, operationsRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operationKey_partialEq_lawful operationKey operationKey_injective
      (fun a b => operationLedger a = b) rust.operations model.operations rep.operations
      key ledger (operationLedger ledger) rfl operationRoom)
  obtain ⟨plans, plansEval, plansRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operationKey_partialEq_lawful operationKey operationKey_injective
      (fun a b => operationPlan a = b) rust.operation_plans model.operationPlans rep.operationPlans
      key plan (initialOperationPlan model (sandboxId sid) (operation op) (operationInput input)) planSame planRoom)
  refine ⟨{ rust with operations := operations, operation_plans := plans }, ?_, ?_,
    ⟨capacity.effects, capacity.logical, capacity.keys, capacity.inflight⟩⟩
  · dsimp [key, ledger] at operationsEval plansEval
    simp [SandboxControllerRust.transition.store_new_operation, SandboxControllerRust.transition.operation_key,
      planEval, operationsEval, plansEval]
  · exact { rep with operations := operationsRep, operationPlans := plansRep }

end SandboxModel.RustRefinement
