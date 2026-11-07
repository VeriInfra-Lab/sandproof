import SandboxModel.Lifecycle.PauseDispatch.Acknowledgements

namespace SandboxModel.Lifecycle.PauseDispatch
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

noncomputable section
local instance receiptPairDecidable1 : DecidableEq model.LogicalEffectId := Classical.decEq _
local instance receiptPairDecidable2 : DecidableEq model.EffectId := Classical.decEq _

theorem acknowledge_current_logical_entries (world next : model.World)
    (command : model.Command) (supported : SandboxModel.Lifecycle.PausePublication.PublicationCommand command)
    (attempt : U64) (digest : ids.ResultDigest) (stored : model.LogicalEffectRecord)
    (read : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects ⟨command⟩ = .ok (some stored))
    (current : stored.current_attempt = attempt)
    (evaluated : effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ (.Succeeded digest) = .ok next) :
    next.logical_effects.entries.val = tablePut ⟨command⟩
      {stored with result := some (.Succeeded attempt digest)} world.logical_effects.entries.val := by
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
  simpa only [frame] using entries

theorem acknowledge_current_other_check (world next : model.World)
    (command query : model.Command)
    (supported : SandboxModel.Lifecycle.PausePublication.PublicationCommand command)
    (querySupported : SandboxModel.Lifecycle.PausePublication.PublicationCommand query)
    (different : query ≠ command) (attempt : U64) (digest : ids.ResultDigest) (stored : model.LogicalEffectRecord)
    (read : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects ⟨command⟩ = .ok (some stored))
    (current : stored.current_attempt = attempt)
    (capacity : TableWriteCapacity world.effects.entries.val ⟨⟨command⟩,attempt⟩)
    (evaluated : effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ (.Succeeded digest) = .ok next) :
    effect.effect_succeeded_check next query = effect.effect_succeeded_check world query := by
  classical
  have logicals := acknowledge_current_logical_entries world next command supported attempt digest stored read current evaluated
  have effects := acknowledge_effect_entries world next command supported attempt (.Succeeded digest) capacity evaluated
  have logicalDifferent : (⟨query⟩ : model.LogicalEffectId) ≠ ⟨command⟩ := by
    intro same
    exact different (congrArg model.LogicalEffectId.command same)
  have attempts : SandboxModel.Lifecycle.PausePublication.currentAttempt next query =
      SandboxModel.Lifecycle.PausePublication.currentAttempt world query := by
    simp only [SandboxModel.Lifecycle.PausePublication.currentAttempt,logicals,table_lookup_put,logicalDifferent,if_false]
  rw [SandboxModel.Lifecycle.PausePublication.effect_exact next query querySupported,
    SandboxModel.Lifecycle.PausePublication.effect_exact world query querySupported]
  simp only [SandboxModel.Lifecycle.PausePublication.effectValue,attempts,effects,table_lookup_put]
  have keyDifferent : (⟨⟨query⟩,SandboxModel.Lifecycle.PausePublication.currentAttempt world query⟩ : model.EffectId) ≠ ⟨⟨command⟩,attempt⟩ := by
    intro same
    exact logicalDifferent (congrArg model.EffectId.logical same)
  simp only [keyDifferent,if_false]

theorem acknowledge_present_index_frame (world next : model.World)
    (command : model.Command) (supported : SandboxModel.Lifecycle.PausePublication.PublicationCommand command)
    (attempt : U64) (outcome : model.EffectOutcome)
    (indexed : (⟨⟨command⟩,attempt⟩ : model.EffectId) ∈ world.effect_keys.val)
    (evaluated : effect.acknowledge_effect world ⟨⟨command⟩,attempt⟩ outcome = .ok next) :
    next.effect_keys = world.effect_keys := by
  classical
  obtain ⟨found,logicals,effects,keys,_read,_logical,_effect,indexWrite,frame⟩ :=
    acknowledge_steps world next ⟨⟨command⟩,attempt⟩ outcome evaluated
  obtain ⟨actual,ran,same⟩ := WP.spec_imp_exists
    (receipt_index_present model.EffectId.Insts.CoreCloneClone model.EffectId.Insts.CoreCmpPartialEqEffectId
      world.effect_keys ⟨⟨command⟩,attempt⟩
      (fun row => SandboxModel.Lifecycle.PausePublication.effect_query_eq row command attempt supported) indexed)
  have sameKeys : keys = actual := by simpa only [Result.ok.injEq] using indexWrite.symm.trans ran
  simpa only [frame,sameKeys] using same

end

noncomputable section
local instance receiptPairDecidable3 : DecidableEq model.LogicalEffectId := Classical.decEq _
local instance receiptPairDecidable4 : DecidableEq model.EffectId := Classical.decEq _

