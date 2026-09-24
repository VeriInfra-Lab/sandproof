import SandboxModel.RustRefinement.EffectHandler

/-! Whole-world refinement of the generated receipt update.  A successful
old attempt must not complete the currently selected logical attempt. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance receiptUpdateOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance receiptUpdateLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance receiptUpdateEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

theorem generated_acknowledgeEffect_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model) (room : HandlerMachineHeadroom rust)
    (id : SandboxControllerRust.model.EffectId)
    (outcome : SandboxControllerRust.model.EffectOutcome) :
    ∃ next, SandboxControllerRust.effect.acknowledge_effect rust id outcome = ok next ∧
      EffectWorldRep next (acknowledgeEffect model (effectId id) (effectOutcome outcome)) := by
  obtain ⟨effects, effectsEval, effectsRep⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_put_preserves_storeRep
      SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId effectId_partialEq_lawful
      effectId effectId_injective (fun concrete abstract => effectStatus concrete = abstract)
      rust.effects model.effects represented.effects id (.Acknowledged outcome)
      (.acknowledged (effectOutcome outcome)) rfl room.effects)
  obtain ⟨keys, keysEval, keysValue⟩ := Aeneas.Std.WP.spec_imp_exists
    (generated_effectInsertUnique_refines rust.effect_keys id room.effectKeys)
  let next := fun logicalStore => { rust with
    logical_effects := logicalStore
    effects := effects
    effect_keys := keys }
  have finish (logicalStore modelLogical)
      (logicalRep : StoreRep logicalEffectId (fun concrete abstract => logicalEffectRecord concrete = abstract)
        logicalStore modelLogical) :
      EffectWorldRep (next logicalStore) { model with
        logicalEffects := modelLogical
        effects := put model.effects (effectId id) (.acknowledged (effectOutcome outcome))
        effectKeys := insertUnique (effectId id) model.effectKeys } := by
    apply worldRep_setEffectProtocol represented logicalStore modelLogical logicalRep
      effects _ effectsRep keys _
    · have representedKeys : rust.effect_keys.val.map effectId = model.effectKeys := represented.effectKeys
      rw [keysValue, map_insertUnique_of_injective effectId_injective, representedKeys]
    · rw [keysValue]
      exact nodup_insertUnique id _ represented.effectKeysUnique
    · exact represented.inflight
    · exact represented.inflightSupport
    · exact represented.inflightUnique
  have getLogical := generated_get_eq
    SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    logicalEffectId_partialEq_lawful rust.logical_effects id.logical
  cases found : storeLookup rust.logical_effects id.logical with
  | none =>
      have modelFound : model.logicalEffects (logicalEffectId id.logical) = none := by
        have agrees := represented.logicalEffects.lookupAgrees id.logical
        change OptionRel (fun concrete abstract => logicalEffectRecord concrete = abstract)
          (storeLookup rust.logical_effects id.logical) (model.logicalEffects (logicalEffectId id.logical)) at agrees
        rw [found] at agrees
        cases lookup : model.logicalEffects (logicalEffectId id.logical) <;> simp_all [OptionRel]
      refine ⟨next rust.logical_effects, ?_, ?_⟩
      · simp [SandboxControllerRust.effect.acknowledge_effect, world_clone_eq, getLogical,
          found, effectsEval, keysEval, next]
      · simpa [acknowledgeEffect, effectId, modelFound] using
          finish rust.logical_effects model.logicalEffects represented.logicalEffects
  | some record =>
      have modelFound := logicalLookup_some represented id.logical record found
      have attemptEq : u64ToNat record.current_attempt = u64ToNat id.attempt ↔ record.current_attempt = id.attempt :=
        u64ToNat_injective.eq_iff
      by_cases current : record.current_attempt = id.attempt
      · cases outcome with
        | Failed failure =>
            refine ⟨next rust.logical_effects, ?_, ?_⟩
            · simp [SandboxControllerRust.effect.acknowledge_effect, world_clone_eq, getLogical,
                found, current, effectsEval, keysEval, next]
            · simpa [acknowledgeEffect, effectId, modelFound, logicalEffectRecord, current, effectOutcome] using
                finish rust.logical_effects model.logicalEffects represented.logicalEffects
        | Succeeded digest =>
            let updated := { record with result := some (.Succeeded id.attempt digest) }
            obtain ⟨logicalStore, logicalEval, logicalRep⟩ := Aeneas.Std.WP.spec_imp_exists
              (generated_put_preserves_storeRep
                SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId logicalEffectId_partialEq_lawful
                logicalEffectId logicalEffectId_injective (fun concrete abstract => logicalEffectRecord concrete = abstract)
                rust.logical_effects model.logicalEffects represented.logicalEffects id.logical updated
                (logicalEffectRecord updated) rfl room.logicalEffects)
            refine ⟨next logicalStore, ?_, ?_⟩
            · dsimp [updated] at logicalEval
              rw [current] at logicalEval
              simp [SandboxControllerRust.effect.acknowledge_effect, world_clone_eq, getLogical,
                found, current, logicalEval, effectsEval, keysEval, next]
            · simpa [acknowledgeEffect, effectId, modelFound, logicalEffectRecord, current, effectOutcome,
                updated, logicalEffectResult] using finish logicalStore _ logicalRep
      · refine ⟨next rust.logical_effects, ?_, ?_⟩
        · simp [SandboxControllerRust.effect.acknowledge_effect, world_clone_eq, getLogical,
            found, current, effectsEval, keysEval, next]
        · simpa [acknowledgeEffect, effectId, modelFound, logicalEffectRecord, attemptEq, current] using
            finish rust.logical_effects model.logicalEffects represented.logicalEffects

end SandboxModel.RustRefinement
