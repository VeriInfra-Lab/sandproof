import SandboxModel.RustRefinement.EffectHandler

/-! Exact admission and successor correspondence for finalizing a failed
attempt.  In particular, a retryable failure with budget left is rejected. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance finalizeOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance finalizeLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance finalizeEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
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

private theorem model_finalize_missing {failure : FailureKind} (up : model.controllerUp = true)
    (missing : model.logicalEffects logical = none) :
    SandboxModel.Controller.handleEffectFinalizeFailure model logical failure =
      .error .logicalEffectNotFound := by
  unfold SandboxModel.Controller.handleEffectFinalizeFailure
  rw [dif_pos up]
  split <;> simp_all

private theorem model_finalize_alreadyFinal {failure : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (final : record.result = some result) :
    SandboxModel.Controller.handleEffectFinalizeFailure model logical failure =
      .error .logicalEffectAlreadyFinal := by
  unfold SandboxModel.Controller.handleEffectFinalizeFailure
  rw [dif_pos up]
  split
  · simp_all
  · rename_i selected selectedEq
    have same := Option.some.inj (selectedEq.symm.trans found)
    subst selected
    split <;> simp_all

private theorem model_finalize_notFailed {failure : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (unfinished : record.result = none)
    (notFailed : ∀ observed,
      model.effects ⟨logical, record.currentAttempt⟩ ≠ some (.acknowledged (.failed observed))) :
    SandboxModel.Controller.handleEffectFinalizeFailure model logical failure =
      .error .currentAttemptNotFailed := by
  unfold SandboxModel.Controller.handleEffectFinalizeFailure
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

private theorem model_finalize_mismatch {failure observed : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (unfinished : record.result = none)
    (failed : model.effects ⟨logical, record.currentAttempt⟩ = some (.acknowledged (.failed observed)))
    (different : observed ≠ failure) :
    SandboxModel.Controller.handleEffectFinalizeFailure model logical failure =
      .error .failureMismatch := by
  unfold SandboxModel.Controller.handleEffectFinalizeFailure
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

private theorem model_finalize_retryAvailable {failure : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (unfinished : record.result = none)
    (failed : model.effects ⟨logical, record.currentAttempt⟩ = some (.acknowledged (.failed failure)))
    (retryable : failure.retryable = true) (budget : record.remainingRetries ≠ 0) :
    SandboxModel.Controller.handleEffectFinalizeFailure model logical failure =
      .error .retryStillAvailable := by
  unfold SandboxModel.Controller.handleEffectFinalizeFailure
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
        have same : value = failure := by simpa using valueEq.symm.trans failed
        subst value
        rw [dif_pos rfl]
        split
        · simp_all
        · split <;> simp_all
      · simp_all

private theorem model_finalize_accepted {failure : FailureKind} (up : model.controllerUp = true)
    (found : model.logicalEffects logical = some record)
    (unfinished : record.result = none)
    (failed : model.effects ⟨logical, record.currentAttempt⟩ = some (.acknowledged (.failed failure)))
    (terminal : failure.retryable = false ∨ record.remainingRetries = 0) :
    ∃ decision, SandboxModel.Controller.handleEffectFinalizeFailure model logical failure = .ok decision ∧
      decision.event = .finalizeEffectFailure ⟨logical, record.currentAttempt⟩ failure ∧
      decision.next = SandboxModel.finalizeLogicalFailure model logical failure := by
  let decision : SandboxModel.Controller.Decision model :=
    ⟨.finalizeEffectFailure ⟨logical, record.currentAttempt⟩ failure,
      SandboxModel.finalizeLogicalFailure model logical failure,
      Step.finalizeEffectFailure failure up found unfinished rfl failed terminal⟩
  refine ⟨decision, ?_, rfl, rfl⟩
  unfold SandboxModel.Controller.handleEffectFinalizeFailure
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
        have same : value = failure := by simpa using valueEq.symm.trans failed
        subst value
        rw [dif_pos rfl]
        split
        · rfl
        · rename_i classification
          split
          · rfl
          · rename_i remaining budget
            have exhausted : record.remainingRetries = 0 :=
              terminal.resolve_left (by simp [classification])
            omega
      · simp_all

private theorem generated_finalize_notFailed
    (rust : SandboxControllerRust.model.World)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (failure : SandboxControllerRust.model.FailureKind)
    (record : SandboxControllerRust.model.LogicalEffectRecord)
    (up : rust.controller_up = true)
    (found : storeLookup rust.logical_effects logical = some record)
    (unfinished : record.result = none)
    (notFailed : ∀ observed, storeLookup rust.effects
      { logical := logical, attempt := record.current_attempt } ≠ some (.Acknowledged (.Failed observed))) :
    SandboxControllerRust.effect.handle_effect_finalize_failure rust logical failure =
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
      simp [SandboxControllerRust.effect.handle_effect_finalize_failure, up,
        logicalGet, found, unfinished, effectGet, effectFound]
  | some status =>
      cases status with
      | Committed | Dispatched | OutcomeUnknown =>
          simp [SandboxControllerRust.effect.handle_effect_finalize_failure, up,
            logicalGet, found, unfinished, effectGet, effectFound]
      | Acknowledged outcome =>
          cases outcome with
          | Failed observed => exact (notFailed observed effectFound).elim
          | Succeeded digest =>
              simp [SandboxControllerRust.effect.handle_effect_finalize_failure, up,
                logicalGet, found, unfinished, effectGet, effectFound]

/-- Complete final-failure handling: every ordered rejection, and the exact
    successor logical ledger on success.  Capacity is a concrete-machine
    precondition, not an assumption that the operation will succeed. -/
theorem handleEffectFinalize_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (headroom : StoreHeadroom rust.logical_effects)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (failure : SandboxControllerRust.model.FailureKind) :
    ∃ result, SandboxControllerRust.effect.handle_effect_finalize_failure rust logical failure = ok result ∧
      EffectResultRep model result (SandboxModel.Controller.handleEffectFinalizeFailure model
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
        · simp [SandboxControllerRust.effect.handle_effect_finalize_failure, up, logicalGet, logicalFound]
        · rw [model_finalize_missing modelUp modelFound]
          rfl
    | some record =>
        have modelFound := logicalLookup_some represented logical record logicalFound
        cases final : record.result with
        | some result =>
            refine ⟨.Err .LogicalEffectAlreadyFinal, ?_, ?_⟩
            · simp [SandboxControllerRust.effect.handle_effect_finalize_failure, up, logicalGet, logicalFound, final]
            · rw [model_finalize_alreadyFinal modelUp modelFound
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
                by_cases available : (failureKind failure).retryable = true ∧ (logicalEffectRecord record).remainingRetries ≠ 0
                · refine ⟨.Err .RetryStillAvailable, ?_, ?_⟩
                  · have budget : record.remaining_retries.val ≠ 0 := available.2
                    simp [SandboxControllerRust.effect.handle_effect_finalize_failure, up, logicalGet,
                      logicalFound, final, effectGet, effectFound, current,
                      failure_ne_eq, failure_retryable_eq, available.1, budget]
                  · rw [model_finalize_retryAvailable modelUp modelFound unfinished modelEffect available.1 available.2]
                    rfl
                · have terminal : (failureKind failure).retryable = false ∨ (logicalEffectRecord record).remainingRetries = 0 := by
                    cases classification : (failureKind failure).retryable <;> simp_all
                  obtain ⟨next, nextEval, nextRelated⟩ := generated_finalizeLogicalFailure_refines
                    represented headroom logical failure record logicalFound effectFound
                  let decision : SandboxControllerRust.effect.EffectDecision :=
                    { event := .FinalizeEffectFailure current failure, next := next }
                  refine ⟨.Ok decision, ?_, ?_⟩
                  · have noAvailable : ¬ ((failureKind failure).retryable = true ∧ record.remaining_retries.val ≠ 0) := available
                    simp [SandboxControllerRust.effect.handle_effect_finalize_failure, up, logicalGet,
                      logicalFound, final, effectGet, effectFound, current,
                      failure_ne_eq, failure_retryable_eq, nextEval, decision]
                    cases classification : (failureKind failure).retryable <;> simp_all
                  · obtain ⟨abstractDecision, evaluated, eventSame, worldSame⟩ :=
                      model_finalize_accepted modelUp modelFound unfinished modelEffect terminal
                    rw [evaluated]
                    change modelEvent decision.event = abstractDecision.event ∧
                      EffectWorldRep next abstractDecision.next
                    rw [eventSame, worldSame]
                    exact ⟨rfl, nextRelated⟩
              · refine ⟨.Err .FailureMismatch, ?_, ?_⟩
                · simp [SandboxControllerRust.effect.handle_effect_finalize_failure, up, logicalGet,
                    logicalFound, final, effectGet, effectFound, current, failure_ne_eq, same]
                · rw [model_finalize_mismatch modelUp modelFound unfinished modelEffect same]
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
              · exact generated_finalize_notFailed rust logical failure record up logicalFound final notFailed
              · rw [model_finalize_notFailed modelUp modelFound unfinished modelNotFailed]
                rfl
  · have down : rust.controller_up = false := by cases h : rust.controller_up <;> simp_all
    have modelDown : model.controllerUp = false := represented.controllerUp.symm.trans down
    refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp [SandboxControllerRust.effect.handle_effect_finalize_failure, down]
    · simp [EffectResultRep, SandboxModel.Controller.handleEffectFinalizeFailure, modelDown, effectReject?]

end SandboxModel.RustRefinement
