import SandboxModel.RustRefinement.EffectHandler

/-! Exact successor correspondence for a new attempt after definite failure.
Unknown outcomes do not satisfy the premise and cannot justify a new ID. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance retryOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance retryLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance retryEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

theorem generated_startNextAttempt_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (room : HandlerMachineHeadroom rust)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (record : SandboxControllerRust.model.LogicalEffectRecord)
    (failure : SandboxControllerRust.model.FailureKind)
    (found : storeLookup rust.logical_effects logical = some record)
    (failed : storeLookup rust.effects ⟨logical, record.current_attempt⟩ =
      some (.Acknowledged (.Failed failure)))
    (retryable : (failureKind failure).retryable = true)
    (budget : 0 < record.remaining_retries.val)
    (attemptRoom : U64Headroom record.current_attempt)
    (fresh : model.effects ⟨logicalEffectId logical, record.current_attempt.val + 1⟩ = none) :
    ∃ next, SandboxControllerRust.effect.start_next_attempt rust logical failure = ok next ∧
      EffectWorldRep next (startNextAttempt model (logicalEffectId logical) (failureKind failure)) := by
  have increment : (record.current_attempt + 1#u64)
      ⦃ next => next.val = record.current_attempt.val + 1 ⦄ := by
    unfold U64Headroom at attemptRoom
    step
    assumption
  have decrement : (record.remaining_retries - 1#u64)
      ⦃ remaining => remaining.val = record.remaining_retries.val - 1 ⦄ := by
    step
    assumption
  obtain ⟨attempt, incrementEval, attemptValue⟩ := Aeneas.Std.WP.spec_imp_exists increment
  obtain ⟨remaining, decrementEval, remainingValue⟩ := Aeneas.Std.WP.spec_imp_exists decrement
  let id : SandboxControllerRust.model.EffectId := ⟨logical, attempt⟩
  let updated := { record with current_attempt := attempt, remaining_retries := remaining }
  have modelFound := logicalLookup_some represented logical record found
  have modelFailed := effectLookup_some represented ⟨logical, record.current_attempt⟩ _ failed
  have idValue : effectId id = ⟨logicalEffectId logical, record.current_attempt.val + 1⟩ := by
    simp [id, effectId, u64ToNat, attemptValue]
  have absent : storeLookup rust.effects id = none := by
    have agrees := effectLookup_agrees represented id
    rw [idValue, fresh] at agrees
    cases lookup : storeLookup rust.effects id <;> simp_all [OptionRel]
  have nextData : nextAttemptData model (logicalEffectId logical) (failureKind failure) =
      some (effectId id, logicalEffectRecord updated) := by
    have failedEq : model.effects ⟨logicalEffectId logical, record.current_attempt.val⟩ =
        some (.acknowledged (.failed (failureKind failure))) := modelFailed
    have budgetEq : ∃ n, record.remaining_retries.val = n + 1 := ⟨record.remaining_retries.val - 1, by omega⟩
    obtain ⟨n, budgetEq⟩ := budgetEq
    simp [nextAttemptData, modelFound, logicalEffectRecord, failedEq, u64ToNat,
      retryable, budgetEq, fresh, idValue, updated, attemptValue, remainingValue]
  obtain ⟨effects, effectsEval, effectsRep⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_put_preserves_storeRep
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId effectId_partialEq_lawful
      effectId effectId_injective (fun concrete abstract => effectStatus concrete = abstract)
      rust.effects model.effects represented.effects id .Committed .committed rfl room.effects)
  obtain ⟨keys, keysEval, keysValue⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_effectInsertUnique_refines rust.effect_keys id room.effectKeys)
  obtain ⟨logicalEffects, logicalEval, logicalRep⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_put_preserves_storeRep
      SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId logicalEffectId_partialEq_lawful
      logicalEffectId logicalEffectId_injective (fun concrete abstract => logicalEffectRecord concrete = abstract)
      rust.logical_effects model.logicalEffects represented.logicalEffects logical updated
      (logicalEffectRecord updated) rfl room.logicalEffects)
  obtain ⟨inflight, inflightEval, inflightValue⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_effectInsertUnique_refines rust.inflight id room.inflight)
  refine ⟨{ rust with
    logical_effects := logicalEffects
    effects := effects
    effect_keys := keys
    inflight := inflight }, ?_, ?_⟩
  · have getLogical := generated_get_eq
      SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      logicalEffectId_partialEq_lawful rust.logical_effects logical
    have getFailed := generated_get_eq
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
      effectId_partialEq_lawful rust.effects ⟨logical, record.current_attempt⟩
    have contains := generated_contains_key_eq
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId effectId_partialEq_lawful rust.effects id
    have nonzero : record.remaining_retries ≠ 0#u64 := by scalar_tac
    have notMax : record.current_attempt ≠ core.num.U64.MAX := by
      intro same
      have valueSame := congrArg Std.UScalar.val same
      have maxValue : (core.num.U64.MAX).val = Std.U64.max := by
        simp [core.num.U64.MAX, Std.U64.max_eq, Std.U64.rMax]
      rw [maxValue] at valueSame
      exact (Nat.ne_of_lt attemptRoom) valueSame
    dsimp [id] at contains absent effectsEval keysEval inflightEval
    dsimp [updated] at logicalEval
    simp [SandboxControllerRust.effect.start_next_attempt, getLogical, found,
      getFailed, failed, generated_effectStatusOption_ne_self, generated_failureRetryable,
      retryable, nonzero, notMax, incrementEval, contains, absent, decrementEval,
      world_clone_eq, effectsEval, keysEval, logicalEval, inflightEval]
  · unfold startNextAttempt
    rw [nextData]
    dsimp only
    apply worldRep_setEffectProtocol represented logicalEffects _ logicalRep effects _ effectsRep keys _
    · have representedKeys : rust.effect_keys.val.map effectId = model.effectKeys := represented.effectKeys
      rw [keysValue, map_insertUnique_of_injective effectId_injective, representedKeys]
    · rw [keysValue]
      exact nodup_insertUnique id _ represented.effectKeysUnique
    · intro candidate
      have representedInflight : candidate ∈ rust.inflight.val ↔ model.inflight (effectId candidate) :=
        represented.inflight candidate
      rw [inflightValue, mem_insertUnique, representedInflight]
      simp [addEffect, effectId_injective.eq_iff]
    · intro candidate present
      rcases present with same | previous
      · exact ⟨id, same.symm⟩
      · exact represented.inflightSupport candidate previous
    · rw [inflightValue]
      exact nodup_insertUnique id _ represented.inflightUnique

