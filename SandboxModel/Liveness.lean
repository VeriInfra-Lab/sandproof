import SandboxModel.Step

/-!
A liveness foundation for the asynchronous protocol.

Safety and liveness need different assumptions.  `Step` says which changes are
legal; it does not force the controller, a worker, or storage to take a legal
step.  Traces in this file therefore allow arbitrary idle transitions.  A
liveness result is obtained only after the caller supplies the relevant
progress or fairness premises.

The old phase-level existential result remains at the end of the file.  It is a
useful lifecycle sanity check, but deliberately is not presented as an
eventual-completion theorem.
-/

namespace SandboxModel.Liveness

/-! ### Infinite executions and temporal predicates -/

/-- One clock tick either performs a protocol step or leaves the world alone.
    Admitting `idle` makes scheduler starvation representable instead of
    silently ruling it out in the definition of a trace. -/
inductive TraceEdge : World → Option Event → World → Prop where
  | idle (world : World) : TraceEdge world none world
  | protocol {before after : World} {event : Event} :
      Step before event after → TraceEdge before (some event) after

/-- A complete, infinite execution. -/
structure Trace where
  state : Nat → World
  action : Nat → Option Event
  edge : ∀ n, TraceEdge (state n) (action n) (state (n + 1))

/-- `predicate` holds at some state no earlier than `start`. -/
def EventuallyFrom (trace : Trace) (start : Nat)
    (predicate : World → Prop) : Prop :=
  ∃ n, start ≤ n ∧ predicate (trace.state n)

/-- `predicate` holds at every state no earlier than `start`. -/
def AlwaysFrom (trace : Trace) (start : Nat)
    (predicate : World → Prop) : Prop :=
  ∀ n, start ≤ n → predicate (trace.state n)

def Eventually (trace : Trace) (predicate : World → Prop) : Prop :=
  EventuallyFrom trace 0 predicate

def Always (trace : Trace) (predicate : World → Prop) : Prop :=
  AlwaysFrom trace 0 predicate

/-- After some point, `predicate` remains true forever. -/
def EventuallyAlwaysFrom (trace : Trace) (start : Nat)
    (predicate : World → Prop) : Prop :=
  ∃ n, start ≤ n ∧ AlwaysFrom trace n predicate

theorem eventuallyFrom_mono_start
    (reached : EventuallyFrom trace later predicate)
    (ordered : earlier ≤ later) :
    EventuallyFrom trace earlier predicate := by
  rcases reached with ⟨n, laterLe, holds⟩
  exact ⟨n, Nat.le_trans ordered laterLe, holds⟩

/-! ### Operation completion predicates -/

/-- The value stored at an operation key really names that key.  Reachability
    invariants establish this for controller-created ledger entries; retaining
    it here prevents malformed arbitrary worlds from counting as completion. -/
def LedgerAt (world : World) (key : OperationKey)
    (ledger : OperationLedgerRecord) : Prop :=
  world.operations key = some ledger ∧
    ledger.sandbox = key.sandbox ∧
    ledger.operation.id = key.operation

/-- An operation is finalized exactly when its durable ledger entry contains a
    result.  Both success and explicit failure are final outcomes. -/
def Finalized (key : OperationKey) (world : World) : Prop :=
  ∃ ledger result, LedgerAt world key ledger ∧ ledger.result = some result

/-- The shape of a successful result required by each accepted request.  This
    predicate intentionally rejects `OperationResult.failed`.  Resource and
    generation identity remain safety obligations of the operation plan. -/
def ResultMatchesIntent : OperationInput → OperationResult → Prop
  | .create _ _ _, .running _ => True
  | .pause _, .paused _ => True
  | .resume _, .running _ => True
  | .kill _, .destroyed => True
  | _, _ => False

/-- The operation reached its requested stable state, rather than merely
    terminating with a failure. -/
