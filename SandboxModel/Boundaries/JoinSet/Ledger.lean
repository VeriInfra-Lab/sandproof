import Std
namespace SandboxModel.Boundaries.JoinSetContract

/-! Finite event projection for one original JoinSet. Counters are mathematical
history counts, not runtime integers. Forgotten means removed by a normally
completed group Drop, not resource destruction or external work exit.
Completion is an A6 observation, not a fairness assumption.
A cancelled borrowed join removes no member; other task events can interleave.
Abort-all is a count-preserving request. Its per-task snapshot is separate.
Accepting is caller policy, not a library close API; its source binding is an
explicit caller obligation. Both projections come from the same execution. -/
structure Ledger where
  group : Nat
  owned : Bool
  accepting : Bool
  running : Nat
  ready : Nat
  taken : Nat
  forgotten : Nat
  spawned : Nat
  deriving DecidableEq, Repr

def tracked (s : Ledger) : Nat := if s.owned then s.running + s.ready else 0

def LedgerSafe (s : Ledger) : Prop :=
  s.running + s.ready + s.taken + s.forgotten = s.spawned ∧
  (s.owned = true → s.forgotten = 0) ∧
  (s.owned = false → s.ready = 0) ∧
  (s.accepting = true → s.owned = true)

def initial (group : Nat) : Ledger := ⟨group, true, true, 0, 0, 0, 0, 0⟩
def Drained (s : Ledger) : Prop := s.running = 0 ∧ s.ready = 0 ∧ s.taken = s.spawned

inductive LedgerStep : Ledger → Ledger → Prop
  | spawn (s) (owned : s.owned = true) (admitting : s.accepting = true) :
      LedgerStep s { s with running := s.running + 1, spawned := s.spawned + 1 }
  | completeOwned (s) (owned : s.owned = true) (live : 0 < s.running) :
      LedgerStep s { s with running := s.running - 1, ready := s.ready + 1 }
  | completeUnowned (s) (unowned : s.owned = false) (live : 0 < s.running) :
      LedgerStep s { s with running := s.running - 1, forgotten := s.forgotten + 1 }
  | take (s) (owned : s.owned = true) (available : 0 < s.ready) :
      LedgerStep s { s with ready := s.ready - 1, taken := s.taken + 1 }
  | observeNone (s) (owned : s.owned = true) (noLive : s.running = 0) (noReady : s.ready = 0) :
      LedgerStep s s
  | abortAll (s) : LedgerStep s s
  | cancelJoin (s) : LedgerStep s s
  | closeAdmission (s) : LedgerStep s { s with accepting := false }
  | dropOwner (s) (owned : s.owned = true) : LedgerStep s
      { s with owned := false, accepting := false, ready := 0, forgotten := s.forgotten + s.ready }

inductive LedgerHistory : Ledger → Ledger → Prop
  | refl (s) : LedgerHistory s s
  | next : LedgerHistory s t → LedgerStep t u → LedgerHistory s u

theorem initial_safe (group) : LedgerSafe (initial group) := by simp [LedgerSafe, initial]

theorem ledger_step_safe {s t} (step : LedgerStep s t) (safe : LedgerSafe s) : LedgerSafe t := by
  cases step <;> simp_all [LedgerSafe] <;> omega

theorem ledger_history_safe {s t} (history : LedgerHistory s t) (safe : LedgerSafe s) :
    LedgerSafe t := by
  induction history with
  | refl => exact safe
  | next _ step ih => exact ledger_step_safe step ih

theorem ledger_step_keeps_original_group {s t} (step : LedgerStep s t) : t.group = s.group := by
  cases step <;> rfl

theorem ledger_history_keeps_original_group {s t} (history : LedgerHistory s t) :
    t.group = s.group := by
  induction history with
  | refl => rfl
  | next _ step ih => exact (ledger_step_keeps_original_group step).trans ih

theorem completed_uncollected_task_still_counts (s) (owned : s.owned = true) :
    s.ready ≤ tracked s := by simp [tracked, owned]

theorem abort_all_does_not_remove_members (s) : LedgerStep s s ∧ tracked s = tracked s :=
  ⟨.abortAll s, rfl⟩

