import SandboxModel.Foundations.Composition

namespace SandboxModel.Foundations.StreamHistory

/-! One direction's finite ghost history, not a model of the actual async
relay. Source/Read/Written are proof histories, not resident runtime buffers.
The provider must connect real ordered reads, confirmed prefix writes,
original binding, authorization, all queued storage and valid protocol-ending
evidence. HTTP status, socket EOF and idle time are not ending evidence here.
Zero accepted writes are allowed: these are finite safety results, not progress.
-/

structure Budgets where
  inflight : Nat
  cumulative : Option Nat

def cumulativeAllows (budgets : Budgets) (readBytes : Nat) : Prop :=
  match budgets.cumulative with
  | none => True
  | some maximum => readBytes ≤ maximum

inductive Fault where
  | readError | writeError | budgetExceeded | deadline
  deriving DecidableEq

inductive Phase where
  | open | completed | failed (reason : Fault) | cancelled | unknown
  deriving DecidableEq

structure State (Byte Evidence : Type) where
  read : List Byte
  written : List Byte
  pending : List Byte
  ending : Option Evidence
  phase : Phase

def initial {Byte Evidence : Type} : State Byte Evidence :=
  ⟨[], [], [], none, .open⟩

def appendRead {Byte Evidence : Type} (s : State Byte Evidence) (bytes : List Byte) :=
  { s with read := s.read ++ bytes, pending := s.pending ++ bytes }

def acceptWrite {Byte Evidence : Type} (s : State Byte Evidence) (count : Nat) :=
  { s with written := s.written ++ s.pending.take count, pending := s.pending.drop count }

def recordEnd {Byte Evidence : Type} (s : State Byte Evidence) (evidence : Evidence) :=
  { s with ending := some evidence }

def setPhase {Byte Evidence : Type} (s : State Byte Evidence) (phase : Phase) :=
  { s with phase := phase }

structure Safe {Byte Evidence : Type} (source : List Byte) (budgets : Budgets)
    (validEnd : Evidence → Prop) (s : State Byte Evidence) : Prop where
  decomposition : s.read = s.written ++ s.pending
  sourcePrefix : s.read <+: source
  inflight : s.pending.length ≤ budgets.inflight
  cumulative : cumulativeAllows budgets s.read.length
  ending : ∀ e, s.ending = some e → validEnd e ∧ s.read = source
  completed : s.phase = .completed → s.ending ≠ none ∧ s.pending = []

inductive Event (Byte Evidence : Type) where
  | readChunk (bytes : List Byte)
  | writeAccepted (count : Nat)
  | sourceEnd (evidence : Evidence)
  | finish
  | failure (reason : Fault)
  | cancel
  | outcomeUnknown

inductive Step {Byte Evidence : Type} (source : List Byte) (budgets : Budgets)
    (validEnd : Evidence → Prop) :
    State Byte Evidence → Event Byte Evidence → State Byte Evidence → Prop where
  | read {s bytes} (openPhase : s.phase = .open) (notEnded : s.ending = none)
      (fromSource : (s.read ++ bytes) <+: source)
      (inflight : (s.pending ++ bytes).length ≤ budgets.inflight)
      (cumulative : cumulativeAllows budgets (s.read ++ bytes).length) :
      Step source budgets validEnd s (.readChunk bytes) (appendRead s bytes)
  | write {s count} (openPhase : s.phase = .open) (bounded : count ≤ s.pending.length) :
      Step source budgets validEnd s (.writeAccepted count) (acceptWrite s count)
  | sourceEnd {s evidence} (openPhase : s.phase = .open) (notEnded : s.ending = none)
      (allRead : s.read = source) (valid : validEnd evidence) :
      Step source budgets validEnd s (.sourceEnd evidence) (recordEnd s evidence)
  | finish {s} (openPhase : s.phase = .open) (ended : s.ending ≠ none)
      (empty : s.pending = []) :
      Step source budgets validEnd s .finish (setPhase s .completed)
  | failure {s reason} (openPhase : s.phase = .open) :
      Step source budgets validEnd s (.failure reason) (setPhase s (.failed reason))
  | cancel {s} (openPhase : s.phase = .open) :
      Step source budgets validEnd s .cancel (setPhase s .cancelled)
  | outcomeUnknown {s} (openPhase : s.phase = .open) :
      Step source budgets validEnd s .outcomeUnknown (setPhase s .unknown)

def historyStep {Byte Evidence : Type} (source : List Byte) (budgets : Budgets)
    (validEnd : Evidence → Prop) (s : State Byte Evidence) (events : List (Event Byte Evidence))
    (next : State Byte Evidence) : Prop :=
  ∃ event, events = [event] ∧ Step source budgets validEnd s event next

