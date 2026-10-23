import SandboxModel.Boundaries.ChildObservation.Model
import SandboxModel.Boundaries.ChildObservation.Interface

open Aeneas Aeneas.Std
/-! Models of the named foreign calls on finite normal-return histories.
Each API has its own response stream, so this does not assume the caller's
business order. `calls` proves properties of these models, not of Rust/Linux.
Ordinary IO Result::Err remains a normal returned response. An unfinished call
has no normal after-state. Panic/unwind/abort cleanup and partial external
effects are outside this normal-result theorem; no rollback or release follows. -/

namespace SandboxModel.Boundaries.ChildObservation.Library
open Interface

/-- Typed external observations only; this does not execute stop/reap logic. -/
def takeReply : List α → Result (α × List α)
  | [] => .div
  | value :: rest => .ok (value, rest)

def completed : List Bool → Result (List Bool)
  | true :: rest => .ok rest
  | _ => .div

def seed (io : Io) (child : Child) : Io :=
  if io.seeded then io else {io with schedule := child.schedule, seeded := true}

theorem seed_history (io : Io) (child : Child) : (seed io child).history = io.history := by
  cases h : io.seeded <;> simp [seed, h]

theorem takeReply_normal (values : List α) : WP.dspec (takeReply values) fun _ => True := by
  cases values with
  | nil => exact WP.dspec.div _
  | cons value rest => exact WP.dspec.ret trivial

theorem completed_normal (values : List Bool) : WP.dspec (completed values) fun _ => True := by
  cases values with
  | nil => exact WP.dspec.div _
  | cons value rest =>
    cases value with
    | false => exact WP.dspec.div _
    | true => exact WP.dspec.ret trivial

private abbrev recordPoll (ready : Io) (child : Child)
    (reply : core.result.Result (Option Int) IoError) (schedule : Schedule) :
    Result (core.result.Result (Option Int) IoError × Io × Child) :=
  let observed := match reply with | .Ok (some status) => some status | _ => child.observed
  .ok (reply, {ready with schedule, history := ready.history ++ [.poll child.binding reply]},
    {child with schedule, observed})

def poll (io : Io) (child : Child) :
    Result (core.result.Result (Option Int) IoError × Io × Child) := do
  let ready := seed io child
  match child.observed with
  | some status => recordPoll ready child (.Ok (some status)) ready.schedule
  | none =>
    let (reply, rest) ← takeReply ready.schedule.polls
    recordPoll ready child reply {ready.schedule with polls := rest}

private abbrev recordKill (ready : Io) (child : Child)
    (reply : core.result.Result Unit IoError) (schedule : Schedule) :
    Result (core.result.Result Unit IoError × Io × Child) :=
  .ok (reply, {ready with schedule, history := ready.history ++ [.kill child.binding reply]},
    {child with schedule})

def kill (io : Io) (child : Child) : Result (core.result.Result Unit IoError × Io × Child) := do
  let ready := seed io child
  match child.observed with
  | some _ => recordKill ready child (.Ok ()) ready.schedule
  | none =>
    let (reply, rest) ← takeReply ready.schedule.kills
    recordKill ready child reply {ready.schedule with kills := rest}

def close (io : Io) (child : Child) : Result (Io × Child) := do
  let ready := seed io child
  let rest ← completed ready.schedule.closes
  let schedule := {ready.schedule with closes := rest}
  .ok ({ready with schedule, history := ready.history ++
      [.closeStdin child.binding, .closeStdout child.binding, .closeStderr child.binding]},
    {child with schedule, stdin := none, stdout := none, stderr := none})

def now (io : Io) : Result (Nat × Io) := do
  let (instant, rest) ← takeReply io.schedule.times
  .ok (instant, {io with
    schedule := {io.schedule with times := rest}
    history := io.history ++ [.now instant]})

def grace (io : Io) (start : Nat) : Result (Nat × Io) := do
  let rest ← completed io.schedule.graces
  let finish := start + 2000000000
  .ok (finish, {io with
    schedule := {io.schedule with graces := rest}
    history := io.history ++ [.deadline start finish]})

def pause (io : Io) : Result Io := do
  let rest ← completed io.schedule.pauses
  .ok {io with
    schedule := {io.schedule with pauses := rest}
    history := io.history ++ [.pause]}

