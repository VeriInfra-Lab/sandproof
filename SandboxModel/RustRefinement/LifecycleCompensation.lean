import SandboxModel.RustRefinement.LifecycleEvidence

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result
set_option maxHeartbeats 2000000
local instance compensationOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance compensationLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance compensationEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

-- Match the generated eliminators explicitly: freshly elaborated match syntax
-- uses different matcher declarations and does not rewrite these expressions.
theorem compensation_phase_choice (value : SandboxControllerRust.model.Phase) :
    SandboxControllerRust.worker.revocation_phase.match_1
      (fun _ => Result (SandboxControllerRust.model.Phase × Bool)) value
      (fun _ => ok (.Starting, true)) (fun _ => ok (.Running, false))
      (fun _ => ok (.Pausing, true)) (fun _ => ok (.Paused, false))
      (fun _ => ok (.Revoking, false)) (fun _ => ok (.Destroyed, false)) =
    ok (value, decide (phase value = .starting ∨ phase value = .pausing)) := by
  cases value <;> rfl

theorem compensation_kind_choice (value : SandboxControllerRust.model.OperationKind) :
    SandboxControllerRust.request.handle_compensation.match_1
      (fun _ => Result (SandboxControllerRust.model.OperationKind × SandboxControllerRust.model.OperationKind × Bool)) value
      (fun _ => ok (.Create, .Create, true)) (fun _ => ok (.Pause, .Pause, true))
      (fun _ => ok (.Resume, .Resume, true)) (fun _ => ok (.Kill, .Kill, false)) =
    ok (value, value, decide (operationKind value = .create ∨ operationKind value = .resume ∨
      operationKind value = .pause)) := by
  cases value <;> rfl

theorem compensation_status_choice (value : SandboxControllerRust.model.OperationStatus) :
    SandboxControllerRust.model.OperationStatus.Insts.CoreCmpPartialEqOperationStatus.eq.match_1
      (fun _ => Result (SandboxControllerRust.model.OperationStatus × SandboxControllerRust.model.OperationStatus × Bool)) value
      (fun _ => ok (.Pending, .Pending, true)) (fun _ => ok (.OutcomeUnknown, .OutcomeUnknown, true))
      (fun _ => ok (value, value, false)) (fun _ => ok (.Succeeded, .Succeeded, false))
      (fun _ => ok (value, value, false)) =
    ok (value, value, decide (operationStatus value = .pending ∨ operationStatus value = .outcomeUnknown)) := by
  cases value <;> rfl

@[simp] theorem compensation_operation_kind (op : SandboxControllerRust.model.Operation) :
    (operation op).kind = operationKind op.kind := rfl
@[simp] theorem compensation_operation_epoch (op : SandboxControllerRust.model.Operation) :
    (operation op).epoch = u64ToNat op.epoch := rfl
@[simp] theorem compensation_ledger_sandbox (ledger : SandboxControllerRust.model.OperationLedgerRecord) :
    (operationLedger ledger).sandbox = sandboxId ledger.sandbox := rfl
@[simp] theorem compensation_ledger_operation (ledger : SandboxControllerRust.model.OperationLedgerRecord) :
    (operationLedger ledger).operation = operation ledger.operation := rfl
@[simp] theorem compensation_ledger_result (ledger : SandboxControllerRust.model.OperationLedgerRecord) :
    (operationLedger ledger).result = ledger.result.map operationResult := rfl

