import SandboxModel.Lifecycle.PausePublication.Queries
import SandboxModel.Lifecycle.PausePublication.Commands

namespace SandboxModel.Lifecycle.PauseDispatch
open Aeneas Aeneas.Std CaptureCallerRust SandboxModel.Foundations
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

def stopId (sandbox epoch generation vm attempt : U64) : model.EffectId :=
  ⟨⟨.StopVm sandbox epoch generation vm⟩,attempt⟩

theorem released_execution_is_not_stop
    (basis : maintenance.released_execution.ReleasedExecution)
    (sandbox epoch generation vm : U64) :
    maintenance.released_execution.ReleasedExecution.command basis ≠
      .ok (.StopVm sandbox epoch generation vm) := by
  simp [maintenance.released_execution.ReleasedExecution.command]

theorem stop_selector_exact (world : model.World)
    (sandbox epoch generation vm attempt : U64) :
    effect.dispatch_stop world (stopId sandbox epoch generation vm attempt) =
      (do let response ← effect.dispatch_eligible world
            (stopId sandbox epoch generation vm attempt)
          .ok (some response)) := by
  simp [effect.dispatch_stop,stopId,model.EffectId.command,model_EffectId_clone_exact]

theorem stop_startup_not_revoked (world : model.World)
    (sandbox epoch generation vm : U64) :
    effect.startup_dispatch_revoked world (.StopVm sandbox epoch generation vm) = .ok false := by
  simp [effect.startup_dispatch_revoked,effect.is_startup_command]

theorem stop_capture_dispatch_allowed (world : model.World)
    (sandbox epoch generation vm attempt : U64) :
    capture.continuation.capture_dispatch_allowed world
      (stopId sandbox epoch generation vm attempt) = .ok true := by
  simp [capture.continuation.capture_dispatch_allowed,
    capture.continuation.capture_command,stopId,model.EffectId.command]

theorem stop_update_exact (world next : model.World)
    (sandbox epoch generation vm attempt : U64)
    (evaluated : effect.dispatch_effect world (stopId sandbox epoch generation vm attempt) = .ok next) :
    ∃ effects,
      store.Store.put model.EffectId.Insts.CoreCmpPartialEqEffectId world.effects
        (stopId sandbox epoch generation vm attempt) .Dispatched = .ok effects ∧
      next = {world with effects} := by
  simp only [effect.dispatch_effect,stopId,model_World_clone_exact,
    model.EffectId.command,model_Command_clone_exact,bind_tc_ok] at evaluated
  obtain ⟨effects,written,returned⟩ := bind_ok_inv _ _ _ evaluated
  exact ⟨effects,written,by simpa only [Result.ok.injEq] using returned.symm⟩

private def exactCompare [DecidableEq K] : core.cmp.PartialEq K K :=
  {eq := fun left right => .ok (decide (left = right))
   ne := fun left right => .ok (decide (left ≠ right))}

