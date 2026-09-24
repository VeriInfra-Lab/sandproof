import SandboxModel.RustRefinement.Effect
import SandboxModel.RustRefinement.Clone
import SandboxModel.SchedulerCompleteness
import SandboxModel.HandlerCompleteness

/-!
End-to-end refinement of the generated Rust durable-effect handler.

The finite Rust implementation has one additional, explicit failure mode:
attempt-number exhaustion.  Every other rejection is shared with the
unbounded model.  Accepted decisions agree on their observable event and
preserve the complete persistent-world representation relation.
-/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance effectHandlerOperationKeyDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance effectHandlerLogicalEffectIdDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance effectHandlerEffectIdDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

abbrev EffectWorldRep :=
  @WorldRep operationKeyDecidableEq logicalEffectIdDecidableEq
    effectIdDecidableEq valueRelations

/-- Equality form of the generic generated-store lookup specification. -/
theorem generated_get_eq [DecidableEq K]
    (partialEq : core.cmp.PartialEq K K) (lawful : LawfulPartialEq partialEq)
    (store : SandboxControllerRust.store.Store K V) (key : K) :
    SandboxControllerRust.store.Store.get partialEq store key =
      ok (storeLookup store key) := by
  obtain ⟨value, evaluation, same⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_get_refines partialEq lawful store key)
  subst value
  exact evaluation

/-- Replacing only the effect store preserves the whole-world relation once
    the replacement store is related. -/
theorem worldRep_setEffects
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : EffectWorldRep rust model)
    (rustEffects : SandboxControllerRust.store.Store
      SandboxControllerRust.model.EffectId SandboxControllerRust.model.EffectStatus)
    (modelEffects : EffectId → Option EffectStatus)
    (effectsRelated : StoreRep effectId
      (fun concrete abstract => effectStatus concrete = abstract)
      rustEffects modelEffects) :
    EffectWorldRep { rust with effects := rustEffects }
      { model with effects := modelEffects } := by
  exact {
    sandboxes := represented.sandboxes
    vms := represented.vms
    filesystems := represented.filesystems
    snapshots := represented.snapshots
    netSlots := represented.netSlots
    appliedFilePolicies := represented.appliedFilePolicies
    workerFences := represented.workerFences
    operations := represented.operations
    operationPlans := represented.operationPlans
    resourceIndexes := represented.resourceIndexes
    logicalEffects := represented.logicalEffects
    effects := effectsRelated
    effectKeys := represented.effectKeys
    effectKeysUnique := represented.effectKeysUnique
    inflight := represented.inflight
    inflightSupport := represented.inflightSupport
    inflightUnique := represented.inflightUnique
    nextVmId := represented.nextVmId
    nextSlotId := represented.nextSlotId
    nextSnapshotId := represented.nextSnapshotId
    nextManifestDigest := represented.nextManifestDigest
    controllerUp := represented.controllerUp }

/-- Replacing only the logical-effect ledger preserves the whole relation
    once that store update has been verified. -/
theorem worldRep_setLogicalEffects
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : EffectWorldRep rust model)
    (rustLogical : SandboxControllerRust.store.Store
      SandboxControllerRust.model.LogicalEffectId
      SandboxControllerRust.model.LogicalEffectRecord)
    (modelLogical : LogicalEffectId → Option LogicalEffectRecord)
    (logicalRelated : StoreRep logicalEffectId
      (fun concrete abstract => logicalEffectRecord concrete = abstract)
      rustLogical modelLogical) :
    EffectWorldRep { rust with logical_effects := rustLogical }
      { model with logicalEffects := modelLogical } := by
  exact {
    sandboxes := represented.sandboxes
    vms := represented.vms
    filesystems := represented.filesystems
    snapshots := represented.snapshots
    netSlots := represented.netSlots
    appliedFilePolicies := represented.appliedFilePolicies
    workerFences := represented.workerFences
    operations := represented.operations
    operationPlans := represented.operationPlans
    resourceIndexes := represented.resourceIndexes
    logicalEffects := logicalRelated
    effects := represented.effects
    effectKeys := represented.effectKeys
    effectKeysUnique := represented.effectKeysUnique
    inflight := represented.inflight
    inflightSupport := represented.inflightSupport
    inflightUnique := represented.inflightUnique
    nextVmId := represented.nextVmId
    nextSlotId := represented.nextSlotId
    nextSnapshotId := represented.nextSnapshotId
    nextManifestDigest := represented.nextManifestDigest
    controllerUp := represented.controllerUp }

/-- General reconstruction lemma for the four fields modified by retry and
    receipt processing.  It lets the executable helper proofs be assembled
    from independent store/vector facts without reopening unrelated world
    fields. -/
