import SandboxModel.Boundaries.Condvar.Core

namespace SandboxModel.Boundaries.CondvarContract.Waiting

/- One live wait invocation on one original Condvar and mutex. Other threads
   may acquire/write/release the mutex while this invocation is suspended.
   Notification makes this single consumer eligible; it does not acquire the
   lock, publish a business predicate or guarantee eventual return.
   originalGuard names the consumed guard value occurrence, not its address. -/
inductive Phase
  | holding (lease : StdMutexContract.Guard)
  | waiting (waiter : Waiter) (timed : Bool)
  | ready (waiter : Waiter) (timed timedOut : Bool)
  | returned (reply : Reply)
  deriving DecidableEq, Repr

structure State (α : Type) where
  condvar : Nat
  originalGuard : Nat
  mutex : StdMutexContract.State α
  phase : Phase
  deriving DecidableEq, Repr

def initial (condvar originalGuard : Nat) (mutex : StdMutexContract.State α)
    (lease : StdMutexContract.Guard) : State α :=
  ⟨condvar, originalGuard, mutex, .holding lease⟩

def waiterFor (s : State α) (lease : StdMutexContract.Guard) : Waiter :=
  ⟨s.condvar, s.originalGuard, lease⟩

def beginWait (s : State α) (lease : StdMutexContract.Guard) (timed : Bool) : State α :=
  { s with mutex := suspend s.mutex, phase := .waiting (waiterFor s lease) timed }

def signal (s : State α) : State α :=
  match s.phase with
  | .waiting w timed => { s with phase := .ready w timed false }
  | _ => s

def finishWait (s : State α) (w : Waiter) (timedOut : Bool) : State α :=
  { s with mutex := (resume s.mutex w timedOut).1, phase := .returned (resume s.mutex w timedOut).2 }

def Suspended : Phase → Prop
  | .waiting _ _ | .ready _ _ _ => True
  | _ => False

def WaiterSafe (s : State α) (w : Waiter) : Prop :=
  w.condvar = s.condvar ∧ w.originalGuard = s.originalGuard ∧
  w.suspendedLease.mutex = s.mutex.mutex ∧
  w.suspendedLease.ticket < s.mutex.issued ∧
  ¬ StdMutexContract.Owns s.mutex w.suspendedLease

def Safe (s : State α) : Prop :=
  StdMutexContract.Safe s.mutex ∧
  match s.phase with
  | .holding g => StdMutexContract.Owns s.mutex g
  | .waiting w _ => WaiterSafe s w
  | .ready w timed timedOut => WaiterSafe s w ∧ (timedOut = true → timed = true)
  | .returned r => StdMutexContract.Owns s.mutex (replyValue r).lease ∧
      (replyValue r).originalGuard = s.originalGuard ∧
      (replyValue r).condvar = s.condvar

/- elapsed is the library's observed timeout event, supplied by the actual
   A6/A8 execution projection. It is not a business deadline proof or an
   assumption that time progresses. No step obliges a blocked waiter to run. -/
inductive Step (writes : α → α → Prop) : State α → State α → Prop
  | begin (s g timed) (holding : s.phase = .holding g)
      (owns : StdMutexContract.Owns s.mutex g) : Step writes s (beginWait s g timed)
  | notify (s) : Step writes s (signal s)
  | spurious (s w timed) (pending : s.phase = .waiting w timed) : Step writes s (signal s)
  | elapsed (s w) (pending : s.phase = .waiting w true) :
      Step writes s { s with phase := .ready w true true }
  | peer (s mutex) (pending : Suspended s.phase)
      (operation : StdMutexContract.Step writes (fun _ => False) s.mutex mutex) :
      Step writes s { s with mutex := mutex }
  | returned (s w timed timedOut) (ready : s.phase = .ready w timed timedOut)
      (allowed : MayReturn s.mutex w) : Step writes s (finishWait s w timedOut)

inductive History (writes : α → α → Prop) : State α → State α → Prop
  | refl (s) : History writes s s
  | next : History writes s t → Step writes t u → History writes s u

theorem mutex_step_issued_monotone {s t : StdMutexContract.State α}
    (operation : StdMutexContract.Step writes unique s t) : s.issued ≤ t.issued := by
  cases operation with
  | tryCall p =>
      cases h : s.holder <;>
        simp [StdMutexContract.tryAcquire, h, StdMutexContract.acquiredState]
  | lockReturned _ _ => simp [StdMutexContract.acquiredState]
  | readGuard _ _ => exact Nat.le_refl _
  | recoverError _ _ => exact Nat.le_refl _
  | writeGuard _ _ _ _ => exact Nat.le_refl _
  | guardDrop _ _ _ => exact Nat.le_refl _
  | forgetGuard _ _ => exact Nat.le_refl _
  | uniqueWrite _ _ _ _ => exact Nat.le_refl _

