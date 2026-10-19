import Std

/-!
Interface contract for the latest single-work-gate Host entry points. This is
not translated async Rust and does not verify Tokio, Arc, Drop or our adapters.
Each state describes ONE task and its exact work/status pool identities. The
external semaphore contract supplies unique permits for those pool identities;
the Capacity module adds a finite-roster bound. No scheduling fairness,
business resource accounting or whole-system progress result is asserted.
The acquired phase includes Tokio assigning a token to a queued future BEFORE
that future is repolled. Cancellation there returns both owned reservations.
Distinct task entries represent fresh invocations, not unique business requests.

Rules describe primitive token transfers and possible observations, not the
business safety target. Callback effects may happen before either a callback
Busy or a JoinError. A separate actual-core proof must cover every observable
callback outcome and any panic-visible prefix before composing business safety.
-/
namespace SandboxModel.Boundaries
namespace Task

inductive Mode | ordinary | status deriving DecidableEq, Repr
inductive Phase | idle | waitingWork | acquired | submitted | running | settled | done | discarded
  deriving DecidableEq, Repr
inductive Exit | none | admissionBusy | callbackOk | callbackBusy | callbackError | joinUnknown
  deriving DecidableEq, Repr

structure Key where
  job : Nat
  workPool : Nat
  statusPool : Nat
  deriving DecidableEq, Repr

structure State where
  key : Key
  mode : Mode
  phase : Phase
  workHeld : Bool
  statusHeld : Bool
  waiting : Bool
  calls : Nat
  effects : Nat
  exit : Exit
  deriving DecidableEq, Repr

def statusToken : Mode → Bool | .ordinary => false | .status => true

def initial (key : Key) (mode : Mode) : State :=
  ⟨key, mode, .idle, false, false, true, 0, 0, .none⟩

inductive Step : State → State → Prop
  | cancelIdle (k m w) : Step
      ⟨k, m, .idle, false, false, w, 0, 0, .none⟩
      ⟨k, m, .discarded, false, false, false, 0, 0, .none⟩
  | ordinaryAcquire (k w) : Step
      ⟨k, .ordinary, .idle, false, false, w, 0, 0, .none⟩
      ⟨k, .ordinary, .acquired, true, false, w, 0, 0, .none⟩
  | statusReserve (k w) : Step
      ⟨k, .status, .idle, false, false, w, 0, 0, .none⟩
      ⟨k, .status, .waitingWork, false, true, w, 0, 0, .none⟩
  | statusAcquireWork (k w) : Step
      ⟨k, .status, .waitingWork, false, true, w, 0, 0, .none⟩
      ⟨k, .status, .acquired, true, true, w, 0, 0, .none⟩
  | rejectInitial (k m w) : Step
      ⟨k, m, .idle, false, false, w, 0, 0, .none⟩
      ⟨k, m, .discarded, false, false, w, 0, 0, .admissionBusy⟩
  | closedWork (k w) : Step
      ⟨k, .status, .waitingWork, false, true, w, 0, 0, .none⟩
      ⟨k, .status, .discarded, false, false, w, 0, 0, .admissionBusy⟩
  | cancelQueuedStatus (k w) : Step
      ⟨k, .status, .waitingWork, false, true, w, 0, 0, .none⟩
      ⟨k, .status, .discarded, false, false, false, 0, 0, .none⟩
  | cancelAcquired (k m w) : Step
      ⟨k, m, .acquired, true, statusToken m, w, 0, 0, .none⟩
      ⟨k, m, .discarded, false, false, false, 0, 0, .none⟩
  | submit (k m w) : Step
      ⟨k, m, .acquired, true, statusToken m, w, 0, 0, .none⟩
      ⟨k, m, .submitted, true, statusToken m, w, 0, 0, .none⟩
  | start (k m w) : Step
      ⟨k, m, .submitted, true, statusToken m, w, 0, 0, .none⟩
      ⟨k, m, .running, true, statusToken m, w, 1, 0, .none⟩
  | discardNotStarted (k m w) : Step
      ⟨k, m, .submitted, true, statusToken m, w, 0, 0, .none⟩
      ⟨k, m, .discarded, false, false, w, 0, 0, .none⟩
  | lostNotStarted (k m w) : Step
      ⟨k, m, .submitted, true, statusToken m, w, 0, 0, .none⟩
      ⟨k, m, .done, false, false, w, 0, 0, .joinUnknown⟩
  | cancelSubmittedWait (k m w) : Step
      ⟨k, m, .submitted, true, statusToken m, w, 0, 0, .none⟩
      ⟨k, m, .submitted, true, statusToken m, false, 0, 0, .none⟩
  | cancelRunningWait (k m w e) : Step
      ⟨k, m, .running, true, statusToken m, w, 1, e, .none⟩
      ⟨k, m, .running, true, statusToken m, false, 1, e, .none⟩
  | effect (k m w e) : Step
      ⟨k, m, .running, true, statusToken m, w, 1, e, .none⟩
      ⟨k, m, .running, true, statusToken m, w, 1, e + 1, .none⟩
  | finish (k m w e x) (kind : x = .callbackOk ∨ x = .callbackBusy ∨ x = .callbackError ∨ x = .joinUnknown) : Step
      ⟨k, m, .running, true, statusToken m, w, 1, e, .none⟩
      ⟨k, m, .done, false, false, w, 1, e, x⟩