theorem worldRep_setEffectProtocol
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : EffectWorldRep rust model)
    (rustLogical : SandboxControllerRust.store.Store
      SandboxControllerRust.model.LogicalEffectId
      SandboxControllerRust.model.LogicalEffectRecord)
    (modelLogical : LogicalEffectId → Option LogicalEffectRecord)
    (logicalRelated : StoreRep logicalEffectId
      (fun concrete abstract => logicalEffectRecord concrete = abstract)
      rustLogical modelLogical)
    (rustEffects : SandboxControllerRust.store.Store
      SandboxControllerRust.model.EffectId SandboxControllerRust.model.EffectStatus)
    (modelEffects : EffectId → Option EffectStatus)
    (effectsRelated : StoreRep effectId
      (fun concrete abstract => effectStatus concrete = abstract)
      rustEffects modelEffects)
    (rustKeys : alloc.vec.Vec SandboxControllerRust.model.EffectId)
    (modelKeys : List EffectId)
    (keysRelated : rustKeys.val.map effectId = modelKeys)
    (keysUnique : rustKeys.val.Nodup)
    (rustInflight : alloc.vec.Vec SandboxControllerRust.model.EffectId)
    (modelInflight : EffectId → Prop)
    (inflightRelated : ∀ effect,
      effect ∈ rustInflight.val ↔ modelInflight (effectId effect))
    (inflightSupport : ∀ effect, modelInflight effect →
      ∃ rustEffect, effectId rustEffect = effect)
    (inflightUnique : rustInflight.val.Nodup) :
    EffectWorldRep
      { rust with
          logical_effects := rustLogical
          effect_keys := rustKeys
          effects := rustEffects
          inflight := rustInflight }
      { model with
          logicalEffects := modelLogical
          effectKeys := modelKeys
          effects := modelEffects
          inflight := modelInflight } := by
  exact {
    sandboxes := represented.sandboxes
    vms := represented.vms
    filesystems := represented.filesystems
    snapshots := represented.snapshots
    netSlots := represented.netSlots
    appliedFilePolicies := represented.appliedFilePolicies
    workerFences := represented.workerFences
    operations := represented.operations
    operationPlans := represented.operationPlans
    resourceIndexes := represented.resourceIndexes
    logicalEffects := logicalRelated
    effects := effectsRelated
    effectKeys := keysRelated
    effectKeysUnique := keysUnique
    inflight := inflightRelated
    inflightSupport := inflightSupport
    inflightUnique := inflightUnique
    nextVmId := represented.nextVmId
    nextSlotId := represented.nextSlotId
    nextSnapshotId := represented.nextSnapshotId
    nextManifestDigest := represented.nextManifestDigest
    controllerUp := represented.controllerUp }

/-! ## Generated `insert_unique` -/

/-- The copying loop used by Rust's `effect::insert_unique` appends every
    source element, in order, after an already constructed prefix. -/
@[step] theorem generated_effectInsertUnique_copyLoop
    (values next : alloc.vec.Vec SandboxControllerRust.model.EffectId)
    (capacity : next.val.length + values.val.length ≤ Std.Usize.max) :
    SandboxControllerRust.effect.insert_unique_loop0_loop0
        SandboxControllerRust.model.EffectId.Insts.CoreCloneClone
        values 0#usize next
      ⦃ result => result.val = next.val ++ values.val ⦄ := by
  unfold SandboxControllerRust.effect.insert_unique_loop0_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun state : Std.Usize × alloc.vec.Vec
        SandboxControllerRust.model.EffectId =>
      values.val.length - state.1.val)
    (fun state : Std.Usize × alloc.vec.Vec
        SandboxControllerRust.model.EffectId =>
      state.1.val ≤ values.val.length ∧
      state.2.val = next.val ++ values.val.take state.1.val)
    (fun (result : alloc.vec.Vec SandboxControllerRust.model.EffectId) =>
      result.val = next.val ++ values.val)
  · intro state invariant
    rcases state with ⟨index, current⟩
    rcases invariant with ⟨indexBound, currentEq⟩
    simp only [Prod.fst, Prod.snd] at indexBound currentEq
    unfold SandboxControllerRust.effect.insert_unique_loop0_loop0.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨value, valuePost⟩
      rw [effect_id_clone_eq]
      have strict : current.val.length < Std.Usize.max := by
        have indexStrict : index.val < values.val.length := by simpa using inBounds
        rw [currentEq, List.length_append, List.length_take]
        simp only [Nat.min_eq_left indexBound]
        omega
      step as ⟨pushed, pushedPost⟩
      step as ⟨nextIndex, nextIndexPost⟩
      have indexStrict : index.val < values.val.length := by simpa using inBounds
      have takeStep : values.val.take (index.val + 1) =
          values.val.take index.val ++ [value] := by
        rw [List.take_succ_eq_append_getElem indexStrict]
        rw [← valuePost]
      constructor
      · scalar_tac
      · constructor
        · rw [nextIndexPost, pushedPost, currentEq, takeStep,
            List.append_assoc]
        · rw [nextIndexPost]
          omega
    · rename_i outOfBounds
      simp only [WP.spec_ok]
      have lower : values.val.length ≤ index.val := by simpa using outOfBounds
      have same : index.val = values.val.length := by omega
      simpa [same] using currentEq
  · exact ⟨by simp, by simp⟩

/-- The complete generated helper implements the model's prepend-if-absent
    list operation.  The explicit headroom premise is exactly the finite
    vector boundary of the absent branch. -/
