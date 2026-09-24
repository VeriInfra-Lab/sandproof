import SandboxModel.RustRefinement.LifecycleCommon

namespace SandboxModel.RustRefinement
open Aeneas Aeneas.Std Result

local instance lifecycleEffectsOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey := operationKeyDecidableEq
local instance lifecycleEffectsLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId := logicalEffectIdDecidableEq
local instance lifecycleEffectsEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId := effectIdDecidableEq

@[step] theorem generated_transitionInsertUnique_copyLoop
    (values next : alloc.vec.Vec SandboxControllerRust.model.EffectId)
    (capacity : next.val.length + values.val.length ≤ Std.Usize.max) :
    SandboxControllerRust.transition.insert_unique_loop0_loop0
        SandboxControllerRust.model.EffectId.Insts.CoreCloneClone
        values next 0#usize
      ⦃ result => result.val = next.val ++ values.val ⦄ := by
  unfold SandboxControllerRust.transition.insert_unique_loop0_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun state : alloc.vec.Vec SandboxControllerRust.model.EffectId × Std.Usize =>
      values.val.length - state.2.val)
    (fun state : alloc.vec.Vec SandboxControllerRust.model.EffectId × Std.Usize =>
      state.2.val ≤ values.val.length ∧
      state.1.val = next.val ++ values.val.take state.2.val)
    (fun (result : alloc.vec.Vec SandboxControllerRust.model.EffectId) =>
      result.val = next.val ++ values.val)
  · intro state invariant
    rcases state with ⟨current, index⟩
    rcases invariant with ⟨indexBound, currentEq⟩
    simp only [Prod.fst, Prod.snd] at indexBound currentEq
    unfold SandboxControllerRust.transition.insert_unique_loop0_loop0.body
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
@[step] theorem generated_transitionInsertUnique_refines
    (values : alloc.vec.Vec SandboxControllerRust.model.EffectId)
    (value : SandboxControllerRust.model.EffectId)
    (headroom : VecHeadroom values) :
    SandboxControllerRust.transition.insert_unique
        SandboxControllerRust.model.EffectId.Insts.CoreCloneClone
        SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
        values value
      ⦃ result => result.val = SandboxModel.insertUnique value values.val ⦄ := by
  unfold SandboxControllerRust.transition.insert_unique
  unfold SandboxControllerRust.transition.insert_unique_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index : Std.Usize => values.val.length - index.val)
    (fun index : Std.Usize =>
      index.val ≤ values.val.length ∧ value ∉ values.val.take index.val)
    (fun (result : alloc.vec.Vec SandboxControllerRust.model.EffectId) =>
      result.val = SandboxModel.insertUnique value values.val)
  · intro index invariant
    rcases invariant with ⟨indexBound, absentPrefix⟩
    unfold SandboxControllerRust.transition.insert_unique_loop0.body
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

theorem generated_commandSandbox_refines (input : SandboxControllerRust.model.Command) :
    ∃ sid, SandboxControllerRust.model.Command.sandbox input = ok sid ∧
      sandboxId sid = (command input).sandbox := by
  cases input <;> simp [SandboxControllerRust.model.Command.sandbox, command, Command.sandbox]

theorem generated_commandEpoch_refines (input : SandboxControllerRust.model.Command) :
    ∃ epoch, SandboxControllerRust.model.Command.epoch input = ok epoch ∧
      epoch.map u64ToNat = (command input).epoch? := by
  cases input <;> simp [SandboxControllerRust.model.Command.epoch, command, Command.epoch?]

theorem generated_inferredEffectOrigin_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (input : SandboxControllerRust.model.Command) :
    ∃ origin, SandboxControllerRust.transition.inferred_effect_origin rust input = ok origin ∧
      effectOrigin origin = inferredEffectOrigin model (command input) := by
  obtain ⟨sid, sidEval, sidSame⟩ := generated_commandSandbox_refines input
  obtain ⟨epoch, epochEval, epochSame⟩ := generated_commandEpoch_refines input
  have get := generated_get_eq SandboxControllerRust.ids.SandboxId.Insts.CoreCmpPartialEqSandboxId
    sandboxId_partialEq_lawful rust.sandboxes sid
  have lookup := storeRep_lookup_map rep.sandboxes sid
  have scalarEq (left right : U64) : u64ToNat left = u64ToNat right ↔ left = right :=
    u64ToNat_injective.eq_iff
  simp only [SandboxControllerRust.transition.inferred_effect_origin, sidEval, epochEval]
  simp [get]
  unfold inferredEffectOrigin
  rw [← sidSame, ← epochSame]
  dsimp only
  rw [lookup]
  cases epoch with
  | none => exact ⟨.Maintenance sid, by simp, rfl⟩
  | some epoch =>
    cases found : storeLookup rust.sandboxes sid with
    | none => exact ⟨.Maintenance sid, by simp, rfl⟩
    | some record =>
      cases current : record.current_operation with
      | none => refine ⟨.Maintenance sid, by simp [current], ?_⟩; simp [sandboxRecord, current, effectOrigin]
      | some op =>
        by_cases epochMatches : op.epoch = epoch
        · refine ⟨.Operation ⟨sid, op.id⟩, ?_, ?_⟩
          · simp [current, epochMatches, SandboxControllerRust.transition.operation_key]
          · simp [sandboxRecord, current, operation, scalarEq, epochMatches, effectOrigin, operationKey]
        · refine ⟨.Maintenance sid, by simp [current, epochMatches], ?_⟩
          simp [sandboxRecord, current, operation, scalarEq, epochMatches, effectOrigin]

