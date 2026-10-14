import Aeneas
import SandproofFoundations.Types

/-! Minimal fixed standard-library models (dependency A6), not project code.
Their agreement with the extraction compiler's standard library is an explicit
external contract. Total definitions and kernel checking do not prove that
compiler/library agreement. See docs/formal/dependencies.md. -/
open Aeneas Aeneas.Std
namespace SandproofFoundationsRust

@[simp, rust_fun "core::option::{core::option::Option<@T>}::as_ref"]
def core.option.Option.as_ref {T : Type} (value : Option T) : Result (Option T) :=
  .ok value

@[rust_fun "core::option::{core::option::Option<@T>}::is_none_or"]
def core.option.Option.is_none_or {T F : Type}
    (callback : Aeneas.Std.core.ops.function.FnOnce F T Bool)
    (value : Option T) (state : F) : Result Bool :=
  match value with
  | none => .ok true
  | some entry => callback.call_once state entry

@[simp, rust_fun "core::option::{core::ops::try_trait::Try<core::option::Option<@T>>}::branch"]
def core.option.Option.Insts.CoreOpsTry_traitTry.branch {T : Type}
    (value : Option T) :
    Result (core.ops.control_flow.ControlFlow (Option core.convert.Infallible) T) :=
  match value with
  | none => .ok (.Break none)
  | some x => .ok (.Continue x)

@[simp, rust_fun "core::option::{core::ops::try_trait::FromResidual<core::option::Option<@T>, core::option::Option<core::convert::Infallible>>}::from_residual"]
def core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual
    (T : Type) (value : Option core.convert.Infallible) : Result (Option T) :=
  match value with
  | none => .ok none
  | some impossible => nomatch impossible

@[simp, rust_fun "core::result::{core::result::Result<@T, @E>}::ok"]
def core.result.Result.ok {T E : Type} (value : core.result.Result T E) :
    Result (Option T) :=
  match value with
  | .Ok result => .ok (some result)
  | .Err _ => .ok none

@[simp, rust_fun "alloc::string::{core::cmp::PartialEq<alloc::string::String, alloc::string::String>}::eq"]
def alloc.string.String.Insts.CoreCmpPartialEqString.eq
    (left right : String) : Result Bool := .ok (decide (left = right))

@[simp, rust_fun "alloc::vec::{alloc::vec::Vec<@T>}::is_empty"]
def alloc.vec.Vec.is_empty {T : Type} (_A : Type) (values : alloc.vec.Vec T) :
    Result Bool := .ok (decide (values.val.length = 0))

/- The callback is stateful: preserve the updated state between calls. On a
callback failure no successful retained vector is fabricated. The real standard
library's successful-return agreement remains an explicit A6 contract. -/
def retainList {T F : Type} (call : F → T → Result (Bool × F))
    (state : F) : List T → Result (List T)
  | [] => .ok []
  | value :: rest => do
    let (keep, next) ← call state value
    let tail ← retainList call next rest
    .ok (if keep then value :: tail else tail)

@[rust_fun "alloc::vec::{alloc::vec::Vec<@T>}::retain"]
def alloc.vec.Vec.retain {T : Type} (_A : Type) {F : Type}
    (callback : core.ops.function.FnMut F T Bool) (values : alloc.vec.Vec T)
    (state : F) : Result (alloc.vec.Vec T) :=
  Aeneas.Std.bind (retainList callback.call_mut state values.val) fun kept =>
    if bound : kept.length ≤ Usize.max then .ok (Aeneas.Std.alloc.vec.Vec.from kept bound)
    else .fail .arrayOutOfBounds

end SandproofFoundationsRust
