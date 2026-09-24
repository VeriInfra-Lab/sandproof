import SandboxModel.RustRefinement.ReceiptEvidence
import SandboxModel.RustRefinement.ReceiptUpdate

/-! Every receipt branch, including repeated and conflicting observations.
Receipt handling remains available while the controller is down. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance receiptOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance receiptLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance receiptEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

private theorem failure_eq (left right : SandboxControllerRust.model.FailureKind) :
    SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq left right =
      ok (decide (failureKind left = failureKind right)) := by
  cases left <;> cases right <;>
    simp [SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq,
      SandboxControllerRust.model.FailureKind.read_discriminant, failureKind]

private theorem outcome_ne (left right : SandboxControllerRust.model.EffectOutcome) :
    core.cmp.PartialEq.ne.trait_default
      SandboxControllerRust.model.EffectOutcome.Insts.CoreCmpPartialEqEffectOutcome left right =
      ok (decide (effectOutcome left ≠ effectOutcome right)) := by
  have scalarEq : ∀ a b : Std.U64, a = b ↔ a.val = b.val := by
    intro a b
    exact ⟨congrArg Std.UScalar.val, Std.UScalar.eq_of_val_eq⟩
  cases left <;> cases right <;>
    simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      SandboxControllerRust.model.EffectOutcome.Insts.CoreCmpPartialEqEffectOutcome.eq,
      SandboxControllerRust.model.EffectOutcome.read_discriminant, effectOutcome, failure_eq,
      SandboxControllerRust.ids.ResultDigest.Insts.CoreCmpPartialEqResultDigest.eq, resultDigest,
      u64ToNat, scalarEq]

variable {id : EffectId}

private theorem model_receipt_missing (found : model.effects id = none) :
    SandboxModel.Controller.handleEffectReceipt model id outcome = .error .effectNotFound := by
  unfold SandboxModel.Controller.handleEffectReceipt
  split <;> simp_all

private theorem model_receipt_committed (found : model.effects id = some .committed) :
    SandboxModel.Controller.handleEffectReceipt model id outcome = .error .notAwaitingResult := by
  unfold SandboxModel.Controller.handleEffectReceipt
  split <;> simp_all

private theorem model_receipt_invalid
    (waiting : model.effects id = some .dispatched ∨ model.effects id = some .outcomeUnknown)
    (invalid : SandboxModel.Controller.effectOutcomeValidCheck model id.command outcome ≠ true) :
    SandboxModel.Controller.handleEffectReceipt model id outcome = .error .invalidOutcomeEvidence := by
  unfold SandboxModel.Controller.handleEffectReceipt
  rcases waiting with found | found <;> split <;> simp_all

private theorem model_receipt_conflict (found : model.effects id = some (.acknowledged recorded))
    (different : recorded ≠ outcome) :
    SandboxModel.Controller.handleEffectReceipt model id outcome = .error .conflictingReceipt := by
  unfold SandboxModel.Controller.handleEffectReceipt
  split <;> simp_all

private theorem model_receipt_replay (found : model.effects id = some (.acknowledged outcome)) :
    ∃ decision, SandboxModel.Controller.handleEffectReceipt model id outcome = .ok decision ∧
      decision.event = .acknowledgeEffect id outcome ∧ decision.next = model := by
  let decision : SandboxModel.Controller.Decision model :=
    ⟨.acknowledgeEffect id outcome, model, Step.replayEffectReceipt found⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  unfold SandboxModel.Controller.handleEffectReceipt
  split
  · rename_i missing
    cases found.symm.trans missing
  · rename_i dispatched
    cases found.symm.trans dispatched
  · rename_i unknown
    cases found.symm.trans unknown
  · rename_i recorded recordedEq
    have same : recorded = outcome := by simpa using recordedEq.symm.trans found
    subst recorded
    simp [decision]
  · rename_i status notDispatched notUnknown notAcknowledged statusEq
    have same : status = .acknowledged outcome := Option.some.inj (statusEq.symm.trans found)
    exact (notAcknowledged outcome same).elim

private theorem model_receipt_accepted
    (waiting : model.effects id = some .dispatched ∨ model.effects id = some .outcomeUnknown)
    (valid : SandboxModel.Controller.effectOutcomeValidCheck model id.command outcome = true) :
    ∃ decision, SandboxModel.Controller.handleEffectReceipt model id outcome = .ok decision ∧
      decision.event = .acknowledgeEffect id outcome ∧ decision.next = acknowledgeEffect model id outcome := by
  let decision : SandboxModel.Controller.Decision model :=
    ⟨.acknowledgeEffect id outcome, acknowledgeEffect model id outcome,
      Step.acknowledgeEffect waiting (SandboxModel.Controller.effectOutcomeValidCheck_spec.mp valid)⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  unfold SandboxModel.Controller.handleEffectReceipt
  rcases waiting with found | found <;> split <;> simp_all [decision]
  all_goals
    rename_i recorded recordedEq
    cases found.symm.trans recordedEq

