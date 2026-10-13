import SandproofFirstChain.Types
import Aeneas

/-! Reuse only V0's reviewed A6 string model. Option equality is extracted
from the fixed compiler's real standard-library body, not modeled here.
This definition does not establish agreement with the native String implementation.
-/
namespace SandproofFirstChainRust
open Aeneas.Std

-- The generated body reads the built-in Option tag. Fixed Rust assigns None
-- tag 0 and Some tag 1; this is a library representation model, not IO.
instance {T : Type} : Discriminant (Option T) Isize where
  read_discriminant
    | none => 0#isize
    | some _ => 1#isize

@[simp, rust_fun "alloc::string::{core::cmp::PartialEq<alloc::string::String, alloc::string::String>}::eq"]
def alloc.string.String.Insts.CoreCmpPartialEqString.eq
    (left right : String) : Result Bool :=
  .ok (decide (left = right))

end SandproofFirstChainRust
