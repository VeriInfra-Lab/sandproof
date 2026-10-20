import Std

namespace SandboxModel.Boundaries.StdMutexContract
/-! A6/B2 event projection of one original Mutex, not Rust memory semantics.
Payloads describe guarded inline fields and original handles, not snapshots
of aliased mutable heaps. Tickets count logical acquisitions; they are not
runtime addresses, OS thread IDs, a FIFO queue, or a progress guarantee.
A forgotten guard leaves the raw lock held. Unique writes summarize a valid
exclusive Rust loan; the caller supplies its identity and body relation.
Panicking flags are the actual library observations, not a guarantee that
every panic, foreign exception or abort is detected or runs a destructor. -/
structure Guard where
  mutex : Nat
  ticket : Nat
  panickingAtAcquire : Bool
  deriving DecidableEq, Repr

structure State (α : Type) where
  mutex : Nat
  payload : α
  poisoned : Bool
  holder : Option Guard
  active : Bool
  issued : Nat
  deriving DecidableEq, Repr

def initial (mutex : Nat) (value : α) : State α := ⟨mutex, value, false, none, false, 0⟩
def Owns (s : State α) (g : Guard) : Prop := s.active = true ∧ s.holder = some g

def Safe (s : State α) : Prop :=
  match s.holder with
  | none => s.active = false
  | some g => g.mutex = s.mutex ∧ g.ticket < s.issued

def freshGuard (s : State α) (panicking : Bool) : Guard :=
  ⟨s.mutex, s.issued, panicking⟩

def acquiredState (s : State α) (panicking : Bool) : State α :=
  { s with holder := some (freshGuard s panicking), active := true, issued := s.issued + 1 }

inductive Outcome
  | busy
  | acquired (guard : Guard)
  | poisoned (guard : Guard)
  deriving DecidableEq, Repr

def outcomeGuard : Outcome → Option Guard
  | .busy => none
  | .acquired guard | .poisoned guard => some guard

def tryAcquire (s : State α) (panicking : Bool) : State α × Outcome :=
  match s.holder with
  | some _ => (s, .busy)
  | none => (acquiredState s panicking,
      if s.poisoned then .poisoned (freshGuard s panicking)
      else .acquired (freshGuard s panicking))

def releasedState (s : State α) (g : Guard) (panickingNow : Bool) : State α :=
  { s with
    holder := none
    active := false
    poisoned := s.poisoned || (!g.panickingAtAcquire && panickingNow) }

def forgottenState (s : State α) : State α := { s with active := false }

def CleanlyReleased (s : State α) : Prop :=
  s.holder = none ∧ s.active = false ∧ s.poisoned = false

inductive Step (writes : α → α → Prop) (uniqueAccess : State α → Prop) :
    State α → State α → Prop
  | tryCall (s panicking) : Step writes uniqueAccess s (tryAcquire s panicking).1
  | lockReturned (s panicking) (free : s.holder = none) :
      Step writes uniqueAccess s (acquiredState s panicking)
  | readGuard (s g) (owns : Owns s g) : Step writes uniqueAccess s s
  | recoverError (s g) (owns : Owns s g) : Step writes uniqueAccess s s
  | writeGuard (s g value) (owns : Owns s g) (body : writes s.payload value) :
      Step writes uniqueAccess s { s with payload := value }
  | guardDrop (s g panicking) (owns : Owns s g) :
      Step writes uniqueAccess s (releasedState s g panicking)
  | forgetGuard (s g) (owns : Owns s g) : Step writes uniqueAccess s (forgottenState s)
  | uniqueWrite (s value) (inactive : s.active = false)
      (unique : uniqueAccess s) (body : writes s.payload value) :
      Step writes uniqueAccess s { s with payload := value }

inductive History (writes : α → α → Prop) (uniqueAccess : State α → Prop) :
    State α → State α → Prop
  | refl (s) : History writes uniqueAccess s s
  | next : History writes uniqueAccess s t → Step writes uniqueAccess t u →
      History writes uniqueAccess s u

theorem initial_safe (mutex : Nat) (value : α) : Safe (initial mutex value) := by rfl

theorem acquisition_safe (s : State α) (panicking : Bool) :
    Safe (acquiredState s panicking) := by
  simp [Safe, acquiredState, freshGuard]