@[step] theorem generated_effectInsertUnique_refines
    (values : alloc.vec.Vec SandboxControllerRust.model.EffectId)
    (value : SandboxControllerRust.model.EffectId)
    (headroom : VecHeadroom values) :
    SandboxControllerRust.effect.insert_unique
        SandboxControllerRust.model.EffectId.Insts.CoreCloneClone
        SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
        values value
      ⦃ result => result.val = SandboxModel.insertUnique value values.val ⦄ := by
  unfold SandboxControllerRust.effect.insert_unique
  unfold SandboxControllerRust.effect.insert_unique_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index : Std.Usize => values.val.length - index.val)
    (fun index : Std.Usize =>
      index.val ≤ values.val.length ∧ value ∉ values.val.take index.val)
    (fun (result : alloc.vec.Vec SandboxControllerRust.model.EffectId) =>
      result.val = SandboxModel.insertUnique value values.val)
  · intro index invariant
    rcases invariant with ⟨indexBound, absentPrefix⟩
    unfold SandboxControllerRust.effect.insert_unique_loop0.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨stored, storedPost⟩
      have indexStrict : index.val < values.val.length := by
        simpa using inBounds
      have compare :
          SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId.eq
              stored value = ok (decide (stored = value)) :=
        effectId_partialEq_lawful stored value
      simp only [compare, bind_ok]
      by_cases same : stored = value
      · simp [same]
        simp only [WP.spec_ok, SandboxModel.insertUnique]
        have present : value ∈ values.val := by
          rw [← same, storedPost]
          exact List.getElem_mem (l := values.val) indexStrict
        simp [present]
      · simp [same]
        step as ⟨nextIndex, nextIndexPost⟩
        constructor
        · rw [nextIndexPost]
          omega
        · constructor
          · rw [nextIndexPost]
            rw [List.take_succ_eq_append_getElem indexStrict]
            rw [← storedPost]
            simp [absentPrefix]
            exact fun equal => same equal.symm
          · rw [nextIndexPost]
            omega
    · rename_i outOfBounds
      have lower : values.val.length ≤ index.val := by simpa using outOfBounds
      have sameIndex : index.val = values.val.length := by omega
      have absent : value ∉ values.val := by
        simpa [sameIndex] using absentPrefix
      have emptyRoom : (alloc.vec.Vec.new
          SandboxControllerRust.model.EffectId).val.length < Std.Usize.max := by
        simp [alloc.vec.Vec.new]
        scalar_tac
      step as ⟨initial, initialPost⟩
      have initialLength : initial.val.length = 1 := by
        rw [initialPost]
        simp [alloc.vec.Vec.new]
      have copyRoom : initial.val.length + values.val.length ≤
          Std.Usize.max := by
        rw [initialLength]
        unfold VecHeadroom at headroom
        omega
      step as ⟨result, resultPost⟩
      simp only [WP.spec_ok, SandboxModel.insertUnique, absent, if_false]
      rw [resultPost, initialPost]
      simp
  · exact ⟨by simp, by simp⟩

theorem map_insertUnique_of_injective [DecidableEq α] [DecidableEq β]
    {f : α → β} (injective : Function.Injective f)
    (value : α) (values : List α) :
    (SandboxModel.insertUnique value values).map f =
      SandboxModel.insertUnique (f value) (values.map f) := by
  unfold SandboxModel.insertUnique
  by_cases present : value ∈ values
  · have mapped : f value ∈ values.map f := List.mem_map.mpr ⟨value, present, rfl⟩
    simp [present, mapped]
  · have mappedAbsent : f value ∉ values.map f := by
      intro mapped
      rcases List.mem_map.mp mapped with ⟨other, otherMem, same⟩
      have equal : other = value := injective same
      subst other
      exact present otherMem
    simp [present, mappedAbsent]

theorem nodup_insertUnique [DecidableEq α]
    (value : α) (values : List α)
    (unique : values.Nodup) :
    (SandboxModel.insertUnique value values).Nodup := by
  unfold SandboxModel.insertUnique
  by_cases present : value ∈ values <;> simp [present, unique]

@[simp] theorem mem_insertUnique [DecidableEq α]
    (candidate value : α) (values : List α) :
    candidate ∈ SandboxModel.insertUnique value values ↔
      candidate = value ∨ candidate ∈ values := by
  unfold SandboxModel.insertUnique
  by_cases present : value ∈ values
  · simp only [present, if_pos]
    constructor
    · exact Or.inr
    · intro either
      cases either with
      | inl same => simpa [same] using present
      | inr member => exact member
  · simp [present]

/-! ## Small executable equality/classification facts -/

@[simp] theorem generated_failureRetryable
    (failure : SandboxControllerRust.model.FailureKind) :
    SandboxControllerRust.model.FailureKind.retryable failure =
      ok (failureKind failure).retryable := by
  cases failure <;>
    rfl

/-- The actual generated Rust update for dispatch commutes with the model's
    `dispatchEffect` update. -/
