import SandboxModel.RustRefinement.LifecycleStart
import SandboxModel.RustRefinement.RpcTimeout

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
local instance lifecycleTerminalOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance lifecycleTerminalLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance lifecycleTerminalEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

@[simp] theorem operation_id_field (op : SandboxControllerRust.model.Operation) :
    (operation op).id = operationId op.id := rfl
@[simp] theorem operation_status_field (op : SandboxControllerRust.model.Operation) :
    (operation op).status = operationStatus op.status := rfl
theorem operation_updateStatus (op : SandboxControllerRust.model.Operation) (status : SandboxControllerRust.model.OperationStatus) :
    operation { op with status := status } = { operation op with status := operationStatus status } := rfl

theorem updateOperation_present (model : World) (sid : SandboxId) (op : Operation)
    (ledger : OperationLedgerRecord) (result : Option OperationResult)
    (found : model.operations (SandboxModel.operationKey sid op.id) = some ledger) :
    updateOperation model sid op result =
      { model with
        operations := put model.operations (SandboxModel.operationKey sid op.id)
          { ledger with operation := op, result := result } } := by
  unfold updateOperation
  dsimp only
  congr 1
  funext candidate
  simp [put, found]

theorem generated_beginCompensation_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 1)
    (sid : SandboxControllerRust.ids.SandboxId) (record : SandboxControllerRust.model.SandboxRecord)
    (op : SandboxControllerRust.model.Operation) (failure : SandboxControllerRust.model.FailureKind)
    (ledger : SandboxControllerRust.model.OperationLedgerRecord)
    (found : storeLookup rust.operations ⟨sid, op.id⟩ = some ledger) :
    ∃ next, SandboxControllerRust.transition.begin_compensation rust sid record op failure = ok next ∧
      EffectWorldRep next (beginCompensation model (sandboxId sid) (sandboxRecord record) (operation op) (failureKind failure)) := by
  let compensating := { op with status := SandboxControllerRust.model.OperationStatus.Compensating failure }
  let updated := { record with
    phase := SandboxControllerRust.model.Phase.Revoking
    terminal := true
    desired := SandboxControllerRust.model.Authority.Closed
    effective := SandboxControllerRust.model.Authority.Closed
    current_operation := some compensating }
  let updatedLedger := { ledger with operation := compensating, result := none }
  obtain ⟨sandboxes, sandboxEval, sandboxRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => sandboxRecord a = b)
      rust.sandboxes model.sandboxes rep.sandboxes sid updated (sandboxRecord updated) rfl room.sandboxes)
  obtain ⟨operations, operationEval, operationRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operationKey_partialEq_lawful operationKey operationKey_injective (fun a b => operationLedger a = b)
      rust.operations model.operations rep.operations ⟨sid, op.id⟩ updatedLedger (operationLedger updatedLedger) rfl room.operations)
  let base := { rust with sandboxes := sandboxes, operations := operations }
  let baseModel := updateOperation (setSandbox model (sandboxId sid) (sandboxRecord updated))
    (sandboxId sid) (operation compensating) none
  have lookup := storeRep_lookup_map rep.operations (SandboxControllerRust.model.OperationKey.mk sid op.id)
  have modelFound : model.operations (SandboxModel.operationKey (sandboxId sid) (operation compensating).id) =
      some (operationLedger ledger) := by
        simpa [found, compensating, operation, valueRelations, operationKey, SandboxModel.operationKey] using lookup
  have baseRep : EffectWorldRep base baseModel := by
    dsimp [baseModel]
    rw [updateOperation_present (setSandbox model (sandboxId sid) (sandboxRecord updated))
      (sandboxId sid) (operation compensating) (operationLedger ledger) none modelFound]
    exact { rep with sandboxes := sandboxRep, operations := operationRep }
  have baseCapacity : EffectCapacity base 1 := ⟨capacity.effects, capacity.logical, capacity.keys, capacity.inflight⟩
  obtain ⟨next, nextEval, nextRep, _⟩ := generated_enqueue_refines base baseModel baseRep baseCapacity (.InstallFence sid record.epoch true)
  refine ⟨next, ?_, ?_⟩
  · have get := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operationKey_partialEq_lawful rust.operations ⟨sid, op.id⟩
    dsimp [updated, compensating] at sandboxEval
    dsimp [updatedLedger, compensating] at operationEval
    dsimp [base] at nextEval
    simp [SandboxControllerRust.transition.begin_compensation, world_clone_eq,
      sandboxEval, SandboxControllerRust.transition.operation_key, get, found, operationEval, nextEval]
  · simpa [beginCompensation, baseModel, compensating, updated, sandboxRecord,
      operation, operationStatus, command, phase, authority] using nextRep