def retain (io : Io) (slot : Slot) (child : Child) : Result Io := do
  let rest ← completed io.schedule.retains
  .ok {io with
    schedule := {io.schedule with retains := rest}
    history := io.history ++ [.handedOff child.binding slot]}

def error (io : Io) : Result (IoError × Io) := do
  let rest ← completed io.schedule.errors
  .ok (unknownExit, {io with
    schedule := {io.schedule with errors := rest}
    history := io.history ++ [.timeoutError unknownExit]})

def natClone : core.clone.Clone Nat := {clone := fun n => .ok n}
def natCopy : core.marker.Copy Nat := {cloneInst := natClone}
def natEq : core.cmp.PartialEq Nat Nat := {eq := fun x y => .ok (decide (x = y))}
def natCmp (x y : Nat) : Result (Option Ordering) := .ok (some (compare x y))
def natOrd : core.cmp.PartialOrd Nat Nat := {partialEqInst := natEq, partial_cmp := natCmp}

def ops : Operations Io Child Slot Int IoError Nat := {
  coremarkerCopyInst := natCopy
  corecmpPartialOrdInst := natOrd
  try_wait := poll
  kill := kill
  close_pipes := close
  now := now
  add_grace := grace
  pause := pause
  retain := retain
  unknown_exit := error
}

theorem calls : NormalCalls ops Child.binding Io.history unknownExit where
  poll := by
    intro io child
    change WP.dspec (poll io child) _
    unfold poll
    cases h : child.observed with
    | some status =>
      apply WP.dspec.ret
      refine ⟨rfl, ?_⟩
      exact congrArg (fun trace => trace ++ [.poll child.binding (.Ok (some status))])
        (seed_history io child)
    | none =>
      apply WP.dspec_bind' (takeReply_normal (seed io child).schedule.polls)
      rintro ⟨reply, rest⟩ _
      apply WP.dspec.ret
      refine ⟨rfl, ?_⟩
      exact congrArg (fun trace => trace ++ [.poll child.binding reply]) (seed_history io child)
  kill := by
    intro io child
    change WP.dspec (kill io child) _
    unfold kill
    cases h : child.observed with
    | some status =>
      apply WP.dspec.ret
      refine ⟨rfl, ?_⟩
      exact congrArg (fun trace => trace ++ [.kill child.binding (.Ok ())]) (seed_history io child)
    | none =>
      apply WP.dspec_bind' (takeReply_normal (seed io child).schedule.kills)
      rintro ⟨reply, rest⟩ _
      apply WP.dspec.ret
      refine ⟨rfl, ?_⟩
      exact congrArg (fun trace => trace ++ [.kill child.binding reply]) (seed_history io child)
  close := by
    intro io child
    unfold ops close
    apply WP.dspec_bind' (completed_normal (seed io child).schedule.closes)
    intro rest _
    apply WP.dspec.ret
    refine ⟨rfl, ?_⟩
    exact congrArg (fun h => h ++ [.closeStdin child.binding, .closeStdout child.binding,
      .closeStderr child.binding]) (seed_history io child)
  now := by
    intro io
    unfold ops now
    apply WP.dspec_bind' (takeReply_normal io.schedule.times)
    rintro ⟨instant, rest⟩ _
    exact WP.dspec.ret rfl
  grace := by
    intro io start
    unfold ops grace
    apply WP.dspec_bind' (completed_normal io.schedule.graces)
    intro rest _
    exact WP.dspec.ret rfl
  compare := by
    intro x y
    unfold ops natOrd core.cmp.PartialOrd.ge_body natCmp
    simp only [bind_tc_ok]
    exact WP.dspec.ret trivial
  pause := by
    intro io
    unfold ops pause
    apply WP.dspec_bind' (completed_normal io.schedule.pauses)
    intro rest _
    exact WP.dspec.ret rfl
  retain := by
    intro io slot child
    unfold ops retain
    apply WP.dspec_bind' (completed_normal io.schedule.retains)
    intro rest _
    exact WP.dspec.ret rfl
  error := by
    intro io
    unfold ops error
    apply WP.dspec_bind' (completed_normal io.schedule.errors)
    intro rest _
    exact WP.dspec.ret ⟨rfl, rfl⟩

end SandboxModel.Boundaries.ChildObservation.Library