theorem generated_dispatchEffect_refines
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : EffectWorldRep rust model)
    (headroom : StoreHeadroom rust.effects)
    (id : SandboxControllerRust.model.EffectId)
    (indexed : effectId id ∈ model.effectKeys) :
    ∃ next,
      SandboxControllerRust.effect.dispatch_effect rust id = ok next ∧
      EffectWorldRep next
        (SandboxModel.dispatchEffect model (effectId id)) := by
  obtain ⟨nextEffects, putEval, nextEffectsRelated⟩ :=
    Aeneas.Std.WP.spec_imp_exists (generated_put_preserves_storeRep
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
      effectId_partialEq_lawful effectId effectId_injective
      (fun concrete abstract => effectStatus concrete = abstract)
      rust.effects model.effects represented.effects id
      .Dispatched .dispatched rfl headroom)
  let next := { rust with effects := nextEffects }
  refine ⟨next, ?_, ?_⟩
  · simp [SandboxControllerRust.effect.dispatch_effect, world_clone_eq,
      putEval, next]
  · simpa [next, SandboxModel.dispatchEffect, SandboxModel.setEffect,
      SandboxModel.insertUnique, indexed] using
      worldRep_setEffects represented nextEffects
        (SandboxModel.put model.effects (effectId id) .dispatched)
        nextEffectsRelated

/-- The actual generated timeout update commutes with the model's
    `markEffectOutcomeUnknown` update. -/
theorem generated_markEffectOutcomeUnknown_refines
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : EffectWorldRep rust model)
    (headroom : StoreHeadroom rust.effects)
    (id : SandboxControllerRust.model.EffectId)
    (indexed : effectId id ∈ model.effectKeys) :
    ∃ next,
      SandboxControllerRust.effect.mark_effect_outcome_unknown rust id =
        ok next ∧
      EffectWorldRep next
        (SandboxModel.markEffectOutcomeUnknown model (effectId id)) := by
  obtain ⟨nextEffects, putEval, nextEffectsRelated⟩ :=
    Aeneas.Std.WP.spec_imp_exists (generated_put_preserves_storeRep
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
      effectId_partialEq_lawful effectId effectId_injective
      (fun concrete abstract => effectStatus concrete = abstract)
      rust.effects model.effects represented.effects id
      .OutcomeUnknown .outcomeUnknown rfl headroom)
  let next := { rust with effects := nextEffects }
  refine ⟨next, ?_, ?_⟩
  · simp [SandboxControllerRust.effect.mark_effect_outcome_unknown,
      world_clone_eq, putEval, next]
  · simpa [next, SandboxModel.markEffectOutcomeUnknown,
      SandboxModel.setEffect, SandboxModel.insertUnique, indexed] using
      worldRep_setEffects represented nextEffects
        (SandboxModel.put model.effects (effectId id) .outcomeUnknown)
        nextEffectsRelated

/-- A successful lookup in the concrete logical-effect ledger determines the
    corresponding abstract lookup.  The explicit `change` exposes the value
    relation hidden behind `valueRelations`. -/
theorem logicalLookup_some
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : EffectWorldRep rust model)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (record : SandboxControllerRust.model.LogicalEffectRecord)
    (found : storeLookup rust.logical_effects logical = some record) :
    model.logicalEffects (logicalEffectId logical) =
      some (logicalEffectRecord record) := by
  have agrees := represented.logicalEffects.lookupAgrees logical
  change OptionRel
    (fun concrete abstract => logicalEffectRecord concrete = abstract)
    (storeLookup rust.logical_effects logical)
    (model.logicalEffects (logicalEffectId logical)) at agrees
  rw [found] at agrees
  cases abstractLookup : model.logicalEffects (logicalEffectId logical) with
  | none =>
      rw [abstractLookup] at agrees
      simp [OptionRel] at agrees
  | some abstractRecord =>
      rw [abstractLookup] at agrees
      change logicalEffectRecord record = abstractRecord at agrees
      rw [← agrees]

/-- A successful lookup in the concrete attempt ledger determines the
    corresponding abstract lookup. -/
theorem effectLookup_some
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : EffectWorldRep rust model)
    (id : SandboxControllerRust.model.EffectId)
    (status : SandboxControllerRust.model.EffectStatus)
    (found : storeLookup rust.effects id = some status) :
    model.effects (effectId id) = some (effectStatus status) := by
  have agrees := represented.effects.lookupAgrees id
  change OptionRel
    (fun concrete abstract => effectStatus concrete = abstract)
    (storeLookup rust.effects id) (model.effects (effectId id)) at agrees
  rw [found] at agrees
  cases abstractLookup : model.effects (effectId id) with
  | none =>
      rw [abstractLookup] at agrees
      simp [OptionRel] at agrees
  | some abstractStatus =>
      rw [abstractLookup] at agrees
      change effectStatus status = abstractStatus at agrees
      rw [← agrees]

/-- The optional-status comparison used by `finalize_logical_failure` is
    reflexive.  This is deliberately specialized: the helper compares the
    value it just read with the very same failed outcome. -/