theorem insertUnique_length_le [DecidableEq T] (value : T) (values : List T) :
    (SandboxModel.insertUnique value values).length ≤ values.length + 1 := by
  unfold SandboxModel.insertUnique
  split <;> simp

theorem generated_commitEffect_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (room : EffectCapacity rust (count + 1)) (input : SandboxControllerRust.model.Command) :
    ∃ next, SandboxControllerRust.transition.commit_effect rust input = ok next ∧
      EffectWorldRep next (commitEffect model (command input)) ∧
      EffectCapacity next count ∧ next.inflight = rust.inflight := by
  let logical : SandboxControllerRust.model.LogicalEffectId := ⟨input⟩
  let id : SandboxControllerRust.model.EffectId := ⟨logical, 0#u64⟩
  have effectsRoom : StoreHeadroom rust.effects := by have := room.effects; unfold StoreHeadroom; omega
  have logicalRoom : StoreHeadroom rust.logical_effects := by have := room.logical; unfold StoreHeadroom; omega
  have keysRoom : VecHeadroom rust.effect_keys := by have := room.keys; unfold VecHeadroom; omega
  have getEffect := generated_get_eq SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
    effectId_partialEq_lawful rust.effects id
  have getLogical := generated_get_eq SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    logicalEffectId_partialEq_lawful rust.logical_effects logical
  have modelEffects := storeRep_lookup_map rep.effects id
  have modelLogical := storeRep_lookup_map rep.logicalEffects logical
  change model.effects (SandboxModel.effectId (command input)) = _ at modelEffects
  change model.logicalEffects (SandboxModel.logicalEffectId (command input)) = _ at modelLogical
  cases found : storeLookup rust.effects id with
  | some status =>
      refine ⟨rust, ?_, ?_, room.mono (by omega), rfl⟩
      · dsimp [id, logical] at getEffect found
        simp [SandboxControllerRust.transition.commit_effect, SandboxControllerRust.model.EffectId.initial,
          SandboxControllerRust.store.Store.contains_key, getEffect, found]
      · simpa [commitEffect, modelEffects, found] using rep
  | none =>
      obtain ⟨effects, effectsEval, effectsRep⟩ := WP.spec_imp_exists
        (generated_put_preserves_storeRep SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
          effectId_partialEq_lawful effectId effectId_injective (fun a b => effectStatus a = b)
          rust.effects model.effects rep.effects id .Committed .committed rfl effectsRoom)
      have effectsSize := generated_put_length_bound _ effectId_partialEq_lawful _ _ id .Committed effectsRoom effectsEval
      obtain ⟨keys, keysEval, keysSame⟩ := WP.spec_imp_exists
        (generated_transitionInsertUnique_refines rust.effect_keys id keysRoom)
      have keysSize : keys.val.length ≤ rust.effect_keys.val.length + 1 := by
        rw [keysSame]; exact insertUnique_length_le id rust.effect_keys.val
      let base := { rust with effects := effects, effect_keys := keys }
      have baseRep : EffectWorldRep base (setEffect model (effectId id) .committed) :=
        { rep with
          effects := effectsRep
          effectKeys := by
            have mapped : rust.effect_keys.val.map effectId = model.effectKeys := rep.effectKeys
            change keys.val.map effectId = _
            rw [keysSame, map_insertUnique_of_injective effectId_injective, mapped]
            rfl
          effectKeysUnique := by rw [keysSame]; exact nodup_insertUnique id _ rep.effectKeysUnique }
      have baseRoom : EffectCapacity base count := by
        rcases room with ⟨a,b,c,d⟩
        constructor <;> dsimp [base] <;> omega
      cases logicalFound : storeLookup rust.logical_effects logical with
      | some record =>
        refine ⟨base, ?_, ?_, baseRoom, rfl⟩
        · dsimp [id, logical] at getEffect found effectsEval keysEval getLogical logicalFound
          simp [SandboxControllerRust.transition.commit_effect, SandboxControllerRust.model.EffectId.initial,
            SandboxControllerRust.store.Store.contains_key, getEffect, found, effectsEval, keysEval,
            getLogical, logicalFound, base]
        · simp only [commitEffect, modelEffects, found, modelLogical, logicalFound, Option.map_none, Option.map_some]
          simpa [id, logical,
            effectId, logicalEffectId, SandboxModel.effectId, SandboxModel.logicalEffectId, effectIdAt, u64ToNat] using baseRep
      | none =>
        obtain ⟨origin, originEval, originSame⟩ := generated_inferredEffectOrigin_refines
          base (setEffect model (effectId id) .committed) baseRep input
        let record : SandboxControllerRust.model.LogicalEffectRecord :=
          ⟨origin, 0#u64, SandboxControllerRust.transition.DEFAULT_RETRY_LIMIT, none⟩
        obtain ⟨logicals, logicalEval, logicalRep⟩ := WP.spec_imp_exists
          (generated_put_preserves_storeRep SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
            logicalEffectId_partialEq_lawful logicalEffectId logicalEffectId_injective
            (fun a b => logicalEffectRecord a = b) rust.logical_effects model.logicalEffects rep.logicalEffects
            logical record (logicalEffectRecord record) rfl logicalRoom)
        have logicalSize := generated_put_length_bound _ logicalEffectId_partialEq_lawful _ _ logical record logicalRoom logicalEval
        let next := { base with logical_effects := logicals }
        have nextRep : EffectWorldRep next
            (setLogicalEffect (setEffect model (effectId id) .committed) (logicalEffectId logical) (logicalEffectRecord record)) :=
          { baseRep with logicalEffects := logicalRep }
        refine ⟨next, ?_, ?_, ?_, rfl⟩
        · dsimp [id, logical] at getEffect found effectsEval keysEval getLogical logicalFound
          dsimp [record, logical] at logicalEval
          dsimp [base] at originEval
          simp [SandboxControllerRust.transition.commit_effect, SandboxControllerRust.model.EffectId.initial,
            SandboxControllerRust.store.Store.contains_key, getEffect, found, effectsEval, keysEval,
            getLogical, logicalFound, originEval, logicalEval, next, base]
        · simp only [commitEffect, modelEffects, found, modelLogical, logicalFound, Option.map_none, Option.map_some]
          simpa [logicalEffectRecord,
            record, id, logical, effectId, logicalEffectId, SandboxModel.effectId, SandboxModel.logicalEffectId,
            effectIdAt, originSame, inferredEffectOrigin, setEffect,
            defaultRetryLimit, SandboxControllerRust.transition.DEFAULT_RETRY_LIMIT, u64ToNat] using nextRep
        · rcases room with ⟨a,b,c,d⟩
          constructor <;> dsimp [next, base] <;> omega