def TargetSucceeded (key : OperationKey) (world : World) : Prop :=
  ∃ ledger result,
    LedgerAt world key ledger ∧
    ledger.operation.status = .succeeded ∧
    ledger.result = some result ∧
    ResultMatchesIntent ledger.input result

theorem targetSucceeded_finalized
    (succeeded : TargetSucceeded key world) : Finalized key world := by
  rcases succeeded with ⟨ledger, result, atKey, _, stored, _⟩
  exact ⟨ledger, result, atKey, stored⟩

/-! ### Observable steps and weak fairness -/

/-- A particular protocol event, as opposed to an idle tick, occurs at `n`. -/
def EventAt (trace : Trace) (n : Nat) (event : Event) : Prop :=
  trace.action n = some event ∧
    Step (trace.state n) event (trace.state (n + 1))

/-- `work` is served by the protocol event performed at tick `n`.  The caller
    chooses the work type and the event-to-work classification. -/
def ServedAt {Work : Type} (trace : Trace)
    (serves : Work → Event → Prop) (work : Work) (n : Nat) : Prop :=
  ∃ event, EventAt trace n event ∧ serves work event

/-- Weak fairness excludes permanent starvation: if one work item is enabled
    at every state of a suffix, some event serving that item eventually occurs.
    Quantification over every suffix means a still-enabled item must be served
    again; fairness says nothing about actions that cease to be enabled.

    This definition does *not* claim that an adapter will return success.  Such
    availability belongs in `enabled`, while the effect of service belongs in
    a separate progress premise. -/
def WeaklyFair {Work : Type} (trace : Trace)
    (enabled : Work → World → Prop)
    (serves : Work → Event → Prop) : Prop :=
  ∀ work start,
    AlwaysFrom trace start (enabled work) →
      ∃ n, start ≤ n ∧ ServedAt trace serves work n

theorem WeaklyFair.eventually_serves
    (fair : WeaklyFair trace enabled serves)
    (continuouslyEnabled : AlwaysFrom trace start (enabled work)) :
    ∃ n, start ≤ n ∧ ServedAt trace serves work n :=
  fair work start continuouslyEnabled

/-! ### Explicit progress assumptions -/

/-- Every unfinished state eventually reaches a strictly smaller natural-number
    rank.  This is an assumption to be discharged for the concrete controller;
    it is not baked into `Trace` or `Step`. -/
def StrictRankProgress (trace : Trace) (start : Nat)
    (finished : World → Prop) (rank : World → Nat) : Prop :=
  ∀ n, start ≤ n → ¬ finished (trace.state n) →
    ∃ m, n < m ∧ rank (trace.state m) < rank (trace.state n)

/-- A finite natural-number rank plus explicit strict progress rules out an
    infinite unfinished execution.  Notice that the conclusion is not present
    in the trace definition: all progress is exposed by `progress`. -/
theorem eventually_of_strictRankProgress
    (progress : StrictRankProgress trace start finished rank) :
    EventuallyFrom trace start finished := by
  have bounded : ∀ bound n, start ≤ n →
      rank (trace.state n) ≤ bound → EventuallyFrom trace n finished := by
    intro bound
    induction bound with
    | zero =>
        intro n startLe rankLe
        by_cases done : finished (trace.state n)
        · exact ⟨n, Nat.le_refl n, done⟩
        · rcases progress n startLe done with ⟨m, _, decreases⟩
          have rankZero : rank (trace.state n) = 0 :=
            Nat.eq_zero_of_le_zero rankLe
          rw [rankZero] at decreases
          exact (Nat.not_lt_zero _ decreases).elim
    | succ bound inductionHypothesis =>
        intro n startLe rankLe
        by_cases done : finished (trace.state n)
        · exact ⟨n, Nat.le_refl n, done⟩
        · rcases progress n startLe done with ⟨m, later, decreases⟩
          have belowSuccessor : rank (trace.state m) < bound + 1 :=
            Nat.lt_of_lt_of_le decreases rankLe
          have belowBound : rank (trace.state m) ≤ bound :=
            Nat.le_of_lt_succ (by simpa [Nat.succ_eq_add_one] using belowSuccessor)
          rcases inductionHypothesis m
              (Nat.le_trans startLe (Nat.le_of_lt later)) belowBound with
            ⟨reached, mLe, holds⟩
          exact ⟨reached, Nat.le_trans (Nat.le_of_lt later) mLe, holds⟩
  exact bounded (rank (trace.state start)) start (Nat.le_refl start)
    (Nat.le_refl _)