@[simp] theorem generated_effectStatusOption_ne_self
    (status : SandboxControllerRust.model.EffectStatus) :
    core.cmp.PartialEq.ne.trait_default
        (SandboxControllerRust.core.option.Option.Insts.CoreCmpPartialEqOption
          (core.cmp.PartialEqShared
            SandboxControllerRust.model.EffectStatus.Insts.CoreCmpPartialEqEffectStatus))
        (some status) (some status) = ok false := by
  cases status with
  | Committed | Dispatched | OutcomeUnknown =>
      simp [core.cmp.PartialEq.ne.trait_default,
        core.cmp.PartialEq.ne.default,
        core.option.Option.Insts.CoreCmpPartialEqOption.eq,
        SandboxControllerRust.model.EffectStatus.Insts.CoreCmpPartialEqEffectStatus.eq,
        SandboxControllerRust.model.EffectStatus.read_discriminant]
  | Acknowledged outcome =>
      cases outcome with
      | Succeeded =>
          simp [core.cmp.PartialEq.ne.trait_default,
            core.cmp.PartialEq.ne.default,
            core.option.Option.Insts.CoreCmpPartialEqOption.eq,
            SandboxControllerRust.model.EffectStatus.Insts.CoreCmpPartialEqEffectStatus.eq,
            SandboxControllerRust.model.EffectStatus.read_discriminant,
            SandboxControllerRust.model.EffectOutcome.Insts.CoreCmpPartialEqEffectOutcome.eq,
            SandboxControllerRust.model.EffectOutcome.read_discriminant,
            SandboxControllerRust.ids.ResultDigest.Insts.CoreCmpPartialEqResultDigest.eq]
      | Failed failure =>
          cases failure <;>
            simp [core.cmp.PartialEq.ne.trait_default,
              core.cmp.PartialEq.ne.default,
              core.option.Option.Insts.CoreCmpPartialEqOption.eq,
              SandboxControllerRust.model.EffectStatus.Insts.CoreCmpPartialEqEffectStatus.eq,
              SandboxControllerRust.model.EffectStatus.read_discriminant,
              SandboxControllerRust.model.EffectOutcome.Insts.CoreCmpPartialEqEffectOutcome.eq,
              SandboxControllerRust.model.EffectOutcome.read_discriminant,
              SandboxControllerRust.model.FailureKind.Insts.CoreCmpPartialEqFailureKind.eq,
              SandboxControllerRust.model.FailureKind.read_discriminant]

/-- Finalizing a proven failed attempt changes only the logical-effect record;
    the generated Rust helper and the model perform the same update. -/
theorem generated_finalizeLogicalFailure_refines
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : EffectWorldRep rust model)
    (headroom : StoreHeadroom rust.logical_effects)
    (logical : SandboxControllerRust.model.LogicalEffectId)
    (failure : SandboxControllerRust.model.FailureKind)
    (stored : SandboxControllerRust.model.LogicalEffectRecord)
    (logicalFound : storeLookup rust.logical_effects logical = some stored)
    (failedFound : storeLookup rust.effects
      { logical := logical, attempt := stored.current_attempt } =
        some (.Acknowledged (.Failed failure))) :
    ∃ next,
      SandboxControllerRust.effect.finalize_logical_failure rust logical failure =
        ok next ∧
      EffectWorldRep next
        (SandboxModel.finalizeLogicalFailure model (logicalEffectId logical)
          (failureKind failure)) := by
  let current : SandboxControllerRust.model.EffectId :=
    { logical := logical, attempt := stored.current_attempt }
  let updated : SandboxControllerRust.model.LogicalEffectRecord :=
    { stored with result := some (.Failed stored.current_attempt failure) }
  have modelLogicalFound := logicalLookup_some represented logical stored
    logicalFound
  have modelFailedFound := effectLookup_some represented current
    (.Acknowledged (.Failed failure)) (by simpa [current] using failedFound)
  have modelFailedFoundExact :
      model.effects
          { logical := logicalEffectId logical,
            attempt := u64ToNat stored.current_attempt } =
        some (.acknowledged (.failed (failureKind failure))) := by
    simpa [current, effectId, effectStatus, effectOutcome] using modelFailedFound
  have updatedRelated : logicalEffectRecord updated =
      { logicalEffectRecord stored with
        result := some (.failed (u64ToNat stored.current_attempt)
          (failureKind failure)) } := by
    rfl
  obtain ⟨nextLogical, putEval, nextLogicalRelated⟩ :=
    Aeneas.Std.WP.spec_imp_exists
      (generated_put_preserves_storeRep
        SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
        logicalEffectId_partialEq_lawful logicalEffectId
        logicalEffectId_injective
        (fun concrete abstract => logicalEffectRecord concrete = abstract)
        rust.logical_effects model.logicalEffects represented.logicalEffects
        logical updated
        { logicalEffectRecord stored with
          result := some (.failed (u64ToNat stored.current_attempt)
            (failureKind failure)) }
        updatedRelated headroom)
  let next := { rust with logical_effects := nextLogical }
  refine ⟨next, ?_, ?_⟩
  · have logicalGet := generated_get_eq
      SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      logicalEffectId_partialEq_lawful rust.logical_effects logical
    have effectGet := generated_get_eq
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
      effectId_partialEq_lawful rust.effects current
    simp [SandboxControllerRust.effect.finalize_logical_failure,
      logicalGet, logicalFound, logical_effect_id_clone_eq, current,
      effectGet, failedFound, world_clone_eq, updated, putEval, next]
  · have base := worldRep_setLogicalEffects represented nextLogical
        (SandboxModel.put model.logicalEffects (logicalEffectId logical)
          { logicalEffectRecord stored with
            result := some (.failed (u64ToNat stored.current_attempt)
              (failureKind failure)) })
        nextLogicalRelated
    simpa [next, SandboxModel.finalizeLogicalFailure, modelLogicalFound,
      modelFailedFoundExact, logicalEffectRecord] using base

