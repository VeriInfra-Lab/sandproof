import Std
import SandboxModel.Boundaries.Task.Model

/-! Conditional B3 trace contract, not an extraction of async Rust. Signal
updates are latest-value observations; they may interleave between the biased
revocation branch and a work poll. workEffects records work-poll effects,
not global mutable state or destructor effects. A poll has an explicit
in-progress state, including visible prefixes before it returns, panics or
never returns. Drop may perform arbitrary cleanup. A normal-path teardown
certificate never certifies physical process exit.
Only normal destructor observations are counted; interruption retains unknown
cleanup rather than manufacturing a Drop event. -/
namespace SandboxModel.Boundaries.Revocation

variable {α σ : Type}

inductive Signal | pending | revoked | closed deriving DecidableEq, Repr
inductive End (α : Type) | revoked | closed | returned (value : α) | cancelled | interrupted
  deriving DecidableEq, Repr
inductive Phase (α : Type) | checking | workAllowed | polling | selected (reason : End α) | done (reason : End α)
  deriving DecidableEq, Repr

structure Key where
  signal : Nat
  work : Nat
  deriving DecidableEq, Repr

structure State (α σ : Type) where
  key : Key
  signal : Signal
  phase : Phase α
  workEffects : σ
  polls : Nat
  -- Normal-path teardown certificates, not a census of all runtime Drop.
  -- Zero on interruption means no certificate, not that no destructor ran.
  drops : Nat
  deriving DecidableEq, Repr

def initial (key : Key) (signal : Signal) (workEffects : σ) : State α σ :=
  ⟨key, signal, .checking, workEffects, 0, 0⟩

