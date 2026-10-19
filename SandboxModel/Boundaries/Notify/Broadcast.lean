import Std

namespace SandboxModel.Boundaries.NotifyContract

/-! A projection for one arbitrary waiter of a monotone latch. The caller
provides the predicate and its lock/identity binding. A6/B2/B3 supply prepare,
read, publication and broadcast events. Per-waiter proofs apply to every
registered observer; they do not assert a physical process/FD exit.

This coarse projection permits either pre-registration observation. No result
here relies on broadcast memory for an unprepared future. Callers use explicit
registration before reading the sticky predicate. Scheduling fairness and
eventual cleanup require separate conditions. -/
structure ObserverKey where
  signal : Nat
  waiter : Nat
  deriving DecidableEq, Repr

inductive WaitPhase | created | prepared | awaiting | notified | finished | cancelled
  deriving DecidableEq, Repr

structure LatchState where
  key : ObserverKey
  closed : Bool
  allObserved : Bool
  closeReturned : Bool
  phase : WaitPhase
  observedOpen : Bool
  remembered : Bool
  deriving DecidableEq, Repr

def LatchSafe (s : LatchState) : Prop :=
  (s.allObserved = true → s.closed = true ∧ s.phase ≠ .awaiting) ∧
  (s.closeReturned = true → s.allObserved = true) ∧
  (s.observedOpen = true → s.phase = .awaiting ∨ s.phase = .notified ∨
      s.phase = .finished ∨ s.phase = .cancelled) ∧
  (s.remembered = true → s.allObserved = true) ∧
  (s.phase = .notified → s.closed = true)

def latchInitial (k : ObserverKey) : LatchState :=
  ⟨k, false, false, false, .created, false, false⟩
def lateInitial (k : ObserverKey) : LatchState :=
  ⟨k, true, true, true, .created, false, false⟩
def allUpdate (s : LatchState) : LatchState :=
  ⟨s.key, s.closed, true, s.closeReturned,
    (match s.phase with
      | .prepared => .notified
      | .awaiting => .notified
      | p => p),
    s.observedOpen, if s.phase = .created then true else s.remembered⟩

def NoLostWait (s : LatchState) : Prop := s.closeReturned = true → s.phase ≠ .awaiting

inductive LatchStep : LatchState → LatchState → Prop
  | enable (k c a r m) : LatchStep
      ⟨k, c, a, r, .created, false, m⟩ ⟨k, c, a, r, .prepared, false, false⟩
  | enableRemembered (k c a r) : LatchStep
      ⟨k, c, a, r, .created, false, true⟩ ⟨k, c, a, r, .notified, false, false⟩
  | readOpen (k a r m) : LatchStep
      ⟨k, false, a, r, .prepared, false, m⟩ ⟨k, false, a, r, .awaiting, true, m⟩
  | rejectClosed (k a r p o m) (eligible : p = .prepared ∨ p = .notified) : LatchStep
      ⟨k, true, a, r, p, o, m⟩ ⟨k, true, a, r, .finished, o, m⟩
  | publishClosed (s) : LatchStep s { s with closed := true }
  | notifyAll (s) (closed : s.closed = true) : LatchStep s (allUpdate s)
  | returnClose (s) (broadcastDone : s.allObserved = true) :
      LatchStep s { s with closeReturned := true }
  | finishIO (k c a r p o m) (eligible : p = .awaiting ∨ p = .notified) : LatchStep
      ⟨k, c, a, r, p, o, m⟩ ⟨k, c, a, r, .finished, o, m⟩
  | cancel (s) : LatchStep s { s with phase := .cancelled }
  | unchanged (s) : LatchStep s s

inductive LatchHistory : LatchState → LatchState → Prop
  | refl (s) : LatchHistory s s
  | next : LatchHistory s t → LatchStep t u → LatchHistory s u

theorem latch_initial_safe (k) : LatchSafe (latchInitial k) := by simp [LatchSafe, latchInitial]
theorem late_initial_safe (k) : LatchSafe (lateInitial k) := by simp [LatchSafe, lateInitial]

theorem all_update_safe (s) (safe : LatchSafe s) (closed : s.closed = true) :
    LatchSafe (allUpdate s) := by
  rcases s with ⟨k, c, a, r, p, o, m⟩
  cases p <;> simp_all [LatchSafe, allUpdate]

theorem latch_step_safe {s t} (step : LatchStep s t) (safe : LatchSafe s) : LatchSafe t := by
  cases step with
  | notifyAll s closed => exact all_update_safe s safe closed
  | readOpen k a r m =>
      cases a <;> cases r <;> cases m <;> simp_all [LatchSafe]
  | returnClose s done =>
      exact ⟨safe.1, (fun _ => done), safe.2.2.1, safe.2.2.2.1, safe.2.2.2.2⟩
  | _ => simp_all [LatchSafe]

theorem latch_history_safe {s t} (history : LatchHistory s t) (safe : LatchSafe s) : LatchSafe t := by
  induction history with
  | refl => exact safe
  | next _ step ih => exact latch_step_safe step ih

theorem latch_step_keeps_original_observer {s t} (step : LatchStep s t) : t.key = s.key := by
  cases step <;> rfl

theorem latch_history_keeps_original_observer {s t} (history : LatchHistory s t) : t.key = s.key := by
  induction history with
  | refl => rfl
  | next _ step ih => exact (latch_step_keeps_original_observer step).trans ih

theorem completed_close_does_not_leave_committed_waiter {s} (safe : LatchSafe s) : NoLostWait s := by
  intro done
  exact (safe.1 (safe.2.1 done)).2

theorem latch_history_has_no_lost_wait {s t} (history : LatchHistory s t) (safe : LatchSafe s) :
    NoLostWait t := completed_close_does_not_leave_committed_waiter (latch_history_safe history safe)

theorem committed_active_waiter_is_notified {s} (safe : LatchSafe s)
    (done : s.closeReturned = true) (checked : s.observedOpen = true)
    (active : s.phase ≠ .finished ∧ s.phase ≠ .cancelled) : s.phase = .notified := by
  have phases := safe.2.2.1 checked
  have notWaiting := completed_close_does_not_leave_committed_waiter safe done
  rcases phases with h | h | h | h <;> simp_all

theorem late_observer_can_reject_without_waiting (k) :
    LatchHistory (lateInitial k) ⟨k, true, true, true, .finished, false, false⟩ := by
  have enabled : LatchHistory (lateInitial k) ⟨k, true, true, true, .prepared, false, false⟩ :=
    .next (.refl _) (.enable k true true true false)
  exact .next enabled (.rejectClosed k true true .prepared false false (Or.inl rfl))

theorem correct_close_can_notify_original_waiter (k) :
    LatchHistory (latchInitial k) ⟨k, true, true, true, .notified, true, false⟩ := by
  have enabled : LatchHistory (latchInitial k) ⟨k, false, false, false, .prepared, false, false⟩ :=
    .next (.refl _) (.enable k false false false false)
  have checked := LatchHistory.next enabled (LatchStep.readOpen k false false false)
  have published := LatchHistory.next checked (LatchStep.publishClosed _)
  have notified := LatchHistory.next published (LatchStep.notifyAll _ rfl)
  exact .next notified (.returnClose _ rfl)

theorem notify_one_other_waiter_breaks_same_property (k) :
    ¬ NoLostWait ⟨k, true, false, true, .awaiting, true, false⟩ := by
  simp [NoLostWait]

end SandboxModel.Boundaries.NotifyContract
