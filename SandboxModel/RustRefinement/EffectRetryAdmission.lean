import SandboxModel.RustRefinement.EffectRetry

/-! Complete admission correspondence for retries, preserving guard order. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance retryAdmissionOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance retryAdmissionLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance retryAdmissionEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

private theorem failure_ne_eq (left right : SandboxControllerRust.model.FailureKind) :
    core.cmp.PartialEq.ne.trait_default
      SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind left right =
      ok (decide (failureKind left ≠ failureKind right)) := by
  cases left <;> cases right <;>
    simp [core.cmp.PartialEq.ne.trait_default, core.cmp.PartialEq.ne.default,
      SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq,
      SandboxControllerRust.model.FailureKind.read_discriminant, failureKind]

private theorem failure_retryable_eq (failure : SandboxControllerRust.model.FailureKind) :
    SandboxControllerRust.model.FailureKind.retryable failure = ok (failureKind failure).retryable := by
  cases failure <;> rfl

private theorem failure_injective : Function.Injective failureKind := by
  intro left right same
  cases left <;> cases right <;> simp_all [failureKind]

private theorem model_retry_missing {failure : FailureKind} (up : model.controllerUp = true)
    (missing : model.logicalEffects logical = none) :
    SandboxModel.Controller.handleEffectRetry model logical failure =
      .error .logicalEffectNotFound := by
  unfold SandboxModel.Controller.handleEffectRetry
  rw [dif_pos up]
  split <;> simp_all