theorem generated_enqueue_refines
    (rust : SandboxControllerRust.model.World) (model : World) (rep : EffectWorldRep rust model)
    (room : EffectCapacity rust (count + 1)) (input : SandboxControllerRust.model.Command) :
    ∃ next, SandboxControllerRust.transition.enqueue rust input = ok next ∧
      EffectWorldRep next (enqueue model (command input)) ∧ EffectCapacity next count := by
  obtain ⟨committed, commitEval, commitRep, commitRoom, inflightSame⟩ :=
    generated_commitEffect_refines rust model rep room input
  let id : SandboxControllerRust.model.EffectId := ⟨⟨input⟩, 0#u64⟩
  have inflightRoom : VecHeadroom committed.inflight := by
    rw [inflightSame]; have := room.inflight; unfold VecHeadroom; omega
  obtain ⟨inflight, inflightEval, inflightValue⟩ := WP.spec_imp_exists
    (generated_transitionInsertUnique_refines committed.inflight id inflightRoom)
  have inflightSize : inflight.val.length ≤ rust.inflight.val.length + 1 := by
    rw [inflightValue, inflightSame]
    exact insertUnique_length_le id rust.inflight.val
  refine ⟨{ committed with inflight := inflight }, ?_, ?_, ?_⟩
  · dsimp [id] at inflightEval
    simp [SandboxControllerRust.transition.enqueue, SandboxControllerRust.model.EffectId.initial,
      commitEval, inflightEval]
  · refine { commitRep with
      inflight := ?_
      inflightSupport := ?_
      inflightUnique := ?_ }
    · intro candidate
      have old := commitRep.inflight candidate
      change candidate ∈ committed.inflight.val ↔ (commitEffect model (command input)).inflight (effectId candidate) at old
      change candidate ∈ inflight.val ↔ _
      rw [inflightValue, mem_insertUnique, old]
      change (candidate = id ∨ _) ↔ (effectId candidate = effectId id ∨ _)
      simp [effectId_injective.eq_iff, valueRelations]
    · intro candidate present
      rcases present with same | previous
      · exact ⟨id, same.symm⟩
      · exact commitRep.inflightSupport candidate previous
    · change inflight.val.Nodup
      rw [inflightValue]
      exact nodup_insertUnique id _ commitRep.inflightUnique
  · refine ⟨commitRoom.effects, commitRoom.logical, commitRoom.keys, ?_⟩
    dsimp only
    have := room.inflight
    omega

end SandboxModel.RustRefinement
