import SandproofFoundations.TypesExternal

/-! Fixed minimal string/decimal observations. Agreement with the reviewed
Rust libraries is an explicit dependency, not a proof of those libraries. -/
open Aeneas Aeneas.Std
namespace SandproofFoundationsRust.IdentifierLibrary

def stringBytes (value : String) : List U8 :=
  value.toByteArray.data.toList.map (fun byte =>
    U8.ofNatCore byte.toNat (by cases byte; simp; omega))

def StringRepresentable (value : String) : Prop :=
  (stringBytes value).length ≤ Usize.max
instance (value : String) : Decidable (StringRepresentable value) :=
  inferInstanceAs (Decidable (_ ≤ _))

def byteArray (bytes : List U8) : ByteArray :=
  ⟨(bytes.map (fun byte => UInt8.ofNat byte.val)).toArray⟩

def decodeString (bytes : List U8) : Option String :=
  String.fromUTF8? (byteArray bytes)

def decimalValue (bytes : List U8) : Nat :=
  bytes.foldl (fun value byte => 10 * value + (byte.val - 48)) 0

-- Rust's u64 parser accepts a leading '+', and leading zeroes. The project
-- separately checks the original frontier and THEN canonical rendering.
def parseUnsigned (bytes : List U8) : Option U64 :=
  let digits := if bytes.head?.map UScalar.val = some 43 then bytes.tail else bytes
  if digits.length = 0 then none
  else if digits.all (fun byte => decide (48 ≤ byte.val ∧ byte.val ≤ 57)) then
    let value := decimalValue digits
    if h : value < 2 ^ 64 then some (U64.ofNatCore value h) else none
  else none

def allBytes {F : Type} (call : F → U8 → Result (Bool × F))
    (state : F) : List U8 → Result (Bool × List U8)
  | [] => .ok (true, [])
  | byte :: rest => do
      let (keep, next) ← call state byte
      if keep then allBytes call next rest else .ok (false, rest)

end SandproofFoundationsRust.IdentifierLibrary