theorem generated_supersedeCurrent_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust count)
    (sid : SandboxControllerRust.ids.SandboxId) (current : Option SandboxControllerRust.model.Operation) :
    ∃ next, SandboxControllerRust.transition.supersede_current_operation rust sid current = ok next ∧
      EffectWorldRep next (supersedeCurrentOperation model (sandboxId sid) (current.map operation)) ∧
      HandlerMachineHeadroom next ∧ EffectCapacity next count := by
  cases current with
  | none => exact ⟨rust, by simp [SandboxControllerRust.transition.supersede_current_operation], rep, room, capacity⟩
  | some op =>
    cases status : op.status <;>
      try { exact ⟨rust, by simp [SandboxControllerRust.transition.supersede_current_operation, status],
        by simpa [supersedeCurrentOperation, operation, status, operationStatus] using rep, room, capacity⟩ }
    all_goals
      have reconstruct (st : SandboxControllerRust.model.OperationStatus) (same : op.status = st) :
          operation { op with status := st } = operation op := by rw [← same]
      have get := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
        operationKey_partialEq_lawful rust.operations ⟨sid, op.id⟩
      have lookup := storeRep_lookup_map rep.operations (SandboxControllerRust.model.OperationKey.mk sid op.id)
      change model.operations (SandboxModel.operationKey (sandboxId sid) (operationId op.id)) = _ at lookup
      cases found : storeLookup rust.operations ⟨sid, op.id⟩ with
      | none =>
        refine ⟨rust, ?_, ?_, room, capacity⟩
        · simp [SandboxControllerRust.transition.supersede_current_operation, status,
            SandboxControllerRust.transition.operation_key, get, found]
        · simpa [supersedeCurrentOperation, operation, status, operationStatus, lookup, found] using rep
      | some ledger =>
        by_cases equal : operation ledger.operation = operation op
        · cases result : ledger.result with
          | some prior =>
            refine ⟨rust, ?_, ?_, room, capacity⟩
            · simp [SandboxControllerRust.transition.supersede_current_operation, status,
                SandboxControllerRust.transition.operation_key, get, found, generated_operation_ne, result]
            · simpa [supersedeCurrentOperation, operation, status, operationStatus, lookup, found, operationLedger, result] using rep
          | none =>
            let updated := { ledger with
              operation := { ledger.operation with status := SandboxControllerRust.model.OperationStatus.Failed .Cancelled }
              result := some (SandboxControllerRust.model.OperationResult.Failed .Cancelled) }
            obtain ⟨operations, operationEval, operationRep, sameLength⟩ := WP.spec_imp_exists
              (generated_put_existing_preserves_storeRep SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
                operationKey_partialEq_lawful operationKey operationKey_injective (fun a b => operationLedger a = b)
                rust.operations model.operations rep.operations ⟨sid, op.id⟩ ledger updated (operationLedger updated)
                rfl found room.operations)
            have modelFound : model.operations (SandboxModel.operationKey (sandboxId sid) (operationId op.id)) =
                some (operationLedger ledger) := by simpa [found] using lookup
            refine ⟨{ rust with operations := operations }, ?_, ?_, ?_,
              ⟨capacity.effects, capacity.logical, capacity.keys, capacity.inflight⟩⟩
            · dsimp [updated] at operationEval
              simp [SandboxControllerRust.transition.supersede_current_operation, status,
                SandboxControllerRust.transition.operation_key, get, found, generated_operation_ne,
                reconstruct _ status, equal, result, operationEval]
            · have representedNext : EffectWorldRep { rust with operations := operations }
                  { model with operations := put model.operations (operationKey ⟨sid, op.id⟩) (operationLedger updated) } :=
                { rep with operations := operationRep }
              simp only [Option.map_some, supersedeCurrentOperation, operation_status_field, status, operationStatus,
                operation_id_field, modelFound, operationLedger, result, Option.map_none, equal, if_true]
              rw [updateOperation_present _ _ _ (operationLedger ledger) _ modelFound]
              simpa only [updated, operationLedger, operation_updateStatus, equal, operationStatus, operationResult, failureKind,
                Option.map_some, operation_id_field,
                operationKey, SandboxModel.operationKey, equal] using representedNext
            · exact { room with
                operations := by unfold StoreHeadroom; rw [sameLength]; exact room.operations
                cursors := ⟨room.cursors.vm, room.cursors.slot, room.cursors.snapshot, room.cursors.manifest⟩ }
        · refine ⟨rust, ?_, ?_, room, capacity⟩
          · simp [SandboxControllerRust.transition.supersede_current_operation, status,
              SandboxControllerRust.transition.operation_key, get, found, generated_operation_ne,
              reconstruct _ status, equal]
          · cases result : ledger.result <;>
              simpa only [Option.map_some, supersedeCurrentOperation, operation_status_field, status, operationStatus,
                operation_id_field, lookup, found, Option.map_some, Option.map_none, operationLedger, result, equal, if_false] using rep

