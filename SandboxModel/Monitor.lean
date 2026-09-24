import SandboxModel.Proofs

/-!
An executable trace monitor derived from the proven protocol, with a soundness
theorem.

Observations are `SandboxView` projections of the abstract `World`. A
`StepTrace` is an initial world with its observed view followed by a legal step
at each subsequent element, each carrying its own observed view. The monitor
rejects a trace that observes a state no reachable history can produce;
`trace_monitor_sound` shows that under the `StepTrace` hypotheses the monitor
always accepts, so a rejection means the observed history is impossible.
-/

namespace SandboxModel.Monitor

/-- Observable projection of a sandbox record. Fields E2B does not expose
    through the API are deliberately absent. -/
structure SandboxView where
  phase : Phase
  terminal : Bool
  generation : Generation
  deriving DecidableEq, Repr

def view (world : World) (sid : SandboxId) : Option SandboxView :=
  match world.sandboxes sid with
  | none => none
  | some record => some ⟨record.phase, record.terminal, record.generation⟩

/-- The observation sequence of a trace: the initial view followed by the view
    carried by each step. -/
def views (v : SandboxView) (steps : List (Event × World × SandboxView)) :
    List (Option SandboxView) :=
  some v :: steps.map (fun s => some s.2.2)

/-- Executable monitor. `seen` records whether a terminal observation already
    happened: after that, every later observation must still be terminal and
    must not be `running`. Unobservable entries (`none`) are skipped. -/
def monitorFrom : Bool → List (Option SandboxView) → Bool
  | false, [] => true
  | true, [] => true
  | false, none :: rest => monitorFrom false rest
  | true, none :: rest => monitorFrom true rest
  | false, some v :: rest => monitorFrom v.terminal rest
  | true, some v :: rest =>
      decide (v.terminal = true) && decide (v.phase ≠ .running) &&
        monitorFrom true rest

def monitor (vs : List (Option SandboxView)) : Bool := monitorFrom false vs

/-! ### Two core facts about the model -/

/-- A reachable sandbox with `terminal = true` is never `running`. -/
theorem reachable_terminal_not_running {world : World} {sid : SandboxId}
    {v : SandboxView} (reachable : Reachable world)
    (hv : view world sid = some v) (terminal : v.terminal = true) :
    v.phase ≠ .running := by
  have consistent := reachable_terminal_consistent reachable
  unfold view at hv
  split at hv
  · simp at hv
  · rename_i record hrecord
    injection hv with hview
    subst hview
    have hterm : record.terminal = true := by simpa using terminal
    have hcon := consistent sid record hrecord
    rw [hterm] at hcon
    simp only [true_iff] at hcon
    intro hphase
    have hpr : record.phase = Phase.running := by simpa using hphase
    rw [hpr] at hcon
    simp at hcon

