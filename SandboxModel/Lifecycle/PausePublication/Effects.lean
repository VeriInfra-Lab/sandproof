import SandboxModel.Lifecycle.CaptureHandoff.CallerCloning
import SandboxModel.Lifecycle.CaptureHandoff.CallerLate
import SandboxModel.Lifecycle.PausePublication.Queries
import SandboxModel.Lifecycle.PausePublication.Commands

namespace SandboxModel.Lifecycle.PausePublication
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

def attemptOf : Option model.LogicalEffectRecord → U64
  | none => 0#u64
  | some record => record.current_attempt

/-- The actual two Rust queries, including the absent-logical-row fallback. -/
theorem effect_success_queries (world : model.World) (command : model.Command)
    (succeeded : effect.effect_succeeded_check world command = .ok true) :
    ∃ logical receipt,
      store.Store.get_copy model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
        model.LogicalEffectRecord.Insts.CoreMarkerCopy world.logical_effects ⟨command⟩ = .ok logical ∧
      store.Store.get model.EffectId.Insts.CoreCmpPartialEqEffectId world.effects
        ⟨⟨command⟩, attemptOf logical⟩ = .ok (some (.Acknowledged (.Succeeded receipt))) := by
  simp only [effect.effect_succeeded_check, model_Command_clone_exact, bind_tc_ok] at succeeded
  obtain ⟨logical, queried, tail⟩ := bind_ok_inv _ _ _ succeeded
  refine ⟨logical, ?_⟩
  cases logical with
  | none =>
    simp only [bind_tc_ok] at tail
    obtain ⟨status, found, check⟩ := bind_ok_inv _ _ _ tail
    cases status with
    | none => simp at check
    | some status =>
      cases status <;> try { simp at check }
      rename_i outcome
      cases outcome <;> try { simp at check }
      rename_i receipt
      exact ⟨receipt, queried, found⟩
  | some logical =>
    simp only [bind_tc_ok] at tail
    obtain ⟨status, found, check⟩ := bind_ok_inv _ _ _ tail
    cases status with
    | none => simp at check
    | some status =>
      cases status <;> try { simp at check }
      rename_i outcome
      cases outcome <;> try { simp at check }
      rename_i receipt
      exact ⟨receipt, queried, found⟩

noncomputable section
local instance : DecidableEq model.EffectId := Classical.decEq _

noncomputable def currentAttempt (world : model.World) (command : model.Command) : U64 := by
  classical
  exact attemptOf (tableLookup ⟨command⟩ world.logical_effects.entries.val)

noncomputable def effectValue (world : model.World) (command : model.Command) : Bool := by
  classical
  exact match tableLookup ⟨⟨command⟩, currentAttempt world command⟩ world.effects.entries.val with
  | some (.Acknowledged (.Succeeded _)) => true
  | _ => false

theorem effect_exact (world : model.World) (command : model.Command)
    (supported : PublicationCommand command) :
    effect.effect_succeeded_check world command = .ok (effectValue world command) := by
  classical
  have logical := get_exact_at model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    ⟨command⟩ (fun stored => logical_query_eq stored command supported) world.logical_effects
  simp only [effect.effect_succeeded_check, model_Command_clone_exact,
    bind_tc_ok, get_copy_agrees, logical]
  cases found : tableLookup ⟨command⟩ world.logical_effects.entries.val with
  | none =>
    simp only [bind_tc_ok]
    rw [get_exact_at model.EffectId.Insts.CoreCmpPartialEqEffectId ⟨⟨command⟩, 0#u64⟩
      (fun stored => effect_query_eq stored command 0#u64 supported) world.effects]
    simp only [effectValue, currentAttempt, found, attemptOf, bind_tc_ok]
    cases statusQuery : tableLookup ⟨⟨command⟩, 0#u64⟩ world.effects.entries.val with
    | none => rfl
    | some status =>
      cases status <;> try { rfl }
      rename_i outcome
      cases outcome <;> rfl
  | some record =>
    simp only [bind_tc_ok]
    rw [get_exact_at model.EffectId.Insts.CoreCmpPartialEqEffectId ⟨⟨command⟩, record.current_attempt⟩
      (fun stored => effect_query_eq stored command record.current_attempt supported) world.effects]
    simp only [bind_tc_ok, effectValue, currentAttempt, found, attemptOf]
    cases statusQuery : tableLookup ⟨⟨command⟩, record.current_attempt⟩ world.effects.entries.val with
    | none => rfl
    | some status =>
      cases status <;> try { rfl }
      rename_i outcome
      cases outcome <;> rfl

theorem effect_success_iff (world : model.World) (command : model.Command)
    (supported : PublicationCommand command) :
    effect.effect_succeeded_check world command = .ok true ↔
      ∃ receipt, tableLookup ⟨⟨command⟩, currentAttempt world command⟩ world.effects.entries.val =
        some (.Acknowledged (.Succeeded receipt)) := by
  classical
  rw [effect_exact world command supported]
  unfold effectValue
  cases found : tableLookup ⟨⟨command⟩, currentAttempt world command⟩ world.effects.entries.val with
  | none => simp
  | some status =>
    cases status <;> try { simp }
    rename_i outcome
    cases outcome <;> simp

end
end SandboxModel.Lifecycle.PausePublication