theorem try_acquisition_safe (s : State α) (panicking : Bool) (safe : Safe s) :
    Safe (tryAcquire s panicking).1 := by
  cases h : s.holder with
  | none => simpa [tryAcquire, h] using acquisition_safe s panicking
  | some g => simpa [tryAcquire, h] using safe

theorem normal_guard_release_safe (s : State α) (g : Guard) (panicking : Bool) :
    Safe (releasedState s g panicking) := by rfl

theorem forgotten_state_safe (s : State α) (safe : Safe s) : Safe (forgottenState s) := by
  cases h : s.holder <;> simp_all [Safe, forgottenState]

theorem try_acquisition_keeps_original_mutex (s : State α) (p : Bool) :
    (tryAcquire s p).1.mutex = s.mutex := by
  cases h : s.holder <;> simp [tryAcquire, h, acquiredState]

theorem try_acquisition_keeps_payload (s : State α) (p : Bool) :
    (tryAcquire s p).1.payload = s.payload := by
  cases h : s.holder <;> simp [tryAcquire, h, acquiredState]

theorem step_safe {s t : State α} (step : Step writes unique s t) (safe : Safe s) :
    Safe t := by
  cases step with
  | tryCall p => exact try_acquisition_safe _ p safe
  | lockReturned p _ => exact acquisition_safe _ p
  | readGuard _ _ => exact safe
  | recoverError _ _ => exact safe
  | writeGuard _ _ _ _ => simpa [Safe] using safe
  | guardDrop g p _ => exact normal_guard_release_safe _ g p
  | forgetGuard _ _ => exact forgotten_state_safe _ safe
  | uniqueWrite _ _ _ _ => simpa [Safe] using safe

theorem history_safe {s t : State α} (history : History writes unique s t) (safe : Safe s) :
    Safe t := by
  induction history with
  | refl => exact safe
  | next _ step ih => exact step_safe step ih

theorem two_live_guards_are_the_same {s : State α} (left : Owns s g) (right : Owns s h) :
    g = h := Option.some.inj (left.2.symm.trans right.2)

theorem owned_guard_matches_original_mutex {s : State α} (safe : Safe s) (owns : Owns s g) :
    g.mutex = s.mutex := by
  have facts : g.mutex = s.mutex ∧ g.ticket < s.issued := by
    simpa [Safe, owns.2] using safe
  exact facts.1

theorem other_mutex_guard_cannot_own {s : State α} (safe : Safe s)
    (different : g.mutex ≠ s.mutex) : ¬ Owns s g := by
  intro owns
  exact different (owned_guard_matches_original_mutex safe owns)

theorem unreached_ticket_cannot_own {s : State α} (safe : Safe s)
    (notIssued : s.issued ≤ g.ticket) : ¬ Owns s g := by
  intro owns
  have facts : g.mutex = s.mutex ∧ g.ticket < s.issued := by
    simpa [Safe, owns.2] using safe
  omega

theorem busy_keeps_original_state (s : State α) (held : s.holder = some g) (p : Bool) :
    tryAcquire s p = (s, .busy) := by simp [tryAcquire, held]

theorem busy_has_no_guard : outcomeGuard .busy = none := by rfl

theorem free_acquisition_keeps_payload_and_poison (s : State α) (p : Bool) :
    (acquiredState s p).payload = s.payload ∧
    (acquiredState s p).poisoned = s.poisoned := by constructor <;> rfl

theorem free_acquisition_returns_original_guard (s : State α) (free : s.holder = none) (p : Bool) :
    outcomeGuard (tryAcquire s p).2 = some (freshGuard s p) ∧
    Owns (tryAcquire s p).1 (freshGuard s p) := by
  cases h : s.poisoned <;> simp [tryAcquire, free, h, outcomeGuard, Owns, acquiredState]

theorem poisoned_result_still_holds_original_guard (s : State α) (free : s.holder = none)
    (poison : s.poisoned = true) (p : Bool) :
    (tryAcquire s p).2 = .poisoned (freshGuard s p) ∧
      Owns (tryAcquire s p).1 (freshGuard s p) := by
  simp [tryAcquire, free, poison, Owns, acquiredState]

theorem poison_error_recovery_returns_same_guard (g : Guard) :
    outcomeGuard (.poisoned g) = some g := by rfl