-- Settled means the callback cannot run again; its guards can still be
-- dropped one by one. This exposes interleavings during tuple destruction.
  | settle (k m w e x)
      (kind : x = .callbackOk ∨ x = .callbackBusy ∨ x = .callbackError ∨ x = .joinUnknown) : Step
      ⟨k, m, .running, true, statusToken m, w, 1, e, .none⟩
      ⟨k, m, .settled, true, statusToken m, w, 1, e, x⟩
  | settleCancelledAcquired (k m w) : Step
      ⟨k, m, .acquired, true, statusToken m, w, 0, 0, .none⟩
      ⟨k, m, .settled, true, statusToken m, false, 0, 0, .none⟩
  | settleCancelledQueued (k w) : Step
      ⟨k, .status, .waitingWork, false, true, w, 0, 0, .none⟩
      ⟨k, .status, .settled, false, true, false, 0, 0, .none⟩
  | settleDiscardedSubmitted (k m w) : Step
      ⟨k, m, .submitted, true, statusToken m, w, 0, 0, .none⟩
      ⟨k, m, .settled, true, statusToken m, w, 0, 0, .joinUnknown⟩
  | dropSettledWork (k m w b c e x) : Step
      ⟨k, m, .settled, true, b, w, c, e, x⟩
      ⟨k, m, .settled, false, b, w, c, e, x⟩
  | dropSettledStatus (k m w b c e x) : Step
      ⟨k, m, .settled, b, true, w, c, e, x⟩
      ⟨k, m, .settled, b, false, w, c, e, x⟩
  | retireSettled (k m w c e x) : Step
      ⟨k, m, .settled, false, false, w, c, e, x⟩
      ⟨k, m, .done, false, false, w, c, e, x⟩
  | cancelSettledWait (k m w a b c e x) : Step
      ⟨k, m, .settled, a, b, w, c, e, x⟩
      ⟨k, m, .settled, a, b, false, c, e, x⟩

inductive Run : State → State → Prop
  | refl (s) : Run s s
  | next : Run s t → Step t u → Run s u

def ResourceSafe (s : State) : Prop :=
  s.calls ≤ 1 ∧
  ((s.phase = .acquired ∨ s.phase = .submitted ∨ s.phase = .running) →
    s.workHeld = true ∧ s.statusHeld = statusToken s.mode) ∧
  (s.phase = .waitingWork → s.mode = .status ∧ s.workHeld = false ∧ s.statusHeld = true) ∧
  ((s.phase = .idle ∨ s.phase = .done ∨ s.phase = .discarded) →
    s.workHeld = false ∧ s.statusHeld = false) ∧
  (s.exit = .admissionBusy → s.calls = 0 ∧ s.effects = 0)