theorem mutex_step_does_not_revive_old_lease {s t : StdMutexContract.State α}
    (operation : StdMutexContract.Step writes unique s t)
    (issued : g.ticket < s.issued) (old : ¬ StdMutexContract.Owns s g) :
    ¬ StdMutexContract.Owns t g := by
  have fresh (p : Bool) : ¬ StdMutexContract.Owns (StdMutexContract.acquiredState s p) g := by
    intro owns
    have same : StdMutexContract.freshGuard s p = g := Option.some.inj owns.2
    have ticket : s.issued = g.ticket := congrArg StdMutexContract.Guard.ticket same
    omega
  cases operation with
  | tryCall p =>
      cases h : s.holder with
      | none => simpa [StdMutexContract.tryAcquire, h] using fresh p
      | some _ => simpa [StdMutexContract.tryAcquire, h] using old
  | lockReturned p _ => exact fresh p
  | readGuard _ _ => exact old
  | recoverError _ _ => exact old
  | writeGuard _ _ _ _ => simpa [StdMutexContract.Owns] using old
  | guardDrop _ _ _ => simp [StdMutexContract.Owns, StdMutexContract.releasedState]
  | forgetGuard _ _ => simp [StdMutexContract.Owns, StdMutexContract.forgottenState]
  | uniqueWrite _ _ _ _ => simpa [StdMutexContract.Owns] using old

theorem initial_safe (condvar originalGuard : Nat) (mutex : StdMutexContract.State α)
    (lease : StdMutexContract.Guard) (safe : StdMutexContract.Safe mutex)
    (owns : StdMutexContract.Owns mutex lease) : Safe (initial condvar originalGuard mutex lease) :=
  ⟨safe, owns⟩

theorem begin_wait_safe (s : State α) (g : StdMutexContract.Guard) (timed : Bool)
    (safe : StdMutexContract.Safe s.mutex) (owns : StdMutexContract.Owns s.mutex g) :
    Safe (beginWait s g timed) := by
  have facts : g.mutex = s.mutex.mutex ∧ g.ticket < s.mutex.issued := by
    simpa [StdMutexContract.Safe, owns.2] using safe
  refine ⟨suspended_state_safe s.mutex, ?_⟩
  simp [beginWait, waiterFor, WaiterSafe, suspend, StdMutexContract.Owns, facts.1, facts.2]

theorem signal_safe (s : State α) (safe : Safe s) : Safe (signal s) := by
  cases h : s.phase <;> simp_all [Safe, signal, WaiterSafe]

theorem elapsed_safe (s : State α) (w : Waiter)
    (pending : s.phase = .waiting w true) (safe : Safe s) :
    Safe { s with phase := .ready w true true } := by
  simpa [Safe, pending, WaiterSafe] using safe

theorem peer_keeps_waiter_safe {s : State α} {w : Waiter}
    (safe : WaiterSafe s w)
    (operation : StdMutexContract.Step writes unique s.mutex mutex) :
    WaiterSafe { s with mutex := mutex } w := by
  rcases safe with ⟨condvar, guard, same, issued, old⟩
  refine ⟨condvar, guard, same.trans (StdMutexContract.step_keeps_original_mutex operation).symm,
    Nat.lt_of_lt_of_le issued (mutex_step_issued_monotone operation), ?_⟩
  exact mutex_step_does_not_revive_old_lease operation issued old

theorem peer_safe {s : State α} (pending : Suspended s.phase)
    (operation : StdMutexContract.Step writes (fun _ => False) s.mutex mutex)
    (safe : Safe s) : Safe { s with mutex := mutex } := by
  cases h : s.phase with
  | holding _ => simp [Suspended, h] at pending
  | waiting w timed =>
      simp only [Safe, h] at safe ⊢
      exact ⟨StdMutexContract.step_safe operation safe.1, peer_keeps_waiter_safe safe.2 operation⟩
  | ready w timed timedOut =>
      simp only [Safe, h] at safe ⊢
      exact ⟨StdMutexContract.step_safe operation safe.1,
        peer_keeps_waiter_safe safe.2.1 operation, safe.2.2⟩
  | returned _ => simp [Suspended, h] at pending

theorem resume_preserves_condvar (s : StdMutexContract.State α) (w : Waiter) (timedOut : Bool) :
    (replyValue (resume s w timedOut).2).condvar = w.condvar := by
  cases h : s.poisoned <;> simp [resume, h, replyValue, returnedValue]

theorem finish_wait_safe {s : State α} {w : Waiter} {timed timedOut : Bool}
    (ready : s.phase = .ready w timed timedOut) (safe : Safe s) :
    Safe (finishWait s w timedOut) := by
  have facts : WaiterSafe s w ∧ (timedOut = true → timed = true) := by
    simpa [ready] using safe.2
  have well : WaiterSafe s w := facts.1
  change StdMutexContract.Safe (resume s.mutex w timedOut).1 ∧
    StdMutexContract.Owns (resume s.mutex w timedOut).1 (replyValue (resume s.mutex w timedOut).2).lease ∧
    (replyValue (resume s.mutex w timedOut).2).originalGuard = s.originalGuard ∧
    (replyValue (resume s.mutex w timedOut).2).condvar = s.condvar
  refine ⟨resume_state_safe s.mutex w timedOut, resume_result_is_live s.mutex w timedOut, ?_, ?_⟩
  · exact (resume_returns_original_guard s.mutex w timedOut).trans well.2.1
  · exact (resume_preserves_condvar s.mutex w timedOut).trans well.1