theorem recovery_does_not_repair_payload_or_clear_poison (s : State α) (g : Guard)
    (owns : Owns s g) : Step writes unique s s := .recoverError s g owns

theorem guard_drop_keeps_original_payload (s : State α) (g : Guard) (p : Bool) :
    (releasedState s g p).payload = s.payload := by rfl

theorem normal_drop_keeps_existing_poison (s : State α) (g : Guard) :
    (releasedState s g false).poisoned = s.poisoned := by simp [releasedState]

theorem existing_poison_remains_after_drop (s : State α) (g : Guard) (p : Bool)
    (poison : s.poisoned = true) : (releasedState s g p).poisoned = true := by
  simp [releasedState, poison]

theorem newly_observed_unwind_sets_poison (s : State α) (g : Guard)
    (beganNormally : g.panickingAtAcquire = false) :
    (releasedState s g true).poisoned = true := by simp [releasedState, beganNormally]

theorem already_unwinding_acquisition_need_not_set_poison (s : State α) (g : Guard)
    (beganUnwinding : g.panickingAtAcquire = true) :
    (releasedState s g true).poisoned = s.poisoned := by simp [releasedState, beganUnwinding]

theorem ended_guard_cannot_be_used_again (s : State α) (g h : Guard) (p : Bool) :
    ¬ Owns (releasedState s g p) h := by simp [Owns, releasedState]

theorem forgotten_guard_does_not_unlock (s : State α) :
    (forgottenState s).holder = s.holder := by rfl

theorem forgotten_guard_cannot_be_used (s : State α) (g : Guard) :
    ¬ Owns (forgottenState s) g := by simp [Owns, forgottenState]

theorem forgotten_guard_still_makes_try_lock_busy (s : State α) (held : s.holder = some g) (p : Bool) :
    (tryAcquire (forgottenState s) p).2 = .busy := by
  simp [tryAcquire, forgottenState, held]

theorem step_keeps_original_mutex {s t : State α} (step : Step writes unique s t) :
    t.mutex = s.mutex := by
  cases step with
  | tryCall p => exact try_acquisition_keeps_original_mutex _ p
  | lockReturned _ _ => rfl
  | readGuard _ _ => rfl
  | recoverError _ _ => rfl
  | writeGuard _ _ _ _ => rfl
  | guardDrop _ _ _ => rfl
  | forgetGuard _ _ => rfl
  | uniqueWrite _ _ _ _ => rfl

theorem step_preserves_payload_invariant (invariant : α → Prop) {s t : State α}
    (step : Step writes unique s t)
    (body : ∀ old next, writes old next → invariant old → invariant next)
    (prior : invariant s.payload) : invariant t.payload := by
  cases step with
  | tryCall p => simpa only [try_acquisition_keeps_payload] using prior
  | lockReturned _ _ => simpa [acquiredState] using prior
  | readGuard _ _ => exact prior
  | recoverError _ _ => exact prior
  | writeGuard _ _ _ change => exact body _ _ change prior
  | guardDrop _ _ _ => simpa [releasedState] using prior
  | forgetGuard _ _ => simpa [forgottenState] using prior
  | uniqueWrite _ _ _ change => exact body _ _ change prior

theorem history_preserves_payload_invariant (invariant : α → Prop) {s t : State α}
    (history : History writes unique s t)
    (body : ∀ old next, writes old next → invariant old → invariant next)
    (prior : invariant s.payload) : invariant t.payload := by
  induction history with
  | refl => exact prior
  | next _ step ih => exact step_preserves_payload_invariant invariant step body ih

def sampleLocked : State Nat := acquiredState (initial 7 19) false

theorem correct_release_before_diagnostic_failure :
    CleanlyReleased (releasedState sampleLocked (freshGuard (initial 7 19) false) false) := by
  simp [CleanlyReleased, releasedState, sampleLocked, acquiredState, freshGuard, initial]

theorem late_release_after_diagnostic_failure_rejected :
    ¬ CleanlyReleased (releasedState sampleLocked (freshGuard (initial 7 19) false) true) := by
  simp [CleanlyReleased, releasedState, sampleLocked, acquiredState, freshGuard, initial]

theorem unique_write_does_not_clear_abandoned_lock (s : State α) (value : α) :
    ({ s with payload := value } : State α).holder = s.holder := by rfl

end SandboxModel.Boundaries.StdMutexContract
