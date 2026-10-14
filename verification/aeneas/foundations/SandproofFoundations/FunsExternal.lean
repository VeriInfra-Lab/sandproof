import Aeneas
import SandproofFoundations.Types
import SandproofFoundations.IdentifierLibrary

/-! Minimal fixed standard-library models (dependency A6), not project code.
Their agreement with the extraction compiler's standard library is an explicit
external contract. Total definitions and kernel checking do not prove that
compiler/library agreement. See docs/formal/dependencies.md. -/
open Aeneas Aeneas.Std
namespace SandproofFoundationsRust

-- Preserve the fixed backend's literal bytes and bound; only discharge the
-- bound with kernel-checked reduction instead of its default native tactic.
-- Generated literals resolve this local adapter without editing the output.
def toStr (text : String)
    (bounded : text.toByteArray.size ≤ U32.max := by rw [U32.max_eq]; decide) : Str :=
  Aeneas.Std.toStr text bounded

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

@[rust_fun "core::option::{core::option::Option<@T>}::is_some_and"]
def core.option.Option.is_some_and {T F : Type}
    (callback : Aeneas.Std.core.ops.function.FnOnce F T Bool)
    (value : Option T) (state : F) : Result Bool :=
  match value with
  | none => .ok false
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

@[simp, rust_fun "core::num::{u8}::is_ascii_alphanumeric"]
def core.num.U8.is_ascii_alphanumeric (byte : U8) : Result Bool :=
  .ok (decide ((48 ≤ byte.val ∧ byte.val ≤ 57) ∨
    (65 ≤ byte.val ∧ byte.val ≤ 90) ∨ (97 ≤ byte.val ∧ byte.val ≤ 122)))

@[rust_fun "core::str::{str}::len"]
def core.str.Str.len (value : Str) : Result Usize :=
  .ok (Usize.ofNatCore value.val.length (by have := value.property; scalar_tac))

@[simp, rust_fun "core::str::{str}::is_empty"]
def core.str.Str.is_empty (value : Str) : Result Bool :=
  .ok (decide (value.val.length = 0))

@[simp, rust_fun "core::str::{str}::bytes"]
def core.str.Str.bytes (value : Str) : Result core.str.iter.Bytes :=
  .ok ⟨value.val⟩

@[rust_fun "core::str::iter::{core::iter::traits::iterator::Iterator<core::str::iter::Bytes<'0>, u8>}::all"]
def core.str.iter.Bytes.Insts.CoreIterTraitsIteratorIteratorU8.all {F : Type}
    (callback : core.ops.function.FnMut F U8 Bool)
    (bytes : core.str.iter.Bytes) (state : F) : Result (Bool × core.str.iter.Bytes) := do
  let (accepted, rest) ← IdentifierLibrary.allBytes callback.call_mut state bytes.remaining
  .ok (accepted, ⟨rest⟩)

@[rust_fun "alloc::string::{core::ops::deref::Deref<alloc::string::String, str>}::deref"]
def alloc.string.String.Insts.CoreOpsDerefDerefStr.deref (value : String) : Result Str :=
  if h : IdentifierLibrary.StringRepresentable value then
    .ok (.from (IdentifierLibrary.stringBytes value) h)
  else .fail .maximumSizeExceeded

@[rust_fun "alloc::str::{alloc::borrow::ToOwned<str, alloc::string::String>}::to_owned"]
def Str.Insts.AllocBorrowToOwnedString.to_owned (value : Str) : Result String :=
  match IdentifierLibrary.decodeString value.val with
  | some text => .ok text
  | none => .fail .panic

@[rust_fun "core::str::{str}::split_at_checked"]
def core.str.Str.split_at_checked (value : Str) (offset : Usize) :
    Result (Option (Str × Str)) :=
  if offset.val ≤ value.val.length then
    if (IdentifierLibrary.byteArray (value.val.take offset.val)).IsValidUTF8 ∧
        (IdentifierLibrary.byteArray (value.val.drop offset.val)).IsValidUTF8 then
      .ok (some
        (.from (value.val.take offset.val) (by have := value.property; simp),
         .from (value.val.drop offset.val) (by have := value.property; simp; all_goals omega)))
    else .ok none
  else .ok none

@[simp, rust_fun "core::str::traits::{core::cmp::PartialEq<str, str>}::eq"]
def Str.Insts.CoreCmpPartialEqStr.eq (left right : Str) : Result Bool :=
  .ok (decide (left.val = right.val))

@[simp, rust_fun "alloc::string::{core::cmp::PartialEq<alloc::string::String, &'0 str>}::eq"]
def alloc.string.String.Insts.CoreCmpPartialEqShared0Str.eq (left : String) (right : Str) :
    Result Bool := .ok (decide (IdentifierLibrary.stringBytes left = right.val))

@[simp, rust_fun "core::str::{str}::parse"]
def core.str.Str.parse {T E : Type} (dictionary : core.str.traits.FromStr T E)
    (value : Str) : Result (core.result.Result T E) := dictionary.from_str value

@[simp, rust_fun "core::num::{core::str::traits::FromStr<u64, core::num::error::ParseIntError>}::from_str"]
def U64.Insts.CoreStrTraitsFromStrParseIntError.from_str (value : Str) :
    Result (core.result.Result U64 core.num.error.ParseIntError) :=
  .ok (match IdentifierLibrary.parseUnsigned value.val with
    | some number => .Ok number
    | none => .Err .invalid)

@[simp, rust_fun "core::bool::{bool}::then_some"]
def core.bool.Bool.then_some {T : Type} (condition : Bool) (value : T) : Result (Option T) :=
  .ok (if condition then some value else none)

@[simp, rust_fun "core::fmt::num::imp::{core::fmt::Display<u64>}::fmt"]
def core.fmt.num.imp.DisplayU64.fmt (value : U64) (formatter : core.fmt.Formatter) :
    Result (core.result.Result Unit core.fmt.Error × core.fmt.Formatter) :=
  .ok (.Ok (), ⟨formatter.output ++ toString value.val⟩)

@[rust_fun "alloc::string::{alloc::string::ToString<@T>}::to_string"]
def alloc.string.ToString.Blanket.to_string {T : Type}
    (display : core.fmt.Display T) (value : T) : Result String := do
  let (result, formatter) ← display.fmt value ⟨""⟩
  match result with
  | .Ok _ => .ok formatter.output
  | .Err _ => .fail .panic

-- Only the registered default String-backed formatting domain is modeled.
-- The actual String writer appends UTF-8 and normally returns Ok; this does
-- not assert that an arbitrary fmt::Write sink succeeds or that OOM recovers.
@[rust_fun "core::fmt::{core::fmt::Formatter<'a>}::write_str"]
def core.fmt.Formatter.write_str (formatter : core.fmt.Formatter) (value : Str) :
    Result (core.result.Result Unit core.fmt.Error × core.fmt.Formatter) :=
  match IdentifierLibrary.decodeString value.val with
  | some text => .ok (.Ok (), ⟨formatter.output ++ text⟩)
  | none => .fail .panic

end SandproofFoundationsRust
