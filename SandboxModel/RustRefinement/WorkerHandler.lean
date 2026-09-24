import SandboxModel.RustRefinement.HandlerValues
import SandboxModel.RustRefinement.Clone
import SandboxModel.WorkerCompleteness

/-! Refinement support for the worker reducer.  The worker's first dynamic
guard scans the finite durable outbox.  The loop proof below connects that
actual generated scan to ordinary list membership; later handler branches use
this theorem instead of assuming transport membership. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result ControlFlow Error

local instance workerHandlerOperationKeyDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance workerHandlerLogicalEffectIdDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance workerHandlerEffectIdDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

abbrev WorkerWorldRep :=
  @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
    effectIdDecidableEq valueRelations

def WorkerResultRep (model : World)
    (rustResult : core.result.Result SandboxControllerRust.worker.WorkerDecision
      SandboxControllerRust.worker.WorkerReject)
    (modelResult : Except SandboxModel.Controller.WorkerReject
      (SandboxModel.Controller.Decision model)) : Prop :=
  match rustResult, modelResult with
  | .Err rustReject, .error modelReject =>
      workerReject rustReject = modelReject
  | .Ok rustDecision, .ok modelDecision =>
      workerEvent rustDecision.event = modelDecision.event ∧
        WorkerWorldRep rustDecision.next modelDecision.next
  | _, _ => False

theorem generated_workerContains_loop_refines [DecidableEq T]
    (partialEq : core.cmp.PartialEq T T) (lawful : LawfulPartialEq partialEq)
    (values : Slice T) (value : T) (start : Std.Usize)
    (startBound : start.val ≤ values.val.length) :
    SandboxControllerRust.worker.contains_loop partialEq values value start
      ⦃ found => found = decide (value ∈ values.val.drop start.val) ⦄ := by
  unfold SandboxControllerRust.worker.contains_loop
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index => values.val.length - index.val)
    (fun index =>
      index.val ≤ values.val.length ∧
        decide (value ∈ values.val.drop index.val) =
          decide (value ∈ values.val.drop start.val))
    (fun found => found = decide (value ∈ values.val.drop start.val))
  · intro index invariant
    unfold SandboxControllerRust.worker.contains_loop.body
    dsimp only
    split
    · rename_i inBounds
      have indexBound : index.val < values.val.length := by
        simpa [Slice.length] using inBounds
      step as ⟨ current, currentPost ⟩
      have currentEq : current = values.val[index.val] := by
        simpa using currentPost
      subst current
      simp only [core.cmp.impls.PartialEqShared.eq]
      rw [lawful values.val[index.val] value]
      by_cases same : values.val[index.val] = value
      · simp [same]
        rcases invariant with ⟨_, membershipEq⟩
        have memberCurrent : value ∈ values.val.drop index.val := by
          rw [List.drop_eq_getElem_cons indexBound]
          simp [same]
        simpa [memberCurrent] using membershipEq.symm
      · simp [same]
        step as ⟨ next, nextPost ⟩
        rcases invariant with ⟨upper, membershipEq⟩
        have valueDifferent : value ≠ values.val[index.val] := by
          exact fun equal => same equal.symm
        constructor
        · scalar_tac
        · constructor
          · have tailMembership :
                decide (value ∈ values.val.drop (index.val + 1)) =
                  decide (value ∈ values.val.drop start.val) := by
              rw [List.drop_eq_getElem_cons indexBound] at membershipEq
              simpa only [List.mem_cons, valueDifferent, false_or] using
                membershipEq
            simpa [nextPost] using tailMembership
          · omega
    · rename_i outOfBounds
      simp only [WP.spec_ok]
      rcases invariant with ⟨upper, membershipEq⟩
      have lower : values.val.length ≤ index.val := by
        simpa [Slice.length] using outOfBounds
      have dropEmpty : values.val.drop index.val = [] :=
        List.drop_eq_nil_of_le lower
      rw [dropEmpty] at membershipEq
      simpa using membershipEq.symm
  · exact ⟨startBound, rfl⟩

theorem generated_workerContains_refines [DecidableEq T]
    (partialEq : core.cmp.PartialEq T T) (lawful : LawfulPartialEq partialEq)
    (values : Slice T) (value : T) :
    SandboxControllerRust.worker.contains partialEq values value
      ⦃ found => found = decide (value ∈ values.val) ⦄ := by
  unfold SandboxControllerRust.worker.contains
  simpa using generated_workerContains_loop_refines partialEq lawful values
    value 0#usize (by simp)

/-- Specialization used by the real worker entry point. -/
theorem generated_workerContainsEffect_refines
    (values : Slice SandboxControllerRust.model.EffectId)
    (value : SandboxControllerRust.model.EffectId) :
    SandboxControllerRust.worker.contains
        SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
        values value
      ⦃ found => found = decide (value ∈ values.val) ⦄ :=
  generated_workerContains_refines _ effectId_partialEq_lawful values value

/-- The generated worker derives exactly the same current attempt identifier
    from the logical-effect ledger as the model. -/