/-- One legal step preserves a `terminal = true` observation. -/
theorem step_terminal_monotone {world next : World} {event : Event} {sid : SandboxId}
    (step : Step world event next) {v : SandboxView}
    (hv : view world sid = some v) (terminal : v.terminal = true)
    {v' : SandboxView} (hv' : view next sid = some v') : v'.terminal = true := by
  unfold view at hv hv'
  split at hv
  · simp at hv
  · rename_i record hrecord
    split at hv'
    · simp at hv'
    · rename_i record' hrecord'
      injection hv with hview
      injection hv' with hview'
      subst hview
      subst hview'
      exact step_never_clears_terminal step sid record record' hrecord
        (by simpa using terminal) hrecord'

/-! ### Traces -/

/-- A world with its observed view, followed by a legal step at each element. -/
inductive StepTrace (sid : SandboxId) : World → SandboxView →
    List (Event × World × SandboxView) → Prop where
  | nil {w v} : view w sid = some v → StepTrace sid w v []
  | cons {w v event w' v' rest} :
      view w sid = some v → Step w event w' → view w' sid = some v' →
      StepTrace sid w' v' rest → StepTrace sid w v ((event, w', v') :: rest)

theorem steptrace_reachable : ∀ {w v steps sid}, StepTrace sid w v steps →
    Reachable w → ∀ s ∈ steps, Reachable s.2.1 := by
  intro w v steps sid h
  induction h with
  | nil hview => intro _ s hs; simp at hs
  | cons hview hstep hview' hrest ih =>
      intro hreach s hs
      have hreach' : Reachable _ := Reachable.step hreach hstep
      simp at hs
      rcases hs with hs | hs
      · subst hs; exact hreach'
      · exact ih hreach' s hs

/-! ### Monitor soundness -/

theorem trace_monitorFrom_true : ∀ {w v steps sid}, StepTrace sid w v steps →
    Reachable w → v.terminal = true → monitorFrom true (views v steps) = true := by
  intro w v steps sid h
  induction h with
  | nil hview =>
      intro hreach hvterm
      have hnr := reachable_terminal_not_running hreach hview hvterm
      simp [views, monitorFrom, hvterm, hnr]
  | cons hview hstep hview' hrest ih =>
      rename_i w0 v0 event0 w1 v1 rest0
      intro hreach hvterm
      have hreach' : Reachable w1 := Reachable.step hreach hstep
      have hvterm' : v1.terminal = true :=
        step_terminal_monotone hstep hview hvterm hview'
      have hnr : v0.phase ≠ .running :=
        reachable_terminal_not_running hreach hview hvterm
      have hih : monitorFrom true (views v1 rest0) = true := ih hreach' hvterm'
      have hhead : (decide (v0.terminal = true) && decide (v0.phase ≠ .running)) = true := by
        simp [hvterm, hnr]
      simp only [views, List.map_cons]
      change ((decide (v0.terminal = true) && decide (v0.phase ≠ .running)) &&
        monitorFrom true (some v1 :: List.map (fun s => some s.2.2) rest0)) = true
      rw [hhead, Bool.true_and]
      simpa [views] using hih

theorem trace_monitorFrom_false : ∀ {w v steps sid}, StepTrace sid w v steps →
    Reachable w → monitorFrom false (views v steps) = true := by
  intro w v steps sid h
  induction h with
  | nil hview =>
      rename_i w0 v0
      intro hreach
      by_cases hvterm : v0.terminal = true <;>
        simp [views, monitorFrom, hvterm]
  | cons hview hstep hview' hrest ih =>
      rename_i w0 v0 event0 w1 v1 rest0
      intro hreach
      have hreach' : Reachable w1 := Reachable.step hreach hstep
      by_cases hvterm : v0.terminal = true
      · have hvterm' : v1.terminal = true :=
          step_terminal_monotone hstep hview hvterm hview'
        have htrue : monitorFrom true (views v1 rest0) = true :=
          trace_monitorFrom_true hrest hreach' hvterm'
        simp only [views, List.map_cons]
        change monitorFrom v0.terminal
          (some v1 :: List.map (fun s => some s.2.2) rest0) = true
        rw [hvterm]
        simpa [views] using htrue
      · have hfalse : v0.terminal = false := by simpa using hvterm
        have hih : monitorFrom false (views v1 rest0) = true := ih hreach'
        simp only [views, List.map_cons]
        change monitorFrom v0.terminal
          (some v1 :: List.map (fun s => some s.2.2) rest0) = true
        rw [hfalse]
        simpa [views] using hih

/-- Soundness: a trace whose observations come from a reachable world and a
    legal step at each element is always accepted by the monitor. -/
theorem trace_monitor_sound {w : World} {v : SandboxView}
    {steps : List (Event × World × SandboxView)} {sid : SandboxId}
    (h : StepTrace sid w v steps) (reachable : Reachable w) :
    monitor (views v steps) = true := by
  unfold monitor
  exact trace_monitorFrom_false h reachable

/-! ### Executable examples -/

/-- A normal lifecycle observation sequence is accepted. -/
example : monitor
    [some ⟨.starting, false, 1⟩, some ⟨.running, false, 1⟩,
     some ⟨.paused, false, 1⟩, some ⟨.starting, false, 2⟩,
     some ⟨.running, false, 2⟩] = true := by decide

/-- The F-1 shape: a terminal (kill) observation followed by `running` is
    rejected. -/
example : monitor
    [some ⟨.paused, false, 1⟩, some ⟨.revoking, true, 1⟩,
     some ⟨.running, false, 1⟩] = false := by decide

end SandboxModel.Monitor