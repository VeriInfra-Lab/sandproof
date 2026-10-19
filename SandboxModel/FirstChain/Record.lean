import Std

/-!
First-chain obligation projection, not a Rust implementation model.
The opaque full binding and node identity are supplied by the system layer.
Historical facts stay true after termination; `potential` means possibly
remaining physical effects (not a VM count), and leases count unconfirmed
holders, including holders surviving a controller crash. Actual evidence,
capacity, local mapping uniqueness, retries and result variants remain module
obligations. Safe is a conclusion, never a guard of advance.
-/
namespace SandboxModel.FirstChain

structure Record where
  -- Center durable acceptance, node inbox, then node local admission: distinct commits.
  accepted : Bool := false
  received : Bool := false
  admitted : Bool := false
  claimed : Bool := false
  -- Conservative physical residue, not a count and not a claim of present health.
  potential : Bool := false
  ready : Bool := false
  readySaved : Bool := false
  terminal : Bool := false
  nodeTerminal : Bool := false
  fenced : Bool := false
  stopped : Bool := false
  cleaned : Bool := false
  -- Ghost counts of actual holders and blocking references, not volatile caches.
  leases : Nat := 0
  references : Nat := 0
  released : Bool := false
  releaseSaved : Bool := false
  confirmed : Bool := false
  deriving DecidableEq, Repr

inductive Action where
  | accept | receive | admitLocal | claim | effect | ready | saveReady | terminate
  | receiveTerminal | fence | stop | clean | acquire | drain | release
  | saveRelease | confirm | retainReference | releaseReference
  deriving DecidableEq, Repr

def advance (r : Record) : Action → Option Record
  | .accept => if !r.accepted then some { r with accepted := true } else none
  | .receive => if r.accepted && !r.nodeTerminal then some { r with received := true } else none
  | .admitLocal => if r.received && !r.nodeTerminal && !r.admitted then some { r with admitted := true } else none
  | .claim => if r.admitted && !r.nodeTerminal && !r.claimed then some { r with claimed := true } else none
  | .effect => if r.claimed && !r.fenced then some { r with potential := true } else none
  | .ready => if r.potential && !r.nodeTerminal then some { r with ready := true } else none
  | .saveReady => if r.ready && !r.terminal then some { r with readySaved := true } else none
  | .terminate => if r.accepted then some { r with terminal := true } else none
  | .receiveTerminal => if r.terminal then some { r with received := true, nodeTerminal := true } else none
  | .fence => if r.nodeTerminal then some { r with fenced := true } else none
  | .stop => if r.fenced then some { r with stopped := true } else none
  | .clean => if r.stopped then some { r with cleaned := true, potential := false } else none
  | .acquire => if r.ready && !r.nodeTerminal then some { r with leases := r.leases + 1 } else none
  | .drain => if r.leases > 0 then some { r with leases := r.leases - 1 } else none
  | .release => if r.cleaned && r.leases == 0 && r.references == 0 then some { r with released := true } else none
  | .saveRelease => if r.released then some { r with releaseSaved := true } else none
  | .retainReference => if r.admitted && !r.released then some { r with references := r.references + 1 } else none
  | .releaseReference => if r.references > 0 then some { r with references := r.references - 1 } else none
  | .confirm => if r.releaseSaved then some { r with confirmed := true } else none

def Safe (r : Record) : Prop :=
  (r.received = true → r.accepted = true) ∧
  (r.claimed = true → r.admitted = true) ∧
  (r.potential = true → r.claimed = true ∧ r.released = false) ∧
  (r.ready = true → r.claimed = true) ∧
  (r.readySaved = true → r.ready = true) ∧
  (r.nodeTerminal = true → r.terminal = true ∧ r.received = true) ∧
  (r.terminal = true → r.accepted = true) ∧
  (r.fenced = true → r.nodeTerminal = true) ∧
  (r.stopped = true → r.fenced = true) ∧
  (r.cleaned = true → r.stopped = true ∧ r.potential = false) ∧
  (r.released = true → r.cleaned = true ∧ r.leases = 0 ∧ r.references = 0) ∧
  (r.releaseSaved = true → r.released = true) ∧
  (r.confirmed = true → r.releaseSaved = true) ∧
  (r.leases > 0 → r.ready = true ∧ r.released = false) ∧
  (r.admitted = true → r.received = true) ∧
  (r.references > 0 → r.admitted = true ∧ r.released = false)