/-! ## Handler-level result relation -/

/-- The common part of the Rust and model effect-handler result types.
    `CounterExhausted` deliberately has no image here; the handler theorem
    rules it out from `HandlerMachineHeadroom`. -/
def EffectResultRep (model : World)
    (rustResult : core.result.Result
      SandboxControllerRust.effect.EffectDecision
      SandboxControllerRust.effect.EffectReject)
    (modelResult : Except SandboxModel.Controller.EffectReject
      (SandboxModel.Controller.Decision model)) : Prop :=
  match rustResult, modelResult with
  | .Err rustReject, .error modelReject =>
      effectReject? rustReject = some modelReject
  | .Ok rustDecision, .ok modelDecision =>
      modelEvent rustDecision.event = modelDecision.event ∧
        EffectWorldRep rustDecision.next modelDecision.next
  | _, _ => False

/-- Lookup agreement specialized to the persistent effect store. -/
theorem effectLookup_agrees
    {rust : SandboxControllerRust.model.World} {model : World}
    (represented : EffectWorldRep rust model)
    (id : SandboxControllerRust.model.EffectId) :
    OptionRel (fun concrete abstract => effectStatus concrete = abstract)
      (storeLookup rust.effects id) (model.effects (effectId id)) :=
  represented.effects.lookupAgrees id

theorem model_handleEffectDispatch_missing (model : World) (id : EffectId)
    (up : model.controllerUp = true) (missing : model.effects id = none) :
    SandboxModel.Controller.handleEffectDispatch model id =
      .error .effectNotFound := by
  unfold SandboxModel.Controller.handleEffectDispatch
  simp only [up, ↓reduceDIte]
  split <;> simp_all

theorem model_handleEffectDispatch_notDispatchable (model : World)
    (id : EffectId) (status : EffectStatus)
    (up : model.controllerUp = true)
    (found : model.effects id = some status)
    (blocked : status ≠ .committed ∧ status ≠ .outcomeUnknown) :
    SandboxModel.Controller.handleEffectDispatch model id =
      .error .notDispatchable := by
  unfold SandboxModel.Controller.handleEffectDispatch
  simp only [up, ↓reduceDIte]
  split <;> simp_all

theorem model_handleEffectDispatch_committed (model : World) (id : EffectId)
    (up : model.controllerUp = true)
    (found : model.effects id = some .committed) :
    ∃ decision, SandboxModel.Controller.handleEffectDispatch model id =
        .ok decision ∧
      decision.event = .dispatchEffect id ∧
      decision.next = SandboxModel.dispatchEffect model id := by
  rcases SandboxModel.Controller.handleEffectDispatch_complete
      (Step.dispatchEffect up (Or.inl found)) with
    ⟨decision, evaluation, eventEq, nextEq⟩
  exact ⟨decision, by simpa [SandboxModel.Controller.handleEffect] using
    evaluation, eventEq, nextEq⟩

theorem model_handleEffectDispatch_unknown (model : World) (id : EffectId)
    (up : model.controllerUp = true)
    (found : model.effects id = some .outcomeUnknown) :
    ∃ decision, SandboxModel.Controller.handleEffectDispatch model id =
        .ok decision ∧
      decision.event = .dispatchEffect id ∧
      decision.next = SandboxModel.dispatchEffect model id := by
  rcases SandboxModel.Controller.handleEffectDispatch_complete
      (Step.dispatchEffect up (Or.inr found)) with
    ⟨decision, evaluation, eventEq, nextEq⟩
  exact ⟨decision, by simpa [SandboxModel.Controller.handleEffect] using
    evaluation, eventEq, nextEq⟩

theorem model_handleEffectTimeout_missing (model : World) (id : EffectId)
    (missing : model.effects id = none) :
    SandboxModel.Controller.handleEffectTimeout model id =
      .error .effectNotFound := by
  unfold SandboxModel.Controller.handleEffectTimeout
  split <;> simp_all

theorem model_handleEffectTimeout_notDispatched (model : World)
    (id : EffectId) (status : EffectStatus)
    (found : model.effects id = some status)
    (blocked : status ≠ .dispatched ∧ status ≠ .outcomeUnknown) :
    SandboxModel.Controller.handleEffectTimeout model id =
      .error .notDispatched := by
  unfold SandboxModel.Controller.handleEffectTimeout
  split <;> simp_all

theorem model_handleEffectTimeout_dispatched (model : World) (id : EffectId)
    (found : model.effects id = some .dispatched) :
    ∃ decision, SandboxModel.Controller.handleEffectTimeout model id =
        .ok decision ∧
      decision.event = .effectTimeout id ∧
      decision.next = SandboxModel.markEffectOutcomeUnknown model id := by
  rcases SandboxModel.Controller.handleEffectTimeout_complete
      (Step.effectTimeout found) with
    ⟨decision, evaluation, eventEq, nextEq⟩
  exact ⟨decision, by simpa [SandboxModel.Controller.handleEffect] using
    evaluation, eventEq, nextEq⟩