theorem handleEffectReceipt_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (room : HandlerMachineHeadroom rust)
    (id : SandboxControllerRust.model.EffectId)
    (outcome : SandboxControllerRust.model.EffectOutcome) :
    ∃ result, SandboxControllerRust.effect.handle_effect_receipt rust id outcome = ok result ∧
      EffectResultRep model result (SandboxModel.Controller.handleEffectReceipt model (effectId id) (effectOutcome outcome)) := by
  have getEffect := generated_get_eq
    SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId effectId_partialEq_lawful rust.effects id
  cases found : storeLookup rust.effects id with
  | none =>
      have modelFound : model.effects (effectId id) = none := by
        have agrees := effectLookup_agrees represented id
        rw [found] at agrees
        cases lookup : model.effects (effectId id) <;> simp_all [OptionRel]
      refine ⟨.Err .EffectNotFound, ?_, ?_⟩
      · simp [SandboxControllerRust.effect.handle_effect_receipt, getEffect, found]
      · rw [model_receipt_missing modelFound]; rfl
  | some status =>
      have modelFound := effectLookup_some represented id status found
      cases status with
      | Committed =>
          refine ⟨.Err .NotAwaitingResult, ?_, ?_⟩
          · simp [SandboxControllerRust.effect.handle_effect_receipt, getEffect, found]
          · rw [model_receipt_committed modelFound]; rfl
      | Acknowledged recorded =>
          by_cases same : effectOutcome recorded = effectOutcome outcome
          · have replay : model.effects (effectId id) = some (.acknowledged (effectOutcome outcome)) := by
              simpa [effectStatus, same] using modelFound
            obtain ⟨decision, modelEval, eventSame, nextSame⟩ := model_receipt_replay replay
            refine ⟨.Ok ⟨.AcknowledgeEffect id outcome, rust⟩, ?_, ?_⟩
            · simp [SandboxControllerRust.effect.handle_effect_receipt, getEffect, found, outcome_ne, same, world_clone_eq]
            · rw [modelEval]
              change modelEvent (.AcknowledgeEffect id outcome) = decision.event ∧ EffectWorldRep rust decision.next
              rw [eventSame, nextSame]
              exact ⟨rfl, represented⟩
          · refine ⟨.Err .ConflictingReceipt, ?_, ?_⟩
            · simp [SandboxControllerRust.effect.handle_effect_receipt, getEffect, found, outcome_ne, same]
            · rw [model_receipt_conflict modelFound same]; rfl
      | Dispatched | OutcomeUnknown =>
          have waiting : model.effects (effectId id) = some .dispatched ∨
              model.effects (effectId id) = some .outcomeUnknown := by
            simp_all [effectStatus]
          have checked := generated_effectOutcomeValid_refines rust model represented id.logical.command outcome
          by_cases valid : SandboxModel.Controller.effectOutcomeValidCheck model (effectId id).command (effectOutcome outcome) = true
          · obtain ⟨next, nextEval, nextRep⟩ := generated_acknowledgeEffect_refines rust model represented room id outcome
            obtain ⟨decision, modelEval, eventSame, nextSame⟩ := model_receipt_accepted waiting valid
            refine ⟨.Ok ⟨.AcknowledgeEffect id outcome, next⟩, ?_, ?_⟩
            · simp [SandboxControllerRust.effect.handle_effect_receipt, getEffect, found,
                SandboxControllerRust.model.EffectId.command, command_clone_eq, checked,
                show SandboxModel.Controller.effectOutcomeValidCheck model (command id.logical.command) (effectOutcome outcome) = true from valid,
                nextEval]
            · rw [modelEval]
              change modelEvent (.AcknowledgeEffect id outcome) = decision.event ∧ EffectWorldRep next decision.next
              rw [eventSame, nextSame]
              exact ⟨rfl, nextRep⟩
          · refine ⟨.Err .InvalidOutcomeEvidence, ?_, ?_⟩
            · have invalid : SandboxModel.Controller.effectOutcomeValidCheck model (command id.logical.command) (effectOutcome outcome) ≠ true := valid
              simp [SandboxControllerRust.effect.handle_effect_receipt, getEffect, found,
                SandboxControllerRust.model.EffectId.command, command_clone_eq, checked, invalid]
            · rw [model_receipt_invalid waiting valid]; rfl

end SandboxModel.RustRefinement
