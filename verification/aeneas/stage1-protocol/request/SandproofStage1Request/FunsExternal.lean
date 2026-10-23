import SandproofFoundations.FunsExternal
import SandproofNetwork.FunsExternal
import SandproofStage1Request.Types

/-! Exact namespace adapters for the existing fixed A6 string/value models.
These do not establish standard-library correctness or business authorization. -/
open Aeneas Aeneas.Std
namespace SandproofStage1RequestRust

def core.option.Option.Insts.CoreCmpPartialEqOption.eq {T : Type}
    (eq : core.cmp.PartialEq T T) (left right : Option T) : Result Bool :=
  SandproofNetworkRust.core.option.Option.Insts.CoreCmpPartialEqOption.eq eq left right

def core.result.Result.is_err {T E : Type} (value : core.result.Result T E) : Result Bool :=
  -- The same two-constructor model registered for SQLite; importing its generated
  -- types would collide with Network's generated auxiliary declarations.
  match value with
  | .Ok _ => .ok false
  | .Err _ => .ok true

def Bool.Insts.CoreCmpPartialEqBool.ne (left right : Bool) : Result Bool :=
  SandproofNetworkRust.Bool.Insts.CoreCmpPartialEqBool.ne left right

def alloc.string.String.Insts.CoreCmpPartialEqString.eq
    (left right : String) : Result Bool :=
  SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq left right

def alloc.vec.Vec.is_empty {T : Type} (allocator : Type) (values : alloc.vec.Vec T) :
    Result Bool := SandproofFoundationsRust.alloc.vec.Vec.is_empty allocator values

end SandproofStage1RequestRust