inductive Step : State α σ → State α σ → Prop
  | changeChecking (k a b q n d) : Step
      ⟨k, a, .checking, q, n, d⟩ ⟨k, b, .checking, q, n, d⟩
  | changeAllowed (k a b q n d) : Step
      ⟨k, a, .workAllowed, q, n, d⟩ ⟨k, b, .workAllowed, q, n, d⟩
  | changePolling (k a b q n d) : Step
      ⟨k, a, .polling, q, n, d⟩ ⟨k, b, .polling, q, n, d⟩
  | startPoll (k a q n d) : Step
      ⟨k, a, .workAllowed, q, n, d⟩ ⟨k, a, .polling, q, n + 1, d⟩
  | effect (k a q q' n d) : Step
      ⟨k, a, .polling, q, n, d⟩ ⟨k, a, .polling, q', n, d⟩
  | allow (k q n d) : Step
      ⟨k, .pending, .checking, q, n, d⟩ ⟨k, .pending, .workAllowed, q, n, d⟩
  | stopRevoked (k q n d) : Step
      ⟨k, .revoked, .checking, q, n, d⟩ ⟨k, .revoked, .selected .revoked, q, n, d⟩
  | stopClosed (k q n d) : Step
      ⟨k, .closed, .checking, q, n, d⟩ ⟨k, .closed, .selected .closed, q, n, d⟩
  | pending (k a q q' n d) : Step
      ⟨k, a, .polling, q, n, d⟩ ⟨k, a, .checking, q', n, d⟩
  | ready (k a q q' n d value) : Step
      ⟨k, a, .polling, q, n, d⟩ ⟨k, a, .selected (.returned value), q', n, d⟩
  | cancel (k a q n d) : Step
      ⟨k, a, .checking, q, n, d⟩ ⟨k, a, .selected .cancelled, q, n, d⟩
  | drop (k a q n d reason) : Step
      ⟨k, a, .selected reason, q, n, d⟩ ⟨k, a, .done reason, q, n, d + 1⟩
  | interruptChecking (k a q n d) : Step
      ⟨k, a, .checking, q, n, d⟩ ⟨k, a, .done .interrupted, q, n, d⟩
  | interruptAllowed (k a q n d) : Step
      ⟨k, a, .workAllowed, q, n, d⟩ ⟨k, a, .done .interrupted, q, n, d⟩
  | interruptPolling (k a q n d) : Step
      ⟨k, a, .polling, q, n, d⟩ ⟨k, a, .done .interrupted, q, n, d⟩
  | interruptSelected (k a q n d reason) : Step
      ⟨k, a, .selected reason, q, n, d⟩ ⟨k, a, .done .interrupted, q, n, d⟩

inductive Run : State α σ → State α σ → Prop
  | refl (s) : Run s s
  | next : Run s t → Step t u → Run s u

def LifecycleSafe (s : State α σ) : Prop :=
  s.drops ≤ 1 ∧ match s.phase with
    | .checking | .workAllowed | .polling => s.drops = 0
    | .selected r => s.drops = 0 ∧ r ≠ .interrupted
    | .done .interrupted => s.drops = 0
    | .done _ => s.drops = 1

def settled (s : State α σ) : Prop :=
  match s.phase with | .selected _ | .done _ => True | _ => False

-- Outer none means no observable return (still running, cancelled or aborted).
-- Inner none is the actual until_revoked return on revoked/closed authority.
def result (s : State α σ) : Option (Option α) :=
  match s.phase with
  | .done (.returned value) => some (some value)
  | .done .revoked | .done .closed => some none
  | _ => none

theorem initial_safe (k sig) (q : σ) : LifecycleSafe (initial (α := α) k sig q) := by
  simp [LifecycleSafe, initial]

theorem step_safe {s t : State α σ} (h : Step s t) (safe : LifecycleSafe s) :
    LifecycleSafe t := by
  cases h <;> simp_all [LifecycleSafe]

theorem run_safe {s t : State α σ} (h : Run s t) (safe : LifecycleSafe s) :
    LifecycleSafe t := by
  induction h with
  | refl => exact safe
  | next _ step ih => exact step_safe step ih

theorem reachable_drop_at_most_once {s : State α σ} (h : Run (initial k sig q) s) :
    s.drops ≤ 1 := (run_safe h (initial_safe k sig q)).1

theorem key_preserved {s t : State α σ} (h : Step s t) : t.key = s.key := by
  cases h <;> rfl

theorem history_key_preserved {s t : State α σ} (h : Run s t) : t.key = s.key := by
  induction h with
  | refl => rfl
  | next _ step ih => exact (key_preserved step).trans ih

theorem settled_step_keeps_work {s t : State α σ} (h : Step s t) (chosen : settled s) :
    settled t ∧ t.polls = s.polls ∧ t.workEffects = s.workEffects := by
  cases h <;> simp_all [settled]

theorem settled_run_keeps_work {s t : State α σ} (h : Run s t) (chosen : settled s) :
    settled t ∧ t.polls = s.polls ∧ t.workEffects = s.workEffects := by
  induction h with
  | refl => exact ⟨chosen, rfl, rfl⟩
  | next _ step ih =>
    have keep := settled_step_keeps_work step ih.1
    exact ⟨keep.1, keep.2.1.trans ih.2.1, keep.2.2.trans ih.2.2⟩

theorem terminal_has_no_more_steps (k sig r q n d) (t : State α σ) :
    ¬ Step ⟨k, sig, .done r, q, n, d⟩ t := by
  intro h
  cases h

theorem selected_revocation_preserves_all_prior_effects
    (h : Run (α := α) ⟨k, .revoked, .selected .revoked, q, n, 0⟩ t) :
    t.polls = n ∧ t.workEffects = q := by
  exact (settled_run_keeps_work h (by trivial)).2

theorem already_revoked_normal_return (k) (q : σ) :
    Run (initial (α := α) k .revoked q)
      ⟨k, .revoked, .done .revoked, q, 0, 1⟩ := by
  exact .next (.next (.refl _) (.stopRevoked k q 0 0)) (.drop k .revoked q 0 0 .revoked)

theorem closed_authority_normal_return (k) (q : σ) :
    Run (initial (α := α) k .closed q)
      ⟨k, .closed, .done .closed, q, 0, 1⟩ := by
  exact .next (.next (.refl _) (.stopClosed k q 0 0)) (.drop k .closed q 0 0 .closed)

theorem revoked_and_closed_have_same_public_return (k) (q : σ) (n) :
    result (α := α) ⟨k, .revoked, .done .revoked, q, n, 1⟩ = some none ∧
    result (α := α) ⟨k, .closed, .done .closed, q, n, 1⟩ = some none := by exact ⟨rfl, rfl⟩

theorem work_result_is_returned_exactly (k sig) (q : σ) (n) (value : α) :
    result ⟨k, sig, .done (.returned value), q, n, 1⟩ = some (some value) := by rfl

theorem caller_cancellation_does_not_return_none (k sig) (q : σ) (n) :
    result (α := α) ⟨k, sig, .done .cancelled, q, n, 1⟩ = none := by rfl

theorem interruption_does_not_invent_drop (k sig) (q : σ) (n) :
    Step (α := α) ⟨k, sig, .checking, q, n, 0⟩
      ⟨k, sig, .done .interrupted, q, n, 0⟩ := .interruptChecking k sig q n 0

theorem revocation_between_branches_may_allow_work (k) (q q' : σ) (value : α) :
    Run (initial k .pending q)
      ⟨k, .revoked, .done (.returned value), q', 1, 1⟩ := by
  have allowed : Run (initial (α := α) k .pending q) ⟨k, .pending, .workAllowed, q, 0, 0⟩ :=
    .next (.refl _) (.allow k q 0 0)
  have revoked := Run.next allowed (Step.changeAllowed k .pending .revoked q 0 0)
  have entered := Run.next revoked (Step.startPoll k .revoked q 0 0)
  have ready := Run.next entered (Step.ready k .revoked q q' 1 0 value)
  exact .next ready (.drop k .revoked q' 1 0 (.returned value))

theorem latest_value_does_not_remember_unobserved_true (k) (q : σ) (value : α) :
    Run (initial k .pending q)
      ⟨k, .pending, .done (.returned value), q, 1, 1⟩ := by
  have raised : Run (initial (α := α) k .pending q) ⟨k, .revoked, .checking, q, 0, 0⟩ :=
    .next (.refl _) (.changeChecking k .pending .revoked q 0 0)
  have lowered := Run.next raised (Step.changeChecking k .revoked .pending q 0 0)
  have allowed := Run.next lowered (Step.allow k q 0 0)
  have entered := Run.next allowed (Step.startPoll k .pending q 0 0)
  have ready := Run.next entered (Step.ready k .pending q q 1 0 value)
  exact .next ready (.drop k .pending q 1 0 (.returned value))

-- Direct composition with the existing B1 interface, without redefining pools.
theorem cancelled_wait_does_not_release_running_task (k w e) :
    Task.Step ⟨k, .status, .running, true, true, w, 1, e, .none⟩
      ⟨k, .status, .running, true, true, false, 1, e, .none⟩ :=
  Task.running_wait_cancel_keeps_both k w e

def StopsBeforeWork (before after : State α σ) : Prop :=
  after.polls = before.polls ∧ after.workEffects = before.workEffects

theorem correct_revocation_observation_has_same_priority (k) (q : σ) (n d) :
    StopsBeforeWork (α := α) ⟨k, .revoked, .checking, q, n, d⟩
      ⟨k, .revoked, .selected .revoked, q, n, d⟩ := ⟨rfl, rfl⟩

theorem work_first_version_breaks_same_priority (k) (q : σ) (n d) (value : α) :
    ¬ StopsBeforeWork ⟨k, .revoked, .checking, q, n, d⟩
      ⟨k, .revoked, .selected (.returned value), q, n + 1, d⟩ := by
  simp [StopsBeforeWork]

-- A visible prefix needs no fabricated Pending or successful poll return.
theorem nonreturning_work_can_have_visible_prefix (k) (q q' : σ) :
    Run (initial (α := α) k .pending q)
      ⟨k, .pending, .polling, q', 1, 0⟩ := by
  have allowed : Run (initial (α := α) k .pending q)
      ⟨k, .pending, .workAllowed, q, 0, 0⟩ := .next (.refl _) (.allow k q 0 0)
  have entered := Run.next allowed (Step.startPoll k .pending q 0 0)
  exact .next entered (.effect k .pending q q' 1 0)

theorem poll_effect_before_panic_is_retained (k) (q q' : σ) :
    Run (initial (α := α) k .pending q)
      ⟨k, .pending, .done .interrupted, q', 1, 0⟩ :=
  .next (nonreturning_work_can_have_visible_prefix k q q')
    (.interruptPolling k .pending q' 1 0)

theorem revocation_during_pending_poll_prevents_repoll (k) (q q' : σ) :
    Run (initial (α := α) k .pending q)
      ⟨k, .revoked, .done .revoked, q', 1, 1⟩ := by
  have workPrefix := nonreturning_work_can_have_visible_prefix (α := α) k q q'
  have changed := Run.next workPrefix (Step.changePolling k .pending .revoked q' 1 0)
  have pending := Run.next changed (Step.pending k .revoked q' q' 1 0)
  have selected := Run.next pending (Step.stopRevoked k q' 1 0)
  exact .next selected (.drop k .revoked q' 1 0 .revoked)

end SandboxModel.Boundaries.Revocation
