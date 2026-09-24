import SandboxModel.RustRefinement.Auxiliary
import SandboxModel.RustRefinement.EffectHandler

/-! Client timeout records uncertainty, not an external execution outcome.
The bridge checks both journal writes and every ordered rejection. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance rpcOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance rpcLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance rpcEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq


private theorem kind_eq (left right : SandboxControllerRust.model.OperationKind) :
    SandboxControllerRust.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq left right =
      ok (decide (operationKind left = operationKind right)) := by
  cases left <;> cases right <;>
    simp [SandboxControllerRust.model.OperationKind.Insts.CoreCmpPartialEqOperationKind.eq,
      SandboxControllerRust.model.OperationKind.read_discriminant, operationKind]

private theorem failure_eq (left right : SandboxControllerRust.model.FailureKind) :
    SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq left right =
      ok (decide (failureKind left = failureKind right)) := by
  cases left <;> cases right <;>
    simp [SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq,
      SandboxControllerRust.model.FailureKind.read_discriminant, failureKind]

private theorem status_eq (left right : SandboxControllerRust.model.OperationStatus) :
    SandboxControllerRust.model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus.eq left right =
      ok (decide (operationStatus left = operationStatus right)) := by
  cases left <;> cases right <;>
    simp [SandboxControllerRust.model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus.eq,
      SandboxControllerRust.model.OperationStatus.read_discriminant, operationStatus, failure_eq]

theorem generated_operation_eq (left right : SandboxControllerRust.model.Operation) :
    SandboxControllerRust.model.Operation.Insts.CoreCmpPartialEqOperation.eq left right =
      ok (decide (operation left = operation right)) := by
  have epochEq : u64ToNat left.epoch = u64ToNat right.epoch ↔ left.epoch = right.epoch := by
    simp only [u64ToNat]
    exact ⟨Std.UScalar.eq_of_val_eq, congrArg Std.UScalar.val⟩
  by_cases idSame : left.id = right.id <;>
  by_cases epochSame : left.epoch = right.epoch <;>
  by_cases kindSame : operationKind left.kind = operationKind right.kind <;>
  simp [SandboxControllerRust.model.Operation.Insts.CoreCmpPartialEqOperation.eq,
    SandboxControllerRust.ids.OperationId.Insts.CoreCmpPartialEqOperationId.eq,
    kind_eq, status_eq, operation, Operation.mk.injEq, epochEq, idSame, epochSame, kindSame]

theorem generated_operation_ne (left right : SandboxControllerRust.model.Operation) :
    core.cmp.PartialEq.ne.trait_default
      SandboxControllerRust.model.Operation.Insts.CoreCmpPartialEqOperation left right =
      ok (decide (operation left ≠ operation right)) := by
  simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default, generated_operation_eq]

private theorem status_ne_pending (status : SandboxControllerRust.model.OperationStatus) :
    core.cmp.PartialEq.ne.trait_default
      SandboxControllerRust.model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus
      status .Pending = ok (decide (operationStatus status ≠ .pending)) := by
  simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    status_eq, operationStatus]

private theorem sandbox_ne (left right : SandboxControllerRust.ids.SandboxId) :
    core.cmp.PartialEq.ne.trait_default
      SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId left right =
      ok (decide (left ≠ right)) := by
  simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq]

