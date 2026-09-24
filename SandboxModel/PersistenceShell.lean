import SandboxModel.Persistence

/-!
The verified reducer is pure.  A deployment shell may persist its decision,
dispatch durable outbox entries, or lose its volatile process state, but it is
not allowed to edit protocol fields independently.

`ShellStep` is the narrow formal contract for that shell:

* a controller decision replaces the complete `JournalState` in one step;
* an adapter dispatch is enabled only by an already committed outbox entry;
* dispatch and process failure cannot mutate the journal; and
* loading always reconstructs exactly the committed journal plus the current
  volatile availability bit.

Database atomicity and adapter implementation correctness remain outside the
controller proof boundary.  A concrete shell satisfies this boundary only if
its transaction and dispatch APIs refine these constructors.
-/

namespace SandboxModel

/-- Observable state at the controller/I/O boundary.  `delivered` is transport
    history, not controller protocol state; it may be lost or duplicated
    without changing the durable journal. -/
structure ShellState where
  persisted : JournalState
  runtime : RuntimeState
  delivered : EffectId → Prop

def ShellState.load (state : ShellState) : World :=
  World.fromParts state.persisted state.runtime

@[simp] theorem ShellState.load_journal (state : ShellState) :
    state.load.journal = state.persisted := by
  simp [ShellState.load]

@[simp] theorem ShellState.load_runtime (state : ShellState) :
    state.load.runtime = state.runtime := by
  simp [ShellState.load]

/-- Transport history is monotone for the purpose of the shell contract.
    Duplicate delivery simply proves the same proposition again. -/
def recordDelivery (delivered : EffectId → Prop) (effect : EffectId) :
    EffectId → Prop :=
  fun candidate => candidate = effect ∨ delivered candidate

inductive ShellAction where
  | commit (event : Event)
  | dispatch (effect : EffectId)
  | processCrash
  | processRecover
  deriving Repr

/-- Atomic deployment-shell semantics.  The `commit` constructor is the only
    constructor permitted to replace durable protocol state, and it accepts
    only the complete successor of a legal model step. -/
inductive ShellStep : ShellState → ShellAction → ShellState → Prop where
  | commit
      (legal : Step before.load event nextWorld) :
      ShellStep before (.commit event)
        { persisted := nextWorld.journal
          runtime := nextWorld.runtime
          delivered := before.delivered }
  | dispatch
      (committed : before.persisted.outbox effect) :
      ShellStep before (.dispatch effect)
        { before with delivered := recordDelivery before.delivered effect }
  | processCrash :
      ShellStep before .processCrash
        { before with runtime := ⟨false⟩ }
  | processRecover :
      ShellStep before .processRecover
        { before with runtime := ⟨true⟩ }

/-- A committed reducer decision exposes the whole successor journal and its
    runtime projection; there is no shell state containing a half decision. -/
theorem ShellStep.commit_exact
    (step : ShellStep before (.commit event) after) :
    ∃ nextWorld, Step before.load event nextWorld ∧
      after.persisted = nextWorld.journal ∧
      after.runtime = nextWorld.runtime ∧
      after.delivered = before.delivered := by
  cases step with
  | commit legal => exact ⟨_, legal, rfl, rfl, rfl⟩

/-- An adapter call can only name an effect which was present in the durable
    outbox before that call. -/
theorem ShellStep.dispatch_requires_committed_outbox
    (step : ShellStep before (.dispatch effect) after) :
    before.persisted.outbox effect := by
  cases step with
  | dispatch committed => exact committed

/-- Dispatch changes transport history only. -/
theorem ShellStep.dispatch_preserves_journal
    (step : ShellStep before (.dispatch effect) after) :
    after.persisted = before.persisted ∧
      after.runtime = before.runtime := by
  cases step
  exact ⟨rfl, rfl⟩

/-- Process failure loses availability, never accepted durable state. -/
theorem ShellStep.processCrash_preserves_journal
    (step : ShellStep before .processCrash after) :
    after.persisted = before.persisted ∧
      after.runtime = ⟨false⟩ := by
  cases step
  exact ⟨rfl, rfl⟩

/-- Restart loads the same journal and changes only availability. -/
theorem ShellStep.processRecover_preserves_journal
    (step : ShellStep before .processRecover after) :
    after.persisted = before.persisted ∧
      after.runtime = ⟨true⟩ := by
  cases step
  exact ⟨rfl, rfl⟩

/-- Every non-commit shell action leaves the durable controller journal
    unchanged. -/
theorem ShellStep.nonCommit_preserves_journal
    (notCommit : ∀ event, action ≠ .commit event)
    (step : ShellStep before action after) :
    after.persisted = before.persisted := by
  cases step with
  | commit legal => exact (notCommit _ rfl).elim
  | dispatch committed => rfl
  | processCrash => rfl
  | processRecover => rfl

/-- Once dispatched, an effect remains in the shell's delivery history.
    Repeated calls are harmless at this layer; protocol idempotence is proved
    separately by the effect receipt handlers. -/
theorem ShellStep.dispatch_records_delivery
    (step : ShellStep before (.dispatch effect) after) :
    after.delivered effect := by
  cases step
  exact Or.inl rfl

/-- A previously recorded delivery is never removed by any shell action. -/
theorem ShellStep.delivery_monotone
    (step : ShellStep before action after)
    (delivered : before.delivered effect) :
    after.delivered effect := by
  cases step with
  | commit legal => exact delivered
  | dispatch committed => exact Or.inr delivered
  | processCrash => exact delivered
  | processRecover => exact delivered

/-- Instantiating an atomic shell commit with an accepted create step also
    commits its operation ledger and initial fence outbox entry. -/
theorem committedCreate_contains_operation_and_fence
    (freshEffect : before.load.effects
      (effectId (.installFence sid 1 false)) = none)
    (step : ShellStep before
      (.commit (.acceptCreate actor owner sid workspace filePolicy policy
        operationId)) after) :
    after.persisted.operations (operationKey sid operationId) = some
        ⟨sid, ⟨operationId, 1, .create, .pending⟩,
          .create workspace policy filePolicy, none⟩ ∧
      after.persisted.effects (effectId (.installFence sid 1 false)) =
        some .committed ∧
      after.persisted.outbox (effectId (.installFence sid 1 false)) := by
  rcases step.commit_exact with
    ⟨nextWorld, legal, persisted, _, _⟩
  rw [persisted]
  exact acceptedCreate_journals_initial_fence freshEffect legal

end SandboxModel