/-- Operation-level specialization: the caller must show that every unfinished
    ledger state eventually lowers the supplied rank. -/
theorem operation_eventually_finalized_of_rank
    (progress : StrictRankProgress trace start (Finalized key) rank) :
    EventuallyFrom trace start (Finalized key) :=
  eventually_of_strictRankProgress progress

/-- Successful convergence is deliberately a separate theorem from terminal
    convergence.  A system may be live in the first sense while correctly
    reporting a terminal failure instead of reaching the requested target. -/
theorem operation_eventually_targetSucceeded_of_rank
    (progress : StrictRankProgress trace start (TargetSucceeded key) rank) :
    EventuallyFrom trace start (TargetSucceeded key) :=
  eventually_of_strictRankProgress progress

/-- Fair scheduling yields completion when every unfinished state exposes a
    work item which (1) stays enabled while the goal remains unfinished and
    (2) strictly decreases the rank when served.  These two concrete protocol
    obligations are explicit; weak fairness alone is not treated as proof that
    workers or storage succeed. -/
theorem eventually_of_weakFair_rank
    {Work : Type}
    {trace : Trace}
    {enabled : Work → World → Prop}
    {serves : Work → Event → Prop}
    {start : Nat}
    {finished : World → Prop}
    (rank : World → Nat)
    (fair : WeaklyFair trace enabled serves)
    (obligation : ∀ n, start ≤ n → ¬ finished (trace.state n) →
      ∃ work,
        (∀ m, n ≤ m → ¬ finished (trace.state m) →
          enabled work (trace.state m)) ∧
        (∀ m, n ≤ m → ¬ finished (trace.state m) →
          ServedAt trace serves work m →
          rank (trace.state (m + 1)) < rank (trace.state n))) :
    EventuallyFrom trace start finished := by
  classical
  have bounded : ∀ bound n, start ≤ n →
      rank (trace.state n) ≤ bound → EventuallyFrom trace n finished := by
    intro bound
    induction bound with
    | zero =>
        intro n startLe rankLe
        by_cases done : finished (trace.state n)
        · exact ⟨n, Nat.le_refl n, done⟩
        · by_cases reached : EventuallyFrom trace n finished
          · exact reached
          · rcases obligation n startLe done with
              ⟨work, staysEnabled, decreases⟩
            have neverFinished : ∀ m, n ≤ m →
                ¬ finished (trace.state m) := by
              intro m nLe finishedAtM
              exact reached ⟨m, nLe, finishedAtM⟩
            have continuouslyEnabled :
                AlwaysFrom trace n (enabled work) := by
              intro m nLe
              exact staysEnabled m nLe (neverFinished m nLe)
            rcases fair work n continuouslyEnabled with
              ⟨m, nLe, served⟩
            have rankZero : rank (trace.state n) = 0 :=
              Nat.eq_zero_of_le_zero rankLe
            have impossible : rank (trace.state (m + 1)) < 0 := by
              simpa [rankZero] using
                decreases m nLe (neverFinished m nLe) served
            exact (Nat.not_lt_zero _ impossible).elim
    | succ bound inductionHypothesis =>
        intro n startLe rankLe
        by_cases done : finished (trace.state n)
        · exact ⟨n, Nat.le_refl n, done⟩
        · by_cases reached : EventuallyFrom trace n finished
          · exact reached
          · rcases obligation n startLe done with
              ⟨work, staysEnabled, decreases⟩
            have neverFinished : ∀ m, n ≤ m →
                ¬ finished (trace.state m) := by
              intro m nLe finishedAtM
              exact reached ⟨m, nLe, finishedAtM⟩
            have continuouslyEnabled :
                AlwaysFrom trace n (enabled work) := by
              intro m nLe
              exact staysEnabled m nLe (neverFinished m nLe)
            rcases fair work n continuouslyEnabled with
              ⟨m, nLe, served⟩
            have decreasesFromN :
                rank (trace.state (m + 1)) < rank (trace.state n) :=
              decreases m nLe (neverFinished m nLe) served
            have belowSuccessor : rank (trace.state (m + 1)) < bound + 1 :=
              Nat.lt_of_lt_of_le decreasesFromN rankLe
            have belowBound : rank (trace.state (m + 1)) ≤ bound :=
              Nat.le_of_lt_succ
                (by simpa [Nat.succ_eq_add_one] using belowSuccessor)
            have startLeNext : start ≤ m + 1 :=
              Nat.le_trans startLe
                (Nat.le_trans nLe (Nat.le_succ m))
            rcases inductionHypothesis (m + 1) startLeNext belowBound with
              ⟨reachedAt, nextLe, holds⟩
            exact ⟨reachedAt,
              Nat.le_trans (Nat.le_trans nLe (Nat.le_succ m)) nextLe,
              holds⟩
  exact bounded (rank (trace.state start)) start (Nat.le_refl start)
    (Nat.le_refl _)