theorem generated_currentEffectId_refines
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : WorkerWorldRep rust model)
    (concreteCommand : SandboxControllerRust.model.Command) :
    SandboxControllerRust.worker.current_effect_id rust concreteCommand
      ⦃ id => effectId id =
        SandboxModel.currentEffectId model (command concreteCommand) ⦄ := by
  let concreteLogical : SandboxControllerRust.model.LogicalEffectId :=
    { command := concreteCommand }
  have agrees := represented.logicalEffects.lookupAgrees concreteLogical
  change OptionRel (fun concrete abstract => logicalEffectRecord concrete = abstract)
    (storeLookup rust.logical_effects concreteLogical)
    (model.logicalEffects (logicalEffectId concreteLogical)) at agrees
  obtain ⟨found, foundEval, foundEq⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_get_copy_refines
      SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      logicalEffectId_partialEq_lawful
      SandboxControllerRust.model.LogicalEffectRecord.Insts.CoreMarkerCopy
      rust.logical_effects concreteLogical)
  subst found
  have evaluation : SandboxControllerRust.worker.current_effect_id rust concreteCommand =
      ok (match storeLookup rust.logical_effects concreteLogical with
        | none => { logical := concreteLogical, attempt := 0#u64 }
        | some record => { logical := concreteLogical, attempt := record.current_attempt }) := by
    simp [SandboxControllerRust.worker.current_effect_id, concreteLogical,
      show SandboxControllerRust.store.Store.get_copy
          SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
          SandboxControllerRust.model.LogicalEffectRecord.Insts.CoreMarkerCopy
          rust.logical_effects { command := concreteCommand } =
            ok (storeLookup rust.logical_effects concreteLogical) from foundEval]
    split <;> simp_all
  rw [evaluation]
  simp only [WP.spec_ok]
  cases concreteLookup : storeLookup rust.logical_effects concreteLogical with
  | none =>
      rw [concreteLookup] at agrees
      cases modelLookup : model.logicalEffects
          (logicalEffectId concreteLogical) with
      | none =>
          change model.logicalEffects
            (SandboxModel.logicalEffectId (command concreteCommand)) = none at modelLookup
          simp only [SandboxModel.currentEffectId, modelLookup]
          rfl
      | some record => simp [OptionRel, modelLookup] at agrees
  | some concreteRecord =>
      rw [concreteLookup] at agrees
      cases modelLookup : model.logicalEffects
          (logicalEffectId concreteLogical) with
      | none => simp [OptionRel, modelLookup] at agrees
      | some modelRecord =>
          have recordEq : logicalEffectRecord concreteRecord = modelRecord := by
            simpa [OptionRel, modelLookup] using agrees
          change model.logicalEffects
            (SandboxModel.logicalEffectId (command concreteCommand)) = some modelRecord at modelLookup
          simp only [SandboxModel.currentEffectId, modelLookup]
          rw [← recordEq]
          rfl

/-- An obsolete attempt is rejected by the generated worker before transport
    membership or any resource operation is inspected. -/
theorem handleWorker_stale_rejected
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : WorkerWorldRep rust model)
    (id : SandboxControllerRust.model.EffectId)
    (stale : SandboxModel.currentEffectId model (command id.logical.command) ≠ effectId id) :
    SandboxControllerRust.worker.handle_worker rust id = ok (.Err .StaleAttempt) := by
  obtain ⟨current, currentEval, currentSame⟩ := WP.spec_imp_exists
    (generated_currentEffectId_refines represented id.logical.command)
  have different : current ≠ id := by
    intro same
    apply stale
    rw [← currentSame, same]
  have compare := effectId_partialEq_lawful current id
  have notEqual : core.cmp.PartialEq.ne.trait_default
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId current id = ok true := by
    unfold core.cmp.PartialEq.ne.trait_default core.cmp.PartialEq.ne.default
    rw [compare]
    simp [different]
  simp [SandboxControllerRust.worker.handle_worker, currentEval,
    notEqual]

/-- The second worker guard rejects an exact current attempt that has not
    been queued, including attempts absent from the effect ledger. -/
theorem handleWorker_unqueued_rejected
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : WorkerWorldRep rust model)
    (id : SandboxControllerRust.model.EffectId)
    (head : SandboxModel.currentEffectId model (command id.logical.command) = effectId id)
    (absent : ¬ model.inflight (effectId id)) :
    SandboxControllerRust.worker.handle_worker rust id = ok (.Err .NotQueued) := by
  obtain ⟨current, currentEval, currentSame⟩ := WP.spec_imp_exists
    (generated_currentEffectId_refines represented id.logical.command)
  have same : current = id := effectId_injective (currentSame.trans head)
  subst current
  have notEqual : core.cmp.PartialEq.ne.trait_default
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId id id = ok false := by
    unfold core.cmp.PartialEq.ne.trait_default core.cmp.PartialEq.ne.default
    rw [effectId_partialEq_lawful id id]
    simp
  have notMember : id ∉ rust.inflight.val := by
    intro member
    exact absent ((represented.inflight id).mp member)
  obtain ⟨found, foundEval, foundSame⟩ := WP.spec_imp_exists
    (generated_workerContainsEffect_refines (alloc.vec.Vec.deref rust.inflight) id)
  have foundFalse : found = false := by
    simpa [alloc.vec.Vec.deref, notMember] using foundSame
  rw [foundFalse] at foundEval
  simp [SandboxControllerRust.worker.handle_worker, currentEval,
    notEqual, foundEval]

end SandboxModel.RustRefinement