/-- Both persisted views of the operation acquire the same unknown status.
No effect, transport message, resource, permission, or counter is changed. -/
theorem markOutcomeUnknown_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : AuxiliaryWorldRep rust model)
    (sandboxRoom : StoreHeadroom rust.sandboxes)
    (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId)
    (record : SandboxControllerRust.model.SandboxRecord)
    (op : SandboxControllerRust.model.Operation)
    (ledger : SandboxControllerRust.model.OperationLedgerRecord)
    (found : storeLookup rust.operations ⟨sid, op.id⟩ = some ledger) :
    ∃ next, SandboxControllerRust.transition.mark_operation_outcome_unknown rust sid record op = ok next ∧
      AuxiliaryWorldRep next (markOutcomeUnknown model (sandboxId sid) (sandboxRecord record) (operation op)) := by
  have agrees := represented.operations.lookupAgrees ⟨sid, op.id⟩
  change OptionRel (fun concrete abstract => operationLedger concrete = abstract)
    (storeLookup rust.operations ⟨sid, op.id⟩)
    (model.operations (operationKey ⟨sid, op.id⟩)) at agrees
  have modelFound : model.operations (operationKey ⟨sid, op.id⟩) = some (operationLedger ledger) := by
    rw [found] at agrees
    cases lookup : model.operations (operationKey ⟨sid, op.id⟩) <;> simp_all [OptionRel]
  let unknown := { op with status := SandboxControllerRust.model.OperationStatus.OutcomeUnknown }
  let updated := { record with current_operation := some unknown }
  let updatedLedger := { ledger with operation := unknown, result := none }
  obtain ⟨sandboxes, putSandbox, sandboxRep⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_put_preserves_storeRep
      SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful sandboxId sandboxId_injective
      (fun concrete abstract => sandboxRecord concrete = abstract)
      rust.sandboxes model.sandboxes represented.sandboxes sid updated
      (sandboxRecord updated) rfl sandboxRoom)
  obtain ⟨operations, putLedger, operationRep⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_put_preserves_storeRep
      SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operationKey_partialEq_lawful operationKey operationKey_injective
      (fun concrete abstract => operationLedger concrete = abstract)
      rust.operations model.operations represented.operations ⟨sid, op.id⟩ updatedLedger
      (operationLedger updatedLedger) rfl operationRoom)
  have modelNext : markOutcomeUnknown model (sandboxId sid) (sandboxRecord record) (operation op) =
      { model with
        sandboxes := put model.sandboxes (sandboxId sid) (sandboxRecord updated)
        operations := put model.operations (operationKey ⟨sid, op.id⟩) (operationLedger updatedLedger) } := by
    simp only [markOutcomeUnknown, updateOperation, setSandbox]
    congr 1
    funext candidate
    change model.operations ⟨sandboxId sid, operationId op.id⟩ = some (operationLedger ledger) at modelFound
    simp [put, SandboxModel.operationKey, operationKey, operation, operationStatus,
      unknown, modelFound, updatedLedger, operationLedger]
  refine ⟨{ rust with sandboxes := sandboxes, operations := operations }, ?_, ?_⟩
  · have getLedger := generated_get_eq
      SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
      operationKey_partialEq_lawful rust.operations ⟨sid, op.id⟩
    dsimp [updated, updatedLedger, unknown] at putSandbox putLedger
    simp [SandboxControllerRust.transition.mark_operation_outcome_unknown,
      world_clone_eq, sandboxRecord_clone_eq, operationLedger_clone_eq,
      SandboxControllerRust.transition.operation_key, getLedger, found,
      putSandbox, putLedger]
  · rw [modelNext]
    exact { represented with sandboxes := sandboxRep, operations := operationRep }

private theorem model_rpc_noSandbox (found : model.sandboxes sid = none) :
    SandboxModel.Controller.handleRpcTimeout model sid = .error .sandboxNotFound := by
  unfold SandboxModel.Controller.handleRpcTimeout
  split <;> simp_all

private theorem model_rpc_noOperation (found : model.sandboxes sid = some record)
    (current : record.currentOperation = none) :
    SandboxModel.Controller.handleRpcTimeout model sid = .error .operationUnavailable := by
  unfold SandboxModel.Controller.handleRpcTimeout
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    split <;> simp_all

private theorem model_rpc_final (found : model.sandboxes sid = some record)
    (current : record.currentOperation = some op) (final : op.status ≠ .pending) :
    SandboxModel.Controller.handleRpcTimeout model sid = .error .operationAlreadyFinal := by
  unfold SandboxModel.Controller.handleRpcTimeout
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    split
    · simp_all
    · rename_i operation operationEq
      have same := Option.some.inj (operationEq.symm.trans current)
      subst operation
      rw [dif_neg final]

private theorem model_rpc_noLedger (found : model.sandboxes sid = some record)
    (current : record.currentOperation = some op) (pending : op.status = .pending)
    (missing : model.operations (SandboxModel.operationKey sid op.id) = none) :
    SandboxModel.Controller.handleRpcTimeout model sid = .error .operationUnavailable := by
  unfold SandboxModel.Controller.handleRpcTimeout
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    split
    · simp_all
    · rename_i operation operationEq
      have same := Option.some.inj (operationEq.symm.trans current)
      subst operation
      rw [dif_pos pending]
      split <;> simp_all