theorem step_safe {s t : State α} (operation : Step writes s t) (safe : Safe s) : Safe t := by
  cases operation with
  | begin g timed _ owns => exact begin_wait_safe _ g timed safe.1 owns
  | notify => exact signal_safe _ safe
  | spurious _ _ _ => exact signal_safe _ safe
  | elapsed w pending => exact elapsed_safe _ w pending safe
  | peer _ pending operation => exact peer_safe pending operation safe
  | returned _ _ _ ready _ => exact finish_wait_safe ready safe

theorem history_safe {s t : State α} (history : History writes s t) (safe : Safe s) : Safe t := by
  induction history with
  | refl => exact safe
  | next _ operation ih => exact step_safe operation ih

theorem notification_keeps_mutex (s : State α) : (signal s).mutex = s.mutex := by
  cases h : s.phase <;> simp [signal, h]

theorem notify_before_wait_has_no_buffer (s : State α) (g : StdMutexContract.Guard)
    (holding : s.phase = .holding g) : signal s = s := by
  simp [signal, holding]

theorem notification_does_not_authorize_suspended_lease {s : State α} {w : Waiter}
    {timed : Bool} (pending : s.phase = .waiting w timed) (safe : Safe s) :
    ¬ StdMutexContract.Owns (signal s).mutex w.suspendedLease := by
  rw [notification_keeps_mutex]
  have facts : WaiterSafe s w := by simpa [pending] using safe.2
  exact facts.2.2.2.2

theorem returned_value_has_live_original_guard {s : State α} {r : Reply}
    (done : s.phase = .returned r) (safe : Safe s) :
    StdMutexContract.Owns s.mutex (replyValue r).lease ∧
    (replyValue r).originalGuard = s.originalGuard ∧ (replyValue r).condvar = s.condvar := by
  simpa [done] using safe.2

theorem step_keeps_original_call {s t : State α} (operation : Step writes s t) :
    t.condvar = s.condvar ∧ t.originalGuard = s.originalGuard := by
  cases operation with
  | begin _ _ _ _ => exact ⟨rfl, rfl⟩
  | notify => cases h : s.phase <;> simp [signal, h]
  | spurious _ _ _ => cases h : s.phase <;> simp [signal, h]
  | elapsed _ _ => exact ⟨rfl, rfl⟩
  | peer _ _ _ => exact ⟨rfl, rfl⟩
  | returned _ _ _ _ _ => exact ⟨rfl, rfl⟩

theorem step_preserves_payload_invariant (invariant : α → Prop) {s t : State α}
    (operation : Step writes s t)
    (body : ∀ old next, writes old next → invariant old → invariant next)
    (prior : invariant s.mutex.payload) : invariant t.mutex.payload := by
  cases operation with
  | begin _ _ _ _ => exact prior
  | notify => simpa only [notification_keeps_mutex] using prior
  | spurious _ _ _ => simpa only [notification_keeps_mutex] using prior
  | elapsed _ _ => exact prior
  | peer _ _ operation => exact StdMutexContract.step_preserves_payload_invariant invariant operation body prior
  | returned _ _ _ _ _ => exact prior

theorem history_preserves_payload_invariant (invariant : α → Prop) {s t : State α}
    (history : History writes s t)
    (body : ∀ old next, writes old next → invariant old → invariant next)
    (prior : invariant s.mutex.payload) : invariant t.mutex.payload := by
  induction history with
  | refl => exact prior
  | next _ operation ih => exact step_preserves_payload_invariant invariant operation body ih

theorem busy_ready_cannot_finish {s : State α} {w : Waiter} {timed timedOut : Bool}
    (_ready : s.phase = .ready w timed timedOut) (held : s.mutex.holder = some g) :
    ¬ (s.phase = .ready w timed timedOut ∧ MayReturn s.mutex w) := by
  intro attempt
  exact busy_mutex_has_no_normal_return held attempt.2

/- An intentionally wrong client/model shortcut, not an extracted Rust body.
   It treats wake-up as ownership without executing the original re-lock. -/
def returnWithoutReacquiring (s : State α) (w : Waiter) : State α :=
  { s with phase := .returned (.normal ⟨w.condvar, w.originalGuard, w.suspendedLease, false⟩) }

theorem wake_without_reacquiring_rejected {s : State α} {w : Waiter}
    (waiting : WaiterSafe s w) : ¬ Safe (returnWithoutReacquiring s w) := by
  intro wrong
  exact waiting.2.2.2.2 wrong.2.1

theorem untimed_ready_is_not_timeout {s : State α} {w : Waiter} {timedOut : Bool}
    (ready : s.phase = .ready w false timedOut) (safe : Safe s) : timedOut = false := by
  have facts : WaiterSafe s w ∧ (timedOut = true → false = true) := by
    simpa [ready] using safe.2
  cases timedOut with
  | false => rfl
  | true => exact Bool.noConfusion (facts.2 rfl)

end SandboxModel.Boundaries.CondvarContract.Waiting
