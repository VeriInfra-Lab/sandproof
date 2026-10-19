import SandboxModel.FirstChain.System

namespace SandboxModel.FirstChain

theorem step_terminal_mono {Binding Node : Type} [DecidableEq Binding] [DecidableEq Node]
    {cfg : Config Binding Node} {s next : State Binding Node} {b : Binding}
    (h : Step cfg s next) (terminal : (s.records b).terminal = true) :
    (next.records b).terminal = true := by
  cases h with
  | perform key a r known _ _ _ ha =>
    by_cases hk : b = key
    · subst b
      simpa [Message.fact, afterAction, put] using
        (advance_fact_mono (m := .terminate) ha terminal)
    · simpa [afterAction, put, hk] using terminal
  | deliver key m r known _ _ _ ha =>
    by_cases hk : b = key
    · subst b
      simpa [Message.fact, afterAction, put] using
        (advance_fact_mono (m := .terminate) ha terminal)
    · simpa [afterAction, put, hk] using terminal
  | _ => exact terminal

theorem crash_center_preserves_records (s : State Binding Node) :
    ({ s with centerUp := false, centerConfirmed := false } : State Binding Node).records =
      s.records := rfl

theorem crash_node_preserves_resources (s : State Binding Node) [DecidableEq Node] (n : Node) :
    ({ s with nodeUp := fun k => if k = n then false else s.nodeUp k } :
      State Binding Node).records = s.records := rfl

def runRecord (r : Record) : List Action → Option Record
  | [] => some r
  | a :: rest => (advance r a).bind fun next => runRecord next rest

def startedRecord : Record :=
  { accepted := true, received := true, admitted := true, claimed := true, potential := true, ready := true }

theorem startup_example :
    runRecord {} [.accept, .receive, .admitLocal, .claim, .effect, .ready] = some startedRecord := by decide

theorem pending_is_not_failure :
    startedRecord.potential = true ∧ startedRecord.readySaved = false ∧
    startedRecord.accepted = true := by decide

theorem live_lease_blocks_release :
    runRecord startedRecord [.acquire, .terminate, .receiveTerminal, .fence, .stop, .clean, .release] =
      none := by decide

theorem completed_cleanup_example :
    (runRecord startedRecord
      [.acquire, .terminate, .receiveTerminal, .fence, .stop, .clean,
       .drain, .release, .saveRelease, .confirm]).map (·.confirmed) = some true := by decide

theorem terminal_first_rejects_late_create :
    runRecord {} [.accept, .terminate, .receiveTerminal, .receive] = none := by decide

theorem stopped_does_not_mean_cleaned :
    (runRecord startedRecord [.terminate, .receiveTerminal, .fence, .stop]).map
      (fun r => r.stopped && !r.cleaned && r.potential && !r.releaseSaved) = some true := by decide

-- Two nodes are only this witness. System preservation is parameterized over
-- any finite registration list and any type of complete bindings.
private def exampleConfig : Config Nat Nat := ⟨[0, 1], fun b => b % 2⟩
private def acceptedRecord : Record := { accepted := true }
private def receivedRecord : Record := { accepted := true, received := true }
private def admittedRecord : Record := { accepted := true, received := true, admitted := true }
private def claimedRecord : Record := { accepted := true, received := true, admitted := true, claimed := true }
private def affectedRecord : Record := { accepted := true, received := true, admitted := true, claimed := true, potential := true }

private def s1 : State Nat Nat := afterAction initial 0 .accept acceptedRecord true
private def s2 : State Nat Nat := { s1 with messages := [(0, .create)] }
private def s3 : State Nat Nat := afterAction s2 0 .receive receivedRecord true
private def s3a : State Nat Nat := afterAction s3 0 .admitLocal admittedRecord true
private def s4 : State Nat Nat := afterAction s3a 0 .claim claimedRecord true
private def s5 : State Nat Nat := afterAction s4 0 .effect affectedRecord true
private def s6 : State Nat Nat := afterAction s5 0 .ready startedRecord true
private def s7 : State Nat Nat := { s6 with messages := (0, .ready) :: s6.messages }
private def s8 : State Nat Nat := { s7 with messages := [] }
private def s9 : State Nat Nat := { s8 with centerUp := false, centerConfirmed := false }