abbrev Trace {Byte Evidence : Type} (source : List Byte) (budgets : Budgets)
    (validEnd : Evidence → Prop) := Execution (historyStep source budgets validEnd)

theorem cumulative_unlimited (maximum readBytes : Nat) :
    cumulativeAllows ⟨maximum, none⟩ readBytes := by trivial

theorem cumulative_bounded_iff (inflight maximum readBytes : Nat) :
    cumulativeAllows ⟨inflight, some maximum⟩ readBytes ↔ readBytes ≤ maximum := Iff.rfl

theorem cumulative_monotone (budgets : Budgets) (first last : Nat)
    (ordered : first ≤ last) (allowed : cumulativeAllows budgets last) :
    cumulativeAllows budgets first := by
  cases bound : budgets.cumulative <;> simp [cumulativeAllows, bound] at allowed ⊢
  omega

theorem initial_safe {Byte Evidence : Type} (source : List Byte) (budgets : Budgets)
    (validEnd : Evidence → Prop) : Safe source budgets validEnd initial := by
  refine ⟨rfl, ?_, Nat.zero_le _, ?_, ?_, ?_⟩
  · exact ⟨source, rfl⟩
  · cases bound : budgets.cumulative <;> simp [cumulativeAllows, bound, initial]
  · intro e equal; simp [initial] at equal
  · intro equal; simp [initial] at equal

theorem read_decomposition {Byte Evidence : Type} (s : State Byte Evidence) (bytes : List Byte)
    (decomposition : s.read = s.written ++ s.pending) :
    (appendRead s bytes).read = (appendRead s bytes).written ++ (appendRead s bytes).pending := by
  simp only [appendRead, decomposition, List.append_assoc]

theorem confirmed_write_exact {Byte Evidence : Type} (s : State Byte Evidence) (count : Nat) :
    (acceptWrite s count).written = s.written ++ s.pending.take count ∧
    (acceptWrite s count).pending = s.pending.drop count ∧
    s.pending.take count ++ s.pending.drop count = s.pending := by
  exact ⟨rfl, rfl, List.take_append_drop _ _⟩

theorem write_decomposition {Byte Evidence : Type} (s : State Byte Evidence) (count : Nat)
    (decomposition : s.read = s.written ++ s.pending) :
    (acceptWrite s count).read = (acceptWrite s count).written ++ (acceptWrite s count).pending := by
  simp only [acceptWrite, decomposition, List.append_assoc, List.take_append_drop]

private theorem phase_safe {Byte Evidence : Type} {source : List Byte} {budgets : Budgets}
    {validEnd : Evidence → Prop} {s : State Byte Evidence} (safe : Safe source budgets validEnd s)
    (phase : Phase) (notComplete : phase ≠ .completed) :
    Safe source budgets validEnd (setPhase s phase) := by
  refine ⟨safe.decomposition, safe.sourcePrefix, safe.inflight, safe.cumulative, safe.ending, ?_⟩
  intro completed
  exact False.elim (notComplete completed)

theorem step_starts_open {Byte Evidence : Type} {source : List Byte} {budgets : Budgets}
    {validEnd : Evidence → Prop} {s next : State Byte Evidence} {event : Event Byte Evidence}
    (move : Step source budgets validEnd s event next) : s.phase = .open := by
  cases move <;> assumption

theorem step_preserves {Byte Evidence : Type} {source : List Byte} {budgets : Budgets}
    {validEnd : Evidence → Prop} {s next : State Byte Evidence} {event : Event Byte Evidence}
    (safe : Safe source budgets validEnd s) (move : Step source budgets validEnd s event next) :
    Safe source budgets validEnd next := by
  cases move with
  | read openPhase notEnded fromSource inflight cumulative =>
    refine ⟨read_decomposition _ _ safe.decomposition, fromSource, inflight, cumulative, ?_, ?_⟩
    · intro e equal; simp [appendRead, notEnded] at equal
    · intro equal; simp [appendRead, openPhase] at equal
  | write openPhase bounded =>
    refine ⟨write_decomposition _ _ safe.decomposition, safe.sourcePrefix, ?_, safe.cumulative,
      safe.ending, ?_⟩
    · simp only [acceptWrite, List.length_drop]
      have pendingBound := safe.inflight
      omega
    · intro equal; simp [acceptWrite, openPhase] at equal
  | sourceEnd openPhase notEnded allRead valid =>
    refine ⟨safe.decomposition, safe.sourcePrefix, safe.inflight, safe.cumulative, ?_, ?_⟩
    · intro e equal
      simp only [recordEnd, Option.some.injEq] at equal
      cases equal
      exact ⟨valid, allRead⟩
    · intro equal; simp [recordEnd, openPhase] at equal
  | finish openPhase ended empty =>
    exact ⟨safe.decomposition, safe.sourcePrefix, safe.inflight, safe.cumulative, safe.ending,
      fun _ => ⟨ended, empty⟩⟩
  | failure openPhase => exact phase_safe safe _ (by simp)
  | cancel openPhase => exact phase_safe safe _ (by simp)
  | outcomeUnknown openPhase => exact phase_safe safe _ (by simp)

