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

end SandproofFoundationsRust
