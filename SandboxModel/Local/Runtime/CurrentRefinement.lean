import SandboxModel.Local.Runtime.CurrentModel
import SandboxModel.Local.Runtime.Refinement
import SandboxModel.Local.State.Commands
import SandboxModel.Access.Lookup

namespace SandboxModel.Local.CurrentRuntime
open Aeneas Aeneas.Std SandproofNetworkRust SandboxModel.Local.State

local instance : DecidableEq sandbox_controller_core.model.EffectId := fun a b =>
  decidable_of_iff (State.model.EffectId.view a = State.model.EffectId.view b)
    State.model.EffectId.view_injective.eq_iff

def resultView :
    core.result.Result sandbox_controller_core.recovery_monitor.RuntimeDecision
      sandbox_controller_core.recovery_monitor.RuntimeReject →
      Except State.recovery_monitor.RuntimeReject State.recovery_monitor.RuntimeDecision
  | .Ok decision => .ok (State.recovery_monitor.RuntimeDecision.view decision)
  | .Err reject => .error (State.recovery_monitor.RuntimeReject.view reject)

theorem effect_equality_lawful :
    SandboxModel.Foundations.TableLawfulEq
      sandbox_controller_core.model.EffectId.Insts.CoreCmpPartialEqEffectId := by
  intro a b
  change sandbox_controller_core.model.EffectId.Insts.CoreCmpPartialEqEffectId.eq a b =
    .ok (decide (a = b))
  rw [State.effect_eq]
  simp only [State.model.EffectId.view_injective.eq_iff]

theorem effect_lookup_view (world : sandbox_controller_core.model.World)
    (effect : sandbox_controller_core.model.EffectId) :
    SandboxModel.Foundations.tableLookup (State.model.EffectId.view effect)
        (State.model.World.view world).effects.entries =
      (SandboxModel.Foundations.tableLookup effect world.effects.entries.val).map
        State.model.EffectStatus.view := by
  exact SandboxModel.Access.table_lookup_map State.model.EffectId.view State.model.EffectStatus.view
    State.model.EffectId.view_injective _ _

theorem duplicate_exact (world : sandbox_controller_core.model.World)
    (effect : sandbox_controller_core.model.EffectId) :
    sandbox_controller_core.recovery_monitor.handle_duplicate_or_late_delivery world effect =
      .ok (if (SandboxModel.Foundations.tableLookup effect world.effects.entries.val).isSome then
        .Ok ⟨.DuplicateOrLateDelivery effect, world⟩ else .Err .EffectNotFound) := by
  simp [sandbox_controller_core.recovery_monitor.handle_duplicate_or_late_delivery,
    sandbox_controller_core.store.Store.contains_key,
    SandboxModel.Access.store_get_refines _ effect_equality_lawful,
    SandboxModel.Access.model_World_clone_exact, core.option.Option.is_some]
  split <;> rfl

theorem root_refines (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.recovery_monitor.RuntimeInput) :
    ∃ out, SandproofNetworkRust.runtime world input = .ok out ∧
      resultView out = transition (State.model.World.view world)
        (State.recovery_monitor.RuntimeInput.view input) := by
  cases input with
  | Crash =>
      cases flag : world.controller_up <;>
        simp [ValueView.view, SandproofNetworkRust.runtime, sandbox_controller_core.recovery_monitor.handle_runtime,
          sandbox_controller_core.recovery_monitor.handle_controller_crash,
          SandboxModel.Access.model_World_clone_exact, resultView, transition,
          State.recovery_monitor.RuntimeInput.view, State.recovery_monitor.RuntimeDecision.view,
          State.recovery_monitor.RuntimeEvent.view, State.recovery_monitor.RuntimeReject.view,
          State.model.World.view, flag]
  | Recover =>
      cases flag : world.controller_up <;>
        simp [ValueView.view, SandproofNetworkRust.runtime, sandbox_controller_core.recovery_monitor.handle_runtime,
          sandbox_controller_core.recovery_monitor.handle_controller_recover,
          SandboxModel.Access.model_World_clone_exact, resultView, transition,
          State.recovery_monitor.RuntimeInput.view, State.recovery_monitor.RuntimeDecision.view,
          State.recovery_monitor.RuntimeEvent.view, State.recovery_monitor.RuntimeReject.view,
          State.model.World.view, flag]
  | DuplicateOrLateDelivery effect =>
      simp only [SandproofNetworkRust.runtime, sandbox_controller_core.recovery_monitor.handle_runtime,
        duplicate_exact, transition, State.recovery_monitor.RuntimeInput.view]
      dsimp only [ValueView.view]
      simp only [effect_lookup_view]
      cases found : SandboxModel.Foundations.tableLookup effect world.effects.entries.val <;>
        simp [ValueView.view, resultView, State.recovery_monitor.RuntimeDecision.view,
          State.recovery_monitor.RuntimeEvent.view, State.recovery_monitor.RuntimeReject.view]

theorem root_refines_represented (world : sandbox_controller_core.model.World)
    (abstract : State.model.World) (rep : State.WorldRep world abstract)
    (input : sandbox_controller_core.recovery_monitor.RuntimeInput) :
    ∃ out, SandproofNetworkRust.runtime world input = .ok out ∧
      resultView out = transition abstract (State.recovery_monitor.RuntimeInput.view input) := by
  rw [← rep]
  exact root_refines world input

theorem model_accepted_frame (world : State.model.World)
    (input : State.recovery_monitor.RuntimeInput) (decision : State.recovery_monitor.RuntimeDecision)
    (accepted : transition world input = .ok decision) :
    {decision.next with controller_up := false} = {world with controller_up := false} := by
  cases input <;> dsimp only [transition] at accepted
  all_goals split at accepted
  all_goals simp only [Except.ok.injEq, reduceCtorEq] at accepted
  all_goals rw [← accepted]

theorem root_accepted_frame (world : sandbox_controller_core.model.World)
    (input : sandbox_controller_core.recovery_monitor.RuntimeInput)
    (decision : sandbox_controller_core.recovery_monitor.RuntimeDecision)
    (accepted : SandproofNetworkRust.runtime world input = .ok (.Ok decision)) :
    {(State.model.World.view decision.next) with controller_up := false} =
      {(State.model.World.view world) with controller_up := false} := by
  obtain ⟨out, actual, related⟩ := root_refines world input
  rw [accepted] at actual
  simp only [Result.ok.injEq] at actual
  subst out
  exact model_accepted_frame _ _ _ related.symm

theorem model_crash_recover_roundtrip (world : State.model.World) (up : world.controller_up = true) :
    transition world .Crash = .ok ⟨.ControllerCrash, {world with controller_up := false}⟩ ∧
    transition {world with controller_up := false} .Recover = .ok ⟨.ControllerRecover, world⟩ := by
  constructor
  · simp [transition, up]
  · simp only [transition, Bool.false_eq_true, ↓reduceIte]
    congr
    cases world
    simp_all

theorem model_duplicate_never_updates (world : State.model.World) (effect : State.model.EffectId)
    (decision : State.recovery_monitor.RuntimeDecision)
    (accepted : transition world (.DuplicateOrLateDelivery effect) = .ok decision) :
    decision.next = world := by
  simp only [transition] at accepted
  split at accepted
  all_goals simp only [Except.ok.injEq, reduceCtorEq] at accepted
  rw [← accepted]

end SandboxModel.Local.CurrentRuntime
