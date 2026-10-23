import SandproofChildObservation.Types
import SandboxModel.Boundaries.ChildObservation.Events

open Aeneas Aeneas.Std
open SandproofChildObservationRust
namespace SandboxModel.Boundaries.ChildObservation

abbrev Operations := child_process.observation_core.Operations
abbrev Outcome C L S E I := core.result.Result S E × Option C × Option L × I

def Tail (history suffix : List α) : Prop := ∃ before, history = before ++ suffix

structure NormalCalls {I C L S E T K : Type} (ops : Operations I C L S E T)
    (identity : C → K) (history : I → List (Event K L S E T)) (unknown : E) where
  poll : ∀ io child, WP.dspec (ops.try_wait io child) fun (reply, io', child') =>
    identity child' = identity child ∧ history io' = history io ++ [.poll (identity child) reply]
  kill : ∀ io child, WP.dspec (ops.kill io child) fun (reply, io', child') =>
    identity child' = identity child ∧ history io' = history io ++ [.kill (identity child) reply]
  close : ∀ io child, WP.dspec (ops.close_pipes io child) fun (io', child') =>
    identity child' = identity child ∧ history io' = history io ++
      [.closeStdin (identity child), .closeStdout (identity child), .closeStderr (identity child)]
  now : ∀ io, WP.dspec (ops.now io) fun (t, io') => history io' = history io ++ [.now t]
  grace : ∀ io t, WP.dspec (ops.add_grace io t) fun (finish, io') =>
    history io' = history io ++ [.deadline t finish]
  compare : ∀ t deadline, WP.dspec (ops.corecmpPartialOrdInst.ge t deadline) fun _ => True
  pause : ∀ io, WP.dspec (ops.pause io) fun io' => history io' = history io ++ [.pause]
  retain : ∀ io slot child, WP.dspec (ops.retain io slot child) fun io' =>
    history io' = history io ++ [.handedOff (identity child) slot]
  error : ∀ io, WP.dspec (ops.unknown_exit io) fun (error, io') =>
    error = unknown ∧ history io' = history io ++ [.timeoutError error]

def Accounted {I C L S E T K : Type} (identity : C → K)
    (history : I → List (Event K L S E T)) (unknown : E) (key : K) (slot : L)
    (outcome : Outcome C L S E I) : Prop :=
  let (reply, child, owner, io) := outcome
  match reply with
  | .Ok status => ∃ finalChild, child = some finalChild ∧ identity finalChild = key ∧
      owner = some slot ∧ Tail (history io) [.poll key (.Ok (some status))]
  | .Err error => child = none ∧ owner = none ∧ error = unknown ∧
      Tail (history io) [.handedOff key slot, .timeoutError error]

end SandboxModel.Boundaries.ChildObservation
