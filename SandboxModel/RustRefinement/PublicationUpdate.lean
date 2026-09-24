import SandboxModel.RustRefinement.EffectHandler

/-! Actual stable-publication mutations, separated from admission checks.
These theorems do not by themselves prove publication may be admitted. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance publicationUpdateOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance publicationUpdateLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance publicationUpdateEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

theorem generated_updateOperationResult_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (room : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId)
    (op : SandboxControllerRust.model.Operation)
    (result : SandboxControllerRust.model.OperationResult) :
    ∃ next, SandboxControllerRust.publication.update_operation_result rust sid op result = ok next ∧
      EffectWorldRep next (updateOperation model (sandboxId sid) (operation op) (some (operationResult result))) := by
  let key : SandboxControllerRust.model.OperationKey := ⟨sid, op.id⟩
  have getOperation := generated_get_eq
    SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operationKey_partialEq_lawful rust.operations key
  have agrees := represented.operations.lookupAgrees key
  change OptionRel (fun concrete abstract => operationLedger concrete = abstract)
    (storeLookup rust.operations key) (model.operations (operationKey key)) at agrees
  cases found : storeLookup rust.operations key with
  | none =>
      have modelFound : model.operations (operationKey key) = none := by
        rw [found] at agrees
        cases lookup : model.operations (operationKey key) <;> simp_all [OptionRel]
      have unchanged : updateOperation model (sandboxId sid) (operation op) (some (operationResult result)) = model := by
        unfold updateOperation
        dsimp only
        change { model with operations := _ } = { model with operations := model.operations }
        congr 1
        funext candidate
        change model.operations ⟨sandboxId sid, operationId op.id⟩ = none at modelFound
        simp only [SandboxModel.operationKey, operation, modelFound]
        split <;> simp_all
      refine ⟨rust, ?_, ?_⟩
      · dsimp [key] at getOperation found
        simp [SandboxControllerRust.publication.update_operation_result,
          SandboxControllerRust.transition.operation_key, getOperation, found]
      · rw [unchanged]; exact represented
  | some ledger =>
      have modelFound : model.operations (operationKey key) = some (operationLedger ledger) := by
        rw [found] at agrees
        cases lookup : model.operations (operationKey key) <;> simp_all [OptionRel]
      let updated := { ledger with operation := op, result := some result }
      obtain ⟨operations, putEval, operationsRep⟩ := Aeneas.Std.WP.spec_imp_exists
        (generated_put_preserves_storeRep
          SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey operationKey_partialEq_lawful
          operationKey operationKey_injective (fun concrete abstract => operationLedger concrete = abstract)
          rust.operations model.operations represented.operations key updated (operationLedger updated) rfl room)
      have modelNext : updateOperation model (sandboxId sid) (operation op) (some (operationResult result)) =
          { model with operations := put model.operations (operationKey key) (operationLedger updated) } := by
        unfold updateOperation
        dsimp only
        congr 1
        funext candidate
        change model.operations ⟨sandboxId sid, operationId op.id⟩ = some (operationLedger ledger) at modelFound
        simp [put, SandboxModel.operationKey, operationKey, operation, key, modelFound, updated, operationLedger]
      refine ⟨{ rust with operations := operations }, ?_, ?_⟩
      · dsimp [key, updated] at getOperation found putEval
        simp [SandboxControllerRust.publication.update_operation_result,
          SandboxControllerRust.transition.operation_key, getOperation, found, putEval]
      · rw [modelNext]
        exact { represented with operations := operationsRep }

theorem generated_publishDestroyed_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId)
    (record : SandboxControllerRust.model.SandboxRecord) :
    ∃ next, SandboxControllerRust.publication.publish_destroyed rust sid record = ok next ∧
      EffectWorldRep next (publishDestroyed model (sandboxId sid) (sandboxRecord record)) := by
  let updated := { record with
    phase := SandboxControllerRust.model.Phase.Destroyed
    terminal := true
    current_vm := none
    net_slot := none
    desired := SandboxControllerRust.model.Authority.Closed
    effective := SandboxControllerRust.model.Authority.Closed
    current_operation := record.current_operation.map
      (fun (op : SandboxControllerRust.model.Operation) => { op with status := .Succeeded }) }
  obtain ⟨sandboxes, putEval, sandboxRep⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_put_preserves_storeRep
      SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful
      sandboxId sandboxId_injective (fun concrete abstract => sandboxRecord concrete = abstract)
      rust.sandboxes model.sandboxes represented.sandboxes sid updated (sandboxRecord updated) rfl sandboxRoom)
  let intermediate := { rust with sandboxes := sandboxes }
  have intermediateRep : EffectWorldRep intermediate (setSandbox model (sandboxId sid) (sandboxRecord updated)) :=
    { represented with sandboxes := sandboxRep }
  cases current : record.current_operation with
  | none =>
      refine ⟨intermediate, ?_, ?_⟩
      · dsimp [updated] at putEval
        simp only [current, Option.map_none] at putEval
        simp [SandboxControllerRust.publication.publish_destroyed, world_clone_eq, current, putEval, intermediate]
      · simpa [publishDestroyed, updateCurrentOperation, sandboxRecord, current,
          updated, setSandbox, phase, authority] using intermediateRep
  | some op =>
      obtain ⟨next, nextEval, nextRep⟩ := generated_updateOperationResult_refines
        intermediate (setSandbox model (sandboxId sid) (sandboxRecord updated)) intermediateRep operationRoom
        sid { op with status := .Succeeded } .Destroyed
      refine ⟨next, ?_, ?_⟩
      · dsimp [updated] at putEval
        simp only [current, Option.map_some] at putEval
        simpa [SandboxControllerRust.publication.publish_destroyed, world_clone_eq,
          current, putEval, intermediate] using nextEval
      · simpa [publishDestroyed, updateCurrentOperation, updateOperation, sandboxRecord,
          current, updated, setSandbox, operation, operationStatus, operationResult, phase, authority] using nextRep