theorem trace_preserves {Byte Evidence : Type} {source : List Byte} {budgets : Budgets}
    {validEnd : Evidence → Prop} {first last : State Byte Evidence}
    {events : List (Event Byte Evidence)} (trace : Trace source budgets validEnd first events last) :
    Safe source budgets validEnd first → Safe source budgets validEnd last := by
  induction trace with
  | refl => exact id
  | cons move tail ih =>
    intro safe
    obtain ⟨event, _, single⟩ := move
    exact ih (step_preserves safe single)

theorem written_prefix_read_and_source {Byte Evidence : Type} {source : List Byte} {budgets : Budgets}
    {validEnd : Evidence → Prop} {s : State Byte Evidence} (safe : Safe source budgets validEnd s) :
    s.written <+: s.read ∧ s.written <+: source := by
  have first : s.written <+: s.read := ⟨s.pending, safe.decomposition.symm⟩
  exact ⟨first, first.trans safe.sourcePrefix⟩

theorem history_counts_and_residency {Byte Evidence : Type} {source : List Byte} {budgets : Budgets}
    {validEnd : Evidence → Prop} {s : State Byte Evidence} (safe : Safe source budgets validEnd s) :
    s.read.length = s.written.length + s.pending.length ∧
    s.written.length ≤ s.read.length ∧ s.pending.length ≤ budgets.inflight := by
  have lengths := congrArg List.length safe.decomposition
  simp only [List.length_append] at lengths
  exact ⟨lengths, by omega, safe.inflight⟩

theorem completed_exact {Byte Evidence : Type} {source : List Byte} {budgets : Budgets}
    {validEnd : Evidence → Prop} {s : State Byte Evidence} (safe : Safe source budgets validEnd s)
    (completed : s.phase = .completed) :
    ∃ evidence, s.ending = some evidence ∧ validEnd evidence ∧
      s.pending = [] ∧ s.written = s.read ∧ s.read = source := by
  obtain ⟨ended, empty⟩ := safe.completed completed
  cases ending : s.ending with
  | none => exact False.elim (ended ending)
  | some evidence =>
    obtain ⟨valid, allRead⟩ := safe.ending evidence ending
    have allWritten : s.written = s.read := by simpa [empty] using safe.decomposition.symm
    exact ⟨evidence, rfl, valid, empty, allWritten, allRead⟩

theorem pending_not_completed {Byte Evidence : Type} {source : List Byte} {budgets : Budgets}
    {validEnd : Evidence → Prop} {s : State Byte Evidence} (safe : Safe source budgets validEnd s)
    (pending : s.pending ≠ []) : s.phase ≠ .completed := by
  intro completed
  exact pending (safe.completed completed).2

theorem source_end_preserves_open {Byte Evidence : Type} (s : State Byte Evidence) (e : Evidence)
    (openPhase : s.phase = .open) : (recordEnd s e).phase = .open := openPhase

theorem abort_never_completed {Byte Evidence : Type} (s : State Byte Evidence) (reason : Fault) :
    (setPhase s (.failed reason)).phase ≠ .completed ∧
    (setPhase s .cancelled).phase ≠ .completed ∧
    (setPhase s .unknown).phase ≠ .completed := by simp [setPhase]

theorem terminal_trace_unchanged {Byte Evidence : Type} {source : List Byte} {budgets : Budgets}
    {validEnd : Evidence → Prop} {first last : State Byte Evidence}
    {events : List (Event Byte Evidence)} (terminal : first.phase ≠ .open)
    (trace : Trace source budgets validEnd first events last) : last = first ∧ events = [] := by
  cases trace with
  | refl => exact ⟨rfl, rfl⟩
  | cons move tail =>
    obtain ⟨event, _, single⟩ := move
    exact False.elim (terminal (step_starts_open single))

theorem empty_complete_realizable {Byte Evidence : Type} (budgets : Budgets)
    (validEnd : Evidence → Prop) (evidence : Evidence) (valid : validEnd evidence) :
    ∃ ending last : State Byte Evidence,
      Step [] budgets validEnd initial (.sourceEnd evidence) ending ∧
      Step [] budgets validEnd ending .finish last ∧ last.phase = .completed := by
  refine ⟨recordEnd initial evidence, setPhase (recordEnd initial evidence) .completed, ?_, ?_, rfl⟩
  · exact Step.sourceEnd rfl rfl rfl valid
  · exact Step.finish rfl (by simp [recordEnd]) rfl