private theorem model_retry_alreadyFinal {failure : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (final : record.result = some result) :
    SandboxModel.Controller.handleEffectRetry model logical failure =
      .error .logicalEffectAlreadyFinal := by
  unfold SandboxModel.Controller.handleEffectRetry
  rw [dif_pos up]
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    split <;> simp_all

private theorem model_retry_notFailed {failure : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (unfinished : record.result = none)
    (notFailed : ∀ observed,
      model.effects ⟨logical, record.currentAttempt⟩ ≠ some (.acknowledged (.failed observed))) :
    SandboxModel.Controller.handleEffectRetry model logical failure =
      .error .currentAttemptNotFailed := by
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
      · rename_i observed failed
        exact (notFailed observed failed).elim
      · rfl

private theorem model_retry_mismatch {failure observed : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (unfinished : record.result = none)
    (failed : model.effects ⟨logical, record.currentAttempt⟩ = some (.acknowledged (.failed observed)))
    (different : observed ≠ failure) :
    SandboxModel.Controller.handleEffectRetry model logical failure =
      .error .failureMismatch := by
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
      · rename_i value valueEq
        have same : value = observed := by simpa using valueEq.symm.trans failed
        subst value
        rw [dif_neg different]
      · simp_all

private theorem generated_retry_notFailed
    (rust : SandboxControllerRust.model.World)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (failure : SandboxControllerRust.model.FailureKind)
    (record : SandboxControllerRust.model.LogicalEffectRecord)
    (up : rust.controller_up = true)
    (found : storeLookup rust.logical_effects logical = some record)
    (unfinished : record.result = none)
    (notFailed : ∀ observed, storeLookup rust.effects
      { logical := logical, attempt := record.current_attempt } ≠ some (.Acknowledged (.Failed observed))) :
    SandboxControllerRust.effect.handle_effect_retry rust logical failure =
      ok (.Err .CurrentAttemptNotFailed) := by
  have logicalGet := generated_get_eq
    SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    logicalEffectId_partialEq_lawful rust.logical_effects logical
  have effectGet := generated_get_eq
    SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
    effectId_partialEq_lawful rust.effects
    { logical := logical, attempt := record.current_attempt }
  cases effectFound : storeLookup rust.effects
      { logical := logical, attempt := record.current_attempt } with
  | none =>
      simp [SandboxControllerRust.effect.handle_effect_retry, up,
        logicalGet, found, unfinished, effectGet, effectFound]
  | some status =>
      cases status with
      | Committed | Dispatched | OutcomeUnknown =>
          simp [SandboxControllerRust.effect.handle_effect_retry, up,
            logicalGet, found, unfinished, effectGet, effectFound]
      | Acknowledged outcome =>
          cases outcome with
          | Failed observed => exact (notFailed observed effectFound).elim
          | Succeeded digest =>
              simp [SandboxControllerRust.effect.handle_effect_retry, up,
                logicalGet, found, unfinished, effectGet, effectFound]

private theorem model_retry_notRetryable {failure : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (unfinished : record.result = none)
    (failed : model.effects ⟨logical, record.currentAttempt⟩ = some (.acknowledged (.failed failure)))
    (notRetryable : failure.retryable ≠ true) :
    SandboxModel.Controller.handleEffectRetry model logical failure = .error .failureNotRetryable := by
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
        rw [dif_pos rfl]
        rw [dif_neg notRetryable]
      · simp_all

private theorem model_retry_noBudget {failure : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (unfinished : record.result = none)
    (failed : model.effects ⟨logical, record.currentAttempt⟩ = some (.acknowledged (.failed failure)))
    (retryable : failure.retryable = true) (budget : record.remainingRetries = 0) :
    SandboxModel.Controller.handleEffectRetry model logical failure = .error .retryBudgetExhausted := by
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
        rw [dif_pos rfl]
        rw [dif_pos retryable]
        split <;> simp_all
      · simp_all

private theorem model_retry_nextExists {failure : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (unfinished : record.result = none)
    (failed : model.effects ⟨logical, record.currentAttempt⟩ = some (.acknowledged (.failed failure)))
    (retryable : failure.retryable = true) (budget : record.remainingRetries ≠ 0)
    (occupied : model.effects ⟨logical, record.currentAttempt + 1⟩ = some status) :
    SandboxModel.Controller.handleEffectRetry model logical failure = .error .nextAttemptExists := by
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
        rw [dif_pos rfl]
        rw [dif_pos retryable]
        split
        · simp_all
        · split <;> simp_all
      · simp_all


/-- All retry outcomes within the explicit finite-machine boundary, including
    every ordered refusal and the complete successor on acceptance. -/
theorem handleEffectRetry_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (headroom : HandlerMachineHeadroom rust)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (failure : SandboxControllerRust.model.FailureKind) :
    ∃ result, SandboxControllerRust.effect.handle_effect_retry rust logical failure = ok result ∧
      EffectResultRep model result (SandboxModel.Controller.handleEffectRetry model
        (logicalEffectId logical) (failureKind failure)) := by
  have logicalGet := generated_get_eq
    SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    logicalEffectId_partialEq_lawful rust.logical_effects logical
  by_cases up : rust.controller_up = true
  · have modelUp : model.controllerUp = true := represented.controllerUp.symm.trans up
    cases logicalFound : storeLookup rust.logical_effects logical with
    | none =>
        have modelFound : model.logicalEffects (logicalEffectId logical) = none := by
          have agrees := represented.logicalEffects.lookupAgrees logical
          change OptionRel (fun concrete abstract => logicalEffectRecord concrete = abstract)
            (storeLookup rust.logical_effects logical) (model.logicalEffects (logicalEffectId logical)) at agrees
          rw [logicalFound] at agrees
          cases abstractFound : model.logicalEffects (logicalEffectId logical) <;> simp_all [OptionRel]
        refine ⟨.Err .LogicalEffectNotFound, ?_, ?_⟩
        · simp [SandboxControllerRust.effect.handle_effect_retry, up, logicalGet, logicalFound]
        · rw [model_retry_missing modelUp modelFound]
          rfl
    | some record =>
        have modelFound := logicalLookup_some represented logical record logicalFound
        cases final : record.result with
        | some result =>
            refine ⟨.Err .LogicalEffectAlreadyFinal, ?_, ?_⟩
            · simp [SandboxControllerRust.effect.handle_effect_retry, up, logicalGet, logicalFound, final]
            · rw [model_retry_alreadyFinal modelUp modelFound
                (show (logicalEffectRecord record).result = some (logicalEffectResult result) by
                  simp [logicalEffectRecord, final])]
              rfl
        | none =>
            have unfinished : (logicalEffectRecord record).result = none := by
              simp [logicalEffectRecord, final]
            let current : SandboxControllerRust.model.EffectId :=
              { logical := logical, attempt := record.current_attempt }
            have effectGet := generated_get_eq
              SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
              effectId_partialEq_lawful rust.effects current
            by_cases failed : ∃ observed, storeLookup rust.effects current = some (.Acknowledged (.Failed observed))
            · obtain ⟨observed, effectFound⟩ := failed
              have modelEffect := effectLookup_some represented current (.Acknowledged (.Failed observed)) effectFound
              change model.effects ⟨logicalEffectId logical, (logicalEffectRecord record).currentAttempt⟩ =
                some (.acknowledged (.failed (failureKind observed))) at modelEffect
              by_cases same : failureKind observed = failureKind failure
              · have concreteSame := failure_injective same
                subst observed
                by_cases retryable : (failureKind failure).retryable = true
                · by_cases exhausted : record.remaining_retries.val = 0
                  · refine ⟨.Err .RetryBudgetExhausted, ?_, ?_⟩
                    · have zero : record.remaining_retries = 0#u64 := by scalar_tac
                      simp [SandboxControllerRust.effect.handle_effect_retry, up, logicalGet,
                        logicalFound, final, effectGet, effectFound, current, failure_ne_eq,
                        generated_failureRetryable, retryable, zero]
                    · rw [model_retry_noBudget modelUp modelFound unfinished modelEffect retryable exhausted]
                      rfl
                  · have budget : 0 < record.remaining_retries.val := by omega
                    have attemptRoom : U64Headroom record.current_attempt :=
                      headroom.logicalAttempt (logical, record) (storeLookup_some_mem logicalFound)
                    have increment : (record.current_attempt + 1#u64)
                        ⦃ next => next.val = record.current_attempt.val + 1 ⦄ := by
                      unfold U64Headroom at attemptRoom
                      step
                      assumption
                    obtain ⟨attempt, incrementEval, attemptValue⟩ := Aeneas.Std.WP.spec_imp_exists increment
                    let nextId : SandboxControllerRust.model.EffectId := ⟨logical, attempt⟩
                    have nextValue : effectId nextId =
                        ⟨logicalEffectId logical, (logicalEffectRecord record).currentAttempt + 1⟩ := by
                      simp [effectId, nextId, logicalEffectRecord, u64ToNat, attemptValue]
                    have contains := generated_contains_key_eq
                      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
                      effectId_partialEq_lawful rust.effects nextId
                    cases nextFound : storeLookup rust.effects nextId with
                    | none =>
                        have fresh : model.effects
                            ⟨logicalEffectId logical, record.current_attempt.val + 1⟩ = none := by
                          have agrees := effectLookup_agrees represented nextId
                          rw [nextValue, nextFound] at agrees
                          cases lookup : model.effects
                              ⟨logicalEffectId logical, record.current_attempt.val + 1⟩ <;>
                            simp_all [OptionRel, logicalEffectRecord, u64ToNat]
                        obtain ⟨decision, evaluated, related⟩ := handleEffectRetry_accepted_refines
                          rust model represented headroom logical record failure up logicalFound
                          final effectFound retryable budget attemptRoom fresh
                        exact ⟨.Ok decision, evaluated, related⟩
                    | some status =>
                        have occupied := effectLookup_some represented nextId status nextFound
                        rw [nextValue] at occupied
                        have nonzero : record.remaining_retries ≠ 0#u64 := by scalar_tac
                        have notMax : record.current_attempt ≠ core.num.U64.MAX := by
                          intro same
                          have valueSame := congrArg Std.UScalar.val same
                          have maxValue : (core.num.U64.MAX).val = Std.U64.max := by
                            simp [core.num.U64.MAX, Std.U64.max_eq, Std.U64.rMax]
                          rw [maxValue] at valueSame
                          exact (Nat.ne_of_lt attemptRoom) valueSame
                        refine ⟨.Err .NextAttemptExists, ?_, ?_⟩
                        · dsimp [nextId] at contains nextFound
                          simp [SandboxControllerRust.effect.handle_effect_retry, up, logicalGet,
                            logicalFound, final, effectGet, effectFound, current, failure_ne_eq,
                            generated_failureRetryable, retryable, nonzero, notMax, incrementEval,
                            contains, nextFound]
                        · rw [model_retry_nextExists modelUp modelFound unfinished modelEffect
                            retryable exhausted occupied]
                          rfl
                · refine ⟨.Err .FailureNotRetryable, ?_, ?_⟩
                  · simp [SandboxControllerRust.effect.handle_effect_retry, up, logicalGet,
                      logicalFound, final, effectGet, effectFound, current, failure_ne_eq,
                      generated_failureRetryable, retryable]
                  · rw [model_retry_notRetryable modelUp modelFound unfinished modelEffect retryable]
                    rfl
              · refine ⟨.Err .FailureMismatch, ?_, ?_⟩
                · simp [SandboxControllerRust.effect.handle_effect_retry, up, logicalGet,
                    logicalFound, final, effectGet, effectFound, current, failure_ne_eq, same]
                · rw [model_retry_mismatch modelUp modelFound unfinished modelEffect same]
                  rfl
            · have notFailed : ∀ observed, storeLookup rust.effects current ≠ some (.Acknowledged (.Failed observed)) := by
                intro observed found
                exact failed ⟨observed, found⟩
              have modelNotFailed : ∀ observed, model.effects
                  ⟨logicalEffectId logical, (logicalEffectRecord record).currentAttempt⟩ ≠
                    some (.acknowledged (.failed observed)) := by
                intro observed abstractFound
                have agrees := effectLookup_agrees represented current
                change OptionRel (fun concrete abstract => effectStatus concrete = abstract)
                  (storeLookup rust.effects current)
                  (model.effects ⟨logicalEffectId logical, (logicalEffectRecord record).currentAttempt⟩) at agrees
                rw [abstractFound] at agrees
                cases concreteFound : storeLookup rust.effects current with
                | none => simp [concreteFound, OptionRel] at agrees
                | some status =>
                    cases status with
                    | Committed | Dispatched | OutcomeUnknown =>
                        simp [concreteFound, OptionRel, effectStatus] at agrees
                    | Acknowledged outcome =>
                        cases outcome with
                        | Succeeded digest => simp [concreteFound, OptionRel, effectStatus, effectOutcome] at agrees
                        | Failed reason => exact notFailed reason concreteFound
              refine ⟨.Err .CurrentAttemptNotFailed, ?_, ?_⟩
              · exact generated_retry_notFailed rust logical failure record up logicalFound final notFailed
              · rw [model_retry_notFailed modelUp modelFound unfinished modelNotFailed]
                rfl
  · have down : rust.controller_up = false := by cases h : rust.controller_up <;> simp_all
    have modelDown : model.controllerUp = false := represented.controllerUp.symm.trans down
    refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp [SandboxControllerRust.effect.handle_effect_retry, down]
    · simp [EffectResultRep, SandboxModel.Controller.handleEffectRetry, modelDown, effectReject?]


end SandboxModel.RustRefinement
