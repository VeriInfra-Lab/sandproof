import Aeneas
import SandproofSourcePreservation.Types
open Aeneas Aeneas.Std
namespace SandproofSourcePreservationRust
-- Existing A6 ordinary Option models, with original callback and full value.
def core.option.Option.is_some_and {T F : Type}
    (callback : core.ops.function.FnOnce F T Bool) (value : Option T) (state : F) : Result Bool :=
  match value with
  | none => .ok false
  | some entry => callback.call_once state entry

def core.option.Option.Insts.CoreCmpPartialEqOption.eq {T : Type}
    (dictionary : core.cmp.PartialEq T T) (left right : Option T) : Result Bool :=
  match left, right with
  | none, none => .ok true
  | some left, some right => dictionary.eq left right
  | _, _ => .ok false
end SandproofSourcePreservationRust