theorem generated_withoutResume_refines (value : SandboxControllerRust.model.Authority) :
    ∃ next, SandboxControllerRust.model.Authority.without_resume value = ok next ∧
      authority next = (authority value).withoutResume := by
  cases value <;> simp [SandboxControllerRust.model.Authority.without_resume, authority, Authority.withoutResume]

theorem generated_beginKill_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 1)
    (sid : SandboxControllerRust.ids.SandboxId) (record : SandboxControllerRust.model.SandboxRecord)
    (oid : SandboxControllerRust.ids.OperationId) (epoch : U64)
    (epochNext : u64ToNat epoch = u64ToNat record.epoch + 1) :
    ∃ next, SandboxControllerRust.transition.begin_kill rust sid record oid epoch = ok next ∧
      EffectWorldRep next (beginKill model (sandboxId sid) (sandboxRecord record) (operationId oid)) := by
  obtain ⟨superseded, supersededEval, supersededRep, supersededRoom, supersededCapacity⟩ :=
    generated_supersedeCurrent_refines rust model rep room capacity sid record.current_operation
  obtain ⟨effective, effectiveEval, effectiveSame⟩ := generated_withoutResume_refines record.effective
  let op : SandboxControllerRust.model.Operation := ⟨oid, epoch, .Kill, .Pending⟩
  let updated := { record with
    phase := SandboxControllerRust.model.Phase.Revoking
    terminal := true
    epoch := epoch
    desired := SandboxControllerRust.model.Authority.Closed
    effective := effective
    current_operation := some op }
  let supersededModel := supersedeCurrentOperation model (sandboxId sid) (record.current_operation.map operation)
  obtain ⟨sandboxes, sandboxEval, sandboxRep⟩ := WP.spec_imp_exists
    (generated_put_preserves_storeRep SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective (fun a b => sandboxRecord a = b)
      superseded.sandboxes supersededModel.sandboxes supersededRep.sandboxes sid updated (sandboxRecord updated) rfl supersededRoom.sandboxes)
  let base := { superseded with sandboxes := sandboxes }
  let baseModel := setSandbox supersededModel (sandboxId sid) (sandboxRecord updated)
  have baseRep : EffectWorldRep base baseModel := { supersededRep with sandboxes := sandboxRep }
  have baseCapacity : EffectCapacity base 1 :=
    ⟨supersededCapacity.effects, supersededCapacity.logical, supersededCapacity.keys, supersededCapacity.inflight⟩
  obtain ⟨stored, storedEval, storedRep, storedCapacity⟩ := generated_storeNewOperation_refines
    base baseModel baseRep supersededRoom.operations supersededRoom.operationPlans baseCapacity sid op (.Kill record.generation)
  obtain ⟨next, nextEval, nextRep, _⟩ := generated_enqueue_refines stored _ storedRep storedCapacity (.InstallFence sid epoch true)
  refine ⟨next, ?_, ?_⟩
  · dsimp [updated, op] at sandboxEval
    dsimp [base, op] at storedEval
    simp [SandboxControllerRust.transition.begin_kill, world_clone_eq, supersededEval,
      effectiveEval, sandboxEval, storedEval, nextEval]
  · simp only [authority] at effectiveSame
    simpa [beginKill, baseModel, supersededModel, updated, op, sandboxRecord,
      operation, operationKind, operationStatus, operationInput, command, phase, authority,
      epochNext, effectiveSame] using nextRep

end SandboxModel.RustRefinement
