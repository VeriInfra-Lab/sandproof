import SandboxModel.Foundations.Focused

namespace SandboxModel.Local.Runtime
open SandboxModel.Foundations

inductive Input where | crash | recover deriving DecidableEq
inductive Reject where | alreadyDown | alreadyUp deriving DecidableEq

def check (up : Bool) (input : Input) : Except Reject Bool :=
  match input with
  | .crash => if up then .ok false else .error .alreadyDown
  | .recover => if up then .error .alreadyUp else .ok true

def transition (state : Focused Bool Rest) (input : Input) : Except Reject (Focused Bool Rest) :=
  state.checked check input

theorem model_frame (before after : Focused Bool Rest) (input : Input)
    (accepted : transition before input = .ok after) : after.rest = before.rest :=
  Focused.checked_preserves_rest check before after input accepted

theorem model_roundtrip (state : Focused Bool Rest) (up : state.focus = true) :
    (transition state .crash).bind (fun after => transition after .recover) = .ok state := by
  cases state
  simp_all [transition, Focused.checked, Focused.set, check, Except.bind]

end SandboxModel.Local.Runtime
