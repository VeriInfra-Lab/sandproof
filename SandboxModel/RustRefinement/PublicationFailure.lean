import SandboxModel.RustRefinement.PublicationDestroyed

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
set_option backward.split false
local instance failureOpDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance failureLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance failureEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

@[simp] theorem publication_compensating (failure : SandboxControllerRust.model.FailureKind) :
    operationStatus (.Compensating failure) = .compensating (failureKind failure) := rfl

macro "finish_failure" validity:ident : tactic => `(tactic| (
  simp only [*, -$validity, bind_tc_ok,
    lifecycle_record_phase, lifecycle_record_terminal, publication_record_epoch, lifecycle_record_operation,
    publication_revoking, publication_kill, operation_id_field, compensation_operation_kind,
    operation_status_field, compensation_ledger_sandbox, compensation_ledger_operation,
    compensation_ledger_result, Option.map_some, Option.map_none,
    if_true, if_false, decide_true, decide_false, Bool.true_eq_false, Bool.false_eq_true,
    not_true_eq_false, not_false_eq_true, ne_eq, eq_self_iff_true, Bool.not_false, Bool.not_true]
  all_goals try simp
  all_goals unfold SandboxModel.Controller.handlePublishFailure
  all_goals try simp only [*, -$validity, dif_pos, dif_neg, Bool.true_eq_false, Bool.false_eq_true]
  all_goals try (rw [publication_sandbox_some (by assumption)])
  all_goals try (rw [publication_sandbox_none (by assumption)])
  all_goals try simp only [*, -$validity, lifecycle_record_phase, lifecycle_record_terminal,
    publication_record_epoch, lifecycle_record_operation, dif_pos, dif_neg,
    Bool.true_eq_false, Bool.false_eq_true, eq_self_iff_true]
  all_goals try (rw [publication_operation_some (by assumption)])
  all_goals try (rw [publication_operation_none (by assumption)])
  all_goals try simp only [*, -$validity, compensation_operation_kind, operation_status_field,
    operation_id_field, dif_pos, dif_neg]
  all_goals try (rw [publication_ledger_some (by assumption)])
  all_goals try (rw [publication_ledger_none (by assumption)])
  all_goals try simp only [*, -$validity, Option.map_some, Option.map_none, Option.some.injEq,
        lifecycle_record_phase, lifecycle_record_terminal, publication_record_epoch,
        lifecycle_record_operation, compensation_operation_kind, operation_status_field,
        operation_id_field, compensation_ledger_sandbox, compensation_ledger_operation,
        compensation_ledger_result, reduceCtorEq, dif_pos, dif_neg, if_true, if_false,
        Bool.true_eq_false, Bool.false_eq_true, not_true_eq_false, not_false_eq_true,
        and_self, eq_self_iff_true]
  all_goals try simp only [sandboxId_injective.eq_iff, *, -$validity, false_and, dif_neg, not_false_eq_true]
  all_goals try (exact ⟨rfl, by assumption⟩)
  all_goals try rfl))

theorem handlePublishFailure_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : PublicationWorldRep rust model)
    (valid : ResourceIndexValid model)
    (sandboxRoom : StoreHeadroom rust.sandboxes) (operationRoom : StoreHeadroom rust.operations)
    (sid : SandboxControllerRust.ids.SandboxId) (failure : SandboxControllerRust.model.FailureKind) :
    ∃ result, SandboxControllerRust.publication.handle_publish_failure rust sid failure = ok result ∧
      PublicationResultRep model result (SandboxModel.Controller.handlePublishFailure model valid (sandboxId sid) (failureKind failure)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have sandboxLookup := storeRep_lookup_map rep.sandboxes sid
  have allFences := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.worker_fences sid
  have allOperations := fun key => generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
    operationKey_partialEq_lawful rust.operations key
  unfold SandboxControllerRust.publication.handle_publish_failure
  simp_rw [getSandbox, allFences, allOperations]
  simp only [bind_tc_ok]
  simp_rw [publication_fence_store_ne rust model rep, generated_phase_ne, generated_operation_ne]
  simp only [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    publication_status_eq, publication_recoverable, publication_compensating, SandboxControllerRust.transition.operation_key,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    core.option.Option.is_some, bind_tc_ok]
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · finish_failure valid
  cases found : storeLookup rust.sandboxes sid with
  | none =>
    have modelFound : model.sandboxes (sandboxId sid) = none := by
      simpa only [found, Option.map_none] using sandboxLookup
    finish_failure valid
  | some record =>
    have modelFound : model.sandboxes (sandboxId sid) = some (sandboxRecord record) := by
      simpa only [found, Option.map_some] using sandboxLookup
    obtain ⟨next, nextEval, nextRep⟩ := generated_publishFailure_refines rust model rep sandboxRoom operationRoom sid record failure
    have getFence := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
      sandboxId_partialEq_lawful rust.worker_fences sid
    have fenceLookup := (storeRep_lookup_map rep.workerFences sid).symm
    have fenceCompare := publication_fence_store_ne rust model rep sid record.epoch true
    have fenceCheck := generated_effectSucceeded_refines rust model rep (.InstallFence sid record.epoch true)
    change SandboxControllerRust.effect.effect_succeeded_check rust (.InstallFence sid record.epoch true) =
      ok (effectSucceededCheck model (.installFence (sandboxId sid) (u64ToNat record.epoch) true)) at fenceCheck
    have vmsCheck := generated_indexedVmsClosed_refines rust model rep sid
    have stopsCheck := generated_indexedStopsSucceeded_refines rust model rep sid
    have slotsCheck := generated_indexedSlotsClosed_refines rust model rep sid
    by_cases revoking : phase record.phase = .revoking
    case' pos =>
      cases terminal : record.terminal
      case' true =>
        by_cases fenced : model.workerFences (sandboxId sid) = some ⟨u64ToNat record.epoch, true⟩
        case' pos =>
          cases fenceSucceeded : effectSucceededCheck model (.installFence (sandboxId sid) (u64ToNat record.epoch) true)
          case' true =>
            cases vmsClosed : indexedVmsClosedCheck model (sandboxId sid)
            case' true =>
              cases stopsSucceeded : SandboxModel.Controller.indexedAnyStopsSucceededCheck model (sandboxId sid)
              case' true =>
                cases slotsClosed : indexedSlotsClosedCheck model (sandboxId sid)
                case' true =>
                  cases current : record.current_operation
                  all_goals have modelCurrent := congrArg (Option.map operation) current
                  case' some op =>
                    by_cases compensating : operationStatus op.status = .compensating (failureKind failure)
                    case' pos =>
                      by_cases recoverable : operationKind op.kind = .create ∨ operationKind op.kind = .resume ∨ operationKind op.kind = .pause
                      case' pos =>
                        have getLedger := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
                          operationKey_partialEq_lawful rust.operations ⟨sid, op.id⟩
                        have ledgerLookup := storeRep_lookup_map rep.operations (SandboxControllerRust.model.OperationKey.mk sid op.id)
                        change model.operations (SandboxModel.operationKey (sandboxId sid) (operationId op.id)) = _ at ledgerLookup
                        cases ledgerFound : storeLookup rust.operations ⟨sid, op.id⟩
                        all_goals have modelLedger := ledgerLookup.trans (congrArg (Option.map operationLedger) ledgerFound)
                        case' some ledger =>
                          by_cases sameSid : ledger.sandbox = sid
                          case' pos =>
                            by_cases sameOp : operation ledger.operation = operation op
                            case' pos =>
                              have sameId : operationId ledger.operation.id = operationId op.id := congrArg Operation.id sameOp
                              have sameKind : operationKind ledger.operation.kind = operationKind op.kind := congrArg Operation.kind sameOp
                              have sameStatus : operationStatus ledger.operation.status = operationStatus op.status := congrArg Operation.status sameOp
                              cases finalResult : ledger.result
    all_goals finish_failure valid

end SandboxModel.RustRefinement
