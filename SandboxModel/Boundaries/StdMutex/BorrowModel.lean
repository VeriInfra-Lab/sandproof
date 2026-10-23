import SandproofStdMutex.Types
open Aeneas Aeneas.Std

namespace SandboxModel.Boundaries.StdMutexContract.Borrow
abbrev Cell (T : Type) := std.sync.poison.mutex.Mutex T
abbrev View (T : Type) := core.result.Result T (std.sync.poison.PoisonError T)

def BorrowView (s : Cell T) (v : View T) : Prop :=
  if s.poisoned then v = .Err ⟨s.payload⟩ else v = .Ok s.payload

def UpdatedPayload (v : View T) : T :=
  match v with | .Ok data => data | .Err error => error.data

def BackView (s t : Cell T) (v : View T) : Prop :=
  t.identity = s.identity ∧ t.poisoned = s.poisoned ∧
  t.rawLocked = s.rawLocked ∧ t.payload = UpdatedPayload v

def GetMutContract (f : Cell T → Result (View T × (View T → Cell T))) : Prop :=
  ∀ s, ∃ v back, f s = .ok (v, back) ∧ BorrowView s v ∧
    ∀ updated, BackView s (back updated) updated

def SameArm (s : Cell T) (v : View T) : Prop :=
  match s.poisoned, v with
  | false, .Ok _ => True
  | true, .Err _ => True
  | _, _ => False

end SandboxModel.Boundaries.StdMutexContract.Borrow