macro "finish_compensation" : tactic => `(tactic| (
  simp only [SandboxControllerRust.request.handle_compensation, compensation_phase_choice,
    compensation_kind_choice, compensation_status_choice, bind_tc_ok]
  simp [*, SandboxControllerRust.request.handle_compensation, SandboxModel.Controller.handleCompensation,
    compensation_phase_choice, compensation_kind_choice, compensation_status_choice,
    SandboxControllerRust.transition.operation_key, generated_operation_ne, generated_operation_eq,
    core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId.eq,
    core.option.Option.is_some]
  all_goals repeat' (split <;> (try simp_all) <;> subst_vars)
  all_goals try simp_all [RequestResultRep, reject?, modelEvent]
  all_goals try simp [lifecycle_record_operation] at *
  all_goals try simp_all
  all_goals subst_vars
  all_goals try simp_all [RequestResultRep, modelEvent, reject?]
  all_goals try grind only [Option.map, sandboxId_injective, compensation_operation_kind, operation_status_field,
    compensation_ledger_sandbox, compensation_ledger_operation, compensation_ledger_result]))

theorem handleCompensation_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : RequestWorldRep rust model)
    (room : HandlerMachineHeadroom rust) (capacity : EffectCapacity rust 1)
    (input : SandboxControllerRust.request.CompensationRequest) :
    ∃ result, SandboxControllerRust.request.handle_compensation rust input = ok result ∧
      RequestResultRep model result (SandboxModel.Controller.handleCompensation model
        (sandboxId input.sandbox) (failureKind input.failure) (input.evidence.map effectId)) := by
  have getSandbox := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes input.sandbox
  have sandboxLookup := storeRep_lookup_map rep.sandboxes input.sandbox
  cases up : rust.controller_up
  all_goals have modelUp : model.controllerUp = _ := rep.controllerUp.symm.trans up
  · finish_compensation
  cases found : storeLookup rust.sandboxes input.sandbox with
  | none => finish_compensation
  | some record =>
    by_cases phaseOK : phase record.phase = .starting ∨ phase record.phase = .pausing
    · cases terminal : record.terminal
      · cases current : record.current_operation with
        | none => finish_compensation
        | some op =>
          have recordEta : { record with current_operation := some op } = record := by rw [← current]
          by_cases kindOK : operationKind op.kind = .create ∨ operationKind op.kind = .resume ∨ operationKind op.kind = .pause
          · by_cases statusOK : operationStatus op.status = .pending ∨ operationStatus op.status = .outcomeUnknown
            · have getLedger := generated_get_eq SandboxControllerRust.model.OperationKey.Insts.CoreCmpPartialEqOperationKey
                operationKey_partialEq_lawful rust.operations ⟨input.sandbox, op.id⟩
              have ledgerLookup := storeRep_lookup_map rep.operations (SandboxControllerRust.model.OperationKey.mk input.sandbox op.id)
              change model.operations (SandboxModel.operationKey (sandboxId input.sandbox) (operationId op.id)) = _ at ledgerLookup
              cases ledgerFound : storeLookup rust.operations ⟨input.sandbox, op.id⟩ with
              | none => finish_compensation
              | some ledger =>
                by_cases sameSid : ledger.sandbox = input.sandbox
                · by_cases sameOp : operation ledger.operation = operation op
                  · have sameId : operationId ledger.operation.id = operationId op.id := congrArg Operation.id sameOp
                    have sameEpoch : u64ToNat ledger.operation.epoch = u64ToNat op.epoch := congrArg Operation.epoch sameOp
                    have sameKind : operationKind ledger.operation.kind = operationKind op.kind := congrArg Operation.kind sameOp
                    have sameStatus : operationStatus ledger.operation.status = operationStatus op.status := congrArg Operation.status sameOp
                    cases finalResult : ledger.result with
                    | some result => finish_compensation
                    | none =>
                      have evidenceCheck := generated_compensationEvidence_refines rust model rep
                        ⟨input.sandbox, op.id⟩ op.epoch input.failure input.evidence
                      change SandboxControllerRust.request.supplied_compensation_evidence rust
                        ⟨input.sandbox, op.id⟩ op.epoch input.failure input.evidence =
                        ok (SandboxModel.Controller.compensationEvidenceCheck model
                          (SandboxModel.operationKey (sandboxId input.sandbox) (operationId op.id))
                          (u64ToNat op.epoch) (failureKind input.failure) (input.evidence.map effectId)) at evidenceCheck
                      cases justified : SandboxModel.Controller.compensationEvidenceCheck model
                        (SandboxModel.operationKey (sandboxId input.sandbox) (operationId op.id))
                        (u64ToNat op.epoch) (failureKind input.failure) (input.evidence.map effectId)
                      · finish_compensation
                      · obtain ⟨next, nextEval, nextRep⟩ := generated_beginCompensation_refines rust model rep room capacity
                          input.sandbox record op input.failure ledger ledgerFound
                        finish_compensation
                  · finish_compensation
                · finish_compensation
            · finish_compensation
          · finish_compensation
      · finish_compensation
    · finish_compensation

end SandboxModel.RustRefinement
