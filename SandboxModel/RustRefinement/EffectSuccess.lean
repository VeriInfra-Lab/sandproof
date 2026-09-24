import SandboxModel.RustRefinement.ReceiptEvidence
import SandboxModel.RustRefinement.WorkerHandler

/-! Success checks used by scheduling and stable publication refer only to
the current exact attempt, never any historical successful attempt. -/

namespace SandboxModel.RustRefinement

open Aeneas Aeneas.Std Result

local instance successOperationDecEq : DecidableEq SandboxControllerRust.model.OperationKey :=
  operationKeyDecidableEq
local instance successLogicalDecEq : DecidableEq SandboxControllerRust.model.LogicalEffectId :=
  logicalEffectIdDecidableEq
local instance successEffectDecEq : DecidableEq SandboxControllerRust.model.EffectId :=
  effectIdDecidableEq

theorem generated_effectSucceeded_refines
    (rust : SandboxControllerRust.model.World) (model : World)
    (represented : EffectWorldRep rust model)
    (input : SandboxControllerRust.model.Command) :
    SandboxControllerRust.effect.effect_succeeded_check rust input =
      ok (SandboxModel.effectSucceededCheck model (command input)) := by
  have getCopy := generated_get_copy_refines
    SandboxControllerRust.model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    logicalEffectId_partialEq_lawful
    SandboxControllerRust.model.LogicalEffectRecord.Insts.CoreMarkerCopy
    rust.logical_effects ⟨input⟩
  obtain ⟨_, copyEval, rfl⟩ := Aeneas.Std.WP.spec_imp_exists getCopy
  have logicalLookup := storeRep_lookup_map represented.logicalEffects
    (SandboxControllerRust.model.LogicalEffectId.mk input)
  have effectLookup := storeRep_lookup_map represented.effects
  have getEffect := fun id => generated_get_eq
    SandboxControllerRust.model.EffectId.Insts.CoreCmpPartialEqEffectId
    effectId_partialEq_lawful rust.effects id
  simp [SandboxControllerRust.effect.effect_succeeded_check, copyEval]
  unfold effectSucceededCheck currentEffectId
  dsimp only
  change model.logicalEffects (SandboxModel.logicalEffectId (command input)) =
    (storeLookup rust.logical_effects ⟨input⟩).map logicalEffectRecord at logicalLookup
  rw [logicalLookup]
  have mapped (attempt : U64) : model.effects (effectIdAt (command input) (u64ToNat attempt)) =
      (storeLookup rust.effects ⟨⟨input⟩, attempt⟩).map effectStatus := by
    exact effectLookup ⟨⟨input⟩, attempt⟩
  cases found : storeLookup rust.logical_effects ⟨input⟩ with
  | none =>
      have zero := mapped (0#u64)
      change model.effects (SandboxModel.effectId (command input)) = _ at zero
      simp [getEffect, zero]
      repeat' (split <;> simp_all [effectStatus, effectOutcome])
  | some record =>
      simp [getEffect, logicalEffectRecord, mapped]
      repeat' (split <;> simp_all [effectStatus, effectOutcome])

end SandboxModel.RustRefinement