theorem two_current_receipts_accept (world : model.World)
    (first second : model.Command)
    (firstSupported : SandboxModel.Lifecycle.PausePublication.PublicationCommand first)
    (secondSupported : SandboxModel.Lifecycle.PausePublication.PublicationCommand second)
    (different : second ≠ first) (firstAttempt secondAttempt : U64)
    (firstDigest secondDigest : ids.ResultDigest) (firstRecord secondRecord : model.LogicalEffectRecord)
    (readFirst : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects ⟨first⟩ = .ok (some firstRecord))
    (readSecond : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId world.logical_effects ⟨second⟩ = .ok (some secondRecord))
    (firstCurrent : firstRecord.current_attempt = firstAttempt)
    (secondCurrent : secondRecord.current_attempt = secondAttempt)
    (firstPresent : tableLookup ⟨⟨first⟩,firstAttempt⟩ world.effects.entries.val ≠ none)
    (secondPresent : tableLookup ⟨⟨second⟩,secondAttempt⟩ world.effects.entries.val ≠ none)
    (firstIndexed : (⟨⟨first⟩,firstAttempt⟩ : model.EffectId) ∈ world.effect_keys.val)
    (secondIndexed : (⟨⟨second⟩,secondAttempt⟩ : model.EffectId) ∈ world.effect_keys.val) :
    ∃ middle next,
      effect.acknowledge_effect world ⟨⟨first⟩,firstAttempt⟩ (.Succeeded firstDigest) = .ok middle ∧
      effect.acknowledge_effect middle ⟨⟨second⟩,secondAttempt⟩ (.Succeeded secondDigest) = .ok next ∧
      store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId middle.logical_effects ⟨second⟩ = .ok (some secondRecord) ∧
      TableWriteCapacity middle.effects.entries.val ⟨⟨second⟩,secondAttempt⟩ ∧
      effect.effect_succeeded_check next first = .ok true ∧
      effect.effect_succeeded_check next second = .ok true ∧
      next.sandboxes = world.sandboxes ∧ next.vms = world.vms := by
  classical
  obtain ⟨middle,firstAck,firstSuccess⟩ := acknowledge_current_accepts world first firstSupported
    firstAttempt firstDigest firstRecord readFirst firstCurrent (Or.inl firstPresent) firstIndexed
  have logicals := acknowledge_current_logical_entries world middle first firstSupported
    firstAttempt firstDigest firstRecord readFirst firstCurrent firstAck
  have effects := acknowledge_effect_entries world middle first firstSupported firstAttempt
    (.Succeeded firstDigest) (Or.inl firstPresent) firstAck
  have logicalDifferent : (⟨second⟩ : model.LogicalEffectId) ≠ ⟨first⟩ := by
    intro same
    exact different (congrArg model.LogicalEffectId.command same)
  have keyDifferent : (⟨⟨second⟩,secondAttempt⟩ : model.EffectId) ≠ ⟨⟨first⟩,firstAttempt⟩ := by
    intro same
    exact logicalDifferent (congrArg model.EffectId.logical same)
  have secondLookup : tableLookup ⟨second⟩ world.logical_effects.entries.val = some secondRecord := by
    rw [SandboxModel.Lifecycle.PausePublication.get_exact_at
      model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨second⟩
      (fun row => SandboxModel.Lifecycle.PausePublication.logical_query_eq row second secondSupported)] at readSecond
    simpa only [Result.ok.injEq] using readSecond
  have secondRead : store.Store.get model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      middle.logical_effects ⟨second⟩ = .ok (some secondRecord) := by
    rw [SandboxModel.Lifecycle.PausePublication.get_exact_at
      model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId ⟨second⟩
      (fun row => SandboxModel.Lifecycle.PausePublication.logical_query_eq row second secondSupported)]
    simp only [logicals,table_lookup_put,logicalDifferent,if_false,secondLookup]
  have secondRoom : TableWriteCapacity middle.effects.entries.val ⟨⟨second⟩,secondAttempt⟩ := by
    apply Or.inl
    simpa only [effects,table_lookup_put,keyDifferent,if_false] using secondPresent
  have indexFrame := acknowledge_present_index_frame world middle first firstSupported
    firstAttempt (.Succeeded firstDigest) firstIndexed firstAck
  have indexed : (⟨⟨second⟩,secondAttempt⟩ : model.EffectId) ∈ middle.effect_keys.val := by
    simpa only [indexFrame] using secondIndexed
  obtain ⟨next,secondAck,secondSuccess⟩ := acknowledge_current_accepts middle second secondSupported
    secondAttempt secondDigest secondRecord secondRead secondCurrent secondRoom indexed
  have firstRemains := acknowledge_current_other_check middle next second first secondSupported
    firstSupported (Ne.symm different) secondAttempt secondDigest secondRecord secondRead secondCurrent secondRoom secondAck
  have frameOne := acknowledge_frame world middle ⟨⟨first⟩,firstAttempt⟩ (.Succeeded firstDigest) firstAck
  have frameTwo := acknowledge_frame middle next ⟨⟨second⟩,secondAttempt⟩ (.Succeeded secondDigest) secondAck
  have sandboxOne := congrArg model.World.sandboxes frameOne
  have sandboxTwo := congrArg model.World.sandboxes frameTwo
  have vmOne := congrArg model.World.vms frameOne
  have vmTwo := congrArg model.World.vms frameTwo
  exact ⟨middle,next,firstAck,secondAck,secondRead,secondRoom,firstRemains.trans firstSuccess,secondSuccess,
    sandboxTwo.trans sandboxOne,vmTwo.trans vmOne⟩

end

end SandboxModel.Lifecycle.PauseDispatch