private def drainedAt {Byte Evidence : Type} (bytes : List Byte) : State Byte Evidence :=
  ⟨bytes, bytes, [], none, .open⟩

private theorem read_write_segment {Byte Evidence : Type} (budgets : Budgets)
    (validEnd : Evidence → Prop) (priorBytes suffix : List Byte)
    (available : 1 ≤ budgets.inflight)
    (total : cumulativeAllows budgets (priorBytes ++ suffix).length) :
    ∃ events, Trace (priorBytes ++ suffix) budgets validEnd
      (drainedAt priorBytes) events (drainedAt (priorBytes ++ suffix)) := by
  induction suffix generalizing priorBytes with
  | nil => exact ⟨[], by simpa using Execution.refl (drainedAt priorBytes)⟩
  | cons byte rest ih =>
    let before : State Byte Evidence := drainedAt priorBytes
    let reading := appendRead before [byte]
    let writing := acceptWrite reading 1
    have nextSource : (priorBytes ++ [byte]) <+: (priorBytes ++ byte :: rest) := by
      exact ⟨rest, by simp [List.append_assoc]⟩
    have totalNext : cumulativeAllows budgets (priorBytes ++ [byte]).length :=
      cumulative_monotone budgets _ _ (by simp) total
    have readStep : Step (priorBytes ++ byte :: rest) budgets validEnd before
        (.readChunk [byte]) reading := by
      exact Step.read rfl rfl nextSource (by simpa [before, drainedAt] using available) totalNext
    have writeStep : Step (priorBytes ++ byte :: rest) budgets validEnd reading
        (.writeAccepted 1) writing := by
      exact Step.write rfl (by simp [reading, before, appendRead, drainedAt])
    have writingDrained : writing = drainedAt (priorBytes ++ [byte]) := rfl
    have totalTail : cumulativeAllows budgets ((priorBytes ++ [byte]) ++ rest).length := by
      simpa [List.append_assoc] using total
    obtain ⟨events, remaining⟩ := ih (priorBytes ++ [byte]) totalTail
    have remainingHere : Trace (priorBytes ++ byte :: rest) budgets validEnd writing events
        (drainedAt (priorBytes ++ byte :: rest)) := by
      simpa [writingDrained, List.append_assoc] using remaining
    refine ⟨.readChunk [byte] :: .writeAccepted 1 :: events, ?_⟩
    simpa using Execution.cons
      (show historyStep _ _ _ before [.readChunk [byte]] reading from ⟨_, rfl, readStep⟩)
      (Execution.cons
        (show historyStep _ _ _ reading [.writeAccepted 1] writing from ⟨_, rfl, writeStep⟩)
        remainingHere)

theorem bounded_resident_completion_realizable {Byte Evidence : Type}
    (source : List Byte) (budgets : Budgets) (validEnd : Evidence → Prop)
    (evidence : Evidence) (valid : validEnd evidence) (available : 1 ≤ budgets.inflight)
    (total : cumulativeAllows budgets source.length) :
    ∃ events last, Trace source budgets validEnd (initial : State Byte Evidence) events last ∧
      Safe source budgets validEnd last ∧ last.phase = .completed ∧ last.written = source := by
  obtain ⟨events, forwarding⟩ := read_write_segment budgets validEnd [] source available (by simpa using total)
  have forwarded : Trace source budgets validEnd initial events (drainedAt source) := by
    simpa [initial, drainedAt] using forwarding
  let ending := recordEnd (drainedAt source) evidence
  let last := setPhase ending .completed
  have endStep : Step source budgets validEnd (drainedAt source) (.sourceEnd evidence) ending :=
    Step.sourceEnd rfl rfl rfl valid
  have finishStep : Step source budgets validEnd ending .finish last :=
    Step.finish rfl (by simp [ending, recordEnd]) rfl
  have completion : Trace source budgets validEnd (drainedAt source)
      [.sourceEnd evidence, .finish] last := by
    simpa using Execution.cons
      (show historyStep _ _ _ (drainedAt source) [.sourceEnd evidence] ending from ⟨_, rfl, endStep⟩)
      (Execution.cons
        (show historyStep _ _ _ ending [.finish] last from ⟨_, rfl, finishStep⟩)
        (Execution.refl last))
  have entire := execution_append forwarded completion
  exact ⟨_, last, entire, trace_preserves entire (initial_safe source budgets validEnd), rfl, rfl⟩

end SandboxModel.Foundations.StreamHistory