theorem cancelling_borrowed_join_removes_no_members (s) : LedgerStep s s :=
  .cancelJoin s

theorem owned_empty_is_drained {s} (safe : LedgerSafe s) (owned : s.owned = true)
    (empty : tracked s = 0) : Drained s := by
  simp only [tracked, owned, ↓reduceIte] at empty
  have accounted := safe.1
  have noForgotten := safe.2.1 owned
  unfold Drained
  omega

theorem history_owned_empty_collected_all {group s} (history : LedgerHistory (initial group) s)
    (owned : s.owned = true) (empty : tracked s = 0) : Drained s :=
  owned_empty_is_drained (ledger_history_safe history (initial_safe group)) owned empty

theorem none_observation_is_drained {s} (safe : LedgerSafe s) (owned : s.owned = true)
    (noLive : s.running = 0) (noReady : s.ready = 0) : Drained s :=
  owned_empty_is_drained safe owned (by simp [tracked, owned, noLive, noReady])

/-! Capacity is checked by the caller at spawn. The library is not bounded.
A semaphore over live callbacks need not bound ready uncollected entries. -/
theorem ledger_step_keeps_checked_capacity {s t} (step : LedgerStep s t)
    (bound : tracked s ≤ capacity)
    (admission : s.spawned < t.spawned → tracked s < capacity) : tracked t ≤ capacity := by
  cases step <;> simp_all [tracked] <;> omega

inductive BoundedHistory (capacity : Nat) : Ledger → Ledger → Prop
  | refl (s) : BoundedHistory capacity s s
  | next : BoundedHistory capacity s t → LedgerStep t u →
      (t.spawned < u.spawned → tracked t < capacity) → BoundedHistory capacity s u

theorem bounded_history_keeps_capacity {s t} (history : BoundedHistory capacity s t)
    (bound : tracked s ≤ capacity) : tracked t ≤ capacity := by
  induction history with
  | refl => exact bound
  | next _ step admission ih => exact ledger_step_keeps_checked_capacity step ih admission

theorem correct_drain_has_nonempty_history (group) :
    LedgerHistory (initial group) ⟨group, true, false, 0, 0, 1, 0, 1⟩ ∧
      Drained ⟨group, true, false, 0, 0, 1, 0, 1⟩ := by
  have spawned : LedgerHistory (initial group) ⟨group, true, true, 1, 0, 0, 0, 1⟩ :=
    .next (.refl _) (.spawn _ rfl rfl)
  have closed := LedgerHistory.next spawned (LedgerStep.closeAdmission _)
  have finished := LedgerHistory.next closed (LedgerStep.completeOwned _ rfl (by simp))
  exact ⟨.next finished (.take _ rfl (by simp)), by simp [Drained]⟩

theorem dropping_group_can_look_empty_without_drain (group) :
    LedgerHistory (initial group) ⟨group, false, false, 1, 0, 0, 0, 1⟩ ∧
      tracked ⟨group, false, false, 1, 0, 0, 0, 1⟩ = 0 ∧
      ¬ Drained ⟨group, false, false, 1, 0, 0, 0, 1⟩ := by
  have spawned : LedgerHistory (initial group) ⟨group, true, true, 1, 0, 0, 0, 1⟩ :=
    .next (.refl _) (.spawn _ rfl rfl)
  exact ⟨.next spawned (.dropOwner _ rfl), rfl, by simp [Drained]⟩

theorem aborting_group_is_not_draining (group) :
    LedgerStep ⟨group, true, false, 1, 0, 0, 0, 1⟩ ⟨group, true, false, 1, 0, 0, 0, 1⟩ ∧
      ¬ Drained ⟨group, true, false, 1, 0, 0, 0, 1⟩ :=
  ⟨.abortAll _, by simp [Drained]⟩

theorem cancelling_join_does_not_freeze_task_completion (group) :
    LedgerHistory ⟨group, true, false, 1, 0, 0, 0, 1⟩ ⟨group, true, false, 0, 1, 0, 0, 1⟩ := by
  exact .next (.next (.refl _) (.completeOwned _ rfl (by simp))) (.cancelJoin _)

end SandboxModel.Boundaries.JoinSetContract
