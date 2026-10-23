import SandboxModel.Boundaries.StdMutex.Guard

namespace SandboxModel.Boundaries.CondvarContract

/-! Conditional A6/B2 core for one original Condvar call.
The original physical guard is suspended, not destroyed, while its logical
mutex lease ends. Re-acquisition keeps its original panic marker and observes
the current protected payload. Poison errors still own the returned guard.
The pure resume constructor is not an enabled API call: ReturnStep requires
the free original mutex. A complete waiting protocol must also supply the
matching registered/ready invocation. No wake-up, predicate or progress
conclusion follows from the value constructor alone. -/

/- A wait suspends a lock lease without dropping the original Rust guard.
   The physical guard identity and its original poison marker survive the call.
   A new logical lease describes the later re-acquisition of that same guard. -/
structure Waiter where
  condvar : Nat
  originalGuard : Nat
  suspendedLease : StdMutexContract.Guard
  deriving DecidableEq, Repr

def suspend (s : StdMutexContract.State α) : StdMutexContract.State α :=
  { s with holder := none, active := false }

structure Returned where
  condvar : Nat
  originalGuard : Nat
  lease : StdMutexContract.Guard
  timedOut : Bool
  deriving DecidableEq, Repr

inductive Reply
  | normal (returned : Returned)
  | poisoned (returned : Returned)
  deriving DecidableEq, Repr

def replyValue : Reply → Returned
  | .normal value | .poisoned value => value

def resumedState (s : StdMutexContract.State α) (w : Waiter) : StdMutexContract.State α :=
  StdMutexContract.acquiredState s w.suspendedLease.panickingAtAcquire

def returnedValue (s : StdMutexContract.State α) (w : Waiter) (timedOut : Bool) : Returned :=
  ⟨w.condvar, w.originalGuard,
    StdMutexContract.freshGuard s w.suspendedLease.panickingAtAcquire, timedOut⟩

def resume (s : StdMutexContract.State α) (w : Waiter) (timedOut : Bool) : StdMutexContract.State α × Reply :=
  (resumedState s w, if s.poisoned then .poisoned (returnedValue s w timedOut)
    else .normal (returnedValue s w timedOut))

/- Only a completed raw re-acquisition may generate a normal API reply.
   Wake-up, a signal and timeout expiry themselves do not acquire the mutex. -/
def MayReturn (s : StdMutexContract.State α) (w : Waiter) : Prop :=
  s.holder = none ∧ w.suspendedLease.mutex = s.mutex

theorem suspend_keeps_payload (s : StdMutexContract.State α) : (suspend s).payload = s.payload := by rfl

theorem suspend_keeps_poison (s : StdMutexContract.State α) : (suspend s).poisoned = s.poisoned := by rfl

theorem suspended_lease_cannot_access_payload (s : StdMutexContract.State α) (g : StdMutexContract.Guard) :
    ¬ StdMutexContract.Owns (suspend s) g := by simp [StdMutexContract.Owns, suspend]

theorem suspended_state_safe (s : StdMutexContract.State α) : StdMutexContract.Safe (suspend s) := by rfl

theorem resume_keeps_current_payload (s : StdMutexContract.State α) (w : Waiter) (timedOut : Bool) :
    (resume s w timedOut).1.payload = s.payload := by rfl

theorem resume_does_not_clear_poison (s : StdMutexContract.State α) (w : Waiter) (timedOut : Bool) :
    (resume s w timedOut).1.poisoned = s.poisoned := by rfl

theorem resume_state_safe (s : StdMutexContract.State α) (w : Waiter) (timedOut : Bool) :
    StdMutexContract.Safe (resume s w timedOut).1 := StdMutexContract.acquisition_safe s _

theorem resume_returns_original_guard (s : StdMutexContract.State α) (w : Waiter) (timedOut : Bool) :
    (replyValue (resume s w timedOut).2).originalGuard = w.originalGuard := by
  cases h : s.poisoned <;> simp [resume, h, replyValue, returnedValue]

theorem resume_preserves_original_poison_marker (s : StdMutexContract.State α) (w : Waiter)
    (timedOut : Bool) :
    (replyValue (resume s w timedOut).2).lease.panickingAtAcquire =
      w.suspendedLease.panickingAtAcquire := by
  cases h : s.poisoned <;> simp [resume, h, replyValue, returnedValue, StdMutexContract.freshGuard]

