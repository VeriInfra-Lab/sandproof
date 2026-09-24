-- Hand-written total models for the small Rust standard-library surface used by
-- sandbox-controller-core.  This file intentionally contains definitions, not
-- axioms: the extracted controller must remain kernel-checkable end to end.
import Aeneas
import SandboxControllerCore.Types

open Aeneas Aeneas.Std Result ControlFlow Error

set_option linter.dupNamespace false
set_option linter.hashCommand false
set_option linter.unusedVariables false
set_option linter.style.whitespace false
set_option linter.style.setOption false
set_option linter.style.longLine false
set_option maxHeartbeats 1000000
set_option maxRecDepth 2048

open SandboxControllerRust

@[simp, rust_fun "core::option::{core::option::Option<@T>}::unwrap_or_default"]
def core.option.Option.unwrap_or_default
    {T : Type} (defaultDefaultInst : core.default.Default T) :
    Option T → Result T
  | some value => .ok value
  | none => defaultDefaultInst.default

@[simp, rust_fun "core::option::{core::option::Option<&'0 @T>}::cloned"]
def core.option.OptionShared0T.cloned
    {T : Type} (cloneCloneInst : core.clone.Clone T) :
    Option T → Result (Option T)
  | none => .ok none
  | some value => do
      let cloned ← cloneCloneInst.clone value
      .ok (some cloned)

@[simp, rust_fun
  "core::option::{core::clone::Clone<core::option::Option<@T>>}::clone"]
def core.option.Option.Insts.CoreCloneClone.clone
    {T : Type} (cloneCloneInst : core.clone.Clone T) :
    Option T → Result (Option T) :=
  core.option.OptionShared0T.cloned cloneCloneInst

@[simp, rust_fun
  "core::option::{core::cmp::PartialEq<core::option::Option<@T>, core::option::Option<@T>>}::eq"]
def core.option.Option.Insts.CoreCmpPartialEqOption.eq
    {T : Type} (cmpPartialEqInst : core.cmp.PartialEq T T) :
    Option T → Option T → Result Bool
  | none, none => .ok true
  | some left, some right => cmpPartialEqInst.eq left right
  | _, _ => .ok false

@[simp, rust_fun
  "alloc::string::{core::cmp::PartialEq<alloc::string::String, alloc::string::String>}::eq"]
def alloc.string.String.Insts.CoreCmpPartialEqString.eq
    (left right : String) : Result Bool :=
  .ok (left == right)

@[simp, rust_fun "alloc::string::{core::clone::Clone<alloc::string::String>}::clone"]
def alloc.string.String.Insts.CoreCloneClone.clone
    (value : String) : Result String :=
  .ok value

@[simp, rust_fun
  "alloc::vec::{core::default::Default<alloc::vec::Vec<@T>>}::default"]
def alloc.vec.Vec.Insts.CoreDefaultDefault.default
    (T : Type) : Result (alloc.vec.Vec T) :=
  .ok (alloc.vec.Vec.new T)
