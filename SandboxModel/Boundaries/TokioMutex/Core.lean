import SandboxModel.Boundaries.TokioRwLock.Core

namespace SandboxModel.Boundaries.TokioMutexContract

/- A6/B2 projection of a *borrowed*, original Tokio Mutex and live guard.
   Reuse the delivered exclusive-writer representation, with no read leases.
   It records returned access, not semaphore queues or poll/Pin internals.
   The original lock must survive each observation. Payload is inline state
   or handles; interior-mutable aliases, IO and resource Drop remain separate. -/
/- Equal tickets denote the same logical acquisition; this does not copy
   Rust guard ownership. Raw value constructors are not evidence of a
   real returned guard: callers bind Safe and Step/History via A6/B2. -/
abbrev Guard := TokioRwLockContract.Guard
abbrev State (α : Type) := TokioRwLockContract.State α
abbrev initial := @TokioRwLockContract.initial
abbrev acquired := @TokioRwLockContract.writeAcquired
abbrev released := @TokioRwLockContract.writeReleased
abbrev forgotten := @TokioRwLockContract.writeForgotten
abbrev owns := @TokioRwLockContract.OwnsWrite

def EmptyReaders (s : State α) : Prop :=
  (∀ i, s.readers i = false) ∧ (∀ i, s.liveReaders i = false)

def Safe (s : State α) : Prop := TokioRwLockContract.Safe s ∧ EmptyReaders s

/- `Busy` may include a reservation for an unpolled waiter. Free projected
   ownership is necessary for a returned guard, never sufficient for success.
   Cancellation below concerns a waiting *acquire*, not an entire business
   future that may already hold a guard or have performed IO. -/
inductive Step (writes : α → α → Prop) : State α → State α → Prop
  | returned (s) (free : s.writer = none) : Step writes s (acquired s)
  | guardDrop (s g) (held : owns s g) : Step writes s (released s)
  | guardForget (s g) (held : owns s g) : Step writes s (forgotten s)
  | writeValue (s g value) (held : owns s g) (body : writes s.payload value) :
      Step writes s { s with payload := value }
  | tryBusy (s) : Step writes s s
  | acquirePendingOrCancelled (s) : Step writes s s

theorem initial_safe (lock : Nat) (value : α) : Safe (initial lock value) := by
  exact ⟨TokioRwLockContract.initial_safe lock value, by simp [EmptyReaders, initial, TokioRwLockContract.initial]⟩

theorem step_refines_existing_lock {s t : State α} (step : Step writes s t)
    (safe : Safe s) : TokioRwLockContract.Step writes s t := by
  cases step with
  | returned free => exact .writeReturned _ free safe.2.1
  | guardDrop _ held => exact .writerDrop _ _ held
  | guardForget _ held => exact .writerForget _ _ held
  | writeValue _ _ held body => exact .writeValue _ _ _ held body
  | tryBusy => exact .pendingOrCancelled _
  | acquirePendingOrCancelled => exact .pendingOrCancelled _

theorem step_keeps_empty_readers {s t : State α} (step : Step writes s t)
    (empty : EmptyReaders s) : EmptyReaders t := by
  cases step <;> exact empty

theorem step_safe {s t : State α} (step : Step writes s t) (safe : Safe s) : Safe t :=
  ⟨TokioRwLockContract.step_safe (step_refines_existing_lock step safe) safe.1,
    step_keeps_empty_readers step safe.2⟩

inductive History (writes : α → α → Prop) : State α → State α → Prop
  | refl (s) : History writes s s
  | next : History writes s t → Step writes t u → History writes s u

theorem history_safe {s t : State α} (history : History writes s t) (safe : Safe s) : Safe t := by
  induction history with
  | refl => exact safe
  | next _ step ih => exact step_safe step ih

theorem history_refines_existing_lock {s t : State α}
    (history : History writes s t) (safe : Safe s) : TokioRwLockContract.History writes s t := by
  induction history with
  | refl => exact .refl _
  | next prior step ih =>
      exact .next ih (step_refines_existing_lock step (history_safe prior safe))

theorem returned_guard_is_original (s : State α) : owns (acquired s) (TokioRwLockContract.fresh s) :=
  TokioRwLockContract.write_return_owns_original s

theorem live_access_is_exclusive {s : State α} (left : owns s g) (right : owns s h) : g = h :=
  TokioRwLockContract.two_live_writers_are_same left right

theorem held_guard_blocks_return {s : State α} (held : owns s g) : s.writer ≠ none := by
  rw [held.1]
  simp

theorem original_lock_identity {s t : State α} (step : Step writes s t)
    (safe : Safe s) : t.lock = s.lock :=
  TokioRwLockContract.step_keeps_original_lock (step_refines_existing_lock step safe)

theorem release_invalidates_access (s : State α) (g : Guard) : ¬ owns (released s) g := by
  simp [owns, TokioRwLockContract.OwnsWrite, released, TokioRwLockContract.writeReleased]

theorem forget_invalidates_access (s : State α) (g : Guard) : ¬ owns (forgotten s) g := by
  simp [owns, TokioRwLockContract.OwnsWrite, forgotten, TokioRwLockContract.writeForgotten]

theorem forget_keeps_exclusion {s : State α} (held : owns s g) : (forgotten s).writer ≠ none := by
  change s.writer ≠ none
  exact held_guard_blocks_return held

theorem normal_release_keeps_inline_payload (s : State α) :
    (released s).payload = s.payload := rfl

theorem invariant_from_caller_writes (invariant : α → Prop) {s t : State α}
    (history : History writes s t) (safe : Safe s)
    (body : ∀ old next, writes old next → invariant old → invariant next)
    (prior : invariant s.payload) : invariant t.payload :=
  TokioRwLockContract.history_preserves_payload_invariant invariant
    (history_refines_existing_lock history safe) body prior

/- A business callback must retain this original access at the point it uses
   the payload. This asserts no authorization or absence of its *own* writes. -/
def ProtectedUse (s : State α) (g : Guard) : Prop := owns s g

theorem acquired_use_is_protected (s : State α) : ProtectedUse (acquired s) (TokioRwLockContract.fresh s) :=
  returned_guard_is_original s

theorem early_release_rejected_by_same_protected_use (s : State α) (g : Guard) :
    ¬ ProtectedUse (released s) g := release_invalidates_access s g

end SandboxModel.Boundaries.TokioMutexContract
