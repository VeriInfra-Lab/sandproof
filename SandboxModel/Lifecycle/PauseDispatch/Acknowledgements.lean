import SandboxModel.Lifecycle.PauseDispatch.Frames
import SandboxModel.Lifecycle.PausePublication.Effects

namespace SandboxModel.Lifecycle.PauseDispatch
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

def ackLogical (world : model.World) (id : model.EffectId) (outcome : model.EffectOutcome)
    (found : Option model.LogicalEffectRecord) : Result (store.Store model.LogicalEffectId model.LogicalEffectRecord) :=
  match found with
  | none => .ok world.logical_effects
  | some stored =>
    if stored.current_attempt = id.attempt then
      match outcome with
      | .Succeeded result => store.Store.put model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
          world.logical_effects id.logical {stored with result := some (.Succeeded id.attempt result)}
      | .Failed _ => .ok world.logical_effects
    else .ok world.logical_effects

theorem acknowledge_normal (world : model.World) (id : model.EffectId) (outcome : model.EffectOutcome) :
    effect.acknowledge_effect world id outcome =
      (do
        let found ← store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects id.logical
        let logicals ← ackLogical world id outcome found
        let effects ← store.Store.put model.EffectId.Insts.CoreCmpPartialEqEffectId world.effects id (.Acknowledged outcome)
        let keys ← effect.insert_unique model.EffectId.Insts.CoreCloneClone model.EffectId.Insts.CoreCmpPartialEqEffectId world.effect_keys id
        .ok {world with logical_effects := logicals, effects := effects, effect_keys := keys}) := by
  simp only [effect.acknowledge_effect,model_World_clone_exact,bind_tc_ok]
  congr 1
  funext found
  cases found with
  | none => simp [ackLogical,model_EffectId_clone_exact]
  | some stored =>
    by_cases same : stored.current_attempt = id.attempt
    · cases outcome <;> simp [ackLogical,same,model_LogicalEffectId_clone_exact,
        model_EffectId_clone_exact]
    · simp [ackLogical,same,model_EffectId_clone_exact]

theorem acknowledge_steps (world next : model.World) (id : model.EffectId)
    (outcome : model.EffectOutcome)
    (evaluated : effect.acknowledge_effect world id outcome = .ok next) :
    ∃ found logicals effects keys,
      store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects id.logical = .ok found ∧
      ackLogical world id outcome found = .ok logicals ∧
      store.Store.put model.EffectId.Insts.CoreCmpPartialEqEffectId world.effects id (.Acknowledged outcome) = .ok effects ∧
      effect.insert_unique model.EffectId.Insts.CoreCloneClone model.EffectId.Insts.CoreCmpPartialEqEffectId world.effect_keys id = .ok keys ∧
      next = {world with logical_effects := logicals, effects := effects, effect_keys := keys} := by
  rw [acknowledge_normal] at evaluated
  obtain ⟨found,read,tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨logicals,updated,tail⟩ := bind_ok_inv _ _ _ tail
  obtain ⟨effects,written,tail⟩ := bind_ok_inv _ _ _ tail
  obtain ⟨keys,indexed,returned⟩ := bind_ok_inv _ _ _ tail
  exact ⟨found,logicals,effects,keys,read,updated,written,indexed,
    by simpa only [Result.ok.injEq] using returned.symm⟩

theorem acknowledge_stale_preserves_logical (world next : model.World) (id : model.EffectId)
    (outcome : model.EffectOutcome) (stored : model.LogicalEffectRecord)
    (read : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects id.logical = .ok (some stored))
    (stale : stored.current_attempt ≠ id.attempt)
    (evaluated : effect.acknowledge_effect world id outcome = .ok next) :
    next.logical_effects = world.logical_effects := by
  obtain ⟨found,logicals,effects,keys,readFound,updated,_written,_indexed,frame⟩ :=
    acknowledge_steps world next id outcome evaluated
  have foundSame : found = some stored := by
    simpa only [Result.ok.injEq] using readFound.symm.trans read
  subst found
  have logicalSame : world.logical_effects = logicals := by
    simpa only [ackLogical,stale,if_false,Result.ok.injEq] using updated
  simpa only [frame] using logicalSame.symm

