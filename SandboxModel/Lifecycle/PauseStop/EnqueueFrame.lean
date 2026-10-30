import SandboxModel.Lifecycle.CaptureHandoff.CallerLate

/-! Success-only frames for the actual effect registration and enqueue code.
The insert/store loops are neither replaced nor proved again. These results
do not claim spare capacity, current-attempt eligibility or eventual dispatch. -/
namespace SandboxModel.Lifecycle.PauseStop
open Aeneas Aeneas.Std CaptureCallerRust
open CaptureCallerRust.sandbox_controller_core
open SandboxModel.Lifecycle.CaptureHandoffCaller

def queueInitialId (command : model.Command) : model.EffectId :=
  { logical := { command }, attempt := 0#u64 }

/-- The real early return applies to commit_effect, not the whole enqueue. -/
theorem commit_effect_initial_present_exact (world : model.World) (command : model.Command)
    (present : store.Store.contains_key model.EffectId.Insts.CoreCmpPartialEqEffectId
      world.effects (queueInitialId command) = .ok true) :
    transition.commit_effect world command = .ok world := by
  simp only [transition.commit_effect, model_Command_clone_exact, bind_tc_ok,
    model.EffectId.initial]
  simpa only [queueInitialId, present, bind_tc_ok] using
    (show (.ok world : Result model.World) = .ok world from rfl)

/-- A successful missing-initial-effect branch writes the actual effect and
key results. The logical table may be preserved or updated by its own branch. -/
theorem commit_effect_missing_initial_shape (world next : model.World)
    (command : model.Command)
    (absent : store.Store.contains_key model.EffectId.Insts.CoreCmpPartialEqEffectId
      world.effects (queueInitialId command) = .ok false)
    (evaluated : transition.commit_effect world command = .ok next) :
    ∃ effects keys logical,
      store.Store.put model.EffectId.Insts.CoreCmpPartialEqEffectId world.effects
        (queueInitialId command) model.EffectStatus.Committed = .ok effects ∧
      transition.insert_unique model.EffectId.Insts.CoreCloneClone
        model.EffectId.Insts.CoreCmpPartialEqEffectId world.effect_keys
        (queueInitialId command) = .ok keys ∧
      next = { world with logical_effects := logical, effect_keys := keys, effects := effects } := by
  simp only [transition.commit_effect, model_Command_clone_exact, bind_tc_ok,
    model.EffectId.initial] at evaluated
  change ((do
    let found ← store.Store.contains_key model.EffectId.Insts.CoreCmpPartialEqEffectId
      world.effects (queueInitialId command)
    if found then .ok world else _) : Result model.World) = .ok next at evaluated
  rw [absent] at evaluated
  simp only [bind_tc_ok, Bool.false_eq_true, if_false,
    model_EffectId_clone_exact] at evaluated
  obtain ⟨effects, effectsEq, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨keys, keysEq, tail⟩ := bind_ok_inv _ _ _ tail
  obtain ⟨foundLogical, _readLogical, tail⟩ := bind_ok_inv _ _ _ tail
  split at tail
  · exact ⟨effects, keys, world.logical_effects, effectsEq, keysEq,
      (by simpa using tail.symm)⟩
  · obtain ⟨origin, _originEq, tail⟩ := bind_ok_inv _ _ _ tail
    obtain ⟨logical, _written, tail⟩ := bind_ok_inv _ _ _ tail
    exact ⟨effects, keys, logical, effectsEq, keysEq, (by simpa using tail.symm)⟩

/-- Only the three effect-registration fields can change on success. -/
theorem commit_effect_shape (world next : model.World) (command : model.Command)
    (evaluated : transition.commit_effect world command = .ok next) :
    ∃ effects keys logical,
      next = { world with logical_effects := logical, effect_keys := keys, effects := effects } := by
  have source := evaluated
  simp only [transition.commit_effect, model_Command_clone_exact, bind_tc_ok,
    model.EffectId.initial] at source
  obtain ⟨found, observed, _tail⟩ := bind_ok_inv _ _ _ source
  cases found with
  | false =>
    have absent : store.Store.contains_key model.EffectId.Insts.CoreCmpPartialEqEffectId
        world.effects (queueInitialId command) = .ok false := by
      simpa only [queueInitialId] using observed
    obtain ⟨effects, keys, logical, _effectsEq, _keysEq, shape⟩ :=
      commit_effect_missing_initial_shape world next command absent evaluated
    exact ⟨effects, keys, logical, shape⟩
  | true =>
    have present : store.Store.contains_key model.EffectId.Insts.CoreCmpPartialEqEffectId
        world.effects (queueInitialId command) = .ok true := by
      simpa only [queueInitialId] using observed
    have same : world = next := by
      simpa only [commit_effect_initial_present_exact world command present,
        Result.ok.injEq] using evaluated
    subst next
    exact ⟨world.effects, world.effect_keys, world.logical_effects, rfl⟩

/-- An already present logical row is not rewritten or reset by registration.
No equality between its current_attempt and the initial attempt is assumed. -/
theorem commit_effect_existing_logical_table (world next : model.World)
    (command : model.Command)
    (present : store.Store.contains_key
      model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      world.logical_effects { command } = .ok true)
    (evaluated : transition.commit_effect world command = .ok next) :
    next.logical_effects = world.logical_effects := by
  simp only [transition.commit_effect, model_Command_clone_exact, bind_tc_ok,
    model.EffectId.initial] at evaluated
  obtain ⟨found, _observed, tail⟩ := bind_ok_inv _ _ _ evaluated
  split at tail
  · have same : world = next := by simpa using tail
    subst next
    rfl
  · simp only [model_EffectId_clone_exact, bind_tc_ok] at tail
    obtain ⟨effects, _effectsEq, tail⟩ := bind_ok_inv _ _ _ tail
    obtain ⟨keys, _keysEq, tail⟩ := bind_ok_inv _ _ _ tail
    rw [present] at tail
    have same : { world with effect_keys := keys, effects := effects } = next := by
      simpa only [bind_tc_ok] using tail
    subst next
    rfl

/-- Enqueue also changes inflight; the three commit fields retain their exact
successful values and every field outside these four is framed out. -/
theorem enqueue_shape (world next : model.World) (command : model.Command)
    (evaluated : transition.enqueue world command = .ok next) :
    ∃ effects keys logical inflight,
      next = { world with logical_effects := logical, effect_keys := keys,
        effects := effects, inflight := inflight } := by
  simp only [transition.enqueue, model_Command_clone_exact, bind_tc_ok,
    model.EffectId.initial] at evaluated
  obtain ⟨registered, committed, tail⟩ := bind_ok_inv _ _ _ evaluated
  obtain ⟨effects, keys, logical, shape⟩ :=
    commit_effect_shape world registered command committed
  obtain ⟨inflight, _inserted, returned⟩ := bind_ok_inv _ _ _ tail
  subst registered
  exact ⟨effects, keys, logical, inflight, (by simpa using returned.symm)⟩

theorem enqueue_initial_present_shape (world next : model.World) (command : model.Command)
    (present : store.Store.contains_key model.EffectId.Insts.CoreCmpPartialEqEffectId
      world.effects (queueInitialId command) = .ok true)
    (evaluated : transition.enqueue world command = .ok next) :
    ∃ inflight, next = { world with inflight := inflight } := by
  simp only [transition.enqueue, model_Command_clone_exact, bind_tc_ok,
    model.EffectId.initial, commit_effect_initial_present_exact world command present] at evaluated
  obtain ⟨inflight, _inserted, returned⟩ := bind_ok_inv _ _ _ evaluated
  exact ⟨inflight, (by simpa using returned.symm)⟩

theorem enqueue_existing_logical_table (world next : model.World)
    (command : model.Command)
    (present : store.Store.contains_key
      model.LogicalEffectId.Insts.CoreCmpPartialEqLogicalEffectId
      world.logical_effects { command } = .ok true)
    (evaluated : transition.enqueue world command = .ok next) :
    next.logical_effects = world.logical_effects := by
  simp only [transition.enqueue, model_Command_clone_exact, bind_tc_ok,
    model.EffectId.initial] at evaluated
  obtain ⟨registered, committed, tail⟩ := bind_ok_inv _ _ _ evaluated
  have kept := commit_effect_existing_logical_table world registered command present committed
  obtain ⟨inflight, _inserted, returned⟩ := bind_ok_inv _ _ _ tail
  have shape : { registered with inflight := inflight } = next := by simpa using returned
  subst next
  exact kept

/-- Full-record equality keeps sandboxes, VM rows, operation ledgers/plans,
fences, snapshots, resource indexes, counters and all remaining fields. -/
theorem enqueue_frame (world next : model.World) (command : model.Command)
    (evaluated : transition.enqueue world command = .ok next) :
    next = { world with logical_effects := next.logical_effects,
      effect_keys := next.effect_keys, effects := next.effects, inflight := next.inflight } := by
  obtain ⟨effects, keys, logical, inflight, shape⟩ := enqueue_shape world next command evaluated
  subst next
  rfl

end SandboxModel.Lifecycle.PauseStop