theorem initial_safe (k m) : ResourceSafe (initial k m) := by
  simp [ResourceSafe, initial]

theorem step_safe {s t} (step : Step s t) (safe : ResourceSafe s) : ResourceSafe t := by
  cases step <;> simp_all [ResourceSafe, statusToken]
  case finish k m w e x kind =>
    rcases kind with rfl | rfl | rfl | rfl <;> simp
  case settle k m w e x kind =>
    rcases kind with rfl | rfl | rfl | rfl <;> simp

theorem run_safe {s t} (run : Run s t) (safe : ResourceSafe s) : ResourceSafe t := by
  induction run with
  | refl => exact safe
  | next _ step ih => exact step_safe step ih

theorem pool_binding_preserved {s t} (step : Step s t) :
    t.key = s.key ∧ t.mode = s.mode := by
  cases step <;> simp

theorem status_wait_cancel_releases_only_reservation (k w) :
    Step ⟨k, .status, .waitingWork, false, true, w, 0, 0, .none⟩
      ⟨k, .status, .discarded, false, false, false, 0, 0, .none⟩ :=
  .cancelQueuedStatus k w

theorem running_wait_cancel_keeps_both (k w e) :
    Step ⟨k, .status, .running, true, true, w, 1, e, .none⟩
      ⟨k, .status, .running, true, true, false, 1, e, .none⟩ :=
  .cancelRunningWait k .status w e

theorem admission_busy_has_no_callback {s t} (run : Run s t)
    (safe : ResourceSafe s) (busy : t.exit = .admissionBusy) :
    t.calls = 0 ∧ t.effects = 0 :=
  (run_safe run safe).2.2.2.2 busy

def externallyBusy (x : Exit) : Bool :=
  x == .admissionBusy || x == .callbackBusy || x == .joinUnknown

-- The real public ApplicationError::Busy merges three origins. These witnesses
-- formally reject using that public error as a rollback/no-execution fact.
theorem callback_busy_after_effect_is_allowed (k m w) :
    Step ⟨k, m, .running, true, statusToken m, w, 1, 1, .none⟩
      ⟨k, m, .done, false, false, w, 1, 1, .callbackBusy⟩ :=
  .finish k m w 1 .callbackBusy (Or.inr (Or.inl rfl))

theorem join_unknown_after_effect_is_allowed (k) :
    Run (initial k .ordinary)
      ⟨k, .ordinary, .done, false, false, true, 1, 1, .joinUnknown⟩ := by
  have acquire : Run (initial k .ordinary)
      ⟨k, .ordinary, .acquired, true, false, true, 0, 0, .none⟩ :=
    .next (.refl _) (.ordinaryAcquire k true)
  have submitted := Run.next acquire (Step.submit k .ordinary true)
  have started := Run.next submitted (Step.start k .ordinary true)
  have effected := Run.next started (Step.effect k .ordinary true 0)
  exact Run.next effected
    (Step.finish k .ordinary true 1 .joinUnknown (Or.inr (Or.inr (Or.inr rfl))))

theorem busy_does_not_imply_no_effect :
    ¬ (∀ s : State, Run (initial ⟨0, 0, 1⟩ .ordinary) s →
      externallyBusy s.exit = true → s.effects = 0) := by
  intro claim
  let bad : State := ⟨⟨0, 0, 1⟩, .ordinary, .done, false, false, true, 1, 1, .joinUnknown⟩
  have reachable : Run (initial ⟨0, 0, 1⟩ .ordinary) bad :=
    join_unknown_after_effect_is_allowed ⟨0, 0, 1⟩
  have busy : externallyBusy bad.exit = true := by decide
  have impossible := claim bad reachable busy
  exact (by decide : (1 : Nat) ≠ 0) impossible

theorem early_release_breaks_same_resource_property (k w e) :
    ¬ ResourceSafe ⟨k, .status, .running, false, false, w, 1, e, .none⟩ := by
  simp [ResourceSafe]

end Task
end SandboxModel.Boundaries
