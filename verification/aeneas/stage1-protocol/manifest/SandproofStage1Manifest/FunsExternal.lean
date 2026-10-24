import SandproofFoundations.FunsExternal
import SandproofStage1Manifest.Types

/-! Exact namespace adapters for the existing fixed A6 string/value models.
These do not establish standard-library correctness or business authorization. -/
open Aeneas Aeneas.Std
namespace SandproofStage1ManifestRust

def alloc.string.String.Insts.CoreCmpPartialEqString.eq
    (left right : String) : Result Bool :=
  SandproofFoundationsRust.alloc.string.String.Insts.CoreCmpPartialEqString.eq left right

def alloc.vec.Vec.is_empty {T : Type} (allocator : Type) (values : alloc.vec.Vec T) :
    Result Bool := SandproofFoundationsRust.alloc.vec.Vec.is_empty allocator values

-- Same kernel-checked literal adapter as the shared library; no native proof oracle.
def toStr (text : String)
    (bounded : text.toByteArray.size ≤ U32.max := by rw [U32.max_eq]; decide) : Str :=
  SandproofFoundationsRust.toStr text bounded

def core.str.Str.len (value : Str) : Result Usize :=
  SandproofFoundationsRust.core.str.Str.len value

def core.str.Str.is_empty (value : Str) : Result Bool :=
  SandproofFoundationsRust.core.str.Str.is_empty value

-- Fixed str::as_bytes preserves the same complete borrowed UTF-8 byte value.
-- Agreement with the real primitive remains the explicit A6 library condition.
def core.str.Str.as_bytes (value : Str) : Result (Slice U8) := .ok value

def Str.Insts.AllocBorrowToOwnedString.to_owned (value : Str) : Result String :=
  SandproofFoundationsRust.Str.Insts.AllocBorrowToOwnedString.to_owned value

def alloc.string.String.Insts.CoreOpsDerefDerefStr.deref (value : String) : Result Str :=
  SandproofFoundationsRust.alloc.string.String.Insts.CoreOpsDerefDerefStr.deref value

end SandproofStage1ManifestRust
