import Std

namespace SandboxModel.Boundaries.NotifyContract

/-! Single-consumer logical notification projection. `notices` and `consumed`
are mathematical history counters, not finite implementation counters. A6
supplies the original Notify events; B3 supplies signal/future identity.
No scheduling, business-state, resource-release or physical-exit claim. -/
inductive OnePhase | fresh | waiting | assigned | ready | cancelled
  deriving DecidableEq, Repr

structure OneState where
  signal : Nat
  stored : Bool
  phase : OnePhase
  notices : Nat
  consumed : Nat
  deriving DecidableEq, Repr

def storedCredit (s : OneState) : Nat := if s.stored then 1 else 0
def assignedCredit (s : OneState) : Nat := if s.phase = .assigned then 1 else 0
def OneSafe (s : OneState) : Prop :=
  s.consumed + storedCredit s + assignedCredit s ≤ s.notices ∧
  (s.phase = .ready → 0 < s.consumed) ∧
  (s.phase = .waiting → s.stored = false)

def oneInitial (k : Nat) : OneState := ⟨k, false, .fresh, 0, 0⟩

inductive OneStep : OneState → OneState → Prop
  | notifyIdle (k c p n d) (idle : p ≠ .waiting) :
      OneStep ⟨k, c, p, n, d⟩ ⟨k, true, p, n + 1, d⟩
  | notifyWaiting (k n d) : OneStep ⟨k, false, .waiting, n, d⟩ ⟨k, false, .assigned, n + 1, d⟩
  | prepareStored (k n d) : OneStep ⟨k, true, .fresh, n, d⟩ ⟨k, false, .ready, n, d + 1⟩
  | prepareEmpty (k n d) : OneStep ⟨k, false, .fresh, n, d⟩ ⟨k, false, .waiting, n, d⟩
  | consumeAssigned (k c n d) : OneStep ⟨k, c, .assigned, n, d⟩ ⟨k, c, .ready, n, d + 1⟩
  | cancelFresh (k c n d) : OneStep ⟨k, c, .fresh, n, d⟩ ⟨k, c, .cancelled, n, d⟩
  | cancelWaiting (k n d) : OneStep ⟨k, false, .waiting, n, d⟩ ⟨k, false, .cancelled, n, d⟩
  | cancelAssigned (k c n d) : OneStep ⟨k, c, .assigned, n, d⟩ ⟨k, true, .cancelled, n, d⟩
  | dropReady (k c n d) : OneStep ⟨k, c, .ready, n, d⟩ ⟨k, c, .cancelled, n, d⟩
  | nextWait (k c n d) : OneStep ⟨k, c, .cancelled, n, d⟩ ⟨k, c, .fresh, n, d⟩
  | unchanged (s) : OneStep s s

inductive OneHistory : OneState → OneState → Prop
  | refl (s) : OneHistory s s
  | next : OneHistory s t → OneStep t u → OneHistory s u

theorem one_initial_safe (k) : OneSafe (oneInitial k) := by
  simp [OneSafe, oneInitial, storedCredit, assignedCredit]

theorem one_step_safe {s t} (step : OneStep s t) (safe : OneSafe s) : OneSafe t := by
  cases step <;> simp_all [OneSafe, storedCredit, assignedCredit]
  all_goals repeat' (split at *)
  all_goals (try simp_all)
  all_goals omega

theorem one_history_safe {s t} (history : OneHistory s t) (safe : OneSafe s) : OneSafe t := by
  induction history with
  | refl => exact safe
  | next _ step ih => exact one_step_safe step ih

theorem one_step_keeps_original_signal {s t} (step : OneStep s t) : t.signal = s.signal := by
  cases step <;> rfl

theorem one_history_keeps_original_signal {s t} (history : OneHistory s t) : t.signal = s.signal := by
  induction history with
  | refl => rfl
  | next _ step ih => exact (one_step_keeps_original_signal step).trans ih

theorem stored_credit_is_not_a_counter (s : OneState) : storedCredit s ≤ 1 := by
  unfold storedCredit
  split <;> decide

theorem one_consumptions_bounded_by_real_notices {s} (safe : OneSafe s) : s.consumed ≤ s.notices := by
  have bound := safe.1
  omega

theorem ready_requires_a_real_notice {s} (safe : OneSafe s) (ready : s.phase = .ready) :
    0 < s.notices := by
  have positive := safe.2.1 ready
  have bound := one_consumptions_bounded_by_real_notices safe
  omega

theorem waiting_has_no_stored_credit {s} (safe : OneSafe s) (waiting : s.phase = .waiting) :
    storedCredit s = 0 := by simp [storedCredit, safe.2.2 waiting]

theorem two_early_notices_can_complete_once (k) :
    OneHistory (oneInitial k) ⟨k, false, .ready, 2, 1⟩ := by
  have first : OneHistory (oneInitial k) ⟨k, true, .fresh, 1, 0⟩ :=
    .next (.refl _) (.notifyIdle k false .fresh 0 0 (by decide))
  have second := OneHistory.next first (OneStep.notifyIdle k true .fresh 1 0 (by decide))
  exact OneHistory.next second (OneStep.prepareStored k 2 0)

theorem early_notices_coalesce (k n d) :
    OneStep ⟨k, true, .fresh, n, d⟩ ⟨k, true, .fresh, n + 1, d⟩ ∧
      storedCredit ⟨k, true, .fresh, n + 1, d⟩ = 1 :=
  ⟨.notifyIdle k true .fresh n d (by decide), rfl⟩

theorem assigned_and_stored_are_distinct_credits (k) :
    OneHistory (oneInitial k) ⟨k, true, .assigned, 2, 0⟩ := by
  have waiting : OneHistory (oneInitial k) ⟨k, false, .waiting, 0, 0⟩ :=
    .next (.refl _) (.prepareEmpty k 0 0)
  have assigned := OneHistory.next waiting (OneStep.notifyWaiting k 0 0)
  exact .next assigned (.notifyIdle k false .assigned 1 0 (by decide))

theorem cancelling_assigned_one_returns_credit (k c n d) :
    OneStep ⟨k, c, .assigned, n, d⟩ ⟨k, true, .cancelled, n, d⟩ := .cancelAssigned ..

theorem dropping_consumed_one_does_not_return_credit (k n d) :
    OneStep ⟨k, false, .ready, n, d⟩ ⟨k, false, .cancelled, n, d⟩ := .dropReady ..

theorem next_wait_does_not_consume_credit (k c n d) :
    OneStep ⟨k, c, .cancelled, n, d⟩ ⟨k, c, .fresh, n, d⟩ := .nextWait ..

theorem history_consumptions_bounded_by_real_notices {k t}
    (history : OneHistory (oneInitial k) t) : t.consumed ≤ t.notices :=
  one_consumptions_bounded_by_real_notices (one_history_safe history (one_initial_safe k))

theorem recreating_consumed_credit_breaks_same_property (k) :
    ¬ OneSafe ⟨k, true, .cancelled, 1, 1⟩ := by
  simp [OneSafe, storedCredit, assignedCredit]

end SandboxModel.Boundaries.NotifyContract