theorem model_handleEffectTimeout_unknown (model : World) (id : EffectId)
    (found : model.effects id = some .outcomeUnknown) :
    ∃ decision, SandboxModel.Controller.handleEffectTimeout model id =
        .ok decision ∧
      decision.event = .effectTimeout id ∧ decision.next = model := by
  rcases SandboxModel.Controller.handleEffectTimeout_complete
      (Step.repeatEffectTimeout found) with
    ⟨decision, evaluation, eventEq, nextEq⟩
  exact ⟨decision, by simpa [SandboxModel.Controller.handleEffect] using
    evaluation, eventEq, nextEq⟩

/-- Dispatch is a complete commuting diagram: every guard rejection agrees;
    either accepted status produces the same event and a related successor. -/
theorem handleEffectDispatch_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (headroom : HandlerMachineHeadroom rust)
    (complete : SandboxModel.Controller.EffectKeyIndexComplete model)
    (id : SandboxControllerRust.model.EffectId) :
    ∃ rustResult,
      SandboxControllerRust.effect.handle_effect_dispatch rust id =
        ok rustResult ∧
      EffectResultRep model rustResult
        (SandboxModel.Controller.handleEffectDispatch model (effectId id)) := by
  have getEq := generated_get_eq
    SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
    effectId_partialEq_lawful rust.effects id
  by_cases up : rust.controller_up = true
  · have modelUp : model.controllerUp = true := by
      rw [← represented.controllerUp]
      exact up
    cases concreteLookup : storeLookup rust.effects id with
    | none =>
        have agrees := effectLookup_agrees represented id
        rw [concreteLookup] at agrees
        cases modelLookup : model.effects (effectId id) with
        | none =>
            refine ⟨.Err .EffectNotFound, ?_, ?_⟩
            · simp [SandboxControllerRust.effect.handle_effect_dispatch, up,
                getEq, concreteLookup]
            · rw [model_handleEffectDispatch_missing model (effectId id)
                modelUp modelLookup]
              rfl
        | some status => simp [OptionRel, modelLookup] at agrees
    | some concreteStatus =>
        have agrees := effectLookup_agrees represented id
        rw [concreteLookup] at agrees
        cases modelLookup : model.effects (effectId id) with
        | none => simp [OptionRel, modelLookup] at agrees
        | some modelStatus =>
            have statusSame : effectStatus concreteStatus = modelStatus := by
              simpa [OptionRel, modelLookup] using agrees
            cases concreteStatus with
            | Committed =>
                subst modelStatus
                have abstractFound :
                    model.effects (effectId id) = some .committed := by
                  simpa [effectStatus] using modelLookup
                obtain ⟨next, nextEval, nextRelated⟩ :=
                  generated_dispatchEffect_refines represented
                    headroom.effects id
                    (complete (effectId id) .committed abstractFound)
                refine ⟨.Ok { event := .DispatchEffect id, next := next }, ?_, ?_⟩
                · simp [SandboxControllerRust.effect.handle_effect_dispatch, up,
                    getEq, concreteLookup, nextEval]
                · rcases model_handleEffectDispatch_committed model
                      (effectId id) modelUp abstractFound with
                    ⟨modelDecision, modelEval, eventEq, nextEq⟩
                  rw [modelEval]
                  constructor
                  · rw [eventEq]
                    rfl
                  · rw [nextEq]
                    exact nextRelated
            | OutcomeUnknown =>
                subst modelStatus
                have abstractFound :
                    model.effects (effectId id) = some .outcomeUnknown := by
                  simpa [effectStatus] using modelLookup
                obtain ⟨next, nextEval, nextRelated⟩ :=
                  generated_dispatchEffect_refines represented
                    headroom.effects id
                    (complete (effectId id) .outcomeUnknown abstractFound)
                refine ⟨.Ok { event := .DispatchEffect id, next := next }, ?_, ?_⟩
                · simp [SandboxControllerRust.effect.handle_effect_dispatch, up,
                    getEq, concreteLookup, nextEval]
                · rcases model_handleEffectDispatch_unknown model
                      (effectId id) modelUp abstractFound with
                    ⟨modelDecision, modelEval, eventEq, nextEq⟩
                  rw [modelEval]
                  constructor
                  · rw [eventEq]
                    rfl
                  · rw [nextEq]
                    exact nextRelated
            | Dispatched =>
                subst modelStatus
                have abstractFound :
                    model.effects (effectId id) = some .dispatched := by
                  simpa [effectStatus] using modelLookup
                refine ⟨.Err .NotDispatchable, ?_, ?_⟩
                · simp [SandboxControllerRust.effect.handle_effect_dispatch, up,
                    getEq, concreteLookup]
                · rw [model_handleEffectDispatch_notDispatchable model
                      (effectId id) .dispatched modelUp abstractFound
                      (by simp)]
                  rfl
            | Acknowledged outcome =>
                subst modelStatus
                have abstractFound : model.effects (effectId id) =
                    some (.acknowledged (effectOutcome outcome)) := by
                  simpa [effectStatus] using modelLookup
                refine ⟨.Err .NotDispatchable, ?_, ?_⟩
                · simp [SandboxControllerRust.effect.handle_effect_dispatch, up,
                    getEq, concreteLookup]
                · rw [model_handleEffectDispatch_notDispatchable model
                      (effectId id) (.acknowledged (effectOutcome outcome))
                      modelUp abstractFound (by simp)]
                  rfl
  · have down : rust.controller_up = false := by simpa using up
    have modelDown : model.controllerUp = false := by
      rw [← represented.controllerUp]
      exact down
    refine ⟨.Err .ControllerDown, ?_, ?_⟩
    · simp [SandboxControllerRust.effect.handle_effect_dispatch, down]
    · simp [EffectResultRep, SandboxModel.Controller.handleEffectDispatch,
        modelDown, effectReject?]