theorem acknowledge_failed_preserves_logical (world next : model.World) (id : model.EffectId)
    (failure : model.FailureKind)
    (evaluated : effect.acknowledge_effect world id (.Failed failure) = .ok next) :
    next.logical_effects = world.logical_effects := by
  obtain ⟨found,logicals,effects,keys,_read,updated,_written,_indexed,frame⟩ :=
    acknowledge_steps world next id (.Failed failure) evaluated
  have logicalSame : world.logical_effects = logicals := by
    cases found with
    | none => simpa only [ackLogical,Result.ok.injEq] using updated
    | some stored =>
      by_cases same : stored.current_attempt = id.attempt <;>
        simpa only [ackLogical,same,if_true,if_false,Result.ok.injEq] using updated
  simpa only [frame] using logicalSame.symm

theorem acknowledge_frame (world next : model.World) (id : model.EffectId)
    (outcome : model.EffectOutcome)
    (evaluated : effect.acknowledge_effect world id outcome = .ok next) :
    next = {world with logical_effects := next.logical_effects, effects := next.effects, effect_keys := next.effect_keys} := by
  obtain ⟨found,logicals,effects,keys,_read,_updated,_written,_indexed,frame⟩ :=
    acknowledge_steps world next id outcome evaluated
  subst next
  rfl

noncomputable section
local instance : DecidableEq model.LogicalEffectId := Classical.decEq _

theorem acknowledge_current_success_result (world next : model.World)
    (command : model.Command) (supported : SandboxModel.Lifecycle.PausePublication.PublicationCommand command)
    (attempt : U64) (digest : ids.ResultDigest) (stored : model.LogicalEffectRecord)
    (read : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects ⟨command⟩ = .ok (some stored))
    (current : stored.current_attempt = attempt)
    (evaluated : effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ (.Succeeded digest) = .ok next) :
    tableLookup ⟨command⟩ next.logical_effects.entries.val =
      some {stored with result := some (.Succeeded attempt digest)} := by
  classical
  obtain ⟨found,logicals,effects,keys,readFound,updated,_written,_indexed,frame⟩ :=
    acknowledge_steps world next ⟨⟨command⟩,attempt⟩ (.Succeeded digest) evaluated
  have foundSame : found = some stored := by
    simpa only [Result.ok.injEq] using readFound.symm.trans read
  subst found
  have write : store.Store.put model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      world.logical_effects ⟨command⟩ {stored with result := some (.Succeeded attempt digest)} = .ok logicals := by
    simpa only [ackLogical,current,if_true] using updated
  have lawfulAt : ∀ row, model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId.eq row ⟨command⟩ =
      .ok (decide (row = ⟨command⟩)) := by
    intro row
    exact SandboxModel.Lifecycle.PausePublication.logical_query_eq row command supported
  have queried := SandboxModel.Lifecycle.PausePublication.get_exact_at
    model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨command⟩ lawfulAt world.logical_effects
  have present : tableLookup ⟨command⟩ world.logical_effects.entries.val = some stored := by
    rw [queried] at read
    simpa only [Result.ok.injEq] using read
  obtain ⟨canonical,written,entries⟩ := put_exact_at
    model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨command⟩ lawfulAt world.logical_effects
    {stored with result := some (.Succeeded attempt digest)} (Or.inl (by rw [present]; simp))
  have same : logicals = canonical := by
    simpa only [Result.ok.injEq] using write.symm.trans written
  subst logicals
  rw [frame]
  change tableLookup ⟨command⟩ canonical.entries.val = _
  rw [entries,table_lookup_put]
  simp

end

noncomputable section
local instance : DecidableEq model.EffectId := Classical.decEq _

theorem acknowledge_effect_entries (world next : model.World)
    (command : model.Command) (supported : SandboxModel.Lifecycle.PausePublication.PublicationCommand command)
    (attempt : U64) (outcome : model.EffectOutcome)
    (capacity : TableWriteCapacity world.effects.entries.val ⟨⟨command⟩,attempt⟩)
    (evaluated : effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ outcome = .ok next) :
    next.effects.entries.val = tablePut ⟨⟨command⟩,attempt⟩ (.Acknowledged outcome) world.effects.entries.val := by
  classical
  obtain ⟨found,logicals,effects,keys,_read,_updated,written,_indexed,frame⟩ :=
    acknowledge_steps world next ⟨⟨command⟩,attempt⟩ outcome evaluated
  obtain ⟨canonical,putCanonical,entries⟩ := put_exact_at
    model.EffectId.Insts.CoreCmpPartialEqEffectId ⟨⟨command⟩,attempt⟩
    (fun row => SandboxModel.Lifecycle.PausePublication.effect_query_eq row command attempt supported)
    world.effects (.Acknowledged outcome) capacity
  have same : effects = canonical := by
    simpa only [Result.ok.injEq] using written.symm.trans putCanonical
  simpa only [frame,same] using entries