theorem put_exact_at [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (query : K) (lawfulAt : ∀ stored, eq.eq stored query = .ok (decide (stored = query)))
    (rows : store.Store K V) (value : V)
    (capacity : TableWriteCapacity rows.entries.val query) :
    ∃ next, store.Store.put eq rows query value = .ok next ∧
      next.entries.val = tablePut query value rows.entries.val := by
  have bodies (cursor : Usize) : store.Store.put_loop.body eq rows query value cursor =
      store.Store.put_loop.body exactCompare rows query value cursor := by
    simp only [store.Store.put_loop.body,lawfulAt,exactCompare]
  have loops : store.Store.put_loop eq rows query value =
      store.Store.put_loop exactCompare rows query value := by
    unfold store.Store.put_loop
    simp only [bodies]
  have lawful : TableLawfulEq (exactCompare (K := K)) := by
    intro left right
    rfl
  simpa only [store.Store.put,loops] using
    put_exact exactCompare lawful rows query value capacity

def dispatchableStatus : model.EffectStatus → Bool
  | .Committed | .OutcomeUnknown => true
  | _ => false

theorem eligible_stop_exact (world : model.World)
    (sandbox epoch generation vm attempt : U64) :
    effect.dispatch_eligible world (stopId sandbox epoch generation vm attempt) =
      (if world.controller_up then do
        let status ← store.Store.get model.EffectId.Insts.CoreCmpPartialEqEffectId
          world.effects (stopId sandbox epoch generation vm attempt)
        match status with
        | none => .ok (.Err .EffectNotFound)
        | some status =>
          if dispatchableStatus status then do
            let next ← effect.dispatch_effect world (stopId sandbox epoch generation vm attempt)
            .ok (.Ok ⟨.DispatchEffect (stopId sandbox epoch generation vm attempt),next⟩)
          else .ok (.Err .NotDispatchable)
       else .ok (.Err .ControllerDown)) := by
  cases up : world.controller_up <;>
    simp only [effect.dispatch_eligible,up,Bool.false_eq_true,if_false,if_true]
  congr 1
  funext status
  cases status with
  | none => rfl
  | some status =>
    cases status <;> simp [dispatchableStatus,stopId,model.EffectId.command,
      effect.startup_dispatch_revoked,effect.is_startup_command,
      capture.continuation.capture_dispatch_allowed,capture.continuation.capture_command,
      model_EffectId_clone_exact]

theorem stop_success_checks (world : model.World)
    (sandbox epoch generation vm attempt : U64) (decision : effect.EffectDecision)
    (evaluated : effect.dispatch_stop world (stopId sandbox epoch generation vm attempt) =
      .ok (some (.Ok decision))) :
    ∃ status next,
      world.controller_up = true ∧
      store.Store.get model.EffectId.Insts.CoreCmpPartialEqEffectId world.effects
        (stopId sandbox epoch generation vm attempt) = .ok (some status) ∧
      dispatchableStatus status = true ∧
      effect.dispatch_effect world (stopId sandbox epoch generation vm attempt) = .ok next ∧
      decision = ⟨.DispatchEffect (stopId sandbox epoch generation vm attempt),next⟩ := by
  rw [stop_selector_exact] at evaluated
  obtain ⟨response,ran,returned⟩ := bind_ok_inv _ _ _ evaluated
  have same : response = .Ok decision := by
    simpa only [Result.ok.injEq,Option.some.injEq] using returned
  subst response
  rw [eligible_stop_exact] at ran
  cases up : world.controller_up with
  | false => simp [up] at ran
  | true =>
    simp only [up,if_true] at ran
    obtain ⟨found,read,tail⟩ := bind_ok_inv _ _ _ ran
    cases found with
    | none => simp at tail
    | some status =>
      cases allowed : dispatchableStatus status with
      | false => simp [allowed] at tail
      | true =>
        simp only [allowed,if_true] at tail
        obtain ⟨next,worked,final⟩ := bind_ok_inv _ _ _ tail
        refine ⟨status,next,rfl,read,allowed,worked,?_⟩
        simpa only [Result.ok.injEq,core.result.Result.Ok.injEq] using final.symm

noncomputable section
local instance framesDecidable1 : DecidableEq model.EffectId := Classical.decEq _

/-- A successful selector proves the same full id is written and only the effects table changes. -/
theorem selected_stop_bound (world : model.World)
    (sandbox epoch generation vm attempt : U64) (decision : effect.EffectDecision)
    (evaluated : effect.dispatch_stop world (stopId sandbox epoch generation vm attempt) =
      .ok (some (.Ok decision))) :
    decision.event = .DispatchEffect (stopId sandbox epoch generation vm attempt) ∧
    decision.next = {world with effects := decision.next.effects} ∧
    tableLookup (stopId sandbox epoch generation vm attempt) decision.next.effects.entries.val =
      some .Dispatched := by
  classical
  obtain ⟨status,next,up,read,allowed,worked,same⟩ :=
    stop_success_checks world sandbox epoch generation vm attempt decision evaluated
  subst decision
  have lawfulAt : ∀ stored,
      model.EffectId.Insts.CoreCmpPartialEqEffectId.eq stored
        (stopId sandbox epoch generation vm attempt) =
      .ok (decide (stored = stopId sandbox epoch generation vm attempt)) := by
    intro stored
    exact SandboxModel.Lifecycle.PausePublication.effect_query_eq stored
      (.StopVm sandbox epoch generation vm) attempt (.stop sandbox epoch generation vm)
  have queried := SandboxModel.Lifecycle.PausePublication.get_exact_at
    model.EffectId.Insts.CoreCmpPartialEqEffectId
    (stopId sandbox epoch generation vm attempt) lawfulAt world.effects
  have present : tableLookup (stopId sandbox epoch generation vm attempt) world.effects.entries.val =
      some status := by
    rw [queried] at read
    simpa only [Result.ok.injEq] using read
  obtain ⟨effects,written,frame⟩ := stop_update_exact world next sandbox epoch generation vm attempt worked
  have capacity : TableWriteCapacity world.effects.entries.val (stopId sandbox epoch generation vm attempt) :=
    Or.inl (by rw [present]; simp)
  obtain ⟨canonical,putCanonical,entries⟩ := put_exact_at
    model.EffectId.Insts.CoreCmpPartialEqEffectId
    (stopId sandbox epoch generation vm attempt) lawfulAt world.effects .Dispatched capacity
  have equal : effects = canonical := by
    simpa only [Result.ok.injEq] using written.symm.trans putCanonical
  subst effects
  subst next
  exact ⟨rfl,rfl,by rw [entries,table_lookup_put]; simp⟩

end

theorem store_entries_ext (left right : store.Store K V)
    (same : left.entries.val = right.entries.val) : left = right := by
  cases left
  cases right
  congr 1
  exact alloc.vec.Vec.ext _ _ same

theorem put_with_exact_entries [DecidableEq K] (eq : core.cmp.PartialEq K K)
    (query : K) (lawfulAt : ∀ stored, eq.eq stored query = .ok (decide (stored = query)))
    (before after : store.Store K V) (value : V)
    (capacity : TableWriteCapacity before.entries.val query)
    (post : tablePut query value before.entries.val = after.entries.val) :
    store.Store.put eq before query value = .ok after := by
  obtain ⟨next,ran,entries⟩ := put_exact_at eq query lawfulAt before value capacity
  have same := store_entries_ext next after (entries.trans post)
  simpa only [same] using ran

end SandboxModel.Lifecycle.PauseDispatch