theorem generated_publishFailure_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId)
    (record : SandboxControllerRust.model.SandboxRecord)
    (failure : SandboxControllerRust.model.FailureKind) :
    ∃ next, SandboxControllerRust.publication.publish_failure rust sid record failure = ok next ∧
      EffectWorldRep next (publishFailure model (sandboxId sid) (sandboxRecord record) (failureKind failure)) := by
  let updated := { record with
    phase := SandboxControllerRust.model.Phase.Destroyed
    terminal := true
    current_vm := none
    net_slot := none
    desired := SandboxControllerRust.model.Authority.Closed
    effective := SandboxControllerRust.model.Authority.Closed
    current_operation := record.current_operation.map
      (fun (op : SandboxControllerRust.model.Operation) => { op with status := .Failed failure }) }
  obtain ⟨sandboxes, putEval, sandboxRep⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_put_preserves_storeRep
      SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful
      sandboxId sandboxId_injective (fun concrete abstract => sandboxRecord concrete = abstract)
      rust.sandboxes model.sandboxes represented.sandboxes sid updated (sandboxRecord updated) rfl sandboxRoom)
  let intermediate := { rust with sandboxes := sandboxes }
  have intermediateRep : EffectWorldRep intermediate (setSandbox model (sandboxId sid) (sandboxRecord updated)) :=
    { represented with sandboxes := sandboxRep }
  cases current : record.current_operation with
  | none =>
      refine ⟨intermediate, ?_, ?_⟩
      · dsimp [updated] at putEval
        simp only [current, Option.map_none] at putEval
        simp [SandboxControllerRust.publication.publish_failure, world_clone_eq, current, putEval, intermediate]
      · simpa [publishFailure, updateCurrentOperation, sandboxRecord, current,
          updated, setSandbox, phase, authority] using intermediateRep
  | some op =>
      obtain ⟨next, nextEval, nextRep⟩ := generated_updateOperationResult_refines
        intermediate (setSandbox model (sandboxId sid) (sandboxRecord updated)) intermediateRep operationRoom
        sid { op with status := .Failed failure } (.Failed failure)
      refine ⟨next, ?_, ?_⟩
      · dsimp [updated] at putEval
        simp only [current, Option.map_some] at putEval
        simpa [SandboxControllerRust.publication.publish_failure, world_clone_eq,
          current, putEval, intermediate] using nextEval
      · simpa [publishFailure, updateCurrentOperation, updateOperation, sandboxRecord,
          current, updated, setSandbox, operation, operationStatus, operationResult, phase, authority] using nextRep

theorem generated_publishPaused_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId)
    (record : SandboxControllerRust.model.SandboxRecord)
    (source : SandboxControllerRust.model.RestoreSource) :
    ∃ next, SandboxControllerRust.publication.publish_paused rust sid record source = ok next ∧
      EffectWorldRep next (publishPaused model (sandboxId sid) (sandboxRecord record) (restoreSource source)) := by
  let updated := { record with
    phase := SandboxControllerRust.model.Phase.Paused
    durable_snapshot := some source
    restore_from := none
    current_vm := none
    net_slot := none
    desired := SandboxControllerRust.model.Authority.Paused sid source
    effective := SandboxControllerRust.model.Authority.Paused sid source
    current_operation := record.current_operation.map
      (fun (op : SandboxControllerRust.model.Operation) => { op with status := .Succeeded }) }
  obtain ⟨sandboxes, putEval, sandboxRep⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_put_preserves_storeRep
      SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful
      sandboxId sandboxId_injective (fun concrete abstract => sandboxRecord concrete = abstract)
      rust.sandboxes model.sandboxes represented.sandboxes sid updated (sandboxRecord updated) rfl sandboxRoom)
  let intermediate := { rust with sandboxes := sandboxes }
  have intermediateRep : EffectWorldRep intermediate (setSandbox model (sandboxId sid) (sandboxRecord updated)) :=
    { represented with sandboxes := sandboxRep }
  cases current : record.current_operation with
  | none =>
      refine ⟨intermediate, ?_, ?_⟩
      · dsimp [updated] at putEval
        simp only [current, Option.map_none] at putEval
        simp [SandboxControllerRust.publication.publish_paused, world_clone_eq, current, putEval, intermediate]
      · simpa [publishPaused, updateCurrentOperation, sandboxRecord, current,
          updated, setSandbox, phase, authority] using intermediateRep
  | some op =>
      obtain ⟨next, nextEval, nextRep⟩ := generated_updateOperationResult_refines
        intermediate (setSandbox model (sandboxId sid) (sandboxRecord updated)) intermediateRep operationRoom
        sid { op with status := .Succeeded } (.Paused source)
      refine ⟨next, ?_, ?_⟩
      · dsimp [updated] at putEval
        simp only [current, Option.map_some] at putEval
        simpa [SandboxControllerRust.publication.publish_paused, world_clone_eq,
          current, putEval, intermediate] using nextEval
      · simpa [publishPaused, updateCurrentOperation, updateOperation, sandboxRecord,
          current, updated, setSandbox, operation, operationStatus, operationResult, phase, authority] using nextRep