private theorem model_rpc_mismatch (found : model.sandboxes sid = some record)
    (current : record.currentOperation = some op) (pending : op.status = .pending)
    (persisted : model.operations (SandboxModel.operationKey sid op.id) = some ledger)
    (mismatch : ¬ (ledger.sandbox = sid ∧ ledger.operation = op ∧ ledger.result = none)) :
    SandboxModel.Controller.handleRpcTimeout model sid = .error .ledgerMismatch := by
  unfold SandboxModel.Controller.handleRpcTimeout
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    split
    · simp_all
    · rename_i operation operationEq
      have same := Option.some.inj (operationEq.symm.trans current)
      subst operation
      rw [dif_pos pending]
      split
      · simp_all
      · rename_i stored storedEq
        have same := Option.some.inj (storedEq.symm.trans persisted)
        subst stored
        rw [dif_neg mismatch]

private theorem model_rpc_accepts (found : model.sandboxes sid = some record)
    (current : record.currentOperation = some op) (pending : op.status = .pending)
    (persisted : model.operations (SandboxModel.operationKey sid op.id) = some ledger)
    (consistent : ledger.sandbox = sid ∧ ledger.operation = op ∧ ledger.result = none) :
    ∃ decision, SandboxModel.Controller.handleRpcTimeout model sid = .ok decision ∧
      decision.event = .rpcTimeout sid ∧ decision.next = markOutcomeUnknown model sid record op := by
  let decision : SandboxModel.Controller.Decision model :=
    ⟨.rpcTimeout sid, markOutcomeUnknown model sid record op,
      Step.rpcTimeout found current pending persisted consistent⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  unfold SandboxModel.Controller.handleRpcTimeout
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    split
    · simp_all
    · rename_i operation operationEq
      have same := Option.some.inj (operationEq.symm.trans current)
      subst operation
      rw [dif_pos pending]
      split
      · simp_all
      · rename_i stored storedEq
        have same := Option.some.inj (storedEq.symm.trans persisted)
        subst stored
        rw [dif_pos consistent]

