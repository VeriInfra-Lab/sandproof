import SandboxModel.Foundations.AllocatedNumbers
import Init.Data.Nat.ToString

/-! General completeness of the existing canonical allocated-number parser.
The decimal/UTF-8 round trip below is proved from the fixed Lean definitions;
it is not assumed as an extra external contract or established by examples.
The actual Rust entrypoints remain connected through allocated_number_refines.
-/
open Aeneas Aeneas.Std SandproofFoundationsRust
namespace SandboxModel.Foundations
open IdentifierLibrary

private def charByte (c : Char) : U8 :=
  U8.ofNatCore c.val.toUInt8.toNat (by have := c.val.toUInt8.toNat_lt; omega)

private theorem charByte_val (c : Char) (ascii : c.toNat < 128) :
    (charByte c).val = c.toNat := by
  change c.val.toUInt8.toNat = c.toNat
  rw [UInt32.toNat_toUInt8]
  change c.val.toNat % 256 = c.val.toNat
  change c.val.toNat < 128 at ascii
  exact Nat.mod_eq_of_lt (by omega)

private theorem stringBytes_ofList_ascii (chars : List Char)
    (ascii : ∀ c ∈ chars, c.toNat < 128) :
    stringBytes (String.ofList chars) = chars.map charByte := by
  revert ascii
  induction chars with
  | nil => intro ascii; simp [stringBytes]
  | cons c tail ih =>
    intro ascii
    have headAscii := ascii c (by simp)
    have tailAscii : ∀ x ∈ tail, x.toNat < 128 := by
      intro x member
      exact ascii x (by simp [member])
    have oneByte : c.utf8Size = 1 := by
      rw [Char.utf8Size_eq_one_iff, UInt32.le_iff_toNat_le]
      change c.toNat ≤ 127
      omega
    have tailBytes := ih tailAscii
    simp only [stringBytes, String.toByteArray_ofList] at tailBytes ⊢
    rw [List.utf8Encode_cons, List.utf8Encode_singleton,
      String.utf8EncodeChar_eq_singleton oneByte]
    simpa only [ByteArray.toList_data_append, List.toList_data_toByteArray,
      List.map_append, List.map_cons, List.map_nil, charByte, List.cons_append, List.nil_append] using
      congrArg (fun bytes => charByte c :: bytes) tailBytes

private theorem decimalValue_chars (chars : List Char)
    (ascii : ∀ c ∈ chars, c.toNat < 128) :
    decimalValue (chars.map charByte) = Nat.ofDigitChars 10 chars 0 := by
  have fold : ∀ initial,
      (chars.map charByte).foldl (fun acc byte => 10 * acc + (byte.val - 48)) initial =
        chars.foldl (fun acc c => 10 * acc + (c.toNat - 48)) initial := by
    revert ascii
    induction chars with
    | nil => intro ascii; simp
    | cons c tail ih =>
      intro ascii initial
      have headAscii := ascii c (by simp)
      have tailAscii : ∀ x ∈ tail, x.toNat < 128 := by
        intro x member
        exact ascii x (by simp [member])
      simp only [List.map_cons, List.foldl_cons, charByte_val c headAscii]
      exact ih tailAscii _
  simpa only [decimalValue, Nat.ofDigitChars, Char.reduceToNat] using fold 0

private theorem rendered_digits (number : Nat) :
    stringBytes (toString number) = (Nat.toDigits 10 number).map charByte := by
  rw [Nat.toString_eq_ofList_toDigits]
  apply stringBytes_ofList_ascii
  intro c member
  have digit := Nat.isDigit_of_mem_toDigits (by decide : 0 < 10)
    (by decide : 10 ≤ 10) member
  have bounds := Char.isDigit_iff_toNat.mp digit
  simp only [Char.reduceToNat] at bounds
  omega

private theorem rendered_decimal_value (number : Nat) :
    decimalValue (stringBytes (toString number)) = number := by
  rw [rendered_digits, decimalValue_chars]
  · exact Nat.ofDigitChars_ten_toDigits
  · intro c member
    have bounds := Char.isDigit_iff_toNat.mp
      (Nat.isDigit_of_mem_toDigits (by decide : 0 < 10) (by decide : 10 ≤ 10) member)
    simp only [Char.reduceToNat] at bounds
    omega

private theorem rendered_byte_bounds (number : Nat) :
    ∀ byte ∈ stringBytes (toString number), 48 ≤ byte.val ∧ byte.val ≤ 57 := by
  rw [rendered_digits]
  intro byte member
  obtain ⟨c, cMember, equal⟩ := List.mem_map.mp member
  have bounds := Char.isDigit_iff_toNat.mp
    (Nat.isDigit_of_mem_toDigits (by decide : 0 < 10) (by decide : 10 ≤ 10) cMember)
  simp only [Char.reduceToNat] at bounds
  rw [← equal, charByte_val c (by omega)]
  exact bounds

