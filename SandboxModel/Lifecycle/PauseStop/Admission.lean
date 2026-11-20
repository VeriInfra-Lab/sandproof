import SandboxModel.Lifecycle.PauseStop.Routes
import SandboxModel.Lifecycle.PausePublication.Effects
import SandboxModel.Foundations.FiniteScan

namespace SandboxModel.Lifecycle.PauseStop
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller
open SandboxModel.Lifecycle.PausePublication

/-- Derive actual prefix observations without assuming a valid attempt or queue. -/
theorem execute_success_prefix (world : model.World) (id : model.EffectId)
    (decision : worker.WorkerDecision)
    (evaluated : worker.pause_stop.execute world id = .ok (.Ok decision)) :
    ∃ current status,
      worker.current_effect_id world id.logical.command = .ok current ∧
      core.cmp.PartialEq.ne.trait_default model.EffectId.Insts.CoreCmpPartialEqEffectId
        current id = .ok false ∧
      worker.contains model.EffectId.Insts.CoreCmpPartialEqEffectId
        (alloc.vec.Vec.deref world.inflight) id = .ok true ∧
      store.Store.get model.EffectId.Insts.CoreCmpPartialEqEffectId world.effects id = .ok status ∧
      worker.effect_awaiting status = .ok true := by
  simp only [worker.pause_stop.execute, model_Command_clone_exact, bind_tc_ok] at evaluated
  obtain ⟨current, currentRead, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨different, compared, tail⟩ := bind_ok_inv _ _ _ tail
  cases different with
  | true => simp at tail
  | false =>
    simp only [Bool.false_eq_true, if_false] at tail
    obtain ⟨present, presence, tail⟩ := bind_ok_inv _ _ _ tail
    cases present with
    | false => simp at tail
    | true =>
      simp only [if_true] at tail
      obtain ⟨status, found, tail⟩ := bind_ok_inv _ _ _ tail
      obtain ⟨awaiting, waiting, tail⟩ := bind_ok_inv _ _ _ tail
      cases awaiting with
      | false => simp at tail
      | true => exact ⟨current, status, currentRead, compared, presence, found, waiting⟩

noncomputable def currentEffectId (world : model.World) (command : model.Command) : model.EffectId :=
  ⟨⟨command⟩, SandboxModel.Lifecycle.PausePublication.currentAttempt world command⟩

theorem current_effect_id_exact (world : model.World) (command : model.Command)
    (kind : PublicationCommand command) :
    worker.current_effect_id world command = .ok (currentEffectId world command) := by
  classical
  have queried := get_exact_at model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
    ⟨command⟩ (fun stored => logical_query_eq stored command kind) world.logical_effects
  simp only [worker.current_effect_id, model_Command_clone_exact, bind_tc_ok,
    get_copy_agrees, queried]
  cases found : tableLookup ⟨command⟩ world.logical_effects.entries.val with
  | none => simp only [currentEffectId, currentAttempt, attemptOf, found]
  | some row => simp only [currentEffectId, currentAttempt, attemptOf, found]

def awaiting : Option model.EffectStatus → Bool
  | some .Dispatched => true
  | some .OutcomeUnknown => true
  | _ => false

theorem effect_awaiting_exact (status : Option model.EffectStatus) :
    worker.effect_awaiting status = .ok (awaiting status) := by
  cases status with
  | none => rfl
  | some status => cases status <;> rfl

theorem awaiting_true_iff (status : Option model.EffectStatus) :
    awaiting status = true ↔ status = some .Dispatched ∨ status = some .OutcomeUnknown := by
  cases status with
  | none => simp [awaiting]
  | some status => cases status <;> simp [awaiting]

end SandboxModel.Lifecycle.PauseStop