theorem generated_publishRunning_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (slotRoom : StoreHeadroom rust.net_slots)
    (sid : SandboxControllerRust.ids.SandboxId)
    (record : SandboxControllerRust.model.SandboxRecord)
    (slotId : SandboxControllerRust.ids.NetSlotId)
    (slot : SandboxControllerRust.model.NetSlot)
    (indexed : netSlotId slotId ∈ (model.resourceIndexes (sandboxId slot.sandbox)).slots) :
    ∃ next, SandboxControllerRust.publication.publish_running rust sid record slotId slot = ok next ∧
      EffectWorldRep next (publishRunning model (sandboxId sid) (sandboxRecord record) (netSlotId slotId) (netSlot slot)) := by
  let auth := SandboxControllerRust.model.Authority.Running sid record.generation
    slot.vm record.workspace record.workspace_access slotId record.policy
  let updated := { record with
    phase := SandboxControllerRust.model.Phase.Running
    terminal := false
    restore_from := none
    desired := auth
    effective := auth
    current_operation := record.current_operation.map
      (fun (op : SandboxControllerRust.model.Operation) => { op with status := .Succeeded }) }
  obtain ⟨sandboxes, putEval, sandboxRep⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_put_preserves_storeRep
      SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId sandboxId_partialEq_lawful
      sandboxId sandboxId_injective (fun concrete abstract => sandboxRecord concrete = abstract)
      rust.sandboxes model.sandboxes represented.sandboxes sid updated (sandboxRecord updated) rfl sandboxRoom)
  obtain ⟨slots, slotEval, slotsRep⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_put_preserves_storeRep
      SandboxControllerRust.ids.NetSlotId.Insts.CoreCmpPartialEqNetSlotId netSlotId_partialEq_lawful
      netSlotId netSlotId_injective (fun concrete abstract => netSlot concrete = abstract)
      rust.net_slots model.netSlots represented.netSlots slotId { slot with active := true }
      (netSlot { slot with active := true }) rfl slotRoom)
  let intermediate := { rust with sandboxes := sandboxes, net_slots := slots }
  have indexes : (setNetSlot model (netSlotId slotId) (netSlot { slot with active := true })).resourceIndexes =
      model.resourceIndexes := by
    funext candidate
    simp [setNetSlot, setResourceIndex, insertUnique, netSlot, indexed]
    intro same
    subst candidate
    rfl
  have intermediateRep : EffectWorldRep intermediate (setSandbox (setNetSlot model (netSlotId slotId) (netSlot { slot with active := true }))
          (sandboxId sid) (sandboxRecord updated)) :=
    { represented with
      sandboxes := sandboxRep
      netSlots := slotsRep
      resourceIndexes := by simpa only [setSandbox, indexes] using represented.resourceIndexes }
  cases current : record.current_operation with
  | none =>
      refine ⟨intermediate, ?_, ?_⟩
      · dsimp [updated, auth] at putEval
        simp only [current, Option.map_none] at putEval
        simp [SandboxControllerRust.publication.publish_running, world_clone_eq, current, putEval, slotEval, intermediate, auth]
      · simpa [publishRunning, updateCurrentOperation, sandboxRecord, current,
          updated, auth, setSandbox, setNetSlot, netSlot, phase, authority] using intermediateRep
  | some op =>
      obtain ⟨next, nextEval, nextRep⟩ := generated_updateOperationResult_refines
        intermediate (setSandbox (setNetSlot model (netSlotId slotId) (netSlot { slot with active := true }))
          (sandboxId sid) (sandboxRecord updated)) intermediateRep operationRoom
        sid { op with status := .Succeeded } (.Running record.generation)
      refine ⟨next, ?_, ?_⟩
      · dsimp [updated, auth] at putEval
        simp only [current, Option.map_some] at putEval
        simpa [SandboxControllerRust.publication.publish_running, world_clone_eq,
          current, putEval, slotEval, intermediate, auth] using nextEval
      · simpa [publishRunning, updateCurrentOperation, updateOperation, sandboxRecord,
          current, updated, auth, setSandbox, setNetSlot, netSlot, operation, operationStatus, operationResult, phase, authority] using nextRep

end SandboxModel.RustRefinement