theorem parse_unsigned_rendered_nat (number : Nat) (bounded : number < 2 ^ 64) :
    parseUnsigned (stringBytes (toString number)) = some (U64.ofNatCore number bounded) := by
  have positiveLength : 0 < (stringBytes (toString number)).length := by
    rw [rendered_digits, List.length_map]
    exact Nat.length_toDigits_pos
  have notPlus : (stringBytes (toString number)).head?.map UScalar.val ≠ some 43 := by
    cases first : (stringBytes (toString number)).head? with
    | none => simp
    | some byte =>
      have bounds := rendered_byte_bounds number byte (List.mem_of_head? first)
      simp only [Option.map_some]
      intro equal
      have valueEqual := Option.some.inj equal
      omega
  have allDigits : (stringBytes (toString number)).all
      (fun byte => decide (48 ≤ byte.val ∧ byte.val ≤ 57)) = true := by
    apply List.all_eq_true.mpr
    intro byte member
    simpa only [decide_eq_true_eq] using rendered_byte_bounds number byte member
  unfold parseUnsigned
  rw [if_neg notPlus]
  simp only []
  rw [if_neg (Nat.ne_of_gt positiveLength), allDigits]
  simp only [↓reduceIte]
  rw [rendered_decimal_value]
  rw [dif_pos bounded]

theorem parse_unsigned_rendered_u64 (number : U64) :
    parseUnsigned (stringBytes (toString number.val)) = some number := by
  have bounded : number.val < 2 ^ 64 := by scalar_tac
  rw [parse_unsigned_rendered_nat number.val bounded]
  congr 1

theorem allocated_number_model_complete (stem : List U8) (number : U64) (highwater : Nat)
    (stemUtf8 : (byteArray stem).IsValidUTF8)
    (positive : 0 < number.val) (capacity : number.val ≤ highwater) :
    allocatedNumberModel (stem ++ stringBytes (toString number.val)) stem highwater =
      some number := by
  apply (allocated_model_success_iff _ _ _ _).mpr
  have suffixUtf8 : (byteArray (stringBytes (toString number.val))).IsValidUTF8 := by
    rw [byteArray_stringBytes]
    exact (toString number.val).isValidUTF8
  simp only [List.length_append, List.take_left, List.drop_left]
  exact ⟨by omega, stemUtf8, suffixUtf8, True.intro,
    parse_unsigned_rendered_u64 number, positive, capacity, True.intro⟩

theorem allocated_number_complete (value stem : Str) (number highwater : U64)
    (stemUtf8 : (byteArray stem.val).IsValidUTF8)
    (canonical : value.val = stem.val ++ stringBytes (toString number.val))
    (positive : 0 < number.val) (capacity : number.val ≤ highwater.val) :
    allocated_number value stem highwater = .ok (some number) := by
  rw [allocated_number_refines, canonical,
    allocated_number_model_complete stem.val number highwater.val stemUtf8 positive capacity]

theorem center_allocated_number_complete (value stem : Str) (number highwater : U64)
    (stemUtf8 : (byteArray stem.val).IsValidUTF8)
    (canonical : value.val = stem.val ++ stringBytes (toString number.val))
    (positive : 0 < number.val) (capacity : number.val ≤ highwater.val) :
    center_allocated_number value stem highwater = .ok (some number) := by
  exact allocated_number_complete value stem number highwater stemUtf8 canonical positive capacity

private theorem stringBytes_append (first second : String) :
    stringBytes (first ++ second) = stringBytes first ++ stringBytes second := by
  simp only [stringBytes, String.toByteArray_append, ByteArray.toList_data_append, List.map_append]

theorem allocated_number_text_complete (value stem : Str) (stemText : String)
    (number highwater : U64) (stemRep : StrTextRep stem stemText)
    (valueRep : StrTextRep value (stemText ++ toString number.val))
    (positive : 0 < number.val) (capacity : number.val ≤ highwater.val) :
    allocated_number value stem highwater = .ok (some number) := by
  have stemUtf8 : (byteArray stem.val).IsValidUTF8 := by
    rw [stemRep, byteArray_stringBytes]
    exact stemText.isValidUTF8
  have canonical : value.val = stem.val ++ stringBytes (toString number.val) := by
    rw [valueRep, stringBytes_append, stemRep]
  exact allocated_number_complete value stem number highwater stemUtf8 canonical positive capacity

theorem center_allocated_number_text_complete (value stem : Str) (stemText : String)
    (number highwater : U64) (stemRep : StrTextRep stem stemText)
    (valueRep : StrTextRep value (stemText ++ toString number.val))
    (positive : 0 < number.val) (capacity : number.val ≤ highwater.val) :
    center_allocated_number value stem highwater = .ok (some number) := by
  exact allocated_number_text_complete value stem stemText number highwater stemRep valueRep
    positive capacity

end SandboxModel.Foundations