theorem lost_reply_and_crash_reachable :
    ∃ s : State Nat Nat, Reachable exampleConfig s ∧
      (s.records 0).potential = true ∧ (s.records 0).readySaved = false ∧
      s.centerUp = false ∧ s.nodeUp 1 = true := by
  have h1 : Reachable exampleConfig s1 :=
    .next .init (.perform initial 0 .accept acceptedRecord true (by decide) ⟨rfl, rfl, rfl⟩
      (by decide) (by decide))
  have h2 : Reachable exampleConfig s2 :=
    .next h1 (.send s1 0 .create ⟨rfl, rfl, rfl⟩ (by decide))
  have h3 : Reachable exampleConfig s3 :=
    .next h2 (.deliver s2 0 .create receivedRecord true (by decide) (by decide)
      rfl (by decide))
  have h3a : Reachable exampleConfig s3a :=
    .next h3 (.perform s3 0 .admitLocal admittedRecord true (by decide) rfl
      (by decide) (by decide))
  have h4 : Reachable exampleConfig s4 :=
    .next h3a (.perform s3a 0 .claim claimedRecord true (by decide) rfl
      (by decide) (by decide))
  have h5 : Reachable exampleConfig s5 :=
    .next h4 (.perform s4 0 .effect affectedRecord true (by decide) trivial
      (by decide) (by decide))
  have h6 : Reachable exampleConfig s6 :=
    .next h5 (.perform s5 0 .ready startedRecord true (by decide) rfl
      (by decide) (by decide))
  have h7 : Reachable exampleConfig s7 :=
    .next h6 (.send s6 0 .ready rfl (by decide))
  have h8 : Reachable exampleConfig s8 :=
    .next h7 (.network s7 [] (by simp))
  exact ⟨s9, .next h8 (.crashCenter s8), by decide, by decide, by decide, by decide⟩

theorem confirmed_write_reply_may_be_lost :
    ∃ s : State Nat Nat, Reachable exampleConfig s ∧
      (s.records 0).accepted = true ∧ s.centerConfirmed = false := by
  let next : State Nat Nat := afterAction initial 0 .accept acceptedRecord false
  refine ⟨next, ?_, by decide, by decide⟩
  exact .next .init (.perform initial 0 .accept acceptedRecord false
    (by decide) ⟨rfl, rfl, rfl⟩ (by decide) (by decide))


theorem premature_release_counterexample :
    ¬ Safe { startedRecord with released := true } := by
  simp [Safe, startedRecord]

theorem invented_ready_counterexample :
    ¬ Safe ({ accepted := true, readySaved := true } : Record) := by
  simp [Safe]

/-- Existence of an original-result recovery path, NOT a fairness/liveness theorem. -/
theorem original_ready_recoverable {Binding Node : Type} [DecidableEq Binding] [DecidableEq Node]
    {cfg : Config Binding Node} {s : State Binding Node} {b : Binding}
    (hr : Reachable cfg s) (registered : cfg.nodeOf b ∈ cfg.nodes)
    (ready : (s.records b).ready = true) (notTerminal : (s.records b).terminal = false)
    (nodeUp : s.nodeUp (cfg.nodeOf b) = true) :
    ∃ next, Reachable cfg next ∧ (next.records b).readySaved = true ∧
      (next.records b).potential = (s.records b).potential := by
  let s1 := { s with centerUp := true, centerConfirmed := false }
  let s2 := { s1 with redisUp := true, centerConfirmed := false }
  let s3 := { s2 with centerConfirmed := true }
  let s4 := { s3 with messages := (b, Message.ready) :: s3.messages }
  let record := { s.records b with readySaved := true }
  let s5 := afterAction s4 b .saveReady record true
  have h1 : Reachable cfg s1 := .next hr (.restartCenter s)
  have h2 : Reachable cfg s2 := .next h1 (.restartRedis s1)
  have h3 : Reachable cfg s3 := .next h2 (.reaffirm s2 rfl rfl)
  have h4 : Reachable cfg s4 := .next h3 (.send s3 b .ready nodeUp ready)
  have h5 : Reachable cfg s5 := .next h4
    (.deliver s4 b .ready record true registered (by simp [s4])
      ⟨rfl, rfl, rfl⟩ (by simp [s4, s3, s2, s1, record, Message.action, advance, ready, notTerminal]))
  exact ⟨s5, h5, by simp [s5, record, afterAction, put],
    by simp [s5, record, afterAction, put]⟩

theorem receipt_is_not_admission :
    advance ({ accepted := true, received := true } : Record) .claim = none := by decide

theorem retained_reference_blocks_release :
    runRecord startedRecord [.retainReference, .terminate, .receiveTerminal, .fence, .stop, .clean,
      .release] = none := by decide

end SandboxModel.FirstChain