theorem acknowledge_current_success_check (world next : model.World)
    (command : model.Command) (supported : SandboxModel.Lifecycle.PausePublication.PublicationCommand command)
    (attempt : U64) (digest : ids.ResultDigest) (stored : model.LogicalEffectRecord)
    (read : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects ⟨command⟩ = .ok (some stored))
    (current : stored.current_attempt = attempt)
    (capacity : TableWriteCapacity world.effects.entries.val ⟨⟨command⟩,attempt⟩)
    (evaluated : effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ (.Succeeded digest) = .ok next) :
    effect.effect_succeeded_check next command = .ok true := by
  classical
  have logical := acknowledge_current_success_result world next command supported attempt digest stored read current evaluated
  have effects := acknowledge_effect_entries world next command supported attempt (.Succeeded digest) capacity evaluated
  rw [SandboxModel.Lifecycle.PausePublication.effect_exact next command supported]
  simp only [SandboxModel.Lifecycle.PausePublication.effectValue,
    SandboxModel.Lifecycle.PausePublication.currentAttempt,logical,
    SandboxModel.Lifecycle.PausePublication.attemptOf,current,effects,table_lookup_put,if_true]

theorem acknowledge_stale_current_check_unchanged (world next : model.World)
    (command : model.Command) (supported : SandboxModel.Lifecycle.PausePublication.PublicationCommand command)
    (attempt : U64) (outcome : model.EffectOutcome) (stored : model.LogicalEffectRecord)
    (read : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects ⟨command⟩ = .ok (some stored))
    (stale : stored.current_attempt ≠ attempt)
    (capacity : TableWriteCapacity world.effects.entries.val ⟨⟨command⟩,attempt⟩)
    (evaluated : effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ outcome = .ok next) :
    effect.effect_succeeded_check next command = effect.effect_succeeded_check world command := by
  classical
  have logicalSame := acknowledge_stale_preserves_logical world next ⟨⟨command⟩,attempt⟩ outcome stored read stale evaluated
  have effects := acknowledge_effect_entries world next command supported attempt outcome capacity evaluated
  have queried := SandboxModel.Lifecycle.PausePublication.get_exact_at
    model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨command⟩
    (fun row => SandboxModel.Lifecycle.PausePublication.logical_query_eq row command supported) world.logical_effects
  have present : tableLookup ⟨command⟩ world.logical_effects.entries.val = some stored := by
    rw [queried] at read
    simpa only [Result.ok.injEq] using read
  rw [SandboxModel.Lifecycle.PausePublication.effect_exact next command supported,
    SandboxModel.Lifecycle.PausePublication.effect_exact world command supported]
  simp only [SandboxModel.Lifecycle.PausePublication.effectValue,
    SandboxModel.Lifecycle.PausePublication.currentAttempt,logicalSame,present,
    SandboxModel.Lifecycle.PausePublication.attemptOf,effects,table_lookup_put]
  have different : (⟨⟨command⟩,stored.current_attempt⟩ : model.EffectId) ≠ ⟨⟨command⟩,attempt⟩ := by
    intro same
    exact stale (congrArg model.EffectId.attempt same)
  simp only [different,if_false]

end