theorem empty_safe : Safe ({} : Record) := by simp [Safe]

theorem advance_safe (h : Safe r) (ha : advance r a = some next) : Safe next := by
  rcases h with
    ⟨receivedAccepted, claimedAdmitted, potentialClaimed, readyClaimed, readySavedReady,
     nodeTerminalCovered, terminalAccepted, fencedTerminal, stoppedFenced, cleanedStopped,
     releasedQuiescent, releaseSavedReleased, confirmedSaved, leasesReady,
     admittedReceived, referencesAdmitted⟩
  cases a <;> simp only [advance] at ha <;> split at ha <;> cases ha <;>
    simp_all [Safe, Bool.and_eq_true, Bool.not_eq_true] <;> omega

inductive Message where
  | create | terminate | ready | released | acknowledgement
  deriving DecidableEq, Repr

def Message.fact (m : Message) (r : Record) : Bool :=
  match m with
  | .create => r.accepted
  | .terminate => r.terminal
  | .ready => r.ready
  | .released => r.released
  | .acknowledgement => r.releaseSaved

def Message.action : Message → Action
  | .create => .receive
  | .terminate => .receiveTerminal
  | .ready => .saveReady
  | .released => .saveRelease
  | .acknowledgement => .confirm

theorem advance_fact_mono {m : Message} (ha : advance r a = some next)
    (hf : m.fact r = true) : m.fact next = true := by
  cases a <;> simp only [advance] at ha <;> split at ha <;> cases ha <;>
    cases m <;> simp_all [Message.fact]

theorem advance_terminal_mono (ha : advance r a = some next)
    (ht : r.nodeTerminal = true) : next.nodeTerminal = true := by
  cases a <;> simp only [advance] at ha <;> split at ha <;> cases ha <;> simp_all

theorem potential_has_responsibility (h : Safe r) (hp : r.potential = true) :
    r.accepted = true ∧ r.received = true ∧ r.released = false ∧ r.releaseSaved = false := by
  rcases h with
    ⟨receivedAccepted, claimedAdmitted, potentialClaimed, readyClaimed, readySavedReady,
     nodeTerminalCovered, terminalAccepted, fencedTerminal, stoppedFenced, cleanedStopped,
     releasedQuiescent, releaseSavedReleased, confirmedSaved, leasesReady,
     admittedReceived, referencesAdmitted⟩
  have hc := potentialClaimed hp
  have hr := admittedReceived (claimedAdmitted hc.1)
  have ha := receivedAccepted hr
  have hs : r.releaseSaved = false := by
    cases he : r.releaseSaved <;> simp_all
  exact ⟨ha, hr, hc.2, hs⟩

theorem release_requires_quiescence (h : Safe r) (hr : r.releaseSaved = true) :
    r.nodeTerminal = true ∧ r.fenced = true ∧ r.stopped = true ∧
    r.cleaned = true ∧ r.potential = false ∧ r.leases = 0 ∧ r.references = 0 := by
  rcases h with
    ⟨receivedAccepted, claimedAdmitted, potentialClaimed, readyClaimed, readySavedReady,
     nodeTerminalCovered, terminalAccepted, fencedTerminal, stoppedFenced, cleanedStopped,
     releasedQuiescent, releaseSavedReleased, confirmedSaved, leasesReady,
     admittedReceived, referencesAdmitted⟩
  have localRelease := releasedQuiescent (releaseSavedReleased hr)
  have cleaned := cleanedStopped localRelease.1
  have fenced := stoppedFenced cleaned.1
  exact ⟨fencedTerminal fenced, fenced, cleaned.1, localRelease.1, cleaned.2, localRelease.2.1, localRelease.2.2⟩

theorem terminal_blocks_receive (ht : r.nodeTerminal = true) :
    advance r .receive = none := by simp [advance, ht]

theorem terminal_blocks_acquire (ht : r.nodeTerminal = true) :
    advance r .acquire = none := by simp [advance, ht]

theorem fence_blocks_effect (hf : r.fenced = true) :
    advance r .effect = none := by simp [advance, hf]

theorem no_second_claim (hc : r.claimed = true) :
    advance r .claim = none := by simp [advance, hc]

end SandboxModel.FirstChain
