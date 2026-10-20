import SandboxModel.Boundaries.ChildObservation.Model
import SandproofChildObservation.Funs

open Aeneas Aeneas.Std
open SandproofChildObservationRust
namespace SandboxModel.Boundaries.ChildObservation

variable {I C L S E T K : Type}
variable (ops : Operations I C L S E T) (identity : C → K)
variable (history : I → List (Event K L S E T)) (unknown : E)
variable (calls : NormalCalls ops identity history unknown)
include calls

private def pending (child : C) (slot : L) (io : I) (deadline : T) :
    Result (ControlFlow (C × I) (Outcome C L S E I)) := do
  let (now, io1) ← ops.now io
  let expired ← ops.corecmpPartialOrdInst.ge now deadline
  if expired then
    let io2 ← ops.retain io1 slot child
    let (error, io3) ← ops.unknown_exit io2
    .ok (.done (.Err error, none, none, io3))
  else
    let io2 ← ops.pause io1
    .ok (.cont (child, io2))

private theorem pending_accounted (child : C) (slot : L) (io : I) (deadline : T) :
    WP.dspec (pending ops child slot io deadline) fun flow =>
      match flow with
      | .cont (child', _) => identity child' = identity child
      | .done outcome => Accounted identity history unknown (identity child) slot outcome := by
  unfold pending
  apply WP.dspec_bind' (calls.now io)
  rintro ⟨now, io1⟩ nowHistory
  apply WP.dspec_bind' (calls.compare now deadline)
  intro expired _
  cases expired
  · simp only [Bool.false_eq_true, ↓reduceIte]
    apply WP.dspec_bind' (calls.pause io1)
    intro io2 _
    exact WP.dspec.ret rfl
  · simp only [↓reduceIte]
    apply WP.dspec_bind' (calls.retain io1 slot child)
    intro io2 retained
    apply WP.dspec_bind' (calls.error io2)
    rintro ⟨error, io3⟩ ⟨sameError, errorHistory⟩
    apply WP.dspec.ret
    refine ⟨rfl, rfl, sameError, ?_⟩
    refine ⟨history io1, ?_⟩
    simp only [errorHistory, retained, List.append_assoc, List.cons_append, List.nil_append]

omit calls in
private theorem loop_body_eq (child : C) (slot : L) (io : I) (deadline : T) :
    child_process.observation_core.stop_loop0.body ops (some slot) deadline child io =
      (do
        let (reply, io1, child1) ← ops.try_wait io child
        match reply with
        | .Ok (some status) =>
          .ok (.done (.Ok status, some child1, some slot, io1))
        | _ => pending ops child1 slot io1 deadline) := by
  unfold child_process.observation_core.stop_loop0.body
  congr 1
  funext value
  rcases value with ⟨reply, io1, child1⟩
  cases reply with
  | Ok option => cases option <;>
      simp [pending, core.option.Option.take]
  | Err error =>
      simp [pending, core.option.Option.take]

private theorem loop_body_accounted (child : C) (slot : L) (io : I) (deadline : T) :
    WP.dspec (child_process.observation_core.stop_loop0.body ops (some slot) deadline child io)
      fun flow => match flow with
      | .cont (child', _) => identity child' = identity child
      | .done outcome => Accounted identity history unknown (identity child) slot outcome := by
  rw [loop_body_eq ops child slot io deadline]
  apply WP.dspec_bind' (calls.poll io child)
  rintro ⟨reply, io1, child1⟩ ⟨sameChild, polled⟩
  cases reply with
  | Ok result =>
    cases result with
    | none =>
      have h := pending_accounted ops identity history unknown calls child1 slot io1 deadline
      rw [sameChild] at h
      exact h
    | some status =>
      apply WP.dspec.ret
      refine ⟨child1, rfl, sameChild, rfl, history io, ?_⟩
      simpa only [sameChild] using polled
  | Err error =>
    have h := pending_accounted ops identity history unknown calls child1 slot io1 deadline
    rw [sameChild] at h
    exact h


private theorem loop_accounted (child : C) (slot : L) (io : I) (deadline : T) :
    WP.dspec (child_process.observation_core.stop_loop0 ops child (some slot) io deadline)
      (Accounted identity history unknown (identity child) slot) := by
  let body := fun (state : C × I) =>
    child_process.observation_core.stop_loop0.body ops (some slot) deadline state.1 state.2
  have allStates : ∀ state : C × I,
      WP.dspec (loop body state) (Accounted identity history unknown (identity state.1) slot) := by
    apply loop.fixpoint_induct body
      (motive := fun run => ∀ state : C × I,
        WP.dspec (run state) (Accounted identity history unknown (identity state.1) slot))
    · apply Lean.Order.admissible_pi
      intro state
      apply Lean.Order.admissible_apply (fun _ value => WP.dspec value _)
      exact WP.dspec_admissible _
    · intro run hypothesis state
      rcases state with ⟨child1, io1⟩
      apply WP.dspec_bind' (loop_body_accounted ops identity history unknown calls child1 slot io1 deadline)
      intro flow valid
      cases flow with
      | done outcome => exact WP.dspec.ret valid
      | cont next =>
        have ih := hypothesis next
        rw [valid] at ih
        exact ih
  unfold child_process.observation_core.stop_loop0
  exact allStates (child, io)

private def afterPoll (child : C) (slot : L) (io : I) : Result (Outcome C L S E I) := do
  let (_, io1, child1) ← ops.kill io child
  let (io2, child2) ← ops.close_pipes io1 child1
  let (now, io3) ← ops.now io2
  let (deadline, io4) ← ops.add_grace io3 now
  child_process.observation_core.stop_loop0 ops child2 (some slot) io4 deadline

private theorem after_poll_accounted (child : C) (slot : L) (io : I) :
    WP.dspec (afterPoll ops child slot io)
      (Accounted identity history unknown (identity child) slot) := by
  unfold afterPoll
  apply WP.dspec_bind' (calls.kill io child)
  rintro ⟨_, io1, child1⟩ ⟨sameChild1, _⟩
  apply WP.dspec_bind' (calls.close io1 child1)
  rintro ⟨io2, child2⟩ ⟨sameChild2, _⟩
  apply WP.dspec_bind' (calls.now io2)
  rintro ⟨now, io3⟩ _
  apply WP.dspec_bind' (calls.grace io3 now)
  rintro ⟨deadline, io4⟩ _
  have h := loop_accounted ops identity history unknown calls child2 slot io4 deadline
  rw [sameChild2, sameChild1] at h
  exact h

omit calls in
private theorem stop_eq (child : C) (slot : L) (io : I) :
    child_process.observation_core.stop ops (some child) (some slot) io =
      (do
        let (reply, io1, child1) ← ops.try_wait io child
        match reply with
        | .Ok (some status) => .ok (.Ok status, some child1, some slot, io1)
        | _ => afterPoll ops child1 slot io1) := by
  unfold child_process.observation_core.stop
  dsimp only
  congr 1
  funext value
  rcases value with ⟨reply, io1, child1⟩
  cases reply with
  | Ok option => cases option <;> rfl
  | Err error => rfl

theorem stop_accounted (child : C) (slot : L) (io : I) :
    WP.dspec (child_process.observation_core.stop ops (some child) (some slot) io)
      (Accounted identity history unknown (identity child) slot) := by
  rw [stop_eq ops child slot io]
  apply WP.dspec_bind' (calls.poll io child)
  rintro ⟨reply, io1, child1⟩ ⟨sameChild, polled⟩
  cases reply with
  | Ok result =>
    cases result with
    | none =>
      have h := after_poll_accounted ops identity history unknown calls child1 slot io1
      rw [sameChild] at h
      exact h
    | some status =>
      apply WP.dspec.ret
      refine ⟨child1, rfl, sameChild, rfl, history io, ?_⟩
      exact polled
  | Err error =>
    have h := after_poll_accounted ops identity history unknown calls child1 slot io1
    rw [sameChild] at h
    exact h

end SandboxModel.Boundaries.ChildObservation