/-- Timeout is a complete commuting diagram, including the idempotent
    `OutcomeUnknown` replay branch. -/
theorem handleEffectTimeout_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (headroom : HandlerMachineHeadroom rust)
    (complete : SandboxModel.Controller.EffectKeyIndexComplete model)
    (id : SandboxControllerRust.model.EffectId) :
    ∃ rustResult,
      SandboxControllerRust.effect.handle_effect_timeout rust id =
        ok rustResult ∧
      EffectResultRep model rustResult
        (SandboxModel.Controller.handleEffectTimeout model (effectId id)) := by
  have getEq := generated_get_eq
    SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
    effectId_partialEq_lawful rust.effects id
  cases concreteLookup : storeLookup rust.effects id with
  | none =>
      have agrees := effectLookup_agrees represented id
      rw [concreteLookup] at agrees
      cases modelLookup : model.effects (effectId id) with
      | none =>
          refine ⟨.Err .EffectNotFound, ?_, ?_⟩
          · simp [SandboxControllerRust.effect.handle_effect_timeout, getEq,
              concreteLookup]
          · rw [model_handleEffectTimeout_missing model (effectId id)
                modelLookup]
            rfl
      | some status => simp [OptionRel, modelLookup] at agrees
  | some concreteStatus =>
      have agrees := effectLookup_agrees represented id
      rw [concreteLookup] at agrees
      cases modelLookup : model.effects (effectId id) with
      | none => simp [OptionRel, modelLookup] at agrees
      | some modelStatus =>
          have statusSame : effectStatus concreteStatus = modelStatus := by
            simpa [OptionRel, modelLookup] using agrees
          cases concreteStatus with
          | Dispatched =>
              subst modelStatus
              have abstractFound :
                  model.effects (effectId id) = some .dispatched := by
                simpa [effectStatus] using modelLookup
              obtain ⟨next, nextEval, nextRelated⟩ :=
                generated_markEffectOutcomeUnknown_refines represented
                  headroom.effects id
                  (complete (effectId id) .dispatched abstractFound)
              refine ⟨.Ok { event := .EffectTimeout id, next := next }, ?_, ?_⟩
              · simp [SandboxControllerRust.effect.handle_effect_timeout,
                  getEq, concreteLookup, nextEval]
              · rcases model_handleEffectTimeout_dispatched model
                    (effectId id) abstractFound with
                  ⟨modelDecision, modelEval, eventEq, nextEq⟩
                rw [modelEval]
                constructor
                · rw [eventEq]
                  rfl
                · rw [nextEq]
                  exact nextRelated
          | OutcomeUnknown =>
              subst modelStatus
              have abstractFound :
                  model.effects (effectId id) = some .outcomeUnknown := by
                simpa [effectStatus] using modelLookup
              refine ⟨.Ok { event := .EffectTimeout id, next := rust }, ?_, ?_⟩
              · simp [SandboxControllerRust.effect.handle_effect_timeout,
                  getEq, concreteLookup, world_clone_eq]
              · rcases model_handleEffectTimeout_unknown model
                    (effectId id) abstractFound with
                  ⟨modelDecision, modelEval, eventEq, nextEq⟩
                rw [modelEval]
                constructor
                · rw [eventEq]
                  rfl
                · rw [nextEq]
                  exact represented
          | Committed =>
              subst modelStatus
              have abstractFound :
                  model.effects (effectId id) = some .committed := by
                simpa [effectStatus] using modelLookup
              refine ⟨.Err .NotDispatched, ?_, ?_⟩
              · simp [SandboxControllerRust.effect.handle_effect_timeout,
                  getEq, concreteLookup]
              · rw [model_handleEffectTimeout_notDispatched model
                    (effectId id) .committed abstractFound (by simp)]
                rfl
          | Acknowledged outcome =>
              subst modelStatus
              have abstractFound : model.effects (effectId id) =
                  some (.acknowledged (effectOutcome outcome)) := by
                simpa [effectStatus] using modelLookup
              refine ⟨.Err .NotDispatched, ?_, ?_⟩
              · simp [SandboxControllerRust.effect.handle_effect_timeout,
                  getEq, concreteLookup]
              · rw [model_handleEffectTimeout_notDispatched model
                    (effectId id) (.acknowledged (effectOutcome outcome))
                    abstractFound (by simp)]
                rfl

end SandboxModel.RustRefinement