theorem resume_result_holds_original_mutex {s : StdMutexContract.State α} {w : Waiter}
    (allowed : MayReturn s w) (timedOut : Bool) :
    (replyValue (resume s w timedOut).2).lease.mutex = w.suspendedLease.mutex := by
  cases h : s.poisoned <;> simp [resume, h, replyValue, returnedValue, StdMutexContract.freshGuard, allowed.2]

theorem resume_result_is_live (s : StdMutexContract.State α) (w : Waiter) (timedOut : Bool) :
    StdMutexContract.Owns (resume s w timedOut).1 (replyValue (resume s w timedOut).2).lease := by
  cases h : s.poisoned <;>
    simp [resume, h, replyValue, returnedValue, resumedState, StdMutexContract.Owns, StdMutexContract.acquiredState]

theorem poison_error_still_returns_guard_and_timeout (s : StdMutexContract.State α) (w : Waiter)
    (timedOut : Bool) (poisoned : s.poisoned = true) :
    (resume s w timedOut).2 = .poisoned (returnedValue s w timedOut) ∧
      (replyValue (resume s w timedOut).2).timedOut = timedOut := by
  simp [resume, poisoned, replyValue, returnedValue]

theorem busy_mutex_has_no_normal_return {s : StdMutexContract.State α} {w : Waiter}
    (held : s.holder = some g) : ¬ MayReturn s w := by
  simp [MayReturn, held]

theorem returned_guard_reads_current_invariant (invariant : α → Prop)
    (s : StdMutexContract.State α) (w : Waiter) (timedOut : Bool) (current : invariant s.payload) :
    invariant (resume s w timedOut).1.payload := current

/- Pure resume values are not executable API calls. A completed return
   transition requires the free original mutex; the waiting-state layer
   additionally supplies the matching live, ready invocation. -/
inductive ReturnStep (s : StdMutexContract.State α) (w : Waiter) (timedOut : Bool) :
    StdMutexContract.State α × Reply → Prop
  | completed (allowed : MayReturn s w) : ReturnStep s w timedOut (resume s w timedOut)

theorem return_step_requires_free_mutex {s : StdMutexContract.State α} {w : Waiter}
    {timedOut : Bool} {target : StdMutexContract.State α × Reply}
    (step : ReturnStep s w timedOut target) : s.holder = none := by
  cases step with
  | completed allowed => exact allowed.1

theorem held_mutex_cannot_return {s : StdMutexContract.State α} {w : Waiter}
    {timedOut : Bool} {target : StdMutexContract.State α × Reply}
    (held : s.holder = some g) : ¬ ReturnStep s w timedOut target := by
  intro step
  have free := return_step_requires_free_mutex step
  cases held.symm.trans free

theorem return_step_uses_original_mutex {s : StdMutexContract.State α} {w : Waiter}
    {timedOut : Bool} {target : StdMutexContract.State α × Reply}
    (step : ReturnStep s w timedOut target) :
    (replyValue target.2).lease.mutex = w.suspendedLease.mutex := by
  cases step with
  | completed allowed => exact resume_result_holds_original_mutex allowed timedOut

theorem return_step_preserves_physical_guard {s : StdMutexContract.State α} {w : Waiter}
    {timedOut : Bool} {target : StdMutexContract.State α × Reply}
    (step : ReturnStep s w timedOut target) :
    (replyValue target.2).originalGuard = w.originalGuard := by
  cases step
  exact resume_returns_original_guard s w timedOut

theorem return_step_returns_live_guard {s : StdMutexContract.State α} {w : Waiter}
    {timedOut : Bool} {target : StdMutexContract.State α × Reply}
    (step : ReturnStep s w timedOut target) :
    StdMutexContract.Owns target.1 (replyValue target.2).lease := by
  cases step
  exact resume_result_is_live s w timedOut

theorem return_step_keeps_current_payload {s : StdMutexContract.State α} {w : Waiter}
    {timedOut : Bool} {target : StdMutexContract.State α × Reply}
    (step : ReturnStep s w timedOut target) : target.1.payload = s.payload := by
  cases step
  exact resume_keeps_current_payload s w timedOut

end SandboxModel.Boundaries.CondvarContract
