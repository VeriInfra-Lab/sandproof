import SandboxModel.Foundations.Model

/-! Independent value specification of center response classification.
The selected original request is checked by the lifecycle specification before
this decision. This model neither selects a record nor commits or mutates one.
Full payloads remain in equality; rank is only an ordering, not an identity. -/
namespace SandboxModel.CenterResponse

inductive Fact where
  | recorded
  | ready (started finish maximum : Nat)
  | renewed (finish : Nat)
  | stopped
  | cleaned
  | notAccepted
  deriving DecidableEq

structure Response where
  identity : Foundations.Binding
  observation : Nat
  fact : Fact
  deriving DecidableEq

def rank : Fact → Nat
  | .recorded => 0
  | .stopped => 1
  | _ => 2

inductive Classification where
  | duplicate | advance | stale
  deriving DecidableEq

def classify (checked : Except Foundations.BindingError Unit)
    (prior : Option Response) (observed : Option Nat) (actual : Response) : Classification :=
  match checked with
  | .error _ => .stale
  | .ok _ =>
      if prior = some actual then .duplicate
      else if observed.any (fun version => actual.observation ≤ version) then .stale
      else if prior.any (fun old => rank actual.fact ≤ rank old.fact) then .stale
      else .advance

end SandboxModel.CenterResponse