/-- All RPC-timeout outcomes, with the exact successor world on acceptance.
The only machine assumptions here concern the two journal stores. -/
theorem handleRpcTimeout_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : AuxiliaryWorldRep rust model)
    (sandboxRoom : StoreHeadroom rust.sandboxes)
    (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId) :
    ∃ result, SandboxControllerRust.auxiliary.handle_rpc_timeout rust sid = ok result ∧
      AccessResultRep model result (SandboxModel.Controller.handleRpcTimeout model (sandboxId sid)) := by
  have getSandbox := generated_get_eq
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have sandboxAgrees := represented.sandboxes.lookupAgrees sid
  change OptionRel (fun concrete abstract => sandboxRecord concrete = abstract)
    (storeLookup rust.sandboxes sid) (model.sandboxes (sandboxId sid)) at sandboxAgrees
  cases foundSandbox : storeLookup rust.sandboxes sid with
  | none =>
      have modelFound : model.sandboxes (sandboxId sid) = none := by
        rw [foundSandbox] at sandboxAgrees
        cases lookup : model.sandboxes (sandboxId sid) <;> simp_all [OptionRel]
      refine ⟨.Err .SandboxNotFound, ?_, ?_⟩
      · simp [SandboxControllerRust.auxiliary.handle_rpc_timeout, getSandbox, foundSandbox]
      · rw [model_rpc_noSandbox modelFound]; rfl
  | some record =>
      have modelFound : model.sandboxes (sandboxId sid) = some (sandboxRecord record) := by
        rw [foundSandbox] at sandboxAgrees
        cases lookup : model.sandboxes (sandboxId sid) <;> simp_all [OptionRel]
      cases current : record.current_operation with
      | none =>
          refine ⟨.Err .OperationUnavailable, ?_, ?_⟩
          · simp [SandboxControllerRust.auxiliary.handle_rpc_timeout, getSandbox, foundSandbox, current]
          · rw [model_rpc_noOperation modelFound (by simp [sandboxRecord, current])]; rfl
      | some op =>
          have modelCurrent : (sandboxRecord record).currentOperation = some (operation op) := by
            simp [sandboxRecord, current]
          by_cases pending : operationStatus op.status = .pending
          · have getLedger := generated_get_eq
              SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
              operationKey_partialEq_lawful rust.operations ⟨sid, op.id⟩
            have ledgerAgrees := represented.operations.lookupAgrees ⟨sid, op.id⟩
            change OptionRel (fun concrete abstract => operationLedger concrete = abstract)
              (storeLookup rust.operations ⟨sid, op.id⟩)
              (model.operations (SandboxModel.operationKey (sandboxId sid) (operation op).id)) at ledgerAgrees
            cases foundLedger : storeLookup rust.operations ⟨sid, op.id⟩ with
            | none =>
                have modelLedger : model.operations (SandboxModel.operationKey (sandboxId sid) (operation op).id) = none := by
                  rw [foundLedger] at ledgerAgrees
                  cases lookup : model.operations (SandboxModel.operationKey (sandboxId sid) (operation op).id) <;>
                    simp_all [OptionRel]
                refine ⟨.Err .OperationUnavailable, ?_, ?_⟩
                · simp [SandboxControllerRust.auxiliary.handle_rpc_timeout, getSandbox, foundSandbox,
                    current, status_ne_pending, pending, SandboxControllerRust.transition.operation_key,
                    getLedger, foundLedger]
                · rw [model_rpc_noLedger modelFound modelCurrent pending modelLedger]; rfl
            | some ledger =>
                have modelLedger : model.operations (SandboxModel.operationKey (sandboxId sid) (operation op).id) =
                    some (operationLedger ledger) := by
                  rw [foundLedger] at ledgerAgrees
                  cases lookup : model.operations (SandboxModel.operationKey (sandboxId sid) (operation op).id) <;>
                    simp_all [OptionRel]
                by_cases consistent : (operationLedger ledger).sandbox = sandboxId sid ∧
                    (operationLedger ledger).operation = operation op ∧ (operationLedger ledger).result = none
                · have sameSid : ledger.sandbox = sid := by simpa [operationLedger] using consistent.1
                  have sameOp : operation ledger.operation = operation op := consistent.2.1
                  have noResult : ledger.result = none := by simpa [operationLedger] using consistent.2.2
                  obtain ⟨next, nextEval, nextRep⟩ := markOutcomeUnknown_refines rust model represented
                    sandboxRoom operationRoom sid record op ledger foundLedger
                  obtain ⟨decision, modelEval, eventSame, nextSame⟩ :=
                    model_rpc_accepts modelFound modelCurrent pending modelLedger consistent
                  refine ⟨.Ok ⟨.RpcTimeout sid, next⟩, ?_, ?_⟩
                  · simp [SandboxControllerRust.auxiliary.handle_rpc_timeout, getSandbox, foundSandbox,
                      current, status_ne_pending, pending, SandboxControllerRust.transition.operation_key,
                      getLedger, foundLedger, sandbox_ne, sameSid, generated_operation_ne, sameOp, noResult,
                      core.option.Option.is_some, nextEval]
                  · simp only [modelEval, AccessResultRep, auxiliaryEvent]
                    exact ⟨eventSame.symm, nextSame.symm ▸ nextRep⟩
                · refine ⟨.Err .LedgerMismatch, ?_, ?_⟩
                  · by_cases sameSid : ledger.sandbox = sid <;>
                    by_cases sameOp : operation ledger.operation = operation op <;>
                    cases resultCase : ledger.result <;>
                    simp_all [SandboxControllerRust.auxiliary.handle_rpc_timeout, status_ne_pending,
                      SandboxControllerRust.transition.operation_key, generated_operation_ne, sandbox_ne,
                      core.option.Option.is_some, operationLedger]
                  · rw [model_rpc_mismatch modelFound modelCurrent pending modelLedger consistent]; rfl
          · refine ⟨.Err .OperationAlreadyFinal, ?_, ?_⟩
            · simp [SandboxControllerRust.auxiliary.handle_rpc_timeout, getSandbox, foundSandbox,
                current, status_ne_pending, pending]
            · rw [model_rpc_final modelFound modelCurrent pending]; rfl

end SandboxModel.RustRefinement