private theorem failure_ne_self (failure : SandboxControllerRust.model.FailureKind) :
    core.cmp.PartialEq.ne.trait_default
      SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind failure failure = ok false := by
  cases failure <;> simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
    SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq,
    SandboxControllerRust.model.FailureKind.read_discriminant]

private theorem model_retry_accepts {failure : FailureKind}
    (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (unfinished : record.result = none)
    (failed : model.effects ⟨logical, record.currentAttempt⟩ = some (.acknowledged (.failed failure)))
    (retryable : failure.retryable = true)
    (budget : record.remainingRetries = remaining + 1)
    (fresh : model.effects ⟨logical, record.currentAttempt + 1⟩ = none) :
    ∃ decision, SandboxModel.Controller.handleEffectRetry model logical failure = .ok decision ∧
      decision.event = .startNextAttempt ⟨logical, record.currentAttempt⟩ ⟨logical, record.currentAttempt + 1⟩ ∧
      decision.next = startNextAttempt model logical failure := by
  let decision : SandboxModel.Controller.Decision model :=
    ⟨.startNextAttempt ⟨logical, record.currentAttempt⟩ ⟨logical, record.currentAttempt + 1⟩,
      startNextAttempt model logical failure,
      Step.startNextAttempt failure up found unfinished rfl failed retryable budget rfl fresh⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  unfold SandboxModel.Controller.handleEffectRetry
  rw [dif_pos up]
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    split
    · simp_all
    · dsimp only
      split
      · rename_i observed observedEq
        have same : observed = failure := by simpa using observedEq.symm.trans failed
        subst observed
        rw [dif_pos rfl, dif_pos retryable]
        split
        · omega
        · split
          · simp_all
          · rfl
      · simp_all

/-- Successful retry correspondence includes the public handler's admission
checks and emitted previous/next attempt IDs, not only its update helper.
This theorem is restricted to the admitted branch, not every rejection. -/
theorem handleEffectRetry_accepted_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (room : HandlerMachineHeadroom rust)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (record : SandboxControllerRust.model.LogicalEffectRecord)
    (failure : SandboxControllerRust.model.FailureKind)
    (up : rust.controller_up = true)
    (found : storeLookup rust.logical_effects logical = some record)
    (unfinished : record.result = none)
    (failed : storeLookup rust.effects ⟨logical, record.current_attempt⟩ =
      some (.Acknowledged (.Failed failure)))
    (retryable : (failureKind failure).retryable = true)
    (budget : 0 < record.remaining_retries.val)
    (attemptRoom : U64Headroom record.current_attempt)
    (fresh : model.effects ⟨logicalEffectId logical, record.current_attempt.val + 1⟩ = none) :
    ∃ decision, SandboxControllerRust.effect.handle_effect_retry rust logical failure = ok (.Ok decision) ∧
      EffectResultRep model (.Ok decision) (SandboxModel.Controller.handleEffectRetry model
        (logicalEffectId logical) (failureKind failure)) := by
  obtain ⟨next, nextEval, nextRep⟩ := generated_startNextAttempt_refines rust model represented room
    logical record failure found failed retryable budget attemptRoom fresh
  have increment : (record.current_attempt + 1#u64)
      ⦃ next => next.val = record.current_attempt.val + 1 ⦄ := by
    unfold U64Headroom at attemptRoom
    step
    assumption
  obtain ⟨attempt, incrementEval, attemptValue⟩ := Aeneas.Std.WP.spec_imp_exists increment
  let id : SandboxControllerRust.model.EffectId := ⟨logical, attempt⟩
  have idValue : effectId id = ⟨logicalEffectId logical, record.current_attempt.val + 1⟩ := by
    simp [id, effectId, u64ToNat, attemptValue]
  have absent : storeLookup rust.effects id = none := by
    have agrees := effectLookup_agrees represented id
    rw [idValue, fresh] at agrees
    cases lookup : storeLookup rust.effects id <;> simp_all [OptionRel]
  have getLogical := generated_get_eq
    SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    logicalEffectId_partialEq_lawful rust.logical_effects logical
  have getFailed := generated_get_eq
    SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
    effectId_partialEq_lawful rust.effects ⟨logical, record.current_attempt⟩
  have contains := generated_contains_key_eq
    SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId effectId_partialEq_lawful rust.effects id
  have nonzero : record.remaining_retries ≠ 0#u64 := by scalar_tac
  have notMax : record.current_attempt ≠ core.num.U64.MAX := by
    intro same
    have valueSame := congrArg Std.UScalar.val same
    have maxValue : (core.num.U64.MAX).val = Std.U64.max := by
      simp [core.num.U64.MAX, Std.U64.max_eq, Std.U64.rMax]
    rw [maxValue] at valueSame
    exact (Nat.ne_of_lt attemptRoom) valueSame
  refine ⟨⟨.StartNextAttempt ⟨logical, record.current_attempt⟩ id, next⟩, ?_, ?_⟩
  · dsimp [id] at contains absent ⊢
    simp [SandboxControllerRust.effect.handle_effect_retry, up, getLogical, found, unfinished,
      getFailed, failed, failure_ne_self, generated_failureRetryable, retryable, nonzero,
      notMax, incrementEval, contains, absent, nextEval]
  · have modelUp := represented.controllerUp.symm.trans up
    have modelFound := logicalLookup_some represented logical record found
    have modelFailed := effectLookup_some represented ⟨logical, record.current_attempt⟩ _ failed
    have modelUnfinished : (logicalEffectRecord record).result = none := by
      simp [logicalEffectRecord, unfinished]
    have modelBudget : (logicalEffectRecord record).remainingRetries = (record.remaining_retries.val - 1) + 1 := by
      change record.remaining_retries.val = _
      omega
    obtain ⟨decision, evaluated, eventSame, worldSame⟩ :=
      model_retry_accepts modelUp modelFound modelUnfinished modelFailed retryable modelBudget fresh
    rw [evaluated]
    change modelEvent (.StartNextAttempt ⟨logical, record.current_attempt⟩ id) = decision.event ∧
      EffectWorldRep next decision.next
    rw [eventSame, worldSame]
    constructor
    · simp [modelEvent, effectId, id, logicalEffectRecord, u64ToNat, attemptValue]
    · exact nextRep

/-- At the finite attempt limit, a valid retry is explicitly refused before
addition.  It cannot wrap around to attempt zero.  This concrete rejection
has no counterpart in the unbounded natural-number model. -/
theorem handleEffectRetry_counterExhausted
    (rust : SandboxControllerRust.model.World)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (record : SandboxControllerRust.model.LogicalEffectRecord)
    (failure : SandboxControllerRust.model.FailureKind)
    (up : rust.controller_up = true)
    (found : storeLookup rust.logical_effects logical = some record)
    (unfinished : record.result = none)
    (failed : storeLookup rust.effects ⟨logical, record.current_attempt⟩ =
      some (.Acknowledged (.Failed failure)))
    (retryable : (failureKind failure).retryable = true)
    (budget : record.remaining_retries ≠ 0#u64)
    (atLimit : record.current_attempt = core.num.U64.MAX) :
    SandboxControllerRust.effect.handle_effect_retry rust logical failure = ok (.Err .CounterExhausted) := by
  have getLogical := generated_get_eq
    SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    logicalEffectId_partialEq_lawful rust.logical_effects logical
  have getFailed := generated_get_eq
    SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
    effectId_partialEq_lawful rust.effects ⟨logical, record.current_attempt⟩
  rw [atLimit] at getFailed failed
  simp [SandboxControllerRust.effect.handle_effect_retry, up, getLogical, found, unfinished,
    getFailed, failed, failure_ne_self, generated_failureRetryable, retryable, budget, atLimit]

end SandboxModel.RustRefinement