/-! ### Phase-level existential sanity check -/

/-- Phase-level transition skeleton, mirroring the lifecycle edges of `Step`. -/
def CanStep : Phase → Phase → Prop
  | .starting, .running => True
  | .starting, .revoking => True
  | .running, .pausing => True
  | .running, .revoking => True
  | .pausing, .paused => True
  | .pausing, .revoking => True
  | .paused, .starting => True
  | .paused, .revoking => True
  | .revoking, .destroyed => True
  | _, _ => False

/-- Reflexive-transitive closure of `CanStep`, defined locally so the module
    does not depend on a library. -/
inductive Path : Phase → Phase → Prop where
  | refl (phase : Phase) : Path phase phase
  | step {a b c : Phase} : CanStep a b → Path b c → Path a c

theorem step_starting_revoking : CanStep .starting .revoking := by simp [CanStep]
theorem step_running_revoking : CanStep .running .revoking := by simp [CanStep]
theorem step_pausing_revoking : CanStep .pausing .revoking := by simp [CanStep]
theorem step_paused_revoking : CanStep .paused .revoking := by simp [CanStep]
theorem step_revoking_destroyed : CanStep .revoking .destroyed := by simp [CanStep]

/-- From every phase, `destroyed` is reachable by finitely many allowed steps. -/
theorem can_reach_destroyed (phase : Phase) :
    ∃ final : Phase, Path phase final ∧ final = Phase.destroyed := by
  cases phase with
  | destroyed => exact ⟨Phase.destroyed, Path.refl _, rfl⟩
  | revoking =>
      exact ⟨Phase.destroyed, Path.step step_revoking_destroyed (Path.refl _), rfl⟩
  | starting =>
      exact ⟨Phase.destroyed,
        Path.step step_starting_revoking
          (Path.step step_revoking_destroyed (Path.refl _)), rfl⟩
  | running =>
      exact ⟨Phase.destroyed,
        Path.step step_running_revoking
          (Path.step step_revoking_destroyed (Path.refl _)), rfl⟩
  | pausing =>
      exact ⟨Phase.destroyed,
        Path.step step_pausing_revoking
          (Path.step step_revoking_destroyed (Path.refl _)), rfl⟩
  | paused =>
      exact ⟨Phase.destroyed,
        Path.step step_paused_revoking
          (Path.step step_revoking_destroyed (Path.refl _)), rfl⟩

end SandboxModel.Liveness