@[step] theorem receipt_index_present (clone : core.clone.Clone T) (eq : core.cmp.PartialEq T T)
    [DecidableEq T] (values : alloc.vec.Vec T) (value : T)
    (lawfulAt : ∀ stored, eq.eq stored value = .ok (decide (stored = value)))
    (present : value ∈ values.val) :
    effect.insert_unique clone eq values value ⦃ result => result = values ⦄ := by
  unfold effect.insert_unique effect.insert_unique_loop0
  apply Aeneas.Std.loop.spec_decr_nat
    (fun index : Usize => values.val.length - index.val)
    (fun index : Usize => index.val ≤ values.val.length ∧ value ∉ values.val.take index.val)
    (fun result => result = values)
  · intro index invariant
    rcases invariant with ⟨indexBound,absentPrefix⟩
    unfold effect.insert_unique_loop0.body
    dsimp only
    split
    · rename_i inBounds
      rw [alloc.vec.Vec.index_slice_index]
      step as ⟨stored,storedPost⟩
      have indexStrict : index.val < values.val.length := by simpa using inBounds
      simp only [lawfulAt stored]
      by_cases same : stored = value
      · simp [same]
      · simp [same]
        step as ⟨nextIndex,nextIndexPost⟩
        constructor
        · rw [nextIndexPost]
          omega
        · constructor
          · rw [nextIndexPost,List.take_succ_eq_append_getElem indexStrict,← storedPost]
            simp [absentPrefix]
            exact fun equal => same equal.symm
          · rw [nextIndexPost]
            omega
    · rename_i outside
      have lower : values.val.length ≤ index.val := by simpa using outside
      have equal : index.val = values.val.length := by omega
      have absent : value ∉ values.val := by simpa [equal] using absentPrefix
      exact False.elim (absent present)
  · exact ⟨by simp,by simp⟩

noncomputable section
local instance : DecidableEq model.EffectId := Classical.decEq _
local instance : DecidableEq model.LogicalEffectId := Classical.decEq _

theorem acknowledge_current_accepts (world : model.World)
    (command : model.Command) (supported : SandboxModel.Lifecycle.PausePublication.PublicationCommand command)
    (attempt : U64) (digest : ids.ResultDigest) (stored : model.LogicalEffectRecord)
    (read : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects ⟨command⟩ = .ok (some stored))
    (current : stored.current_attempt = attempt)
    (capacity : TableWriteCapacity world.effects.entries.val ⟨⟨command⟩,attempt⟩)
    (indexed : (⟨⟨command⟩,attempt⟩ : model.EffectId) ∈ world.effect_keys.val) :
    ∃ next, effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ (.Succeeded digest) = .ok next ∧
      effect.effect_succeeded_check next command = .ok true := by
  classical
  have logicalLaw := fun row => SandboxModel.Lifecycle.PausePublication.logical_query_eq row command supported
  have queried := SandboxModel.Lifecycle.PausePublication.get_exact_at
    model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨command⟩ logicalLaw world.logical_effects
  have present : tableLookup ⟨command⟩ world.logical_effects.entries.val = some stored := by
    rw [queried] at read
    simpa only [Result.ok.injEq] using read
  obtain ⟨logicals,writeLogical,_entries⟩ := put_exact_at
    model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨command⟩ logicalLaw world.logical_effects
    {stored with result := some (.Succeeded attempt digest)} (Or.inl (by rw [present]; simp))
  obtain ⟨effects,writeEffect,_effectEntries⟩ := put_exact_at
    model.EffectId.Insts.CoreCmpPartialEqEffectId ⟨⟨command⟩,attempt⟩
    (fun row => SandboxModel.Lifecycle.PausePublication.effect_query_eq row command attempt supported)
    world.effects (.Acknowledged (.Succeeded digest)) capacity
  obtain ⟨keys,writeKeys,sameKeys⟩ := WP.spec_imp_exists
    (receipt_index_present model.EffectId.Insts.CoreCloneClone model.EffectId.Insts.CoreCmpPartialEqEffectId
      world.effect_keys ⟨⟨command⟩,attempt⟩
      (fun row => SandboxModel.Lifecycle.PausePublication.effect_query_eq row command attempt supported) indexed)
  let next := {world with logical_effects := logicals, effects := effects, effect_keys := keys}
  simp only [current] at writeLogical
  have accepts : effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ (.Succeeded digest) = .ok next := by
    simp only [acknowledge_normal,read,bind_tc_ok,ackLogical,current,if_true,writeLogical,writeEffect,writeKeys]
    rfl
  exact ⟨next,accepts,acknowledge_current_success_check world next command supported attempt digest stored read current capacity accepts⟩

end

end SandboxModel.Lifecycle.PauseDispatch
